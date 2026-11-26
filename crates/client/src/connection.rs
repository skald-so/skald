//! The client's connection to a Skald server: it states the protocol version on every request and
//! WebSocket opening request and acts on the answers as `docs/protocol/README.md`, "Server answers",
//! says. It goes over HTTPS and WSS, through the proxy `HTTPS_PROXY` names if any, or in plain
//! HTTP and WS to this machine only: `docs/dev/architecture.md`, "Network". A device proves itself
//! on a WebSocket in its first frame, and in each HTTPS request of its mailbox in a header
//! (`docs/protocol/delivery.md`, "Connecting").

use std::cell::Cell;
use std::fmt;
use std::io::{self, Read as _, Write as _};
use std::net::{Ipv4Addr, Ipv6Addr, TcpStream};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use serde::Deserialize;
use skald_crypto::SigningKey;
use skald_proto::{
    AUTH_LABEL, Cbor as _, DeviceId, Frame, LATEST_HEADER, PROOF_HEADER, REQUEST_LABEL,
    RequestProof, VERSION_HEADER, VersionProblem, VersionRange, describe_versions, parse_version,
};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::http::{HeaderMap, HeaderValue, Uri, header};
use tungstenite::stream::MaybeTlsStream;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, RustlsConnector, Transport,
};

use crate::events::Events;
use crate::proxy::Proxy;
use crate::tls::Roots;

/// How long a request to the server may take by default.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// A device session's challenge for a WebSocket, `POST`: `docs/protocol/delivery.md`,
/// "Connecting".
const CHALLENGE_PATH: &str = "/session/challenge";
/// The label of a device's signature for a session by its key.
const DEVICE_SESSION_LABEL: &str = "skald device session";

/// An open WebSocket to the server.
pub type WebSocket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

/// The problem of a request without a session, or with one the server does not hold.
const NOT_SIGNED_IN: &str = "https://skald.so/problems/not-signed-in";

/// Whether a CLI request got [`NOT_SIGNED_IN`] since [`forget_refused`].
static SESSION_REFUSED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default)]
struct SdkRefusals {
    session: bool,
    account_deleted: bool,
}

thread_local! {
    /// Refusals observed on the calling thread during an SDK operation.
    static SCOPED_REFUSALS: Cell<Option<SdkRefusals>> = const { Cell::new(None) };
}

/// Whether a request of the current SDK operation or CLI run got a session refusal.
pub fn session_refused() -> bool {
    SCOPED_REFUSALS.get().map_or_else(
        || SESSION_REFUSED.load(Ordering::SeqCst),
        |refusals| refusals.session,
    )
}

/// Forgets the CLI's refusals.
pub fn forget_refused() {
    SESSION_REFUSED.store(false, Ordering::SeqCst);
}

/// Tracks HTTP and mailbox refusals separately for an SDK operation and its caller.
pub fn track_refusals() -> Refusals {
    Refusals(SCOPED_REFUSALS.replace(Some(SdkRefusals::default())))
}

pub struct Refusals(Option<SdkRefusals>);

impl Drop for Refusals {
    fn drop(&mut self) {
        SCOPED_REFUSALS.set(self.0);
    }
}

/// Whether the server closed this SDK operation's mailbox because the account was deleted.
pub fn sdk_account_deleted() -> bool {
    SCOPED_REFUSALS
        .get()
        .is_some_and(|refusals| refusals.account_deleted)
}

/// Records a server confirmation of account deletion if an SDK operation is active.
pub fn note_sdk_account_deleted() -> bool {
    note_sdk_refusal(|refusals| refusals.account_deleted = true)
}

fn note_sdk_refusal(note: impl FnOnce(&mut SdkRefusals)) -> bool {
    let Some(mut refusals) = SCOPED_REFUSALS.get() else {
        return false;
    };
    note(&mut refusals);
    SCOPED_REFUSALS.set(Some(refusals));
    true
}

