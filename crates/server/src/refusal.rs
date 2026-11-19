//! Why the JSON API refuses a request, and the steps its handlers share: the session's account and
//! the request's body.

use std::collections::HashMap;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::de::{DeserializeOwned, IgnoredAny};
use sqlx::PgPool;

use crate::service_tokens::{self, Token};
use crate::sign_in::{bearer, digest, not_signed_in, signed_in};
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
    /// No such device of the session's account.
    NoDevice,
    /// No such session of the session's account, or an ended one.
    NoSession,
    /// No such sign-in method of the session's account.
    NoSignInMethod,
    /// The account's last sign-in method, which is never removed.
    LastSignInMethod,
    /// The web session's sign-in is older than the server allows for what it asks.
    SignInTooOld,
    /// A KeyPackage that fails the checks, for the reason it holds.
    KeyPackage(String),
    /// No such channel in the space's latest revision.
    NoChannel,
    /// A GroupInfo the server does not keep, for the reason it holds.
    GroupInfo(String),
    /// A GroupInfo not of the group the server follows, for the reason it holds.
    GroupInfoConflict(String),
    /// The session's account may not do this in the space, for the reason it holds.
    NotAllowed(String),
    /// A roster revision the server does not store, for the reason it holds.
    Roster(String),
    /// A revocation the server does not store, for the reason it holds.
    Revocation(String),
    /// An anchors object the server does not store, for the reason it holds.
    Anchors(String),
    SpaceNameTaken,
    /// The account owns as many spaces as it may: the number it holds.
    OwnedSpacesLimit(u32),
    /// The account has as many devices with a certificate as it may: the number it holds.
    DevicesLimit(u32),
    /// A roster revision out of order or with a taken number, for the reason it holds.
    RevisionConflict(String),
    /// A link message the server does not relay there, for the reason it holds.
    LinkMessage(String),
    /// A hello naming a device that has a certificate.
    DeviceCertified,
    /// A hello whose `session` another device's run of the account holds.
    LinkSessionTaken,
    /// No such open run of the session's account.
    NoLinkRun,
    /// A grant the server does not take, for the reason it holds.
    Grant(String),
    /// No such service token of the space.
    NoServiceToken,
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
            Refusal::NoDevice => problem(
                StatusCode::NOT_FOUND,
                "no-device",
                "No such device",
                "The session's account has no such device.",
            ),
            Refusal::NoSession => problem(
                StatusCode::NOT_FOUND,
                "no-session",
                "No such session",
                "The session's account has no such session, or it has ended.",
            ),
            Refusal::NoSignInMethod => problem(
                StatusCode::NOT_FOUND,
                "no-sign-in-method",
                "No such sign-in method",
                "The session's account has no such sign-in method.",
            ),
            Refusal::LastSignInMethod => problem(
                conflict,
                "last-sign-in-method",
                "The account's last sign-in method",
                "The account's last sign-in method is never removed: link another one first.",
            ),
            Refusal::SignInTooOld => problem(
                StatusCode::FORBIDDEN,
                "sign-in-too-old",
                "Sign in again",
                "The web session's sign-in is older than the server allows for this: sign in again first.",
            ),
            Refusal::KeyPackage(detail) => {
                problem(bad, "invalid-key-package", "Invalid KeyPackage", &detail)
            }
            Refusal::NoChannel => problem(
                StatusCode::NOT_FOUND,
                "no-channel",
                "No such channel",
                "The space's latest revision lists no such channel.",
            ),
            Refusal::GroupInfo(detail) => {
                problem(bad, "invalid-group-info", "Invalid GroupInfo", &detail)
            }
            Refusal::GroupInfoConflict(detail) => problem(
                conflict,
                "group-info-conflict",
                "Not the channel's group",
                &detail,
            ),
            Refusal::NotAllowed(detail) => {
                problem(StatusCode::FORBIDDEN, "not-allowed", "Not allowed", &detail)
            }
            Refusal::Roster(detail) => {
                problem(bad, "invalid-roster", "Invalid roster revision", &detail)
            }
            Refusal::Revocation(detail) => {
                problem(bad, "invalid-revocation", "Invalid revocation", &detail)
            }
            Refusal::Anchors(detail) => {
                problem(bad, "invalid-anchors", "Invalid anchors object", &detail)
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
            Refusal::DevicesLimit(limit) => problem(
                StatusCode::FORBIDDEN,
                "devices-limit",
                "Device limit reached",
                &format!("The account has as many devices with a certificate as it may: {limit}."),
            ),
            Refusal::RevisionConflict(detail) => problem(
                conflict,
                "revision-conflict",
                "Revision out of order",
                &detail,
            ),
            Refusal::LinkMessage(detail) => {
                problem(bad, "invalid-link-message", "Invalid link message", &detail)
            }
            Refusal::DeviceCertified => problem(
                conflict,
                "device-certified",
                "The device has a certificate",
                "The device has a certificate, so it links no more.",
            ),
            Refusal::LinkSessionTaken => problem(
                conflict,
                "link-session-taken",
                "Session taken",
                "A run of another device of the account has this session.",
            ),
            Refusal::NoLinkRun => problem(
                StatusCode::NOT_FOUND,
                "no-link-run",
                "No such link run",
                "The session's account has no open run with this session: it ended or never opened.",
            ),
            Refusal::Grant(detail) => problem(bad, "invalid-grant", "Invalid grant", &detail),
            Refusal::NoServiceToken => problem(
                StatusCode::NOT_FOUND,
                "no-service-token",
                "No such service token",
                "The space has no such service token, or it has ended.",
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

/// Who a request comes from, for the requests a service token alone also makes, or whom a
/// connection's challenge is of: a session's account, or a service token that has not ended.
pub(crate) enum Caller {
    /// The ID of the session's account.
    Session(String),
    Token(Token),
}

impl Caller {
    /// The ID of its account: the session's, or the token's device's.
    pub(crate) fn account(&self) -> String {
        match self {
            Caller::Session(account) => account.clone(),
            Caller::Token(token) => token.account.to_string(),
        }
    }

    /// Its service token, if it is one.
    pub(crate) fn token(&self) -> Option<&Token> {
        match self {
            Caller::Session(_) => None,
            Caller::Token(token) => Some(token),
        }
    }
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Refusal> {
        let failed = |err| Refusal::Database("read a session", err);
        let headers = &parts.headers;
        if let Some(session) = signed_in(&state.db, headers).await.map_err(failed)? {
            return Ok(Caller::Session(session.account));
        }
        let Some(Some(token)) = bearer(headers) else {
            return Err(Refusal::NotSignedIn);
        };
        let token = service_tokens::live(&state.db, &digest(token))
            .await
            .map_err(failed)?;
        Ok(Caller::Token(token.ok_or(Refusal::NotSignedIn)?))
    }
}

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
