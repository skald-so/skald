//! A recording HTTP proxy of the kind `HTTPS_PROXY` names, on a free local port. It answers each
//! `CONNECT <host>:<port>` with a tunnel to that port on this machine, as if it resolved every host
//! there, `skald.test` too, which no resolver of the machines knows; or, when it refuses tunnels, with
//! the answer the test gives. It keeps the head of each request and every byte the client sent
//! through each tunnel.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// Its answers that refuse a tunnel: forbidden, credentials asked for by Basic, and an upstream it
/// cannot reach.
pub const FORBIDDEN: &str =
    "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
pub const BASIC_ASKED: &str = "HTTP/1.1 407 Proxy Authentication Required\r\n\
    proxy-authenticate: Basic realm=\"corp\"\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
pub const BAD_GATEWAY: &str =
    "HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
/// Its answer asking for credentials by every scheme but Basic.
pub const OTHER_SCHEMES_ASKED: &str = "HTTP/1.1 407 Proxy Authentication Required\r\n\
    proxy-authenticate: NTLM\r\nproxy-authenticate: Negotiate\r\n\
    proxy-authenticate: Digest realm=\"corp\", qop=\"auth\", nonce=\"5f1c9a7d2e4b\"\r\n\
    content-length: 0\r\nconnection: close\r\n\r\n";

/// A request the proxy got.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// Its head as it came, lines and all.
    pub head: String,
    /// What the client sent through its tunnel, if it got one.
    pub tunneled: Vec<u8>,
}

impl Request {
    /// Its request line, such as `CONNECT skald.test:41234 HTTP/1.1`.
    pub fn line(&self) -> &str {
        self.head.lines().next().unwrap_or_default()
    }

    /// The values of its header `name`, in any case, in order.
    pub fn headers(&self, name: &str) -> Vec<&str> {
        self.head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .filter(|(field, _)| field.trim().eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
            .collect()
    }
}

pub struct Proxy {
    pub port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Proxy {
    /// A proxy that tunnels every `CONNECT`.
    pub fn tunneling() -> Self {
        Self::start(None)
    }

    /// A proxy that answers every request with `answer`, a whole HTTP response, and closes.
    pub fn refusing(answer: &'static str) -> Self {
        Self::start(Some(answer))
    }

    fn start(refusal: Option<&'static str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("the proxy's address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let recorded = Arc::clone(&recorded);
                thread::spawn(move || serve(client, refusal, &recorded));
            }
        });
        Proxy { port, requests }
    }

    /// Its URL, for `HTTPS_PROXY`.
    pub fn url(&self) -> String {
        format!("http://{}", self.address())
    }

    /// Its URL with the credentials `userinfo`, such as `user:password`, as they go in a URL.
    pub fn url_as(&self, userinfo: &str) -> String {
        format!("http://{userinfo}@{}", self.address())
    }

    /// Its host and port: `127.0.0.1:<port>`.
    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// The requests it got so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    /// Fails unless it got a `CONNECT` to `host` at `port`.
    #[track_caller]
    pub fn assert_tunneled(&self, host: &str, port: u16) {
        let connect = format!("CONNECT {host}:{port} ");
        let tunnels = self.requests();
        assert!(
            tunnels
                .iter()
                .any(|request| request.line().starts_with(&connect)),
            "no {connect}through the proxy: {tunnels:?}"
        );
    }
}

/// A free local port that nothing listens on: a proxy that is down.
pub fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    listener.local_addr().expect("a local address").port()
}

/// Takes one client's request: refuses it with `refusal` if given, otherwise tunnels a `CONNECT`
/// and keeps what the client sends through it as it comes.
fn serve(mut client: TcpStream, refusal: Option<&str>, requests: &Arc<Mutex<Vec<Request>>>) {
    let Some(head) = read_head(&mut client) else {
        return;
    };
    let mut words = head.split_whitespace();
    let port = match (words.next(), words.next()) {
        (Some("CONNECT"), Some(authority)) => authority.rsplit_once(':').map(|(_, port)| port),
        _ => None,
    }
    .map(str::to_owned);
    let index = {
        let mut requests = requests.lock().unwrap();
        requests.push(Request {
            head,
            tunneled: Vec::new(),
        });
        requests.len() - 1
    };
    let upstream = port
        .filter(|_| refusal.is_none())
        .and_then(|port| super::common::reach("proxy", &format!("127.0.0.1:{port}")).ok());
    let Some(mut upstream) = upstream else {
        let _ = client.write_all(refusal.unwrap_or(BAD_GATEWAY).as_bytes());
        return;
    };
    let established = client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
    let (Ok(()), Ok(from_client), Ok(to_upstream)) =
        (established, client.try_clone(), upstream.try_clone())
    else {
        return;
    };
    let kept = Arc::clone(requests);
    thread::spawn(move || keep_sent(from_client, to_upstream, &kept, index));
    let _ = io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(Shutdown::Write);
}

/// Forwards what `client` sends to `upstream` until it closes, keeping it in the request `index` of
/// `requests`.
fn keep_sent(
    mut client: TcpStream,
    mut upstream: TcpStream,
    requests: &Mutex<Vec<Request>>,
    index: usize,
) {
    let mut buffer = [0; 16 * 1024];
    while let Ok(count @ 1..) = client.read(&mut buffer) {
        let chunk = &buffer[..count];
        requests.lock().unwrap()[index]
            .tunneled
            .extend_from_slice(chunk);
        if upstream.write_all(chunk).is_err() {
            break;
        }
    }
    let _ = upstream.shutdown(Shutdown::Write);
}

/// The head of the request on `client`, up to its empty line; `None` if it never ends.
pub fn read_head(client: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 64 << 10 || client.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.extend_from_slice(&byte);
    }
    String::from_utf8(head).ok()
}
