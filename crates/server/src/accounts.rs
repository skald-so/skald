//! Accounts, their sign-in methods and usernames: every provider's sign-in reaches the account that
//! holds its method, and the method's first sign-in signs up; any session of an account sets its
//! username.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Map, Value};
use skald_proto::{AccountId, Name};
use sqlx::{PgPool, Postgres, Transaction};

use crate::sign_in::{digest, not_signed_in, random_token, signed_in};
use crate::{AppState, database_failed, problem};

/// The provider of GitHub methods; an OIDC provider is named by its issuer.
pub(crate) const GITHUB: &str = "github";

/// A sign-in method: a provider and the subject it gives the person.
pub(crate) struct Method<'a> {
    /// [`GITHUB`] or an OIDC provider's issuer.
    pub provider: &'a str,
    /// For GitHub, the GitHub user's ID in decimal.
    pub subject: String,
    /// A GitHub method's GitHub login, for display; `None` for other providers.
    pub login: Option<&'a str>,
}

/// Starts a web session of the account that holds `method`, and gives its token. The method's first
/// sign-in signs up: it creates an account with that method. A GitHub method's login is updated.
pub(crate) async fn sign_in(db: &PgPool, method: &Method<'_>) -> Result<String, sqlx::Error> {
    let mut tx = db.begin().await?;
    // Of two first sign-ins with one method at once, the second waits for the first to commit and
    // finds its account.
    let (account, created): (String, bool) = sqlx::query_as(
        "INSERT INTO sign_in_methods (provider, subject, account_id, login)
         VALUES ($1, $2, $3::uuid, $4)
         ON CONFLICT (provider, subject) DO UPDATE SET login = EXCLUDED.login
         RETURNING account_id::text, account_id = $3::uuid",
    )
    .bind(method.provider)
    .bind(&method.subject)
    .bind(AccountId::generate().to_string())
    .bind(method.login)
    .fetch_one(&mut *tx)
    .await?;
    if created {
        sign_up(&mut tx, &account, method.login.unwrap_or_default()).await?;
    }
    let session = random_token();
    sqlx::query("INSERT INTO sessions (token_hash, account_id, kind) VALUES ($1, $2::uuid, 'web')")
        .bind(digest(&session))
        .bind(&account)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(session)
}

/// Creates the account `account`, whose first sign-in method `tx` added, with the first free
/// username that `source`, such as a GitHub login, makes: the one sign-up of every provider.
async fn sign_up(
    tx: &mut Transaction<'_, Postgres>,
    account: &str,
    source: &str,
) -> Result<(), sqlx::Error> {
    // Of sign-ups at once that make one username, the later ones wait for the first to commit, take
    // nothing and try the next free one.
    loop {
        let created = sqlx::query(
            "INSERT INTO accounts (id, username) VALUES ($1::uuid, free_username($2))
             ON CONFLICT (username) DO NOTHING",
        )
        .bind(account)
        .bind(source)
        .execute(&mut **tx)
        .await?;
        if created.rows_affected() == 1 {
            return Ok(());
        }
    }
}

/// `UsernameRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsernameRequest {
    username: String,
}

/// Why the server refuses a username.
enum Refusal {
    NotSignedIn,
    /// A malformed body, or a username that is no `name`, for the reason it holds.
    Invalid(&'static str, String),
    Reserved,
    Taken,
    /// The database failed at what it holds, such as `read a session`.
    Database(&'static str, sqlx::Error),
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let conflict = StatusCode::CONFLICT;
        match self {
            Refusal::NotSignedIn => not_signed_in(),
            Refusal::Invalid(kind, detail) => {
                problem(StatusCode::BAD_REQUEST, kind, "Invalid request", &detail)
            }
            Refusal::Reserved => problem(
                conflict,
                "username-reserved",
                "Username reserved",
                "No account takes this username.",
            ),
            Refusal::Taken => problem(
                conflict,
                "username-taken",
                "Username taken",
                "Another account has this username.",
            ),
            Refusal::Database(doing, err) => database_failed(doing, err),
        }
    }
}

/// The username in the JSON `body`, a `UsernameRequest` object, checked to be a `name`.
fn requested(body: &[u8]) -> Result<Name, Refusal> {
    let invalid = |err: serde_json::Error| {
        Refusal::Invalid("invalid-request", format!("not a valid body: {err}"))
    };
    // An object only: serde would also take a struct from an array of its fields.
    let object: Map<String, Value> = serde_json::from_slice(body).map_err(invalid)?;
    let request: UsernameRequest =
        serde_json::from_value(Value::Object(object)).map_err(invalid)?;
    request
        .username
        .parse()
        .map_err(|err| Refusal::Invalid("invalid-username", format!("the username {err}")))
}

/// `PUT /account/username`: sets the username of the session's account, web or device, unless it
/// is invalid, reserved or another account's.
pub(crate) async fn set_username(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let set = async {
        let session = signed_in(&state.db, &headers)
            .await
            .map_err(|err| Refusal::Database("read a session", err))?
            .ok_or(Refusal::NotSignedIn)?;
        let username = requested(&body)?;
        // Of two accounts setting one username at once, the second waits for the first to commit
        // and breaks the uniqueness.
        sqlx::query("UPDATE accounts SET username = $1 WHERE id = $2::uuid")
            .bind(username.as_str())
            .bind(&session.account)
            .execute(&state.db)
            .await
            .map_err(|err| match err.as_database_error().map(|db| db.kind()) {
                Some(sqlx::error::ErrorKind::UniqueViolation) => Refusal::Taken,
                // The username is a `name`, so the only check it breaks is the reserved one.
                Some(sqlx::error::ErrorKind::CheckViolation) => Refusal::Reserved,
                _ => Refusal::Database("set a username", err),
            })?;
        Ok(StatusCode::NO_CONTENT.into_response())
    };
    set.await
        .unwrap_or_else(|refusal: Refusal| refusal.into_response())
}
