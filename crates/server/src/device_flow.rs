//! The device flow (RFC 8628): a machine without a browser gets a device session once its person,
//! signed in with any provider in a browser elsewhere, approves its user code on the verification
//! page.

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sqlx::PgPool;

use crate::sign_in::{
    self, Session, digest, escape, flows, off, param, random_token, sign_in_links, web_session,
};
use crate::{AppState, database_failed};

/// The verification page's path under the public address.
pub(crate) const VERIFICATION_PATH: &str = "/device";
/// The `client_id` of `skald`, the CLI: the only client of the device flow.
const CLIENT_ID: &str = "skald";
/// The device flow's `grant_type` (RFC 8628, §3.4).
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The letters of user codes: no vowels, so no words, and none that read alike (RFC 8628, §6.1).
const USER_CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";
/// A user code's letters: 20⁸, about 2³⁴, codes.
const USER_CODE_LENGTH: usize = 8;
/// How many wrong user codes an account may type within [`WRONG_CODE_WINDOW`] before the page
/// refuses its codes, right ones included (RFC 8628, §5.1).
const WRONG_CODE_LIMIT: i64 = 10;
const WRONG_CODE_WINDOW: Duration = Duration::from_secs(900);
/// How long expired codes are kept, so that polls for them answer `expired_token`.
const EXPIRED_KEPT: Duration = Duration::from_secs(3_600);
/// The verification page's content security policy: no scripts, its forms only to this server,
/// never in a frame.
const PAGE_POLICY: &str = "default-src 'none'; form-action 'self'; frame-ancestors 'none'";

/// The fields of the form `body`; `Err` with the name of a field it has twice.
fn form(body: &[u8]) -> Result<HashMap<String, String>, String> {
    let mut fields = HashMap::new();
    for (name, value) in url::form_urlencoded::parse(body) {
        if fields
            .insert(name.to_string(), value.into_owned())
            .is_some()
        {
            return Err(name.into_owned());
        }
    }
    Ok(fields)
}

/// `OAuthError` in the spec: the answer `400` with `error`.
#[derive(Serialize)]
struct OAuthError {
    error: &'static str,
    error_description: String,
}

impl OAuthError {
    fn new(error: &'static str, description: impl Into<String>) -> Self {
        OAuthError {
            error,
            error_description: description.into(),
        }
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        no_store(StatusCode::BAD_REQUEST, self)
    }
}

/// `body` in JSON with `status`, never cached: it holds codes, a token or why there is none.
fn no_store(status: StatusCode, body: impl Serialize) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

/// The fields of a machine's form `body`, which names this client.
fn machine_form(body: &[u8]) -> Result<HashMap<String, String>, OAuthError> {
    let fields = form(body).map_err(|name| {
        OAuthError::new("invalid_request", format!("The request has {name} twice."))
    })?;
    match fields.get("client_id").map(String::as_str) {
        Some(CLIENT_ID) => Ok(fields),
        Some(_) => Err(OAuthError::new(
            "invalid_client",
            "The client_id is not skald.",
        )),
        None => Err(OAuthError::new(
            "invalid_request",
            "The request has no client_id.",
        )),
    }
}

/// A user code from the operating system's cryptographically secure generator, as `XXXX-XXXX`.
fn user_code() -> String {
    let mut bytes = [0; 4 * USER_CODE_LENGTH];
    getrandom::fill(&mut bytes).expect("the operating system's random generator failed");
    // A letter from each 32 random bits: the remainder favors some letters by 16 in 2³², nothing.
    let letters: String = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&chunk| {
            let bits = u32::from_le_bytes(chunk);
            char::from(USER_CODE_ALPHABET[bits as usize % USER_CODE_ALPHABET.len()])
        })
        .collect();
    let (first, second) = letters.split_at(USER_CODE_LENGTH / 2);
    format!("{first}-{second}")
}

/// The user code a person typed as `typed`, ignoring case and every character but letters and
/// digits; `None` if it cannot be one.
fn typed_code(typed: &str) -> Option<String> {
    let code: String = typed
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (code.len() == USER_CODE_LENGTH && code.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)))
        .then_some(code)
}

/// `DeviceAuthorization` in the spec.
#[derive(Serialize)]
struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}

