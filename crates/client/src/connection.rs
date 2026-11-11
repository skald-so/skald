//! The client's connection to a Skald server: it states the protocol version on every request and
//! WebSocket opening request and acts on the answers as `docs/protocol/README.md`, "Server answers",
//! says.

use std::fmt;
use std::net::TcpStream;
use std::num::NonZeroU64;
use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;
use skald_proto::{LATEST_HEADER, PROTOCOL_VERSION, VERSION_HEADER, VersionProblem, parse_version};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::{HeaderMap, HeaderValue, header};
use tungstenite::stream::MaybeTlsStream;

/// How long a request to the server may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// An open WebSocket to the server.
pub type WebSocket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

/// The protocol versions the client speaks: `oldest` to `latest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versions {
    oldest: NonZeroU64,
    latest: NonZeroU64,
}

impl Versions {
    /// The newest version the client speaks among `supported`.
    fn best_of(self, supported: &[u64]) -> Option<NonZeroU64> {
        supported
            .iter()
            .filter(|&&version| (self.oldest.get()..=self.latest.get()).contains(&version))
            .max()
            .and_then(|&version| NonZeroU64::new(version))
    }

    fn list(self) -> Vec<u64> {
        (self.oldest.get()..=self.latest.get()).collect()
    }
}

/// [`PROTOCOL_VERSION`] and the version before it; version 1 alone while it is the only one.
impl Default for Versions {
    fn default() -> Self {
        Versions {
            oldest: NonZeroU64::new(PROTOCOL_VERSION.get() - 1).unwrap_or(PROTOCOL_VERSION),
            latest: PROTOCOL_VERSION,
        }
    }
}

/// `<n>`, or `<oldest>-<latest>`, such as `2-3`.
impl FromStr for Versions {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let version = |text: &str| {
            parse_version(text)
                .ok_or_else(|| format!("{text:?} is not a protocol version, an integer from 1"))
        };
        let (oldest, latest) = match text.split_once('-') {
            Some((oldest, latest)) => (version(oldest)?, version(latest)?),
            None => (version(text)?, version(text)?),
        };
        if oldest > latest {
            return Err(format!(
                "the oldest version, {oldest}, is above the latest, {latest}"
            ));
        }
        Ok(Versions { oldest, latest })
    }
}

/// Why a request to the server failed.
#[derive(Debug)]
pub enum Error {
    /// The server URL is not one the client connects to.
    Url(String),
    /// No answer from the server.
    Unreachable(String),
    /// The server no longer accepts any version the client speaks: `protocol-too-old`.
    TooOld {
        supported: Vec<u64>,
        speaks: Versions,
    },
    /// The server accepts only versions older than any the client speaks: `protocol-too-new`.
    ServerOlder {
        supported: Vec<u64>,
        speaks: Versions,
    },
    /// The server refused the request for another reason, given in `detail`.
    Refused { status: u16, detail: String },
    /// An answer the protocol does not define.
    Unexpected(String),
}

impl Error {
    /// Whether the server answered.
    pub fn reached(&self) -> bool {
        !matches!(self, Error::Url(_) | Error::Unreachable(_))
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
                "{url:?} is not a server URL the client connects to: it must start with http://"
            ),
            Error::Unreachable(cause) => write!(f, "cannot reach the server: {cause}"),
            Error::TooOld { supported, speaks } => write!(
                f,
                "please update skald: the server accepts {}; this skald speaks {}",
                describe(supported),
                describe(&speaks.list())
            ),
            Error::ServerOlder { supported, speaks } => write!(
                f,
                "the server is older than this skald: it accepts {}; this skald speaks {}",
                describe(supported),
                describe(&speaks.list())
            ),
            Error::Refused { status, detail } => {
                write!(f, "the server refused the request with {status}: {detail}")
            }
            Error::Unexpected(answer) => write!(f, "unexpected answer from the server: {answer}"),
        }
    }
}

