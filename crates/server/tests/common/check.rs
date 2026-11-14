//! A check script of a server's sign-in, run against a local `skald-server` behind a front that can
//! change the server's answers as a broken or misconfigured server would.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{self, AssertUnwindSafe};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use url::Url;

use super::device_flow::{DEVICE_AUTHORIZATION, json as json_of};
use super::sign_in::with_param;
use super::{Response, get_url, send};

/// The variables that would send a check's requests through a proxy.
const PROXIES: [&str; 6] = [
    "http_proxy",
    "HTTP_PROXY",
    "https_proxy",
    "HTTPS_PROXY",
    "all_proxy",
    "ALL_PROXY",
];

/// A run of a check: how it exited and everything it printed, stdout and stderr as they came.
pub struct Run {
    pub case: String,
    pub status: ExitStatus,
    pub output: String,
}

/// Runs `script` with `args` and the environment variables `envs`, no proxy among them; fails if it
/// does not end within 90 seconds.
pub fn run_script(case: &str, script: &str, args: &[&str], envs: &[(&str, &str)]) -> Run {
    let (mut reader, writer) = io::pipe().expect("a pipe");
    let mut command = Command::new(script);
    command
        .args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(writer.try_clone().expect("clone a pipe"))
        .stderr(writer);
    for proxy in PROXIES {
        command.env_remove(proxy);
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|err| panic!("run {script}: {err}"));
    drop(command);
    let read = thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output).expect("read the output");
        String::from_utf8_lossy(&output).into_owned()
    });
    let within = Duration::from_secs(90);
    let deadline = Instant::now() + within;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for the check") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{case}: the check did not end within {within:?}");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let output = read.join().expect("read the output");
    Run {
        case: case.to_owned(),
        status,
        output,
    }
}

/// Checks that `run` printed none of `codes`, in any case, with or without its hyphens.
pub fn assert_no_codes(run: &Run, codes: &[String]) {
    let output = run.output.to_lowercase();
    for code in codes {
        for form in [code.to_lowercase(), code.replace('-', "").to_lowercase()] {
            assert!(
                !output.contains(&form),
                "{}: the check printed a code it got\n{}",
                run.case,
                run.output
            );
        }
    }
}

/// A server's answer, as the front gives it to the check.
pub type Answer = Response;

impl Answer {
    /// An answer of `status` with the body `body`, `content_type`.
    pub fn made(status: u16, content_type: &str, body: &str) -> Self {
        Answer {
            status,
            content_type: content_type.to_owned(),
            headers: vec![("cache-control".to_owned(), "no-store".to_owned())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn of_json(status: u16, body: &Value) -> Self {
        Self::made(status, "application/json", &body.to_string())
    }

    pub fn problem(status: u16) -> Self {
        let body = json!({ "type": "about:blank", "title": "The server failed", "status": status });
        Self::made(status, "application/problem+json", &body.to_string())
    }

    /// Sets the header `name` to `value`.
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.retain(|(header, _)| header != name);
        self.headers.push((name.to_owned(), value.to_owned()));
    }
}

/// Gives the answer the check gets instead of the server's answer to `<method> <path>`, the path
/// without its query.
pub type Tamper = Arc<dyn Fn(&str, &str, Answer) -> Answer + Send + Sync>;

/// A tamper that changes nothing.
pub fn unchanged() -> Tamper {
    Arc::new(|_, _, answer| answer)
}

/// A tamper that changes only the answers to `<method> <path>`, by `change`.
pub fn on(
    method: &'static str,
    path: &'static str,
    change: impl Fn(Answer) -> Answer + Send + Sync + 'static,
) -> Tamper {
    Arc::new(move |asked, at, answer| {
        if asked == method && at == path {
            change(answer)
        } else {
            answer
        }
    })
}

/// A tamper that answers `<method> <path>` with `answer` instead.
pub fn answer_with(method: &'static str, path: &'static str, answer: Answer) -> Tamper {
    on(method, path, move |_| answer.clone())
}

/// A request the front forwarded and the server's answer to it, before the tamper.
#[derive(Clone)]
pub struct Exchange {
    pub method: String,
    /// The path, without its query.
    pub path: String,
    pub answer: Answer,
}

/// A local HTTP front for the server: forwards each request and gives the check the answer its
/// tamper makes of the server's. It keeps the server's answers, and the messages of the tamper's
/// panics, for which it answers `500`.
pub struct Front {
    pub url: String,
    listener: Option<TcpListener>,
    tamper: Arc<Mutex<Tamper>>,
    exchanges: Arc<Mutex<Vec<Exchange>>>,
    panics: Arc<Mutex<Vec<String>>>,
}

impl Front {
    /// A front on a free local port, not yet forwarding.
    pub fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        Front {
            url,
            listener: Some(listener),
            tamper: Arc::new(Mutex::new(unchanged())),
            exchanges: Arc::default(),
            panics: Arc::default(),
        }
    }

    /// Forwards every request to the server at `base`, such as `http://127.0.0.1:41234`.
    pub fn serve(&mut self, base: &str) {
        let listener = self.listener.take().expect("the front serves once");
        let base = base.to_owned();
        let tamper = Arc::clone(&self.tamper);
        let exchanges = Arc::clone(&self.exchanges);
        let panics = Arc::clone(&self.panics);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let base = base.clone();
                let (tamper, exchanges) = (Arc::clone(&tamper), Arc::clone(&exchanges));
                let panics = Arc::clone(&panics);
                thread::spawn(move || {
                    let _ = relay(stream, &base, &tamper, &exchanges, &panics);
                });
            }
        });
    }

    pub fn set_tamper(&self, tamper: Tamper) {
        *self.tamper.lock().expect("the front's tamper") = tamper;
    }

    /// The requests forwarded so far and the server's answers, in order.
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.exchanges
            .lock()
            .expect("the front's exchanges")
            .clone()
    }

    /// Fails if the tamper panicked: a test's precondition failed there.
    #[track_caller]
    pub fn assert_tamper_held(&self) {
        let panics = self.panics.lock().expect("the front's panics");
        assert!(panics.is_empty(), "the tamper failed: {panics:?}");
    }

    /// The device and user codes the server's device authorizations gave.
    pub fn codes(&self) -> Vec<String> {
        let mut codes = Vec::new();
        for exchange in self.exchanges() {
            let answer = &exchange.answer;
            if exchange.method != "POST"
                || exchange.path != DEVICE_AUTHORIZATION
                || answer.status != 200
            {
                continue;
            }
            let grant = json_of(answer);
            for name in ["device_code", "user_code"] {
                if let Some(code) = grant[name].as_str() {
                    codes.push(code.to_owned());
                }
            }
        }
        codes
    }
}

