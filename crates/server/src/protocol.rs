//! The protocol version check: `docs/protocol/README.md`, "Server answers".

use std::num::NonZeroU64;
use std::str::FromStr;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use serde::Serialize;
use skald_proto::{
    LATEST_HEADER, SUBPROTOCOL_PREFIX, VERSION_HEADER, VersionProblem, VersionRange, parse_version,
};

/// The protocol versions the server accepts: its current version and, optionally, the previous one.
/// The default is [`VersionRange`]'s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Versions(VersionRange);

impl Versions {
    /// The accepted versions, ascending.
    pub fn supported(self) -> Vec<u64> {
        self.0.list()
    }

    /// Whether the server accepts `version`, and if not, why.
    fn check(self, version: NonZeroU64) -> Result<(), VersionProblem> {
        if version < self.0.oldest() {
            Err(VersionProblem::TooOld)
        } else if version > self.0.latest() {
            Err(VersionProblem::TooNew)
        } else {
            Ok(())
        }
    }
}

/// A [`VersionRange`] of at most two adjacent versions, such as `2-3`.
impl FromStr for Versions {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let range: VersionRange = text.parse()?;
        let (oldest, latest) = (range.oldest(), range.latest());
        if latest.get() - oldest.get() > 1 {
            return Err(format!(
                "{oldest}-{latest} spans more than two versions: the server accepts its current version and the previous one"
            ));
        }
        Ok(Versions(range))
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
        .insert(LATEST_HEADER, HeaderValue::from(versions.0.latest().get()));
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
    let status = StatusCode::BAD_REQUEST;
    let problem = Problem {
        kind: problem.type_uri(),
        title,
        status: status.as_u16(),
        detail: format!(
            "This server accepts {}; the request stated {stated}.",
            versions.0
        ),
        supported: versions.supported(),
    };
    crate::problem_details(status, problem)
}