/// `protocol version 3`, `protocol versions 2 and 3`, `protocol versions 1, 2 and 3`.
fn describe(versions: &[u64]) -> String {
    let text: Vec<String> = versions.iter().map(u64::to_string).collect();
    match text.as_slice() {
        [] => "no protocol version".to_owned(),
        [only] => format!("protocol version {only}"),
        [rest @ .., last] => format!("protocol versions {} and {last}", rest.join(", ")),
    }
}

/// A connection to the server at one base URL.
pub struct Connection {
    base: String,
    speaks: Versions,
    agent: ureq::Agent,
    /// The version the next request states: the newest the client speaks until the server answers
    /// that it is too new.
    version: NonZeroU64,
    /// The server's current version, from its last answer.
    latest: Option<NonZeroU64>,
}

/// The server's answer to one request.
enum Answer<T> {
    /// The server served the request.
    Served(T, Option<NonZeroU64>),
    /// The server refused it.
    Refused {
        status: u16,
        problem_json: bool,
        body: Vec<u8>,
    },
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
    /// A connection to the server at `server`, such as `http://localhost:8080`, speaking `speaks`.
    pub fn new(server: &str, speaks: Versions) -> Result<Self, Error> {
        if !server.starts_with("http://") {
            return Err(Error::Url(server.to_owned()));
        }
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .new_agent();
        Ok(Connection {
            base: server.trim_end_matches('/').to_owned(),
            speaks,
            agent,
            version: speaks.latest,
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

    /// `GET <path>`; the body of the answer that serves it.
    pub fn get(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        let url = format!("{}{path}", self.base);
        let agent = self.agent.clone();
        self.exchange(|version| {
            let mut response = agent
                .get(&url)
                .header(VERSION_HEADER, version.to_string())
                .call()
                .map_err(|err| Error::Unreachable(err.to_string()))?;
            let status = response.status().as_u16();
            let latest = latest(response.headers());
            let problem_json = is_problem(response.headers());
            let body = response
                .body_mut()
                .read_to_vec()
                .map_err(|err| Error::Unreachable(err.to_string()))?;
            Ok(if response.status().is_success() {
                Answer::Served(body, latest)
            } else {
                Answer::Refused {
                    status,
                    problem_json,
                    body,
                }
            })
        })
    }

    /// Opens a WebSocket at `path`, offering the subprotocol `skald.<n>`.
    pub fn open_websocket(&mut self, path: &str) -> Result<WebSocket, Error> {
        let url = format!("{}{path}", self.base.replacen("http://", "ws://", 1));
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
            match tungstenite::connect(request) {
                Ok((socket, response)) => Ok(Answer::Served(socket, latest(response.headers()))),
                Err(tungstenite::Error::Http(response)) => Ok(Answer::Refused {
                    status: response.status().as_u16(),
                    problem_json: is_problem(response.headers()),
                    body: response.body().clone().unwrap_or_default(),
                }),
                Err(tungstenite::Error::Io(err)) => Err(Error::Unreachable(err.to_string())),
                Err(err) => Err(Error::Unexpected(err.to_string())),
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
            let (status, problem_json, body) = match send(self.version)? {
                Answer::Served(value, latest) => {
                    self.latest = latest;
                    return Ok(value);
                }
                Answer::Refused {
                    status,
                    problem_json,
                    body,
                } => (status, problem_json, body),
            };
            let problem = problem_json
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
            match speaks
                .best_of(&supported)
                .filter(|&version| version < self.version)
            {
                Some(version) => self.version = version,
                None => return Err(Error::ServerOlder { supported, speaks }),
            }
        }
    }
}

/// The server's current version, from `Skald-Protocol-Latest`.
fn latest(headers: &HeaderMap) -> Option<NonZeroU64> {
    parse_version(headers.get(LATEST_HEADER)?.to_str().ok()?)
}

/// Whether the body is a problem details object.
fn is_problem(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/problem+json"))
}