/// Forwards the one request on `stream` to `base` and answers it as `tamper` says.
fn relay(
    mut stream: TcpStream,
    base: &str,
    tamper: &Mutex<Tamper>,
    exchanges: &Mutex<Vec<Exchange>>,
    panics: &Mutex<Vec<String>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(());
    };
    let (method, target) = (method.to_owned(), target.to_owned());
    let mut headers = Vec::new();
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_owned());
        if name == "content-length" {
            length = value.parse().unwrap_or(0);
        }
        let hop = [
            "host",
            "content-length",
            "connection",
            "expect",
            "transfer-encoding",
        ];
        if !hop.contains(&name.as_str()) {
            headers.push((name, value));
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;

    let url = format!("{base}{target}");
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let path = target.split('?').next().unwrap_or_default();
    let answer = match method.as_str() {
        "GET" => get_url(&url, &headers),
        "POST" => send("post", &url, &headers, &body),
        _ => Answer::problem(405),
    };
    exchanges
        .lock()
        .expect("the front's exchanges")
        .push(Exchange {
            method: method.clone(),
            path: path.to_owned(),
            answer: answer.clone(),
        });
    let tamper = Arc::clone(&*tamper.lock().expect("the front's tamper"));
    let answer = panic::catch_unwind(AssertUnwindSafe(|| tamper(&method, path, answer)))
        .unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                .unwrap_or_default();
            panics.lock().expect("the front's panics").push(message);
            Answer::problem(500)
        });

    let mut head = format!("HTTP/1.1 {} Answer\r\n", answer.status);
    if !answer.content_type.is_empty() {
        head.push_str(&format!("content-type: {}\r\n", answer.content_type));
    }
    for (name, value) in &answer.headers {
        if !["content-length", "transfer-encoding", "connection"].contains(&name.as_str()) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str(&format!(
        "content-length: {}\r\nconnection: close\r\n\r\n",
        answer.body.len()
    ));
    stream.write_all(head.as_bytes())?;
    stream.write_all(&answer.body)?;
    stream.flush()
}

/// A host no address resolves to.
pub const ELSEWHERE: &str = "elsewhere.test";

/// A tamper that changes the address the sign-in start `start` sends the browser to, by `change`.
pub fn to_provider(
    start: &'static str,
    change: impl Fn(&mut Url) + Send + Sync + 'static,
) -> Tamper {
    on("GET", start, move |mut answer| {
        let location = answer.header("location").unwrap_or_else(|| {
            let body = String::from_utf8_lossy(&answer.body);
            panic!("the sign-in start {start} sends the browser on: {body}")
        });
        let mut url = Url::parse(location).expect("a URL");
        change(&mut url);
        answer.set_header("location", url.as_str());
        answer
    })
}

/// Sets the query parameter `name` of `url` to `value`, or removes it if `value` is `None`.
pub fn set_param(url: &mut Url, name: &str, value: Option<&str>) {
    *url = with_param(url.as_str(), name, value)
        .parse()
        .expect("a URL");
}

/// Callbacks off the server at `address`, a front's `http://127.0.0.1:<port>`, each with what makes
/// it so: `callback`, a path, on another host, port or scheme, or another host after the address.
pub fn callbacks_off(address: &str, callback: &str) -> Vec<(&'static str, String)> {
    let front = Url::parse(address).expect("a URL");
    let port = front.port().expect("a port");
    let other_port = if port == u16::MAX { port - 1 } else { port + 1 };
    vec![
        (
            "a callback on another host",
            format!("http://{ELSEWHERE}:{port}{callback}"),
        ),
        (
            "a callback on another port",
            format!("http://127.0.0.1:{other_port}{callback}"),
        ),
        (
            "a callback over another scheme",
            format!("https://127.0.0.1:{port}{callback}"),
        ),
        (
            "a callback on another host after the server's address",
            format!("http://127.0.0.1:{port}@{ELSEWHERE}{callback}"),
        ),
    ]
}