/// `POST /auth/device`: device authorization (RFC 8628, §3.1).
pub(crate) async fn authorize(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(settings) = sign_in::settings(&state) else {
        return off();
    };
    if let Err(refusal) = machine_form(&body) {
        return refusal.into_response();
    }
    let lifetime = settings.device_code_lifetime.as_secs();
    let interval = settings.device_poll_interval.as_secs();
    // New codes that earlier ones have, expired or not, are drawn again.
    for _ in 0..3 {
        let device_code = random_token();
        let user_code = user_code();
        let typed = typed_code(&user_code).expect("a user code is typed as it is");
        let stored = sqlx::query_as::<_, (bool,)>(
            "WITH expired AS (
                 DELETE FROM device_codes WHERE expires_at <= now() - make_interval(secs => $5)
             )
             INSERT INTO device_codes (device_code_hash, user_code_hash, expires_at, poll_interval)
             VALUES ($1, $2, now() + make_interval(secs => $3), $4)
             ON CONFLICT DO NOTHING RETURNING true",
        )
        .bind(digest(&device_code))
        .bind(digest(&typed))
        .bind(lifetime as f64)
        .bind(i32::try_from(interval).expect("the polling interval fits in an integer"))
        .bind(EXPIRED_KEPT.as_secs_f64())
        .fetch_optional(&state.db)
        .await;
        match stored {
            Ok(Some((true,))) => {
                let public = &settings.public_url;
                let verification_uri = public.at(VERIFICATION_PATH);
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("user_code", &user_code)
                    .finish();
                return no_store(
                    StatusCode::OK,
                    DeviceAuthorization {
                        device_code,
                        verification_uri_complete: format!("{verification_uri}?{query}"),
                        verification_uri,
                        user_code,
                        expires_in: lifetime,
                        interval,
                    },
                );
            }
            Ok(_) => {}
            Err(err) => return database_failed("store a device code", err),
        }
    }
    database_failed(
        "store a device code",
        "three new user codes in a row were taken",
    )
}

/// `DeviceToken` in the spec.
#[derive(Serialize)]
struct DeviceToken {
    access_token: String,
    token_type: &'static str,
}

/// `POST /auth/token`: the token endpoint (RFC 8628, §3.4).
pub(crate) async fn token(State(state): State<AppState>, body: Bytes) -> Response {
    if sign_in::settings(&state).is_none() {
        return off();
    }
    let device_code = match device_code(&body) {
        Ok(device_code) => device_code,
        Err(refusal) => return refusal.into_response(),
    };
    match poll(&state.db, &device_code).await {
        Ok(Ok(access_token)) => no_store(
            StatusCode::OK,
            DeviceToken {
                access_token,
                token_type: "Bearer",
            },
        ),
        Ok(Err(refusal)) => refusal.into_response(),
        Err(err) => database_failed("poll a device code", err),
    }
}

/// The device code that a poll's form `body` polls for.
fn device_code(body: &[u8]) -> Result<String, OAuthError> {
    let mut fields = machine_form(body)?;
    match fields.get("grant_type").map(String::as_str) {
        Some(GRANT_TYPE) => {}
        Some(_) => {
            return Err(OAuthError::new(
                "unsupported_grant_type",
                "This server gives tokens only for the device flow.",
            ));
        }
        None => {
            return Err(OAuthError::new(
                "invalid_request",
                "The request has no grant_type.",
            ));
        }
    }
    fields
        .remove("device_code")
        .filter(|code| !code.is_empty())
        .ok_or_else(|| OAuthError::new("invalid_request", "The request has no device_code."))
}

/// A poll for `device_code`: the new session token of an approved code, or the error and why.
async fn poll(db: &PgPool, device_code: &str) -> Result<Result<String, OAuthError>, sqlx::Error> {
    let hash = digest(device_code);
    let mut tx = db.begin().await?;
    // The row's lock makes polls for one code wait for each other: only one gives its token.
    let found: Option<(String, bool, bool)> = sqlx::query_as(
        "SELECT state, expires_at > now(),
                coalesce(polled_at > now() - make_interval(secs => poll_interval), false)
         FROM device_codes WHERE device_code_hash = $1 FOR UPDATE",
    )
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((code_state, live, too_soon)) = found else {
        return Ok(Err(OAuthError::new(
            "invalid_grant",
            "This device code is unknown.",
        )));
    };
    if code_state == "issued" {
        return Ok(Err(OAuthError::new(
            "invalid_grant",
            "This device code gave its token.",
        )));
    }
    if !live {
        return Ok(Err(OAuthError::new(
            "expired_token",
            "This device code expired.",
        )));
    }
    sqlx::query("UPDATE device_codes SET polled_at = now() WHERE device_code_hash = $1")
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    let answer = if too_soon {
        Err(OAuthError::new(
            "slow_down",
            "Polls come more often than the interval.",
        ))
    } else {
        match code_state.as_str() {
            "approved" => {
                let token = random_token();
                sqlx::query(
                    "WITH issued AS (
                         UPDATE device_codes SET state = 'issued' WHERE device_code_hash = $1
                         RETURNING account_id
                     )
                     INSERT INTO sessions (token_hash, account_id, kind)
                     SELECT $2, account_id, 'device' FROM issued",
                )
                .bind(&hash)
                .bind(digest(&token))
                .execute(&mut *tx)
                .await?;
                Ok(token)
            }
            "denied" => Err(OAuthError::new(
                "access_denied",
                "The person denied this code.",
            )),
            _ => Err(OAuthError::new(
                "authorization_pending",
                "The person has not decided yet.",
            )),
        }
    };
    tx.commit().await?;
    Ok(answer)
}

