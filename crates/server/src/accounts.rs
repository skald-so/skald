//! Accounts and their sign-in methods: every provider's sign-in reaches the account that holds its
//! method, and the method's first sign-in signs up.

use skald_proto::AccountId;
use sqlx::{PgPool, Postgres, Transaction};

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
        sign_up(&mut tx, &account).await?;
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

/// Creates the account `account`, whose first sign-in method `tx` added: the one sign-up of every
/// provider.
async fn sign_up(tx: &mut Transaction<'_, Postgres>, account: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO accounts (id) VALUES ($1::uuid)")
        .bind(account)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
