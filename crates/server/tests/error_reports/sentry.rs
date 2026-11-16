//! A stand-in for Sentry's ingestion endpoint: it takes every HTTP request on a local port, answers
//! `200`, and keeps what came, its body decoded from its `content-encoding`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

/// A request the stand-in got.
#[derive(Debug, Clone)]
pub struct Report {
    pub method: String,
    /// The request target: the path and the query.
    pub target: String,
    /// Lowercase names and their values.
    pub headers: Vec<(String, String)>,
    /// The body, decoded from its `content-encoding`.
    pub body: Vec<u8>,
    /// The `content-encoding` the stand-in could not decode, if any; `body` is then as it came.
    pub unread: Option<String>,
}

impl Report {
    /// Whether it goes to the project `project` of the DSN whose public key is `key`: its path is
    /// under `/api/<project>/`, and it names `key` in `X-Sentry-Auth`, in its query or in the DSN
    /// of its envelope's header.
    pub fn is_for(&self, key: &str, project: u32) -> bool {
        let path = self.target.split('?').next().unwrap_or_default();
        if !path.starts_with(&format!("/api/{project}/")) {
            return false;
        }
        let named = format!("sentry_key={key}");
        let in_auth = self.headers.iter().any(|(name, auth)| {
            name == "x-sentry-auth" && auth.split([',', ' ']).any(|part| part == named)
        });
        let in_query = self
            .target
            .split_once('?')
            .is_some_and(|(_, query)| query.split('&').any(|pair| pair == named));
        let in_envelope = self
            .envelope_header()
            .and_then(|header| header["dsn"].as_str().map(str::to_owned))
            .is_some_and(|dsn| dsn.contains(&format!("://{key}@")));
        in_auth || in_query || in_envelope
    }

    /// The header of its envelope, if it is a request to the envelope endpoint.
    fn envelope_header(&self) -> Option<Value> {
        if !self.target.split('?').next()?.ends_with("/envelope/") {
            return None;
        }
        let first = self.body.split(|&b| b == b'\n').next()?;
        serde_json::from_slice(first).ok()
    }

    /// The events it holds: the body of a request to the store endpoint, or the `event` items of
    /// an envelope.
    pub fn events(&self) -> Vec<Value> {
        let path = self.target.split('?').next().unwrap_or_default();
        if path.ends_with("/store/") {
            return serde_json::from_slice(&self.body).into_iter().collect();
        }
        if !path.ends_with("/envelope/") {
            return Vec::new();
        }
        let mut events = Vec::new();
        let mut rest = &self.body[..];
        // The envelope's header.
        let Some(end) = rest.iter().position(|&b| b == b'\n') else {
            return events;
        };
        rest = &rest[end + 1..];
        while !rest.is_empty() {
            let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
            let Ok(header) = serde_json::from_slice::<Value>(&rest[..end]) else {
                return events;
            };
            rest = &rest[(end + 1).min(rest.len())..];
            let payload = match header["length"].as_u64() {
                Some(length) => {
                    let length = usize::try_from(length)
                        .unwrap_or(usize::MAX)
                        .min(rest.len());
                    let payload = &rest[..length];
                    rest = &rest[length..];
                    if rest.first() == Some(&b'\n') {
                        rest = &rest[1..];
                    }
                    payload
                }
                None => {
                    let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
                    let payload = &rest[..end];
                    rest = &rest[(end + 1).min(rest.len())..];
                    payload
                }
            };
            if header["type"] == "event"
                && let Ok(event) = serde_json::from_slice(payload)
            {
                events.push(event);
            }
        }
        events
    }

    /// Everything it holds, as bytes: its target, its headers and its body.
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = format!("{} {}\n", self.method, self.target).into_bytes();
        for (name, value) in &self.headers {
            bytes.extend_from_slice(format!("{name}: {value}\n").as_bytes());
        }
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

/// The stand-in, listening until the test binary ends.
pub struct Sentry {
    port: u16,
    reports: Arc<Mutex<Vec<Report>>>,
}

impl Sentry {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("local address").port();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&reports);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let kept = Arc::clone(&kept);
                thread::spawn(move || serve(stream, &kept));
            }
        });
        Sentry { port, reports }
    }

    /// The DSN of its project `project` with the public key `key`.
    pub fn dsn(&self, key: &str, project: u32) -> String {
        format!("http://{key}@127.0.0.1:{}/{project}", self.port)
    }

    /// Every request it got so far.
    pub fn reports(&self) -> Vec<Report> {
        self.reports.lock().expect("the reports").clone()
    }

    /// Waits up to `within` until `found` holds for the requests it got; gives them.
    #[track_caller]
    pub fn wait_until(&self, within: Duration, found: impl Fn(&[Report]) -> bool) -> Vec<Report> {
        let deadline = Instant::now() + within;
        loop {
            let reports = self.reports();
            if found(&reports) {
                return reports;
            }
            assert!(
                Instant::now() < deadline,
                "not reported within {within:?}; the stand-in got {} requests: {:?}",
                reports.len(),
                reports
                    .iter()
                    .map(|report| format!("{} {}", report.method, report.target))
                    .collect::<Vec<_>>()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Serves the requests of `stream`, keeping each in `reports`.
fn serve(stream: TcpStream, reports: &Mutex<Vec<Report>>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    while let Some(report) = read_request(&mut reader, &mut writer) {
        reports.lock().expect("the reports").push(report);
        let answer =
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
        if writer.write_all(answer.as_bytes()).is_err() {
            return;
        }
    }
}

/// The next request on `reader`; `None` once the connection ends or breaks HTTP/1.1.
fn read_request(reader: &mut BufReader<TcpStream>, writer: &mut TcpStream) -> Option<Report> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_lowercase(), value.trim().to_owned()));
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.to_lowercase())
    };
    if header("expect").as_deref() == Some("100-continue") {
        writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").ok()?;
    }
    let raw = if header("transfer-encoding").is_some_and(|coding| coding.contains("chunked")) {
        read_chunked(reader)?
    } else {
        let length: usize =
            header("content-length").map_or(Some(0), |length| length.parse().ok())?;
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        body
    };
    let (body, unread) = decode(raw, header("content-encoding").as_deref());
    Some(Report {
        method,
        target,
        headers,
        body,
        unread,
    })
}

/// A chunked body.
fn read_chunked(reader: &mut BufReader<TcpStream>) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let size = line.trim().split(';').next()?;
        let size = usize::from_str_radix(size, 16).ok()?;
        if size == 0 {
            // The trailer, up to an empty line.
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).ok()? == 0 || line.trim().is_empty() {
                    return Some(body);
                }
            }
        }
        let mut chunk = vec![0; size + 2];
        reader.read_exact(&mut chunk).ok()?;
        body.extend_from_slice(&chunk[..size]);
    }
}

/// `raw` decoded from `encoding`; with an encoding it cannot decode, `raw` and that encoding.
fn decode(raw: Vec<u8>, encoding: Option<&str>) -> (Vec<u8>, Option<String>) {
    let mut decoded = Vec::new();
    let read = match encoding.map(str::trim) {
        None | Some("" | "identity") => return (raw, None),
        Some("gzip" | "x-gzip") => flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut decoded),
        Some("deflate") => flate2::read::ZlibDecoder::new(&raw[..]).read_to_end(&mut decoded),
        Some(other) => return (raw, Some(other.to_owned())),
    };
    match read {
        Ok(_) => (decoded, None),
        Err(err) => (
            raw,
            Some(format!("{} ({err})", encoding.unwrap_or_default())),
        ),
    }
}