fn note_refused() {
    if !note_sdk_refusal(|refusals| refusals.session = true) {
        SESSION_REFUSED.store(true, Ordering::SeqCst);
    }
}

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
    /// `HTTPS_PROXY` holds no URL of an HTTP proxy.
    ProxyUrl,
    /// The proxy at `proxy`, its host and port, gave no tunnel to the server: `problem` says why.
    Proxy { proxy: String, problem: String },
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
    /// The server refused the request for another reason: the problem's type `kind`, its
    /// `detail` and, of a `duplicate` task, its extension member `task`.
    Refused {
        status: u16,
        kind: String,
        detail: String,
        task: Option<String>,
    },
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
            Error::Url(_)
                | Error::Cleartext(_)
                | Error::Unreachable(_)
                | Error::Timeout(_)
                | Error::ProxyUrl
                | Error::Proxy { .. }
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
            Error::ProxyUrl => write!(
                f,
                "HTTPS_PROXY is not the URL of an HTTP proxy: use http://[user:password@]host[:port]"
            ),
            Error::Proxy { proxy, problem } => write!(
                f,
                "cannot reach the server through the proxy {proxy}: {}",
                printable(problem)
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
            Error::Refused { status, detail, .. } => write!(
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

impl std::error::Error for Error {}

pub use skald_adapters::label::printable;

/// The error of an HTTP request that got no answer: its connection's own, from [`Dial`], if it
/// failed.
fn unanswered(err: ureq::Error, timeout: Duration) -> Error {
    match err {
        ureq::Error::Timeout(_) => Error::Timeout(timeout),
        ureq::Error::Io(err) if err.get_ref().is_some_and(|inner| inner.is::<Error>()) => *err
            .into_inner()
            .and_then(|inner| inner.downcast().ok())
            .expect("an error of the connection"),
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

/// The origin (RFC 6454) of `url`, an `http` or `https` URL with a host, in its ASCII
/// serialization: such as `https://api.skald.so`, without a path or its scheme's port.
pub fn origin(url: &str) -> Option<String> {
    let uri: Uri = url.parse().ok()?;
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let default = match scheme.as_str() {
        "https" => 443,
        "http" => 80,
        _ => return None,
    };
    let host = uri
        .host()
        .filter(|host| !host.is_empty())?
        .to_ascii_lowercase();
    Some(match uri.port_u16().filter(|&port| port != default) {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
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
pub(crate) fn io_error(err: io::Error, timeout: Duration) -> Error {
    if timed_out(&err) {
        Error::Timeout(timeout)
    } else {
        Error::Unreachable(err.to_string())
    }
}

/// Whether `err`, of a read or write with a timeout, is that the timeout ran out.
pub(crate) fn timed_out(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
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

/// The longest answer head a proxy may give to `CONNECT`.
const PROXY_HEAD_LIMIT: usize = 64 << 10;

/// A TCP connection to the host and port of `uri`, directly or, if given, through a tunnel of
/// `proxy`; as [`connect`] says, with `timeout`, `resolver` and `config`. A failing proxy fails it:
/// it never falls back to a direct connection.
fn dial(
    proxy: Option<&Proxy>,
    uri: &Uri,
    timeout: Duration,
    resolver: &Pinned,
    config: &ureq::config::Config,
) -> Result<TcpStream, Error> {
    let Some(proxy) = proxy else {
        return connect(uri, timeout, resolver, config);
    };
    let failed = |problem: String| Error::Proxy {
        proxy: proxy.address(),
        problem,
    };
    let unreachable = |err: Error| {
        failed(match err {
            Error::Timeout(_) => format!("no answer within {} s", timeout.as_secs()),
            Error::Unreachable(cause) => cause,
            err => err.to_string(),
        })
    };
    let broken = |err: io::Error| unreachable(io_error(err, timeout));
    let address = format!("http://{}", proxy.address())
        .parse()
        .map_err(|_| Error::ProxyUrl)?;
    let mut stream = connect(&address, timeout, resolver, config).map_err(unreachable)?;
    let authority = format!("{}:{}", uri.host().unwrap_or_default(), port(uri));
    stream
        .write_all(proxy.connect_request(&authority).as_bytes())
        .map_err(broken)?;
    let head = proxy_head(&mut stream).map_err(broken)?;
    let line = head.lines().next().unwrap_or_default();
    let mut words = line.splitn(3, ' ');
    let (version, status, reason) = (words.next(), words.next(), words.next());
    let status = status
        .filter(|_| version.is_some_and(|version| version.starts_with("HTTP/1.")))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| failed("its answer is not HTTP".to_owned()))?;
    if (200..300).contains(&status) {
        return Ok(stream);
    }
    let mut problem = format!(
        "it refused the tunnel with {status} {}",
        reason.unwrap_or_default()
    );
    if status == 407 {
        problem.push_str("; the client answers it only by Basic, with the credentials of its URL");
    }
    Err(failed(problem.trim_end().to_owned()))
}

/// The head of a proxy's answer on `stream`, up to its empty line, read a byte at a time so that
/// nothing after it is taken from the tunnel.
fn proxy_head(stream: &mut TcpStream) -> io::Result<String> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= PROXY_HEAD_LIMIT || stream.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "its answer ended early",
            ));
        }
        head.extend_from_slice(&byte);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// What opens a connection's WebSockets and event streams: streams it dials as [`dial`] says, as
/// its HTTP requests do, over TLS to an `https://` server.
struct Streams {
    tls: Option<Arc<ClientConfig>>,
    resolver: Pinned,
    proxy: Option<Proxy>,
    agent: ureq::Agent,
    timeout: Duration,
}

impl Streams {
    /// A stream to the host and port of `uri`; a TLS handshake runs on its first read or write.
    fn open(&self, uri: &Uri) -> Result<MaybeTlsStream<TcpStream>, Error> {
        let config = self.agent.config();
        let stream = dial(
            self.proxy.as_ref(),
            uri,
            self.timeout,
            &self.resolver,
            config,
        )?;
        Ok(match &self.tls {
            Some(config) => MaybeTlsStream::Rustls(over_tls(config, uri, stream)?),
            None => MaybeTlsStream::Plain(stream),
        })
    }
}

/// The connector of the connection's HTTP requests: it dials as [`dial`] says, directly or through
/// the proxy, and fails with the connection's error inside [`ureq::Error::Io`].
#[derive(Debug)]
struct Dial {
    proxy: Option<Proxy>,
    resolver: Pinned,
    timeout: Duration,
}

impl Connector for Dial {
    type Out = Socket;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _: Option<()>,
    ) -> Result<Option<Socket>, ureq::Error> {
        self.socket(details.uri, details.config)
            .map(Some)
            .map_err(|err| ureq::Error::Io(io::Error::other(err)))
    }
}

impl Dial {
    /// A connection to the host and port of `uri`, with `config`'s buffers and `TCP_NODELAY`.
    fn socket(&self, uri: &Uri, config: &ureq::config::Config) -> Result<Socket, Error> {
        let stream = dial(
            self.proxy.as_ref(),
            uri,
            self.timeout,
            &self.resolver,
            config,
        )?;
        stream
            .set_nodelay(config.no_delay())
            .map_err(|err| Error::Unreachable(err.to_string()))?;
        let buffers = LazyBuffers::new(config.input_buffer_size(), config.output_buffer_size());
        Ok(Socket { stream, buffers })
    }
}

/// A TCP connection that [`Dial`] made, as a ureq transport.
#[derive(Debug)]
struct Socket {
    stream: TcpStream,
    buffers: LazyBuffers,
}

impl Socket {
    /// What a read or write within `timeout` may take: `None` for no limit.
    fn limit(timeout: NextTimeout) -> Result<Option<Duration>, ureq::Error> {
        if timeout.after.is_not_happening() {
            return Ok(None);
        }
        let after = *timeout.after;
        if after.is_zero() {
            return Err(ureq::Error::Timeout(timeout.reason));
        }
        Ok(Some(after))
    }

    /// The error of a read or write within `timeout` that failed with `err`.
    fn failed(err: io::Error, timeout: NextTimeout) -> ureq::Error {
        if timed_out(&err) {
            ureq::Error::Timeout(timeout.reason)
        } else {
            ureq::Error::Io(err)
        }
    }
}

impl Transport for Socket {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let timeout = timeout.for_write();
        self.stream.set_write_timeout(Self::limit(timeout)?)?;
        let output = &self.buffers.output()[..amount];
        (&self.stream)
            .write_all(output)
            .map_err(|err| Self::failed(err, timeout))
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let timeout = timeout.for_read();
        self.stream.set_read_timeout(Self::limit(timeout)?)?;
        let input = self.buffers.input_append_buf();
        let amount = uninterrupted(|| (&self.stream).read(input))
            .map_err(|err| Self::failed(err, timeout))?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    /// Whether the connection is still open, so that the agent may send another request on it: no
    /// byte waits on it, and it is not closed.
    fn is_open(&mut self) -> bool {
        idle(
            |on| self.stream.set_nonblocking(on),
            |byte| (&self.stream).read(byte),
        )
    }
}

/// Whether a stream is open with no byte waiting on it: once `set_nonblocking(true)` has made it
/// nonblocking, `read` would block, and `set_nonblocking(false)` makes it blocking again.
pub(crate) fn idle(
    set_nonblocking: impl Fn(bool) -> io::Result<()>,
    read: impl FnOnce(&mut [u8]) -> io::Result<usize>,
) -> bool {
    if set_nonblocking(true).is_err() {
        return false;
    }
    let idle = matches!(
        read(&mut [0]),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock
    );
    set_nonblocking(false).is_ok() && idle
}

/// What `read` gives, read again while a signal interrupts it.
pub(crate) fn uninterrupted<T>(mut read: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match read() {
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            read => return read,
        }
    }
}

/// The resolver of the connection's HTTP agent: it resolves nothing, as [`Dial`] does it.
#[derive(Debug)]
struct Deferred;

impl Resolver for Deferred {
    fn resolve(
        &self,
        _: &Uri,
        _: &ureq::config::Config,
        _: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        Ok(self.empty())
    }
}

/// The proxy the client reaches `base`, a URL it connects to, through: the one
/// [`Proxy::for_host`] gives for an `https://` URL, none for plain HTTP to this machine.
fn proxy_of(base: &str) -> Result<Option<Proxy>, Error> {
    match scheme(base)? {
        Scheme::Https => {
            let uri: Uri = base.parse().map_err(|_| Error::Url(base.to_owned()))?;
            Proxy::for_host(uri.host().unwrap_or_default()).map_err(|_| Error::ProxyUrl)
        }
        Scheme::Loopback => Ok(None),
    }
}

/// The agent of HTTP requests that dials as [`dial`] says, through `proxy` if given, with
/// `resolver`, over TLS trusting `roots` if given; it follows no redirect, and each request fails
/// after `timeout`.
fn http_agent(
    roots: Option<&Roots>,
    proxy: Option<Proxy>,
    resolver: Pinned,
    timeout: Duration,
) -> ureq::Agent {
    let mut config = ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        // A redirect could take a request to another host, in the clear.
        .max_redirects(0)
        .timeout_global(Some(timeout));
    if let Some(roots) = roots {
        config = config.tls_config(roots.ureq());
    }
    let dial = Dial {
        proxy,
        resolver,
        timeout,
    };
    ureq::Agent::with_parts(
        config.build(),
        ().chain(dial).chain(RustlsConnector::default()),
        Deferred,
    )
}

/// An agent of plain HTTP requests, with none of a connection's headers, to the service at `base`,
/// such as `https://api.github.com`, that reaches it as a connection reaches its server: over
/// HTTPS through the proxy `HTTPS_PROXY` names, or in plain HTTP to this machine only.
pub fn agent_to(base: &str, timeout: Duration) -> Result<ureq::Agent, Error> {
    let proxy = proxy_of(base)?;
    let roots = (scheme(base)? == Scheme::Https).then(Roots::load);
    let resolver = Pinned {
        system: Arc::new(DefaultResolver::default()),
    };
    Ok(http_agent(roots.as_ref(), proxy, resolver, timeout))
}

/// A connection to the server at one base URL.
pub struct Connection {
    base: String,
    /// The TLS config of its WebSockets and event streams, to an `https://` server.
    tls: Option<Arc<ClientConfig>>,
    /// The resolver of its WebSockets and event streams; its HTTP requests' is in `agent`.
    resolver: Pinned,
    /// The proxy of its WebSockets and event streams, if any; its HTTP requests' is in `agent`.
    proxy: Option<Proxy>,
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
    /// A `POST` without a body.
    Post,
    /// A form: a `POST`.
    Form(&'a [(&'a str, &'a str)]),
    /// JSON.
    Json(Method, &'a [u8]),
    /// None, a `GET` with the device's proof `proof`.
    ProvedGet { proof: &'a str },
    /// None, a `DELETE` with the device's proof `proof`.
    ProvedDelete { proof: &'a str },
    /// JSON, sent by `method` with the device's proof `proof`.
    ProvedJson {
        method: Method,
        json: &'a [u8],
        proof: &'a str,
    },
    /// CBOR, sent by `method` with the device's proof `proof`.
    ProvedCbor {
        method: Method,
        cbor: &'a [u8],
        proof: &'a str,
    },
    /// A frame, `POST`ed with the device's proof `proof`, its answer awaited for `wait` more than
    /// the timeout.
    Frame {
        frame: &'a [u8],
        proof: &'a str,
        wait: Duration,
    },
}

/// The method of a request with a body.
#[derive(Clone, Copy)]
pub enum Method {
    Post,
    Put,
    Patch,
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

/// `Challenge` in the API spec.
#[derive(Deserialize)]
struct Challenge {
    challenge: String,
}

/// `DeviceKeySession` in the API spec.
#[derive(Deserialize)]
struct Session {
    token: String,
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
    #[serde(default)]
    task: Option<String>,
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
        let proxy = proxy_of(server)?;
        Self::proxied(server, speaks, timeout, system, proxy)
    }

    /// As [`Connection::resolving`], through `proxy` if given.
    fn proxied(
        server: &str,
        speaks: VersionRange,
        timeout: Duration,
        system: Arc<dyn Resolver>,
        proxy: Option<Proxy>,
    ) -> Result<Self, Error> {
        let roots = (scheme(server)? == Scheme::Https).then(Roots::load);
        let resolver = Pinned { system };
        let agent = http_agent(roots.as_ref(), proxy.clone(), resolver.clone(), timeout);
        Ok(Connection {
            base: server.trim_end_matches('/').to_owned(),
            tls: roots.map(|roots| roots.rustls()),
            resolver,
            proxy,
            speaks,
            timeout,
            agent,
            version: speaks.latest(),
            latest: None,
        })
    }

    /// A new connection to the same server, speaking and waiting as this one does.
    pub fn again(&self) -> Result<Self, Error> {
        Self::new(&self.base, self.speaks, self.timeout)
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

    /// `POST <path>` without a body as the session of `token`; the body of the answer that serves
    /// it.
    pub fn post_as(&mut self, path: &str, token: &str) -> Result<Vec<u8>, Error> {
        self.http(path, Some(token), Body::Post)
    }

    /// `POST <path>` of the JSON `json` without a session; the body of the answer that serves it.
    pub fn post_json(&mut self, path: &str, json: &[u8]) -> Result<Vec<u8>, Error> {
        self.http(path, None, Body::Json(Method::Post, json))
    }

    /// `PUT <path>` of the JSON `json` without a session.
    pub fn put_json(&mut self, path: &str, json: &[u8]) -> Result<(), Error> {
        self.http(path, None, Body::Json(Method::Put, json))
            .map(drop)
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

    /// `GET <path>` as the device of `proof`, on the device session or service token `token`.
    pub fn get_proved(
        &mut self,
        path: &str,
        (token, proof): (&str, &str),
    ) -> Result<Vec<u8>, Error> {
        self.http(path, Some(token), Body::ProvedGet { proof })
    }

    /// `POST <path>` of the JSON `json` as the device of `proof`, on the device session of `token`.
    pub fn post_json_proved(
        &mut self,
        path: &str,
        (token, proof): (&str, &str),
        json: &[u8],
    ) -> Result<Vec<u8>, Error> {
        self.send_json_proved(Method::Post, path, (token, proof), json)
    }

    /// `<method> <path>` of the JSON `json` as the device of `proof`, on the device session of
    /// `token`.
    pub fn send_json_proved(
        &mut self,
        method: Method,
        path: &str,
        (token, proof): (&str, &str),
        json: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let body = Body::ProvedJson {
            method,
            json,
            proof,
        };
        self.http(path, Some(token), body)
    }

    /// `DELETE <path>` as the device of `proof`, on the device session of `token`.
    pub fn delete_proved(&mut self, path: &str, (token, proof): (&str, &str)) -> Result<(), Error> {
        self.http(path, Some(token), Body::ProvedDelete { proof })
            .map(drop)
    }

    /// `PUT <path>` of the CBOR `cbor` as the device of `proof`, on the device session of `token`.
    pub fn put_cbor_proved(
        &mut self,
        path: &str,
        (token, proof): (&str, &str),
        cbor: &[u8],
    ) -> Result<Vec<u8>, Error> {
        self.send_cbor_proved(Method::Put, path, (token, proof), cbor)
    }

    /// `<method> <path>` of the CBOR `cbor` as the device of `proof`, on the device session of
    /// `token`.
    pub fn send_cbor_proved(
        &mut self,
        method: Method,
        path: &str,
        (token, proof): (&str, &str),
        cbor: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let body = Body::ProvedCbor {
            method,
            cbor,
            proof,
        };
        self.http(path, Some(token), body)
    }

    /// `POST <path>` of `frame` as the device of `proof`, on the device session of `token`; the body
    /// of the answer that serves it, awaited for `wait` more than the timeout.
    pub fn post_frame_as(
        &mut self,
        path: &str,
        (token, proof): (&str, &str),
        frame: &[u8],
        wait: Duration,
    ) -> Result<Vec<u8>, Error> {
        self.http(path, Some(token), Body::Frame { frame, proof, wait })
    }

    /// A request to `path` with `body`, with the session of `token` if given.
    fn http(&mut self, path: &str, token: Option<&str>, body: Body<'_>) -> Result<Vec<u8>, Error> {
        let url = format!("{}{path}", self.base);
        let agent = self.agent.clone();
        let timeout = match body {
            Body::Frame { wait, .. } => self.timeout + wait,
            _ => self.timeout,
        };
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
                Body::Json(method, json) => {
                    with_headers(with_body(&agent, method, &url), &version, bearer)
                        .content_type("application/json")
                        .send(json)
                }
                Body::ProvedGet { proof } => with_headers(agent.get(&url), &version, bearer)
                    .header(PROOF_HEADER, proof)
                    .call(),
                Body::ProvedDelete { proof } => with_headers(agent.delete(&url), &version, bearer)
                    .header(PROOF_HEADER, proof)
                    .call(),
                Body::ProvedJson {
                    method,
                    json,
                    proof,
                } => with_headers(with_body(&agent, method, &url), &version, bearer)
                    .header(PROOF_HEADER, proof)
                    .content_type("application/json")
                    .send(json),
                Body::ProvedCbor {
                    method,
                    cbor,
                    proof,
                } => with_headers(with_body(&agent, method, &url), &version, bearer)
                    .header(PROOF_HEADER, proof)
                    .content_type("application/cbor")
                    .send(cbor),
                Body::Frame { frame, proof, .. } => {
                    with_headers(agent.post(&url), &version, bearer)
                        .header(PROOF_HEADER, proof)
                        .content_type("application/cbor")
                        .config()
                        .timeout_global(Some(timeout))
                        .build()
                        .send(frame)
                }
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
        let streams = self.streams();
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
            let stream = streams.open(request.uri())?;
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

    /// A new challenge of the device session of `token`, and the signature of `key` with `label`
    /// over it and the server's origin.
    fn signed_challenge(
        &mut self,
        token: &str,
        key: &SigningKey,
        label: &str,
    ) -> Result<(Vec<u8>, [u8; 64]), Error> {
        let answer = self.post_as(CHALLENGE_PATH, token)?;
        self.signed(&answer, key, label)
    }

    /// The challenge in `answer`, and the signature of `key` with `label` over it and the server's
    /// origin.
    fn signed(
        &self,
        answer: &[u8],
        key: &SigningKey,
        label: &str,
    ) -> Result<(Vec<u8>, [u8; 64]), Error> {
        let challenge = serde_json::from_slice::<Challenge>(answer)
            .ok()
            .and_then(|answer| URL_SAFE_NO_PAD.decode(answer.challenge).ok())
            .ok_or_else(|| Error::Unexpected("a challenge that is not base64url".to_owned()))?;
        let origin = origin(&self.base).expect("a server URL is an http or https URL");
        let signature = key.sign(label, &[&challenge, origin.as_bytes()]);
        Ok((challenge, signature))
    }

    /// A device session of the account of the device `device`, whose signing key is `key`, by that
    /// key and its complete certificate `cert` in CBOR, with no session
    /// (`docs/protocol/membership.md`, "Device key session"): gives its token.
    pub fn device_key_session(
        &mut self,
        device: DeviceId,
        key: &SigningKey,
        cert: &[u8],
    ) -> Result<String, Error> {
        let answer = self.http(&format!("/devices/{device}/challenge"), None, Body::Post)?;
        let (challenge, signature) = self.signed(&answer, key, DEVICE_SESSION_LABEL)?;
        let request = serde_json::json!({
            "challenge": URL_SAFE_NO_PAD.encode(&challenge),
            "signature": URL_SAFE_NO_PAD.encode(signature),
            "cert": URL_SAFE_NO_PAD.encode(cert),
        })
        .to_string();
        let path = format!("/devices/{device}/session");
        let answer = self.post_json(&path, request.as_bytes())?;
        serde_json::from_slice::<Session>(&answer)
            .map(|session| session.token)
            .map_err(|_| Error::Unexpected("a device session without its token".to_owned()))
    }

    /// The proof of an HTTPS request of the device `device`, whose signing key is `key`, on a new
    /// challenge of the device session of `token`: the value of its `Skald-Device-Proof` header.
    pub fn request_proof(
        &mut self,
        token: &str,
        device: DeviceId,
        key: &SigningKey,
    ) -> Result<String, Error> {
        let (challenge, signature) = self.signed_challenge(token, key, REQUEST_LABEL)?;
        let proof = RequestProof {
            challenge,
            device,
            signature,
        };
        Ok(URL_SAFE_NO_PAD.encode(proof.to_cbor()))
    }

    /// Opens a WebSocket at `path` as the device `device`, whose signing key is `key`, on the device
    /// session of `token`: once it is open, its first frame, `auth`, signs a new challenge of the
    /// session over the server's origin (`docs/protocol/delivery.md`, "Connecting"). A WebSocket a
    /// proxy refuses costs no challenge.
    pub fn open_websocket_as(
        &mut self,
        path: &str,
        token: &str,
        device: DeviceId,
        key: &SigningKey,
    ) -> Result<WebSocket, Error> {
        let mut socket = self.open_websocket(path)?;
        let (challenge, signature) = self.signed_challenge(token, key, AUTH_LABEL)?;
        let auth = Frame::Auth {
            challenge,
            device,
            signature,
        };
        socket
            .send(Message::Binary(auth.to_cbor().into()))
            .map_err(|err| match err {
                tungstenite::Error::Io(err) => io_error(err, self.timeout),
                err => Error::Unreachable(err.to_string()),
            })?;
        Ok(socket)
    }

    /// Opens the event stream at `path` as the device of `proof`, on the device session of `token`,
    /// stating `cursor` in `Last-Event-ID` (`docs/protocol/delivery.md`, "Frames"). The connect
    /// and the answer's head fail after the connection's timeout.
    pub fn open_events_as(
        &mut self,
        path: &str,
        (token, proof): (&str, &str),
        cursor: u64,
    ) -> Result<Events, Error> {
        let uri: Uri = format!("{}{path}", self.base)
            .parse()
            .map_err(|err: tungstenite::http::uri::InvalidUri| Error::Url(err.to_string()))?;
        let timeout = self.timeout;
        let streams = self.streams();
        self.exchange(|version| {
            let stream = streams.open(&uri)?;
            let target = uri.path_and_query().map_or("/", |target| target.as_str());
            let authority = uri.authority().map_or("", |authority| authority.as_str());
            let request = format!(
                "GET {target} HTTP/1.1\r\nhost: {authority}\r\naccept: text/event-stream\r\n\
                 cache-control: no-cache\r\n{VERSION_HEADER}: {version}\r\n\
                 authorization: Bearer {token}\r\n{PROOF_HEADER}: {proof}\r\n\
                 last-event-id: {cursor}\r\n\r\n"
            );
            let mut events = Events::new(stream);
            events
                .send(request.as_bytes(), timeout)
                .map_err(|err| io_error(err, timeout))?;
            let (status, headers) = events.head(timeout)?;
            if status != 200 {
                return Ok(Answer::Refused {
                    status,
                    content: content(&headers),
                    body: events.body(timeout)?,
                });
            }
            let media = headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            if !media.starts_with("text/event-stream") {
                return Err(Error::Unexpected(format!(
                    "an answer of {} that is not an event stream",
                    printable(media)
                )));
            }
            Ok(Answer::Served(events, latest(&headers)))
        })
    }

    /// What opens the connection's WebSockets and event streams.
    fn streams(&self) -> Streams {
        Streams {
            tls: self.tls.clone(),
            resolver: self.resolver.clone(),
            proxy: self.proxy.clone(),
            agent: self.agent.clone(),
            timeout: self.timeout,
        }
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
                task,
            } = problem;
            let speaks = self.speaks;
            if kind == VersionProblem::TooOld.type_uri() {
                return Err(Error::TooOld { supported, speaks });
            }
            if kind != VersionProblem::TooNew.type_uri() {
                if status == 401 && kind == NOT_SIGNED_IN {
                    note_refused();
                }
                return Err(Error::Refused {
                    status,
                    kind,
                    detail,
                    task,
                });
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

/// A request to `url` by `method`, which has a body.
fn with_body(
    agent: &ureq::Agent,
    method: Method,
    url: &str,
) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
    match method {
        Method::Post => agent.post(url),
        Method::Put => agent.put(url),
        Method::Patch => agent.patch(url),
    }
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
pub(crate) mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::thread;

    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

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

    /// Control characters and the line and paragraph separators are escaped; other text is kept.
    #[test]
    fn printable_escapes_control_characters_and_line_separators() {
        assert_eq!(
            printable("a\nb\u{1b}c\u{85}d\u{2028}e\u{2029}f · é"),
            "a\\nb\\u{1b}c\\u{85}d\\u{2028}e\\u{2029}f · é"
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
    fn origin_is_the_server_urls_without_a_path_or_its_schemes_port() {
        for (base, expected) in [
            ("https://api.skald.so", "https://api.skald.so"),
            ("HTTPS://API.Skald.so:443/base", "https://api.skald.so"),
            ("https://api.skald.so:8443", "https://api.skald.so:8443"),
            ("http://localhost:80", "http://localhost"),
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("http://[::1]:8080", "http://[::1]:8080"),
            ("http://127.0.0.1:9/?a#b", "http://127.0.0.1:9"),
        ] {
            assert_eq!(origin(base).as_deref(), Some(expected), "{base}");
        }
        for url in [
            "https://",
            "ftp://skald.test",
            "skald.test",
            "javascript:alert(1)",
        ] {
            assert_eq!(origin(url), None, "{url}");
        }
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

        // The connection's requests a listener took, by their first lines. Another test process may
        // connect to a port it freed that a listener here then got: its connection is no request
        // of this connection's.
        let requests = |listener: &TcpListener| {
            listener.set_nonblocking(true).unwrap();
            std::iter::from_fn(|| listener.accept().ok())
                .map(|(stream, _)| {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(SECOND)).unwrap();
                    let mut line = String::new();
                    let _ = BufReader::new(stream).read_line(&mut line);
                    line.trim_end().to_owned()
                })
                .filter(|line| line.starts_with("GET /status ") || line.starts_with("GET /ws "))
                .collect::<Vec<_>>()
        };
        let mut at_server = requests(&server);
        at_server.sort();
        assert_eq!(at_server, ["GET /status HTTP/1.1", "GET /ws HTTP/1.1"]);
        assert_eq!(requests(&elsewhere), Vec::<String>::new());
    }

    /// What the stand-in server saw: the challenge request's line and headers, in lowercase, and
    /// the WebSocket's first frame.
    struct Seen {
        request: Vec<String>,
        first: Vec<u8>,
    }

    /// A stand-in server that takes one WebSocket, selecting the subprotocol it offers, answers one
    /// challenge request with `challenge`, then reads the WebSocket's first frame.
    fn challenging(challenge: &[u8]) -> (String, thread::JoinHandle<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("the address"));
        let (_, answer) = challenge_answer(challenge);
        let served = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the WebSocket");
            let mut socket = tungstenite::accept_hdr(stream, select).expect("a WebSocket");
            let (stream, _) = listener.accept().expect("the challenge request");
            let request = take(&stream).head;
            (&stream).write_all(answer.as_bytes()).expect("answer");
            drop(stream);
            let first = match socket.read().expect("a frame") {
                Message::Binary(bytes) => bytes.to_vec(),
                other => panic!("not a binary frame: {other:?}"),
            };
            Seen { request, first }
        });
        (url, served)
    }

    /// Selects the subprotocol `request` offers.
    #[expect(clippy::result_large_err, reason = "tungstenite's callback")]
    fn select(request: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
        let offered = request.headers()["sec-websocket-protocol"].clone();
        response
            .headers_mut()
            .insert("sec-websocket-protocol", offered);
        Ok(response)
    }

    /// A device opens its WebSocket, asks for a challenge under the server's path with its
    /// session's token, then sends `auth` first: the device, signed with its key over the challenge
    /// and the server's origin, without the path of its URL.
    #[test]
    fn websocket_as_a_device_proves_its_key_on_a_challenge() {
        const TOKEN: &str = "token-1";
        let challenge = [0x3c; 32];
        let (origin, served) = challenging(&challenge);
        let key = SigningKey::generate();
        let id = DeviceId::from_signing_key(&key.public_key());
        let server = format!("{origin}/skald");
        let mut connection =
            Connection::new(&server, VersionRange::default(), 10 * SECOND).expect("a connection");

        let socket = connection
            .open_websocket_as("/ws", TOKEN, id, &key)
            .expect("a WebSocket");

        let Seen { request, first } = served.join().unwrap();
        drop(socket);
        assert_eq!(request[0], "post /skald/session/challenge http/1.1");
        assert!(
            request.contains(&format!("authorization: bearer {TOKEN}")),
            "{request:?}"
        );
        let Frame::Auth {
            challenge: signed,
            device,
            signature,
        } = Frame::from_cbor(&first).expect("a frame")
        else {
            panic!("not auth: {first:?}");
        };
        assert_eq!((signed.as_slice(), device), (challenge.as_slice(), id));
        skald_crypto::verify(
            &key.public_key(),
            AUTH_LABEL,
            &[&challenge, origin.as_bytes()],
            &signature,
        )
        .expect("signed over the challenge and the origin");
    }

    /// A request a stand-in server took: its line and headers, in lowercase, and its body.
    struct Taken {
        head: Vec<String>,
        body: Vec<u8>,
    }

    /// A connection to a server that gives `answers`, each at once, one per request.
    fn answered<const N: usize>(
        answers: [String; N],
    ) -> (Connection, thread::JoinHandle<Vec<Taken>>) {
        let (origin, served) = serving(answers.map(|it| (Duration::ZERO, it)).into());
        let connection =
            Connection::new(&origin, VersionRange::default(), 10 * SECOND).expect("a connection");
        (connection, served)
    }

    /// A stand-in server that answers each request in turn with the next of `answers`, a head and
    /// body after a pause; gives what it took.
    fn serving(answers: Vec<(Duration, String)>) -> (String, thread::JoinHandle<Vec<Taken>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let served = thread::spawn(move || {
            (answers.into_iter())
                .map(|(pause, answer)| {
                    let (stream, _) = listener.accept().expect("a request");
                    let request = take(&stream);
                    thread::sleep(pause);
                    // The client may have left.
                    let _ = (&stream).write_all(answer.as_bytes());
                    request
                })
                .collect()
        });
        (origin, served)
    }

    /// The request on `stream`, its body by its `content-length`.
    fn take(stream: &TcpStream) -> Taken {
        let mut reader = BufReader::new(stream);
        let mut head: Vec<String> = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("a request line");
            if line == "\r\n" {
                break;
            }
            head.push(line.trim_end().to_ascii_lowercase());
        }
        let length = (head.iter())
            .find_map(|line| line.strip_prefix("content-length: "))
            .map_or(0, |length| length.parse().expect("a length"));
        let mut body = vec![0; length];
        reader.read_exact(&mut body).expect("the body");
        Taken { head, body }
    }

    /// An answer with the JSON `json`.
    fn json(json: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{json}",
            json.len()
        )
    }

    /// A challenge's answer.
    fn challenge_answer(challenge: &[u8]) -> (Duration, String) {
        let body = format!(r#"{{"challenge":"{}"}}"#, URL_SAFE_NO_PAD.encode(challenge));
        (Duration::ZERO, json(&body))
    }

    /// An HTTPS request of a device's mailbox carries its frame as CBOR and, in
    /// `Skald-Device-Proof`, the device, signed with its key, with the label of requests, over a
    /// new challenge of its session and the server's origin.
    #[test]
    fn mailbox_request_proves_the_device_on_a_challenge() {
        const TOKEN: &str = "token-2";
        let challenge = [0x5a; 32];
        let answer = "HTTP/1.1 200 OK\r\ncontent-type: application/cbor\r\ncontent-length: 3\r\n\
                      connection: close\r\n\r\nabc"
            .to_owned();
        let (origin, served) =
            serving(vec![challenge_answer(&challenge), (Duration::ZERO, answer)]);
        let key = SigningKey::generate();
        let id = DeviceId::from_signing_key(&key.public_key());
        let mut connection =
            Connection::new(&origin, VersionRange::default(), 10 * SECOND).expect("a connection");

        let proof = connection.request_proof(TOKEN, id, &key).expect("a proof");
        let body = connection
            .post_frame_as("/mailbox/send", (TOKEN, &proof), b"frame", Duration::ZERO)
            .expect("an answer");

        assert_eq!(body, b"abc");
        let taken = served.join().unwrap();
        assert_eq!(taken[0].head[0], "post /session/challenge http/1.1");
        let request = &taken[1];
        assert_eq!(request.head[0], "post /mailbox/send http/1.1");
        assert_eq!(request.body, b"frame");
        for header in [
            "content-type: application/cbor",
            "authorization: bearer token-2",
        ] {
            assert!(request.head.iter().any(|line| line == header), "{header}");
        }
        let sent = (request.head.iter())
            .find_map(|line| line.strip_prefix("skald-device-proof: "))
            .expect("a proof");
        assert_eq!(sent, proof.to_ascii_lowercase());
        let decoded = RequestProof::from_cbor(&URL_SAFE_NO_PAD.decode(&proof).unwrap()).unwrap();
        assert_eq!(
            (decoded.challenge.as_slice(), decoded.device),
            (challenge.as_slice(), id)
        );
        skald_crypto::verify(
            &key.public_key(),
            REQUEST_LABEL,
            &[&challenge, origin.as_bytes()],
            &decoded.signature,
        )
        .expect("signed over the challenge and the origin");
    }

    /// A long-poll's answer is awaited for its wait more than the timeout, and no longer.
    #[test]
    fn mailbox_request_awaits_its_wait_more_than_the_timeout() {
        let late = || {
            let answer = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
            (SECOND / 2, answer.to_owned())
        };
        let (origin, served) = serving(vec![late(), late()]);
        let mut connection =
            Connection::new(&origin, VersionRange::default(), SECOND / 4).expect("a connection");
        let poll = |connection: &mut Connection, wait| {
            connection.post_frame_as("/mailbox/poll?wait=1", ("token", "proof"), b"", wait)
        };

        poll(&mut connection, SECOND).expect("answered within the wait");
        let error = poll(&mut connection, Duration::ZERO).expect_err("not within the timeout");

        assert!(matches!(error, Error::Timeout(_)), "{error:?}");
        served.join().unwrap();
    }

    /// The event stream states the cursor in `Last-Event-ID` with the device's proof, and gives the
    /// frames of its events; a refusal that is not the server's problem is unexpected, and so is
    /// an answer that is not an event stream.
    #[test]
    fn event_stream_states_the_cursor_and_gives_frames() {
        let stream = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\
             \r\n{:x}\r\ndata: {}\n\n\r\n",
            "data: \n\n".len() + URL_SAFE_NO_PAD.encode(b"frame").len(),
            URL_SAFE_NO_PAD.encode(b"frame")
        );
        let forbidden = "HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain\r\n\
                         content-length: 7\r\n\r\nblocked"
            .to_owned();
        let page = "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: 0\r\n\r\n";
        let (mut connection, served) = answered([stream, forbidden, page.to_owned()]);
        let open = |connection: &mut Connection| {
            connection.open_events_as("/mailbox/events", ("token", "proof"), 7)
        };

        let mut events = open(&mut connection).expect("a stream");
        assert_eq!(events.next(10 * SECOND).unwrap(), Some(b"frame".to_vec()));
        let refused = open(&mut connection).err().expect("refused");
        let page = open(&mut connection).err().expect("not a stream");

        assert!(
            matches!(&refused, Error::Unexpected(why) if why == "HTTP 403"),
            "{refused:?}"
        );
        assert!(matches!(page, Error::Unexpected(_)), "{page:?}");
        let taken = served.join().unwrap();
        let head = &taken[0].head;
        assert_eq!(head[0], "get /mailbox/events http/1.1");
        for header in [
            "accept: text/event-stream",
            "last-event-id: 7",
            "skald-device-proof: proof",
            "authorization: bearer token",
        ] {
            assert!(head.iter().any(|line| line == header), "{header}: {head:?}");
        }
    }

    /// Only the server's `not-signed-in` notes a refused session, until it is forgotten: not
    /// another refusal with 401, nor another problem.
    #[test]
    fn only_not_signed_in_notes_a_refused_session() {
        let problem = |status: &str, kind: &str| {
            let body = format!(r#"{{"type":"{kind}","title":"t","status":0}}"#);
            format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/problem+json\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        let answers = [
            problem("401 Unauthorized", "https://skald.so/problems/other"),
            problem("403 Forbidden", NOT_SIGNED_IN),
            problem("401 Unauthorized", NOT_SIGNED_IN),
        ];
        let (mut connection, served) = answered(answers);
        forget_refused();

        for noted in [false, false, true] {
            let refused = connection.get_as("/session", "token").err();
            assert!(
                matches!(refused, Some(Error::Refused { .. })),
                "{refused:?}"
            );
            assert_eq!(session_refused(), noted, "{refused:?}");
        }
        forget_refused();
        assert!(!session_refused());
        served.join().unwrap();
    }

    /// A connection over this machine: the client's end and its peer's.
    pub(crate) fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let stream = TcpStream::connect(listener.local_addr().unwrap()).expect("connect");
        let (peer, _) = listener.accept().expect("accept");
        stream.set_read_timeout(Some(5 * SECOND)).unwrap();
        (stream, peer)
    }

    /// A [`Socket`] on `stream`.
    fn socket(stream: TcpStream) -> Socket {
        Socket {
            stream,
            buffers: LazyBuffers::new(1024, 1024),
        }
    }

    /// Waits until a byte or the end of `stream` has come, taking nothing from it.
    fn arrived(stream: &TcpStream) {
        stream.peek(&mut [0]).expect("a byte or the end");
    }

    /// The connections of HTTP requests send each segment at once, as `TCP_NODELAY` says, unless
    /// the agent's config turns it off.
    #[test]
    fn requests_go_without_delay_unless_the_config_says_otherwise() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let uri: Uri = format!("http://{}/status", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let dial = Dial {
            proxy: None,
            resolver: system(),
            timeout: SECOND,
        };
        let delayed = ureq::Agent::config_builder().no_delay(false).build();

        let default = dial
            .socket(&uri, &ureq::config::Config::default())
            .expect("a socket");
        let off = dial.socket(&uri, &delayed).expect("a socket");

        assert!(default.stream.nodelay().unwrap());
        assert!(!off.stream.nodelay().unwrap());
    }

    /// A connection takes another request only while idle: not once its peer closed it or sent
    /// bytes nobody asked for.
    #[test]
    fn socket_is_open_only_while_idle() {
        let (stream, _peer) = pair();
        let mut idle = socket(stream);
        assert!(idle.is_open());
        assert!(idle.is_open());

        let (stream, mut peer) = pair();
        let mut stray = socket(stream);
        peer.write_all(b"x").unwrap();
        arrived(&stray.stream);
        assert!(!stray.is_open());

        let (stream, peer) = pair();
        let mut closed = socket(stream);
        drop(peer);
        arrived(&closed.stream);
        assert!(!closed.is_open());
    }

    /// Each read appends what has come to what came before; at the end of the connection it makes
    /// no progress.
    #[test]
    fn socket_takes_a_head_in_two_reads() {
        const LINE: &[u8] = b"HTTP/1.1 200 OK\r\n";
        let (stream, mut peer) = pair();
        let mut socket = socket(stream);
        let within = NextTimeout {
            after: (5 * SECOND).into(),
            reason: ureq::Timeout::PerRead,
            ..NextTimeout::default()
        };

        peer.write_all(LINE).unwrap();
        assert!(socket.await_input(within).expect("the first read"));
        assert_eq!(socket.buffers.input(), LINE);
        peer.write_all(b"\r\n").unwrap();
        assert!(socket.await_input(within).expect("the second read"));
        assert_eq!(socket.buffers.input(), b"HTTP/1.1 200 OK\r\n\r\n");
        drop(peer);
        assert!(!socket.await_input(within).expect("the end"));
        assert_eq!(socket.buffers.input(), b"HTTP/1.1 200 OK\r\n\r\n");
    }

    /// A read a signal interrupts is read again; any other error ends it.
    #[test]
    fn interrupted_reads_are_read_again() {
        let mut reads = vec![Ok(3), Err(io::Error::from(io::ErrorKind::Interrupted))];
        let read = uninterrupted(|| reads.pop().expect("one read too many"));
        assert_eq!(read.expect("the second read"), 3);

        let mut reads: Vec<io::Result<usize>> =
            vec![Err(io::Error::from(io::ErrorKind::ConnectionReset))];
        let read = uninterrupted(|| reads.pop().expect("one read too many"));
        assert_eq!(
            read.expect_err("a reset").kind(),
            io::ErrorKind::ConnectionReset
        );
    }

    /// A proxy's answer head of [`PROXY_HEAD_LIMIT`] bytes is read, and what follows it stays in
    /// the tunnel; a longer one is refused.
    #[test]
    fn proxy_head_is_read_up_to_its_limit_and_no_further() {
        let head = |length: usize| {
            let (start, end) = ("HTTP/1.1 200 OK\r\nPadding: ", "\r\n\r\n");
            let padding = "a".repeat(length - start.len() - end.len());
            format!("{start}{padding}{end}")
        };
        let sent = |mut peer: TcpStream, bytes: String| {
            thread::spawn(move || {
                // The client stops reading a head that is too long.
                let _ = peer.write_all(bytes.as_bytes());
                peer
            })
        };

        let (mut stream, peer) = pair();
        let longest = head(PROXY_HEAD_LIMIT);
        let writer = sent(peer, format!("{longest}tunnel"));
        assert_eq!(proxy_head(&mut stream).expect("the head"), longest);
        let mut after = [0; 6];
        stream.read_exact(&mut after).expect("the tunnel");
        assert_eq!(&after, b"tunnel");
        writer.join().unwrap();

        let (mut stream, peer) = pair();
        let writer = sent(peer, head(PROXY_HEAD_LIMIT + 1));
        let error = proxy_head(&mut stream).expect_err("too long");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        writer.join().unwrap();
    }

    /// A stand-in proxy that answers one `CONNECT` with the head `answer`.
    fn answering(answer: &'static str) -> (Proxy, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let proxy = Proxy::parse(&url).expect("a proxy URL");
        let served = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the CONNECT");
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            while reader.read_line(&mut line).expect("a request line") > 0 && line != "\r\n" {
                line.clear();
            }
            (&stream).write_all(answer.as_bytes()).expect("answer");
        });
        (proxy, served)
    }

    /// The error of an HTTP request through `proxy` to a server that only the proxy could reach.
    fn through(proxy: Proxy) -> Error {
        let mut connection = Connection::proxied(
            "https://skald.test:8443",
            VersionRange::default(),
            10 * SECOND,
            Arc::new(DefaultResolver::default()),
            Some(proxy),
        )
        .expect("a connection");
        let error = connection.get("/status").expect_err("no tunnel");
        assert!(!error.reached(), "{error:?}");
        error
    }

    /// A proxy that refuses the tunnel, asks for credentials or is down fails an HTTP request with
    /// a message naming it.
    #[test]
    fn proxy_that_gives_no_tunnel_fails_a_request_naming_it() {
        let failed =
            |address: &str| format!("cannot reach the server through the proxy {address}: ");

        let (proxy, served) = answering("HTTP/1.1 403 Forbidden\r\n\r\n");
        let address = proxy.address();
        let error = through(proxy);
        served.join().unwrap();
        assert_eq!(
            error.to_string(),
            format!(
                "{}it refused the tunnel with 403 Forbidden",
                failed(&address)
            )
        );

        let (proxy, served) = answering(
            "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Negotiate\r\n\r\n",
        );
        let address = proxy.address();
        let error = through(proxy);
        served.join().unwrap();
        assert_eq!(
            error.to_string(),
            format!(
                "{}it refused the tunnel with 407 Proxy Authentication Required; the client \
                 answers it only by Basic, with the credentials of its URL",
                failed(&address)
            )
        );

        let closed = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let address = closed.local_addr().unwrap().to_string();
        drop(closed);
        let error = through(Proxy::parse(&format!("http://{address}")).expect("a proxy URL"));
        assert!(
            matches!(&error, Error::Proxy { proxy, .. } if *proxy == address),
            "{error:?}"
        );
        assert!(error.to_string().starts_with(&failed(&address)), "{error}");
    }
}
