//! A DNS-over-HTTPS resolver stand-in [RFC 8484], on a free local port over plain HTTP: it answers
//! `GET /dns-query?dns=<base64url>` and `POST /dns-query` with `application/dns-message`, both in
//! DNS wire format, with the TXT records the tests set for a name and their TTL. A name it has no
//! records for answers NXDOMAIN. It keeps each query it got. The tests make it fail: answer every
//! request with an HTTP status, answer SERVFAIL, or drop each connection unanswered, as when it
//! cannot be reached; its address stays the same either way.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

// Other helpers of the same test binary load it too.
#[path = "../common/local_listener.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod local_listener;
use local_listener::local_listener;

/// The DNS type of TXT records, and the class IN.
pub const TXT: u16 = 16;
const IN: u16 = 1;

/// How the resolver fails.
#[derive(Debug, Clone, Copy)]
pub enum Fault {
    /// Every request gets this HTTP status and no body.
    Status(u16),
    /// Every query gets SERVFAIL.
    ServFail,
    /// Every connection is dropped unanswered.
    Unreachable,
}

/// A query it got.
#[derive(Debug, Clone)]
pub struct Query {
    /// `GET` or `POST`.
    pub method: String,
    /// The name asked for, in lowercase, without a trailing dot.
    pub name: String,
    pub qtype: u16,
}

#[derive(Default)]
struct State {
    /// TXT records by name, in lowercase: their TTL in seconds and their texts.
    records: HashMap<String, (u32, Vec<String>)>,
    fault: Option<Fault>,
    queries: Vec<Query>,
}

/// The resolver, serving until the tests end.
pub struct Doh {
    port: u16,
    state: Arc<Mutex<State>>,
}

impl Doh {
    pub fn start() -> Self {
        let listener = local_listener();
        let port = listener
            .local_addr()
            .expect("the resolver's address")
            .port();
        let state = Arc::new(Mutex::new(State::default()));
        let serving = Arc::clone(&state);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let state = Arc::clone(&serving);
                thread::spawn(move || serve(client, &state));
            }
        });
        Doh { port, state }
    }

    /// Its address, such as `127.0.0.1:41234`.
    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Its RFC 8484 URL: `http://127.0.0.1:<port>/dns-query`.
    pub fn url(&self) -> String {
        format!("http://{}/dns-query", self.address())
    }

    /// From now on, `name` has the TXT records `texts`, each one record, with the TTL `ttl`.
    pub fn set_txt(&self, name: &str, ttl: u32, texts: &[String]) {
        let mut state = self.state.lock().unwrap();
        (state.records).insert(name.to_lowercase(), (ttl, texts.to_vec()));
    }

    /// From now on, it fails as `fault` says.
    pub fn fail(&self, fault: Fault) {
        self.state.lock().unwrap().fault = Some(fault);
    }

    /// From now on, it answers again.
    pub fn recover(&self) {
        self.state.lock().unwrap().fault = None;
    }

    /// The queries it got so far, in order.
    pub fn queries(&self) -> Vec<Query> {
        self.state.lock().unwrap().queries.clone()
    }

    /// The queries it got so far for TXT records of `name`.
    pub fn txt_queries(&self, name: &str) -> usize {
        let name = name.to_lowercase();
        (self.queries().iter())
            .filter(|query| query.qtype == TXT && query.name == name)
            .count()
    }
}

/// Serves the requests of one connection until it closes; while it is unreachable, drops the
/// connection unanswered, also one it took before.
fn serve(client: TcpStream, state: &Mutex<State>) {
    let unreachable = || matches!(state.lock().unwrap().fault, Some(Fault::Unreachable));
    let Ok(mut writer) = client.try_clone() else {
        return;
    };
    if unreachable() {
        let _ = writer.shutdown(Shutdown::Both);
        return;
    }
    let mut reader = BufReader::new(client);
    while let Some((line, body)) = read_request(&mut reader) {
        if unreachable() {
            let _ = writer.shutdown(Shutdown::Both);
            return;
        }
        let response = respond(&line, &body, state);
        if writer.write_all(&response).is_err() {
            return;
        }
    }
}

/// The request line and body of the next request on `reader`, or `None` once it closes.
fn read_request(reader: &mut BufReader<TcpStream>) -> Option<(String, Vec<u8>)> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut head = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        match header.trim_end() {
            "" => break,
            header => head.push(header.to_owned()),
        }
    }
    let length = match (head.iter())
        .filter_map(|header| header.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
    {
        Some((_, value)) => value.trim().parse().ok()?,
        None => 0,
    };
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some((line.trim_end().to_owned(), body))
}