/// A verification page of `status` with `title` and `body`, under [`PAGE_POLICY`].
fn page_of(status: StatusCode, title: &str, body: &str) -> Response {
    sign_in::form_page(status, title, body, PAGE_POLICY)
}

/// The page's answer when the database fails.
fn failed(doing: &str, err: sqlx::Error) -> Response {
    sign_in::form_page_failed(doing, err, PAGE_POLICY)
}

/// Links that sign in with each provider the server has settings for and return to the
/// verification page with `user_code`.
fn sign_in_here(state: &AppState, user_code: &str) -> String {
    let mut to = VERIFICATION_PATH.to_owned();
    if !user_code.is_empty() {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("user_code", user_code)
            .finish();
        to = format!("{to}?{query}");
    }
    sign_in_links(&flows(state), "", &to)
}

/// `GET /device`: the verification page.
pub(crate) async fn page(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    if sign_in::settings(&state).is_none() {
        return off();
    }
    let user_code = param(query.as_deref(), "user_code").unwrap_or_default();
    match web_session(&state.db, &headers).await {
        Ok(Some(session)) => ask(&session, &user_code),
        Ok(None) => page_of(
            StatusCode::OK,
            "Sign in a machine",
            &format!(
                "<h1>Sign in a machine</h1><p>To sign a machine in to your Skald account, sign in first. {}</p>",
                sign_in_here(&state, &user_code)
            ),
        ),
        Err(err) => failed("read a session", err),
    }
}

/// The page that asks the person of `session` to approve or deny `user_code`.
fn ask(session: &Session, user_code: &str) -> Response {
    let body = format!(
        "<h1>Sign in a machine</h1>\
         <p>You are signed in as <strong>{username}</strong>. Approving a code signs the machine that \
         shows it in to this account: approve only a code that your own machine shows.</p>\
         <form method=\"post\" action=\"{VERIFICATION_PATH}\">\
         <p><label>Code <input name=\"user_code\" value=\"{code}\" autocomplete=\"off\" \
         spellcheck=\"false\"></label></p>\
         <p><button name=\"decision\" value=\"approve\">Approve</button> \
         <button name=\"decision\" value=\"deny\">Deny</button></p></form>",
        username = escape(&session.username),
        code = escape(user_code),
    );
    page_of(StatusCode::OK, "Sign in a machine", &body)
}

/// What became of a person's decision on a user code.
enum Outcome {
    Decided,
    /// The code is wrong, used or expired.
    Wrong,
    /// The account typed too many wrong codes of late.
    Locked,
}

/// `POST /device`: the person's decision on a user code, from the verification page.
pub(crate) async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(settings) = sign_in::settings(&state) else {
        return off();
    };
    let again = format!("<a href=\"{VERIFICATION_PATH}\">Try again</a>");
    // The session cookie is `SameSite=Strict`, and browsers state the origin of every POST.
    if !sign_in::from_this_server(settings, &headers) {
        return page_of(
            StatusCode::FORBIDDEN,
            "Refused",
            &format!("<p>Refused: the decision did not come from this server's page. {again}</p>"),
        );
    }
    let session = match web_session(&state.db, &headers).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return page_of(
                StatusCode::FORBIDDEN,
                "Not signed in",
                &format!("<p>Not signed in. {}</p>", sign_in_here(&state, "")),
            );
        }
        Err(err) => return failed("read a session", err),
    };
    let fields = form(&body).unwrap_or_default();
    let (approve, decided) = match fields.get("decision").map(String::as_str) {
        Some("approve") => (true, "approved"),
        Some("deny") => (false, "denied"),
        _ => {
            return page_of(
                StatusCode::BAD_REQUEST,
                "Refused",
                &format!("<p>Approve or deny the code. {again}</p>"),
            );
        }
    };
    let typed = fields.get("user_code").map_or("", String::as_str);
    match record(&state.db, &session.account, typed, decided).await {
        Ok(Outcome::Decided) if approve => page_of(
            StatusCode::OK,
            "Approved",
            "<p>Approved: your machine signs in to your account now. You can close this page.</p>",
        ),
        Ok(Outcome::Decided) => page_of(
            StatusCode::OK,
            "Denied",
            "<p>Denied: the machine is not signed in. You can close this page.</p>",
        ),
        Ok(Outcome::Wrong) => page_of(
            StatusCode::BAD_REQUEST,
            "Wrong code",
            &format!(
                "<p>This code is wrong, used or expired: check the code your machine shows. {again}</p>"
            ),
        ),
        Ok(Outcome::Locked) => page_of(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many wrong codes",
            "<p>Too many wrong codes: try again in 15 minutes.</p>",
        ),
        Err(err) => failed("record a decision on a device code", err),
    }
}

