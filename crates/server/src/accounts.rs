//! Accounts, their sign-in methods and usernames: every provider's sign-in reaches the account that
//! holds its method, and the method's first sign-in signs up; any session of an account sets its
//! username.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use skald_proto::{AccountId, Name};
use sqlx::{PgPool, Postgres, Transaction};

use crate::AppState;
use crate::refusal::{Refusal, account, answer, parse};
use crate::sign_in::{digest, random_token};

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
/// sign-in signs up, if `may_sign_up`: it creates an account with that method, its username made
/// from `source`, such as a GitHub login or an email's local part; else it creates nothing and gives
/// `None`. A GitHub method's login is updated.
pub(crate) async fn sign_in(
    db: &PgPool,
    method: &Method<'_>,
    source: &str,
    may_sign_up: bool,
) -> Result<Option<String>, sqlx::Error> {
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
        if !may_sign_up {
            // Dropped, the transaction rolls the method back.
            return Ok(None);
        }
        sign_up(&mut tx, &account, source).await?;
    }
    let session = random_token();
    sqlx::query("INSERT INTO sessions (token_hash, account_id, kind) VALUES ($1, $2::uuid, 'web')")
        .bind(digest(&session))
        .bind(&account)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(session))
}

/// Creates the account `account`, whose first sign-in method `tx` added, with the first free
/// username that `source` makes: the one sign-up of every provider.
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

/// The username in the JSON `body`, a `UsernameRequest`, checked to be a `name`.
fn requested(body: &[u8]) -> Result<Name, Refusal> {
    let request: UsernameRequest = parse(body)?;
    request
        .username
        .parse()
        .map_err(|err| Refusal::Username(format!("the username {err}")))
}

/// `PUT /account/username`: sets the username of the session's account, web or device, unless it
/// is invalid, reserved or another account's.
pub(crate) async fn set_username(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let username = requested(&body)?;
        // Of two accounts setting one username at once, the second waits for the first to commit
        // and breaks the uniqueness.
        sqlx::query("UPDATE accounts SET username = $1 WHERE id = $2::uuid")
            .bind(username.as_str())
            .bind(&account)
            .execute(&state.db)
            .await
            .map_err(|err| match err.as_database_error().map(|db| db.kind()) {
                Some(sqlx::error::ErrorKind::UniqueViolation) => Refusal::UsernameTaken,
                // The username is a `name`, so the only check it breaks is the reserved one.
                Some(sqlx::error::ErrorKind::CheckViolation) => Refusal::UsernameReserved,
                _ => Refusal::Database("set a username", err),
            })?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}