/// The HTTP response to the request with the line `line` and the body `body`.
fn respond(line: &str, body: &[u8], state: &Mutex<State>) -> Vec<u8> {
    let fault = state.lock().unwrap().fault;
    if let Some(Fault::Status(status)) = fault {
        return http(status, &[]);
    }
    let mut parts = line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (path, params) = target.split_once('?').unwrap_or((target, ""));
    if path != "/dns-query" {
        return http(404, &[]);
    }
    let message = match method {
        "GET" => params
            .split('&')
            .find_map(|param| param.strip_prefix("dns="))
            .and_then(|dns| URL_SAFE_NO_PAD.decode(dns.trim_end_matches('=')).ok()),
        "POST" => Some(body.to_vec()),
        _ => None,
    };
    let Some(question) = message.as_deref().and_then(parse_query) else {
        return http(400, &[]);
    };
    let mut state = state.lock().unwrap();
    state.queries.push(Query {
        method: method.to_owned(),
        name: question.name.clone(),
        qtype: question.qtype,
    });
    let answer = match fault {
        Some(Fault::ServFail) => answer(&question, 2, None),
        _ => match state.records.get(&question.name) {
            None => answer(&question, 3, None),
            Some(_) if question.qtype != TXT => answer(&question, 0, None),
            Some(records) => answer(&question, 0, Some(records)),
        },
    };
    http(200, &answer)
}

/// An HTTP response with `status` and, if any, the DNS message `body`.
fn http(status: u16, body: &[u8]) -> Vec<u8> {
    let mut head = format!(
        "HTTP/1.1 {status} Stand-in\r\ncontent-length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        head.push_str("content-type: application/dns-message\r\n");
    }
    head.push_str("\r\n");
    [head.into_bytes(), body.to_vec()].concat()
}

/// The question of a DNS query.
struct Question {
    id: [u8; 2],
    /// The query's recursion desired bit, in place.
    rd: u8,
    /// The question section as it came.
    raw: Vec<u8>,
    name: String,
    qtype: u16,
}

/// The first question of the DNS query `message`, if it is one.
fn parse_query(message: &[u8]) -> Option<Question> {
    if message.len() < 12
        || message[2] & 0x80 != 0
        || u16::from_be_bytes([message[4], message[5]]) == 0
    {
        return None;
    }
    let mut at = 12;
    let mut labels = Vec::new();
    loop {
        let length = usize::from(*message.get(at)?);
        at += 1;
        if length == 0 {
            break;
        }
        if length > 63 {
            return None;
        }
        let label = message.get(at..at + length)?;
        labels.push(String::from_utf8_lossy(label).to_lowercase());
        at += length;
    }
    let fixed = message.get(at..at + 4)?;
    let qtype = u16::from_be_bytes([fixed[0], fixed[1]]);
    Some(Question {
        id: [message[0], message[1]],
        rd: message[2] & 0x01,
        raw: message[12..at + 4].to_vec(),
        name: labels.join("."),
        qtype,
    })
}

/// The DNS response to `question` with the response code `rcode` and, if any, the TXT records
/// `records`, their TTL and texts.
fn answer(question: &Question, rcode: u8, records: Option<&(u32, Vec<String>)>) -> Vec<u8> {
    let texts: &[String] = records.map_or(&[], |(_, texts)| texts);
    let ttl = records.map_or(0, |(ttl, _)| *ttl);
    let count = u16::try_from(texts.len()).expect("a few records");
    let mut out = Vec::new();
    out.extend(question.id);
    out.push(0x80 | question.rd);
    out.push(0x80 | rcode);
    out.extend(1u16.to_be_bytes());
    out.extend(count.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(&question.raw);
    for text in texts {
        let mut data = Vec::new();
        for chunk in text.as_bytes().chunks(255) {
            data.push(u8::try_from(chunk.len()).expect("a chunk of 255 bytes at most"));
            data.extend(chunk);
        }
        // The name: a pointer to the question's.
        out.extend([0xc0, 0x0c]);
        out.extend(TXT.to_be_bytes());
        out.extend(IN.to_be_bytes());
        out.extend(ttl.to_be_bytes());
        out.extend(
            u16::try_from(data.len())
                .expect("a short record")
                .to_be_bytes(),
        );
        out.extend(data);
    }
    out
}
