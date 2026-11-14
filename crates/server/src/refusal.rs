//! Why the JSON API refuses a request, and the steps its handlers share: the session's account and
//! the request's body.

use std::collections::HashMap;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::de::{DeserializeOwned, IgnoredAny};
use sqlx::PgPool;

use crate::sign_in::{not_signed_in, signed_in};
use crate::{AppState, database_failed, problem};

/// Why the server refuses a request.
pub(crate) enum Refusal {
    NotSignedIn,
    /// A malformed or invalid body, for the reason it holds.
    Invalid(String),
    /// A certificate the server does not take, for the reason it holds.
    Cert(String),
    /// A username that is no `name`, for the reason it holds.
    Username(String),
    DeviceTaken,
    MasterKeyTaken,
    UsernameReserved,
    UsernameTaken,
    /// No such space, or the session's account does not get it.
    NoSpace,
    /// The session's account may not do this in the space, for the reason it holds.
    NotAllowed(String),
    /// A roster revision the server does not store, for the reason it holds.
    Roster(String),
    SpaceNameTaken,
    /// The account owns as many spaces as it may: the number it holds.
    OwnedSpacesLimit(u32),
    /// A roster revision out of order or with a taken number, for the reason it holds.
    RevisionConflict(String),
    /// The database failed at what it holds, such as `read a session`.
    Database(&'static str, sqlx::Error),
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let bad = StatusCode::BAD_REQUEST;
        let conflict = StatusCode::CONFLICT;
        match self {
            Refusal::NotSignedIn => not_signed_in(),
            Refusal::Invalid(detail) => problem(bad, "invalid-request", "Invalid request", &detail),
            Refusal::Cert(detail) => {
                problem(bad, "invalid-certificate", "Invalid certificate", &detail)
            }
            Refusal::Username(detail) => {
                problem(bad, "invalid-username", "Invalid username", &detail)
            }
            Refusal::DeviceTaken => problem(
                conflict,
                "device-taken",
                "Device ID taken",
                "The server has this device ID with other keys or name, or on another account.",
            ),
            Refusal::MasterKeyTaken => problem(
                conflict,
                "master-key-taken",
                "The account has a master key",
                "The account has another master key, or took another certificate with it.",
            ),
            Refusal::UsernameReserved => problem(
                conflict,
                "username-reserved",
                "Username reserved",
                "No account takes this username.",
            ),
            Refusal::UsernameTaken => problem(
                conflict,
                "username-taken",
                "Username taken",
                "Another account has this username.",
            ),
            Refusal::NoSpace => problem(
                StatusCode::NOT_FOUND,
                "no-space",
                "No such space",
                "No such space, or the session's account does not get it.",
            ),
            Refusal::NotAllowed(detail) => {
                problem(StatusCode::FORBIDDEN, "not-allowed", "Not allowed", &detail)
            }
            Refusal::Roster(detail) => {
                problem(bad, "invalid-roster", "Invalid roster revision", &detail)
            }
            Refusal::SpaceNameTaken => problem(
                conflict,
                "space-name-taken",
                "Space name taken",
                "Another space has this name, or the account's space with it has a revision.",
            ),
            Refusal::OwnedSpacesLimit(limit) => problem(
                StatusCode::FORBIDDEN,
                "owned-spaces-limit",
                "Space limit reached",
                &format!("The account owns as many spaces as it may: {limit}."),
            ),
            Refusal::RevisionConflict(detail) => problem(
                conflict,
                "revision-conflict",
                "Revision out of order",
                &detail,
            ),
            Refusal::Database(doing, err) => database_failed(doing, err),
        }
    }
}

/// What a handler answers: a response, or why it refuses.
pub(crate) type Answer = Result<Response, Refusal>;

/// Runs a handler's `answer`, which gives a response or why it refuses.
pub(crate) async fn answer(answer: impl Future<Output = Answer>) -> Response {
    answer.await.unwrap_or_else(IntoResponse::into_response)
}

/// The ID of the account of the request's session.
pub(crate) async fn account(db: &PgPool, headers: &HeaderMap) -> Result<String, Refusal> {
    let session = signed_in(db, headers)
        .await
        .map_err(|err| Refusal::Database("read a session", err))?;
    Ok(session.ok_or(Refusal::NotSignedIn)?.account)
}

/// The ID of the account of the request's session, as a handler's argument.
pub(crate) struct SignedIn(pub String);

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Refusal> {
        Ok(SignedIn(account(&state.db, &parts.headers).await?))
    }
}

/// The JSON object `body` as a `T`.
pub(crate) fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, Refusal> {
    let invalid = |err: serde_json::Error| Refusal::Invalid(format!("not a valid body: {err}"));
    // An object only: serde would also take a struct from an array of its fields. `T` is read from
    // the bytes, so a field given twice is refused.
    serde_json::from_slice::<HashMap<String, IgnoredAny>>(body).map_err(invalid)?;
    serde_json::from_slice(body).map_err(invalid)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        name: String,
    }

    /// Why `parse` refuses `body`.
    fn refused(body: &str) -> String {
        match parse::<Request>(body.as_bytes()) {
            Err(Refusal::Invalid(detail)) => detail,
            Ok(request) => panic!("{body} taken as {request:?}"),
            Err(_) => panic!("{body} refused for another reason"),
        }
    }

    #[test]
    fn parse_takes_an_object_of_the_fields() {
        let request: Request = parse(br#"{"name": "a"}"#).unwrap_or_else(|_| panic!("refused"));
        assert_eq!(request.name, "a");
    }

    #[test]
    fn parse_refuses_a_field_given_twice() {
        let detail = refused(r#"{"name": "a", "name": "b"}"#);
        assert!(detail.contains("duplicate field"), "{detail}");
    }

    #[test]
    fn parse_refuses_anything_but_an_object() {
        for body in [r#"["a"]"#, r#""a""#, "null", "not JSON", ""] {
            refused(body);
        }
    }
}