/// Records the decision of `account`, `approved` or `denied`, on the user code it `typed`, unless
/// the code is not pending or the account typed too many wrong codes of late.
async fn record(
    db: &PgPool,
    account: &str,
    typed: &str,
    decided: &str,
) -> Result<Outcome, sqlx::Error> {
    let window = WRONG_CODE_WINDOW.as_secs_f64();
    let mut tx = db.begin().await?;
    // The account's lock makes its decisions wait for each other, so none slips past the limit.
    sqlx::query("SELECT 1 FROM accounts WHERE id = $1::uuid FOR UPDATE")
        .bind(account)
        .execute(&mut *tx)
        .await?;
    let (wrong,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM wrong_user_codes
         WHERE account_id = $1::uuid AND typed_at > now() - make_interval(secs => $2)",
    )
    .bind(account)
    .bind(window)
    .fetch_one(&mut *tx)
    .await?;
    if wrong >= WRONG_CODE_LIMIT {
        return Ok(Outcome::Locked);
    }
    // Of two decisions at once on one code, the second waits for the first and finds it decided.
    let decided = match typed_code(typed) {
        Some(code) => {
            sqlx::query(
                "UPDATE device_codes SET state = $1, account_id = $2::uuid
                 WHERE user_code_hash = $3 AND state = 'pending' AND expires_at > now()",
            )
            .bind(decided)
            .bind(account)
            .bind(digest(&code))
            .execute(&mut *tx)
            .await?
            .rows_affected()
                == 1
        }
        None => false,
    };
    if !decided {
        sqlx::query(
            "WITH old AS (
                 DELETE FROM wrong_user_codes
                 WHERE account_id = $1::uuid AND typed_at <= now() - make_interval(secs => $2)
             )
             INSERT INTO wrong_user_codes (account_id) VALUES ($1::uuid)",
        )
        .bind(account)
        .bind(window)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(if decided {
        Outcome::Decided
    } else {
        Outcome::Wrong
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn user_codes_are_short_and_from_the_alphabet() {
        for _ in 0..200 {
            let code = user_code();
            assert_eq!(code.len(), 9, "{code}");
            assert_eq!(&code[4..5], "-", "{code}");
            assert!(
                code.bytes()
                    .filter(|&b| b != b'-')
                    .all(|b| USER_CODE_ALPHABET.contains(&b)),
                "{code}"
            );
        }
        assert_ne!(user_code(), user_code());
    }

    #[test]
    fn user_codes_use_every_letter_in_every_place() {
        let codes: Vec<Vec<char>> = (0..1000)
            .map(|_| user_code().replace('-', "").chars().collect())
            .collect();
        for at in 0..USER_CODE_LENGTH {
            let seen: BTreeSet<char> = codes.iter().map(|code| code[at]).collect();
            assert_eq!(seen.len(), USER_CODE_ALPHABET.len(), "place {at}");
        }
        // Letters in one code are drawn apart.
        assert!(
            codes
                .iter()
                .any(|code| code.iter().collect::<BTreeSet<_>>().len() > 4)
        );
    }

    #[test]
    fn typed_codes_ignore_case_and_separators() {
        assert_eq!(typed_code("bcdf-ghjk").as_deref(), Some("BCDFGHJK"));
        assert_eq!(typed_code(" BCDF GHJK ").as_deref(), Some("BCDFGHJK"));
        assert_eq!(typed_code("BCDFGHJK").as_deref(), Some("BCDFGHJK"));
        for typed in ["", "BCDF-GHJ", "BCDF-GHJKL", "ABCD-GHJK", "BCD1-GHJK"] {
            assert_eq!(typed_code(typed), None, "{typed}");
        }
    }

    #[test]
    fn forms_with_a_field_twice_are_refused() {
        let fields = form(b"client_id=skald&device_code=a%2Bb").expect("a form");
        assert_eq!(fields["device_code"], "a+b");
        assert_eq!(
            form(b"client_id=skald&client_id=other"),
            Err("client_id".to_owned())
        );
    }
}
