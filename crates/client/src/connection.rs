//! The client's connection to a Skald server: it states the protocol version on every request and
//! WebSocket opening request and acts on the answers as `docs/protocol/README.md`, "Server answers",
//! says. It goes over HTTPS and WSS, or in plain HTTP and WS to this machine only:
//! `docs/dev/architecture.md`, "Network".

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, TcpStream};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use serde::Deserialize;
use skald_proto::{
    LATEST_HEADER, VERSION_HEADER, VersionProblem, VersionRange, describe_versions, parse_version,
};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::http::{HeaderMap, HeaderValue, Uri, header};
use tungstenite::stream::MaybeTlsStream;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;

use crate::tls::Roots;

/// How long a request to the server may take by default.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// An open WebSocket to the server.
pub type WebSocket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

/// Why a request to the server failed.
#[derive(Debug)]
pub enum Error {
    /// The server URL is not one the client connects to.
    Url(String),
    /// The server URL is plain HTTP to another machine: what the client sends would cross the
    /// network in the clear.
    Cleartext(String),
    /// No answer from the server.
    Unreachable(String),
    /// No answer from the server within the timeout.
    Timeout(Duration),
    /// The server no longer accepts any version the client speaks: `protocol-too-old`.
    TooOld {
        supported: Vec<u64>,
        speaks: VersionRange,
    },
    /// The server accepts only versions older than any the client speaks: `protocol-too-new`.
    ServerOlder {
        supported: Vec<u64>,
        speaks: VersionRange,
    },
    /// The server refused the request for another reason, given in `detail`.
    Refused { status: u16, detail: String },
    /// An OAuth error answer (RFC 6749, §5.2; RFC 8628, §3.5), such as `authorization_pending`.
    OAuth { error: String, description: String },
    /// An answer the protocol does not define.
    Unexpected(String),
}

impl Error {
    /// Whether the server answered.
    pub fn reached(&self) -> bool {
        !matches!(
            self,
            Error::Url(_) | Error::Cleartext(_) | Error::Unreachable(_) | Error::Timeout(_)
        )
    }

    /// Whether the server answered that no version the client speaks is compatible.
    pub fn incompatible(&self) -> bool {
        matches!(self, Error::TooOld { .. } | Error::ServerOlder { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Url(url) => write!(
                f,
                "{url:?} is not a server URL the client connects to: use https://"
            ),
            Error::Cleartext(url) => write!(
                f,
                "{url:?} is plain HTTP to another machine: use https://; plain http:// goes only \
                 to this machine: localhost, 127.0.0.0/8 or [::1]"
            ),
            Error::Unreachable(cause) => {
                write!(f, "cannot reach the server: {}", printable(cause))
            }
            Error::Timeout(timeout) => write!(
                f,
                "cannot reach the server: no answer within {} s",
                timeout.as_secs()
            ),
            Error::TooOld { supported, speaks } => write!(
                f,
                "please update skald: the server accepts {}; this skald speaks {speaks}",
                describe_versions(supported)
            ),
            Error::ServerOlder { supported, speaks } => write!(
                f,
                "the server is older than this skald: it accepts {}; this skald speaks {speaks}",
                describe_versions(supported)
            ),
            Error::Refused { status, detail } => write!(
                f,
                "the server refused the request with {status}: {}",
                printable(detail)
            ),
            Error::OAuth { error, description } => {
                write!(f, "the server refused the request: {}", printable(error))?;
                if !description.is_empty() {
                    write!(f, ": {}", printable(description))?;
                }
                Ok(())
            }
            Error::Unexpected(answer) => {
                write!(
                    f,
                    "unexpected answer from the server: {}",
                    printable(answer)
                )
            }
        }
    }
}

/// `text` with its control characters escaped, such as `\u{1b}` for ESC: the server's text must not
/// drive the terminal.
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|char| {
            if char.is_control() {
                char.escape_debug().to_string()
            } else {
                char.to_string()
            }
        })
        .collect()
}

/// The error of an HTTP request that got no answer.
fn unanswered(err: ureq::Error, timeout: Duration) -> Error {
    match err {
        ureq::Error::Timeout(_) => Error::Timeout(timeout),
        err => Error::Unreachable(err.to_string()),
    }
}

