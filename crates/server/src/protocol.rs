//! The protocol version check: `docs/protocol/README.md`, "Server answers".

use std::num::NonZeroU64;
use std::str::FromStr;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use skald_proto::{
    LATEST_HEADER, PROTOCOL_VERSION, SUBPROTOCOL_PREFIX, VERSION_HEADER, VersionProblem,
    parse_version,
};

/// The protocol versions the server accepts: its current version and, optionally, the previous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versions {
    oldest: NonZeroU64,
    latest: NonZeroU64,
}

impl Versions {
    /// The accepted versions, ascending.
    pub fn supported(self) -> Vec<u64> {
        (self.oldest.get()..=self.latest.get()).collect()
    }

    /// Whether the server accepts `version`, and if not, why.
    fn check(self, version: NonZeroU64) -> Result<(), VersionProblem> {
        if version < self.oldest {
            Err(VersionProblem::TooOld)
        } else if version > self.latest {
            Err(VersionProblem::TooNew)
        } else {
            Ok(())
        }
    }
}

/// [`PROTOCOL_VERSION`] and the version before it; version 1 alone while it is the only one.
impl Default for Versions {
    fn default() -> Self {
        Versions {
            oldest: NonZeroU64::new(PROTOCOL_VERSION.get().saturating_sub(1))
                .unwrap_or(PROTOCOL_VERSION),
            latest: PROTOCOL_VERSION,
        }
    }
}

/// `<n>`, or `<oldest>-<latest>` for two adjacent versions, such as `2-3`.
impl FromStr for Versions {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let version = |text: &str| {
            text.parse::<NonZeroU64>()
                .map_err(|_| format!("{text:?} is not a protocol version, an integer from 1"))
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
        if latest.get() - oldest.get() > 1 {
            return Err(format!(
                "{oldest}-{latest} spans more than two versions: the server accepts its current version and the previous one"
            ));
        }
        Ok(Versions { oldest, latest })
    }
}

/// Where a request states its protocol version.
#[derive(Debug, Clone, Copy)]
pub enum Statement {
    /// HTTPS: the `Skald-Protocol-Version` header.
    Header,
    /// A WebSocket opening request: the offered subprotocol `skald.<n>`.
    Subprotocol,
}

/// The version an accepted request states; the check adds it to the request's extensions.
#[derive(Debug, Clone, Copy)]
pub struct Stated(pub NonZeroU64);

/// Middleware: passes on a request that states a version in `versions`, and refuses any other.
/// Every response carries `Skald-Protocol-Latest`.
pub async fn check(
    State((versions, statement)): State<(Versions, Statement)>,
    mut request: Request,
    next: Next,
) -> Response {
    let stated = match statement {
        Statement::Header => header_version(request.headers()),
        Statement::Subprotocol => subprotocol_version(request.headers()),
    };
    let mut response = match stated {
        None => refusal(versions, VersionProblem::Invalid, "no valid version"),
        Some((version, text)) => match versions.check(version) {
            Ok(()) => {
                request.extensions_mut().insert(Stated(version));
                next.run(request).await
            }
            Err(problem) => refusal(versions, problem, &text),
        },
    };
    response
        .headers_mut()
        .insert(LATEST_HEADER, HeaderValue::from(versions.latest.get()));
    response
}

/// The version in the only `Skald-Protocol-Version` header, with its text.
fn header_version(headers: &HeaderMap) -> Option<(NonZeroU64, String)> {
    let mut values = headers.get_all(VERSION_HEADER).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return None;
    };
    let text = value.to_str().ok()?;
    Some((parse_version(text)?, text.to_owned()))
}

/// The version in the only offered subprotocol `skald.<n>`, with its text.
fn subprotocol_version(headers: &HeaderMap) -> Option<(NonZeroU64, String)> {
    let mut offers = headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|offer| offer.trim().strip_prefix(SUBPROTOCOL_PREFIX));
    let (Some(text), None) = (offers.next(), offers.next()) else {
        return None;
    };
    Some((parse_version(text)?, text.to_owned()))
}

/// `ProtocolProblem` in the OpenAPI spec.
#[derive(Serialize)]
struct Problem {
    #[serde(rename = "type")]
    kind: String,
    title: &'static str,
    status: u16,
    detail: String,
    supported: Vec<u64>,
}

/// The answer to a request whose version `versions` does not include; `stated` is what it states.
fn refusal(versions: Versions, problem: VersionProblem, stated: &str) -> Response {
    let title = match problem {
        VersionProblem::TooOld => "Please update skald",
        VersionProblem::TooNew => "This server is older than the client",
        VersionProblem::Invalid => "Missing or invalid protocol version",
    };
    let Versions { oldest, latest } = versions;
    let accepts = if oldest == latest {
        format!("protocol version {latest}")
    } else {
        format!("protocol versions {oldest} and {latest}")
    };
    let status = StatusCode::BAD_REQUEST;
    let problem = Problem {
        kind: problem.type_uri(),
        title,
        status: status.as_u16(),
        detail: format!("This server accepts {accepts}; the request stated {stated}."),
        supported: versions.supported(),
    };
    (
        status,
        [(header::CONTENT_TYPE, "application/problem+json")],
        Json(problem),
    )
        .into_response()
}