/// How the client reaches a server.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scheme {
    /// `https://`, to any host: over TLS.
    Https,
    /// `http://` to this machine: in the clear, never crossing a network.
    Loopback,
}

/// How the client reaches the server at `server`: over `https://` to any host, over `http://` only
/// to this machine.
fn scheme(server: &str) -> Result<Scheme, Error> {
    let not_a_server = || Error::Url(server.to_owned());
    let uri: Uri = server.parse().map_err(|_| not_a_server())?;
    let host = uri.host().filter(|host| !host.is_empty());
    match (uri.scheme_str(), host) {
        (Some("https"), Some(_)) => Ok(Scheme::Https),
        (Some("http"), Some(host)) if loopback(host) => Ok(Scheme::Loopback),
        (Some("http"), Some(_)) => Err(Error::Cleartext(server.to_owned())),
        _ => Err(not_a_server()),
    }
}

/// Whether `host`, as a URL has it, is this machine: `localhost`, an address in `127.0.0.0/8` or
/// `[::1]`.
fn loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.strip_prefix('[').and_then(|ip| ip.strip_suffix(']')) {
        Some(ip) => ip.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback()),
        None => host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback()),
    }
}

/// The URL of the WebSocket at `path` on the server at `base`: `wss://` over TLS, otherwise
/// `ws://`.
fn websocket_url(base: &str, path: &str, tls: bool) -> String {
    let (_, rest) = base.split_once("://").expect("a server URL has a scheme");
    let scheme = if tls { "wss" } else { "ws" };
    format!("{scheme}://{rest}{path}")
}

/// The host of `uri`, an IPv6 address without its brackets.
fn host(uri: &Uri) -> &str {
    let host = uri.host().unwrap_or_default();
    host.trim_start_matches('[').trim_end_matches(']')
}

/// The port of `uri`: its own, or its scheme's, 443 for `https://` and `wss://`, 80 for `http://`
/// and `ws://`.
fn port(uri: &Uri) -> u16 {
    let default = match uri.scheme_str() {
        Some("https" | "wss") => 443,
        _ => 80,
    };
    uri.port_u16().unwrap_or(default)
}

/// The resolver of the server connection, for its HTTP requests and its WebSockets. It answers
/// `localhost` itself, with 127.0.0.1 and ::1, so plain HTTP to it stays on this machine whatever a
/// resolver would answer; any other host goes to `system`.
#[derive(Clone, Debug)]
struct Pinned {
    system: Arc<dyn Resolver>,
}

impl Resolver for Pinned {
    fn resolve(
        &self,
        uri: &Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        if !host(uri).eq_ignore_ascii_case("localhost") {
            return self.system.resolve(uri, config, timeout);
        }
        let mut addrs = self.empty();
        addrs.push((Ipv4Addr::LOCALHOST, port(uri)).into());
        addrs.push((Ipv6Addr::LOCALHOST, port(uri)).into());
        Ok(addrs)
    }
}

/// A TCP connection to the host and port of `uri`, its addresses from `resolver` with `config`,
/// whose lookup, connect, reads and writes each fail after `timeout`.
fn connect(
    uri: &Uri,
    timeout: Duration,
    resolver: &Pinned,
    config: &ureq::config::Config,
) -> Result<TcpStream, Error> {
    let host = uri.host().unwrap_or_default();
    // The resolver takes only HTTP URLs.
    let lookup: Uri = format!("http://{host}:{}", port(uri))
        .parse()
        .map_err(|err: tungstenite::http::uri::InvalidUri| Error::Unreachable(err.to_string()))?;
    let within = NextTimeout {
        after: timeout.into(),
        reason: ureq::Timeout::Resolve,
        ..NextTimeout::default()
    };
    let addrs = resolver
        .resolve(&lookup, config, within)
        .map_err(|err| unanswered(err, timeout))?;
    let mut last = io::Error::new(io::ErrorKind::NotFound, format!("no address for {host}"));
    for addr in addrs.iter() {
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(timeout))
                    .and_then(|()| stream.set_write_timeout(Some(timeout)))
                    .map_err(|err| Error::Unreachable(err.to_string()))?;
                return Ok(stream);
            }
            Err(err) => last = err,
        }
    }
    Err(io_error(last, timeout))
}

/// The error of an I/O operation on the server connection.
fn io_error(err: io::Error, timeout: Duration) -> Error {
    match err.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Error::Timeout(timeout),
        _ => Error::Unreachable(err.to_string()),
    }
}

/// `stream`, a connection to the server of `uri`, under TLS by `config`; the handshake runs on its
/// first read or write.
fn over_tls(
    config: &Arc<ClientConfig>,
    uri: &Uri,
    stream: TcpStream,
) -> Result<StreamOwned<ClientConnection, TcpStream>, Error> {
    let name = ServerName::try_from(host(uri).to_owned())
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    let client = ClientConnection::new(Arc::clone(config), name)
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    Ok(StreamOwned::new(client, stream))
}

/// A connection to the server at one base URL.
pub struct Connection {
    base: String,
    /// The TLS config of its WebSockets, to an `https://` server.
    tls: Option<Arc<ClientConfig>>,
    /// The resolver of its WebSockets; its HTTP requests' is in `agent`.
    resolver: Pinned,
    speaks: VersionRange,
    timeout: Duration,
    agent: ureq::Agent,
    /// The version the next request states: the newest the client speaks until the server answers
    /// that it is too new.
    version: NonZeroU64,
    /// The server's current version, from its last answer.
    latest: Option<NonZeroU64>,
}

/// The body of a request.
#[derive(Clone, Copy)]
enum Body<'a> {
    /// None: a `GET`.
    None,
    /// None: a `DELETE`.
    Delete,
    /// None: a `POST`.
    Post,
    /// A form: a `POST`.
    Form(&'a [(&'a str, &'a str)]),
    /// JSON.
    Json(Method, &'a [u8]),
}

/// The method of a request with a JSON body.
#[derive(Clone, Copy)]
pub enum Method {
    Post,
    Put,
}

/// The server's answer to one request.
enum Answer<T> {
    /// The server served the request.
    Served(T, Option<NonZeroU64>),
    /// The server refused it.
    Refused {
        status: u16,
        content: Content,
        body: Vec<u8>,
    },
}

/// The media type of a refusal's body.
#[derive(Clone, Copy, PartialEq)]
enum Content {
    /// `application/problem+json`: a problem details object.
    Problem,
    /// `application/json`: an OAuth error answer, on the device flow's endpoints.
    Json,
    Other,
}

/// The fields of an OAuth error answer the client reads.
#[derive(Deserialize)]
struct OAuthError {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// The fields of a problem details object the client reads.
#[derive(Deserialize)]
struct Problem {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    supported: Vec<u64>,
}

impl Connection {
    /// A connection to the server at `server`, such as `https://api.skald.so` or, on this machine,
    /// `http://localhost:8080`, speaking `speaks`. A request fails if the server does not answer it
    /// within `timeout`.
    pub fn new(server: &str, speaks: VersionRange, timeout: Duration) -> Result<Self, Error> {
        Self::resolving(
            server,
            speaks,
            timeout,
            Arc::new(DefaultResolver::default()),
        )
    }

    /// As [`Connection::new`], with `system` resolving every host but `localhost`.
    fn resolving(
        server: &str,
        speaks: VersionRange,
        timeout: Duration,
        system: Arc<dyn Resolver>,
    ) -> Result<Self, Error> {
        let roots = match scheme(server)? {
            Scheme::Https => Some(Roots::load()),
            Scheme::Loopback => None,
        };
        let mut config = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            // A redirect could take a request to another host, in the clear.
            .max_redirects(0)
            .timeout_global(Some(timeout));
        if let Some(roots) = &roots {
            config = config.tls_config(roots.ureq());
        }
        let resolver = Pinned { system };
        let agent = ureq::Agent::with_parts(
            config.build(),
            ureq::unversioned::transport::DefaultConnector::new(),
            resolver.clone(),
        );
        Ok(Connection {
            base: server.trim_end_matches('/').to_owned(),
            tls: roots.map(|roots| roots.rustls()),
            resolver,
            speaks,
            timeout,
            agent,
            version: speaks.latest(),
            latest: None,
        })
    }

    /// The version the connection states.
    pub fn version(&self) -> NonZeroU64 {
        self.version
    }

    /// The server's current version, if it is newer than the one the connection states.
    pub fn update_available(&self) -> Option<NonZeroU64> {
        self.latest.filter(|&latest| latest > self.version)
    }

    /// The server's base URL, without a trailing `/`.
    pub fn server(&self) -> &str {
        &self.base
    }

    /// `GET <path>`; the body of the answer that serves it.
    pub fn get(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        self.http(path, None, Body::None)
    }

    /// `GET <path>` as the session of `token`, sent as `Authorization: Bearer <token>`.
    pub fn get_as(&mut self, path: &str, token: &str) -> Result<Vec<u8>, Error> {
        self.http(path, Some(token), Body::None)
    }

    /// `DELETE <path>` as the session of `token`.
    pub fn delete_as(&mut self, path: &str, token: &str) -> Result<(), Error> {
        self.http(path, Some(token), Body::Delete).map(drop)
    }

    /// `POST <path>` without a body as the session of `token`.
    pub fn post_as(&mut self, path: &str, token: &str) -> Result<Vec<u8>, Error> {
        self.http(path, Some(token), Body::Post)
    }

    /// `POST <path>` with the form `form`; the body of the answer that serves it.
    pub fn post_form(&mut self, path: &str, form: &[(&str, &str)]) -> Result<Vec<u8>, Error> {
        self.http(path, None, Body::Form(form))
    }

    /// `<method> <path>` with the JSON `json` as the session of `token`.
    pub fn send_json_as(
        &mut self,
        method: Method,
        path: &str,
        token: &str,
        json: &[u8],
    ) -> Result<Vec<u8>, Error> {
        self.http(path, Some(token), Body::Json(method, json))
    }

    /// A request to `path` with `body`, with the session of `token` if given.
    fn http(&mut self, path: &str, token: Option<&str>, body: Body<'_>) -> Result<Vec<u8>, Error> {
        let url = format!("{}{path}", self.base);
        let agent = self.agent.clone();
        let timeout = self.timeout;
        let bearer = token.map(|token| format!("Bearer {token}"));
        self.exchange(|version| {
            let version = version.to_string();
            let bearer = bearer.as_deref();
            let sent = match body {
                Body::None => with_headers(agent.get(&url), &version, bearer).call(),
                Body::Delete => with_headers(agent.delete(&url), &version, bearer).call(),
                Body::Post => with_headers(agent.post(&url), &version, bearer).send_empty(),
                Body::Form(form) => {
                    with_headers(agent.post(&url), &version, bearer).send_form(form.iter().copied())
                }
                Body::Json(method, json) => match method {
                    Method::Post => with_headers(agent.post(&url), &version, bearer),
                    Method::Put => with_headers(agent.put(&url), &version, bearer),
                }
                .content_type("application/json")
                .send(json),
            };
            let mut response = sent.map_err(|err| unanswered(err, timeout))?;
            let status = response.status().as_u16();
            let latest = latest(response.headers());
            let content = content(response.headers());
            let body = response
                .body_mut()
                .read_to_vec()
                .map_err(|err| unanswered(err, timeout))?;
            Ok(if response.status().is_success() {
                Answer::Served(body, latest)
            } else {
                Answer::Refused {
                    status,
                    content,
                    body,
                }
            })
        })
    }

    /// Opens a WebSocket at `path`, offering the subprotocol `skald.<n>`. The connect and each read
    /// and write of the socket, after the opening too, fail after the connection's timeout.
    pub fn open_websocket(&mut self, path: &str) -> Result<WebSocket, Error> {
        let url = websocket_url(&self.base, path, self.tls.is_some());
        let timeout = self.timeout;
        let tls = self.tls.clone();
        let resolver = self.resolver.clone();
        let agent = self.agent.clone();
        self.exchange(|version| {
            let mut request = url
                .as_str()
                .into_client_request()
                .map_err(|err| Error::Unreachable(err.to_string()))?;
            let offer = HeaderValue::from_str(&skald_proto::subprotocol(version))
                .expect("a subprotocol is a header value");
            request
                .headers_mut()
                .insert(header::SEC_WEBSOCKET_PROTOCOL, offer);
            let stream = connect(request.uri(), timeout, &resolver, agent.config())?;
            let stream = match &tls {
                Some(config) => MaybeTlsStream::Rustls(over_tls(config, request.uri(), stream)?),
                None => MaybeTlsStream::Plain(stream),
            };
            match tungstenite::client(request, stream) {
                Ok((socket, response)) => Ok(Answer::Served(socket, latest(response.headers()))),
                // A blocking stream is interrupted only when a read or write times out.
                Err(HandshakeError::Interrupted(_)) => Err(Error::Timeout(timeout)),
                Err(HandshakeError::Failure(tungstenite::Error::Http(response))) => {
                    Ok(Answer::Refused {
                        status: response.status().as_u16(),
                        content: content(response.headers()),
                        body: response.body().clone().unwrap_or_default(),
                    })
                }
                Err(HandshakeError::Failure(tungstenite::Error::Io(err))) => {
                    Err(io_error(err, timeout))
                }
                Err(HandshakeError::Failure(err)) => Err(Error::Unexpected(err.to_string())),
            }
        })
    }

    /// Sends a request with `send`, which states the version it is given, and acts on the answer:
    /// on `protocol-too-new` it sends it again with an older version the server accepts, if the
    /// client speaks one.
    fn exchange<T>(
        &mut self,
        mut send: impl FnMut(NonZeroU64) -> Result<Answer<T>, Error>,
    ) -> Result<T, Error> {
        loop {
            let (status, content, body) = match send(self.version)? {
                Answer::Served(value, latest) => {
                    self.latest = latest;
                    return Ok(value);
                }
                Answer::Refused {
                    status,
                    content,
                    body,
                } => (status, content, body),
            };
            if content == Content::Json
                && let Ok(OAuthError {
                    error,
                    error_description,
                }) = serde_json::from_slice(&body)
            {
                return Err(Error::OAuth {
                    error,
                    description: error_description,
                });
            }
            let problem = (content == Content::Problem)
                .then(|| serde_json::from_slice::<Problem>(&body).ok())
                .flatten()
                .ok_or_else(|| Error::Unexpected(format!("HTTP {status}")))?;
            let Problem {
                kind,
                detail,
                supported,
            } = problem;
            let speaks = self.speaks;
            if kind == VersionProblem::TooOld.type_uri() {
                return Err(Error::TooOld { supported, speaks });
            }
            if kind != VersionProblem::TooNew.type_uri() {
                return Err(Error::Refused { status, detail });
            }
            // An older version each time: the loop ends.
            match best_of(speaks, &supported).filter(|&version| version < self.version) {
                Some(version) => self.version = version,
                None => return Err(Error::ServerOlder { supported, speaks }),
            }
        }
    }
}

/// The newest version in `speaks` among `supported`.
fn best_of(speaks: VersionRange, supported: &[u64]) -> Option<NonZeroU64> {
    supported
        .iter()
        .filter(|&&version| speaks.contains(version))
        .max()
        .and_then(|&version| NonZeroU64::new(version))
}

/// `request` stating `version`, with the session of `bearer`, `Bearer <token>`, if given.
fn with_headers<B>(
    request: ureq::RequestBuilder<B>,
    version: &str,
    bearer: Option<&str>,
) -> ureq::RequestBuilder<B> {
    let request = request.header(VERSION_HEADER, version);
    match bearer {
        Some(bearer) => request.header("authorization", bearer),
        None => request,
    }
}

/// The server's current version, from `Skald-Protocol-Latest`.
fn latest(headers: &HeaderMap) -> Option<NonZeroU64> {
    parse_version(headers.get(LATEST_HEADER)?.to_str().ok()?)
}

/// The media type of the body, from `Content-Type`.
fn content(headers: &HeaderMap) -> Content {
    let media = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if media.starts_with("application/problem+json") {
        Content::Problem
    } else if media.starts_with("application/json") {
        Content::Json
    } else {
        Content::Other
    }
}

#[cfg(test)]
mod tests {
    use std::net::{SocketAddr, TcpListener};

    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    /// A resolver that answers `at` for every host, as one that sends `localhost` off the machine.
    #[derive(Debug)]
    struct Elsewhere(SocketAddr);

    impl Resolver for Elsewhere {
        fn resolve(
            &self,
            _: &Uri,
            _: &ureq::config::Config,
            _: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, ureq::Error> {
            let mut addrs = self.empty();
            addrs.push(self.0);
            Ok(addrs)
        }
    }

    /// The resolver of a connection on the system's resolver.
    fn system() -> Pinned {
        Pinned {
            system: Arc::new(DefaultResolver::default()),
        }
    }

    #[test]
    fn io_timeouts_are_timeouts() {
        for kind in [io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock] {
            let error = io_error(io::Error::from(kind), SECOND);
            assert!(
                matches!(error, Error::Timeout(SECOND)),
                "{kind:?}: {error:?}"
            );
        }
        let error = io_error(io::Error::from(io::ErrorKind::ConnectionReset), SECOND);
        assert!(matches!(error, Error::Unreachable(_)), "{error:?}");
    }

    #[test]
    fn oauth_error_shows_its_description_if_any() {
        let error = |description: &str| Error::OAuth {
            error: "invalid_grant".to_owned(),
            description: description.to_owned(),
        };
        assert_eq!(
            error("").to_string(),
            "the server refused the request: invalid_grant"
        );
        assert_eq!(
            error("Unknown code.\u{1b}").to_string(),
            "the server refused the request: invalid_grant: Unknown code.\\u{1b}"
        );
    }

    #[test]
    fn https_goes_to_any_host_and_http_only_to_this_machine() {
        for server in [
            "https://api.skald.so",
            "https://api.skald.so/",
            "HTTPS://API.SKALD.SO",
            "https://203.0.113.7:8443",
            "https://[2001:db8::1]",
            "https://localhost:8443",
        ] {
            assert_eq!(scheme(server).ok(), Some(Scheme::Https), "{server}");
        }
        for server in [
            "http://localhost:8080",
            "http://LocalHost",
            "http://127.0.0.1:8080/",
            "http://127.0.0.0",
            "http://127.255.255.255:1",
            "http://[::1]:8080",
            "http://[0:0:0:0:0:0:0:1]",
            "http://someone@localhost",
        ] {
            assert_eq!(scheme(server).ok(), Some(Scheme::Loopback), "{server}");
        }
    }

    #[test]
    fn http_to_another_machine_is_cleartext() {
        for server in [
            "http://api.skald.so",
            "http://0.0.0.0:8080",
            "http://10.0.0.1",
            "http://126.255.255.255",
            "http://128.0.0.1",
            "http://127.1",
            "http://127.0.0.01",
            "http://[::]",
            "http://[::2]",
            "http://[::ffff:127.0.0.1]",
            "http://localhost.skald.test",
            "http://127.0.0.1.skald.test",
            "http://localhost@skald.test",
            "http://localhost.",
        ] {
            let error = scheme(server).expect_err(server);
            assert!(
                matches!(&error, Error::Cleartext(url) if url == server),
                "{error:?}"
            );
            assert!(!error.reached());
        }
        assert_eq!(
            Error::Cleartext("http://api.skald.so".to_owned()).to_string(),
            "\"http://api.skald.so\" is plain HTTP to another machine: use https://; plain http:// \
             goes only to this machine: localhost, 127.0.0.0/8 or [::1]"
        );
    }

    #[test]
    fn other_urls_are_no_server_urls() {
        for server in [
            "",
            "api.skald.so",
            "localhost:8080",
            "ws://localhost:8080",
            "wss://api.skald.so",
            "ftp://api.skald.so",
            "https://",
            "http://",
            "http:///ws",
            "https://:443",
            "http://:8080",
            "http://@:8080",
            "https://api skald so",
        ] {
            let error = scheme(server).expect_err(server);
            assert!(
                matches!(&error, Error::Url(url) if url == server),
                "{error:?}"
            );
            assert!(!error.reached());
        }
        assert_eq!(
            Error::Url("ftp://api.skald.so".to_owned()).to_string(),
            "\"ftp://api.skald.so\" is not a server URL the client connects to: use https://"
        );
    }

    #[test]
    fn websocket_goes_over_tls_to_an_https_server() {
        assert_eq!(
            websocket_url("https://api.skald.so", "/ws", true),
            "wss://api.skald.so/ws"
        );
        assert_eq!(
            websocket_url("HTTPS://api.skald.so:8443/base", "/ws", true),
            "wss://api.skald.so:8443/base/ws"
        );
        assert_eq!(
            websocket_url("http://127.0.0.1:8080", "/ws", false),
            "ws://127.0.0.1:8080/ws"
        );
        let port_of = |url: &str| port(&url.parse().unwrap());
        assert_eq!(port_of("wss://api.skald.so/ws"), 443);
        assert_eq!(port_of("ws://localhost/ws"), 80);
        assert_eq!(port_of("wss://api.skald.so:8443/ws"), 8443);
        assert_eq!(port_of("ws://localhost:8080/ws"), 8080);
        assert_eq!(host(&"wss://[::1]:8443/ws".parse().unwrap()), "::1");
    }

    #[test]
    fn closed_port_is_unreachable() {
        let closed = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let uri: Uri = format!("ws://{}/ws", closed.local_addr().unwrap())
            .parse()
            .unwrap();
        drop(closed);

        // Windows refuses a connection to a closed port only after retrying it for about 2 s.
        let error = connect(
            &uri,
            SECOND * 10,
            &system(),
            &ureq::config::Config::default(),
        )
        .expect_err("nothing listens");

        assert!(matches!(error, Error::Unreachable(_)), "{error:?}");
    }

    #[test]
    fn connection_has_timeouts() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let uri: Uri = format!("ws://{}/ws", listener.local_addr().unwrap())
            .parse()
            .unwrap();

        let stream =
            connect(&uri, SECOND, &system(), &ureq::config::Config::default()).expect("connect");

        assert_eq!(stream.read_timeout().unwrap(), Some(SECOND));
        assert_eq!(stream.write_timeout().unwrap(), Some(SECOND));
    }

    #[test]
    fn localhost_is_only_this_machine() {
        let elsewhere: SocketAddr = "192.0.2.1:8080".parse().unwrap();
        let resolver = Pinned {
            system: Arc::new(Elsewhere(elsewhere)),
        };
        let resolve = |url: &str| {
            let addrs = resolver
                .resolve(
                    &url.parse().unwrap(),
                    &ureq::config::Config::default(),
                    NextTimeout::default(),
                )
                .expect(url);
            addrs.to_vec()
        };
        let loopback = |port| -> Vec<SocketAddr> {
            vec![
                (Ipv4Addr::LOCALHOST, port).into(),
                (Ipv6Addr::LOCALHOST, port).into(),
            ]
        };

        assert_eq!(resolve("http://localhost:8080/ws"), loopback(8080));
        assert_eq!(resolve("http://LocalHost"), loopback(80));
        assert_eq!(resolve("https://localhost"), loopback(443));
        assert_eq!(resolve("http://api.skald.so:8080"), vec![elsewhere]);
    }

    /// The connection's HTTP requests and WebSockets reach `localhost` at 127.0.0.1, never where
    /// the system's resolver points: here another listener on this machine.
    #[test]
    fn connection_reaches_localhost_only_at_loopback() {
        let server = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let elsewhere = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = server.local_addr().unwrap().port();
        let system = Arc::new(Elsewhere(elsewhere.local_addr().unwrap()));
        let url = format!("http://localhost:{port}");
        let mut connection =
            Connection::resolving(&url, VersionRange::default(), SECOND / 4, system)
                .expect("a loopback URL");

        // Nothing answers: each request fails, once its connection is made.
        connection.get("/status").expect_err("no answer");
        connection.open_websocket("/ws").expect_err("no answer");

        let accepted = |listener: &TcpListener| {
            listener.set_nonblocking(true).unwrap();
            std::iter::from_fn(|| listener.accept().ok()).count()
        };
        assert_eq!(accepted(&server), 2);
        assert_eq!(accepted(&elsewhere), 0);
    }
}
