//! Links of a sign-in method: a web session from a recent sign-in links another GitHub or Google
//! method to its account, through the provider's sign-in callback, and removes one, never the last;
//! and the account's sign-in methods, on a page and in the API. Accounts never merge or move a
//! method: a removed one reaches no account until it signs up anew or another account links it.

use axum::Json;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{GITHUB, Method};
use crate::refusal::{Refusal, answer};
use crate::sessions::{self, WEB_LIFETIME};
use crate::sign_in::{
    self, Failure, Flow, Provider, ProviderKind, SignInSettings, escape, flows, form_page,
    form_page_failed, from_this_server, off, param, provider_off, refresh_page, session_hashes,
    sign_in_links, signed_in, start_with,
};

/// The page of the account's sign-in methods.
pub(crate) const PAGE_PATH: &str = "/sign-in-methods";
/// Where the page's forms start a link.
pub(crate) const GITHUB_PATH: &str = "/auth/github/link";
pub(crate) const GOOGLE_PATH: &str = "/auth/google/link";
/// The pages' content security policy. Its forms go to the providers through a redirect, which
/// `form-action` would block.
const PAGE_POLICY: &str = "default-src 'none'; frame-ancestors 'none'";

/// A page of the account's methods of `status` with `title` and `body`.
fn page_of(status: StatusCode, title: &str, body: &str) -> Response {
    form_page(status, title, body, PAGE_POLICY)
}

/// The page's answer when the database fails.
fn failed(doing: &str, err: sqlx::Error) -> Response {
    form_page_failed(doing, err, PAGE_POLICY)
}

/// A sign-in method of an account, as the database holds it.
struct Stored {
    /// Its ID, which reveals nothing of its subject.
    id: String,
    provider: String,
    login: Option<String>,
    /// When it was linked, as an RFC 3339 date-time in UTC.
    linked_at: String,
    /// Whether the web session asked about signed in with it.
    current: bool,
}

impl Stored {
    fn kind(&self) -> ProviderKind {
        ProviderKind::from_key(&self.provider)
    }
}

/// The sign-in methods of `account`, oldest first, each marked if the web session whose token's
/// SHA-256 is `session` signed in with it.
async fn stored(db: &PgPool, account: &str, session: &[u8]) -> Result<Vec<Stored>, sqlx::Error> {
    let rows: Vec<(String, String, Option<String>, String, bool)> = sqlx::query_as(
        "SELECT m.id::text, m.provider, m.login,
                to_char(m.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
                EXISTS (
                    SELECT 1 FROM sessions s WHERE s.token_hash = $2
                        AND s.method_provider = m.provider AND s.method_subject = m.subject
                )
         FROM sign_in_methods m WHERE m.account_id = $1::uuid
         ORDER BY m.created_at, m.provider, m.subject",
    )
    .bind(account)
    .bind(session)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, provider, login, linked_at, current)| Stored {
            id,
            provider,
            login,
            linked_at,
            current,
        })
        .collect())
}

/// `GET /sign-in-methods`: the page of the web session's account's sign-in methods, with a form
/// that links each provider the server has settings for.
pub(crate) async fn page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if sign_in::settings(&state).is_none() {
        return off();
    }
    let flows = flows(&state);
    let session = match sign_in::web_session(&state.db, &headers).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return page_of(
                StatusCode::OK,
                "Sign-in methods",
                &format!(
                    "<h1>Sign-in methods</h1><p>Sign in to see your account's sign-in methods. {}</p>{}",
                    sign_in_links(&flows, "", PAGE_PATH),
                    sign_in::privacy_policy_link(&state)
                ),
            );
        }
        Err(err) => return failed("read a session", err),
    };
    let methods = match stored(&state.db, &session.account, &session.token_hash).await {
        Ok(methods) => methods,
        Err(err) => return failed("read the sign-in methods", err),
    };
    let items: String = methods
        .iter()
        .map(|method| {
            let name = match (method.kind(), &method.login) {
                (ProviderKind::GitHub, Some(login)) => {
                    format!("GitHub <strong>{}</strong>", escape(login))
                }
                (kind, _) => kind.label().to_owned(),
            };
            let at = &method.linked_at;
            format!(
                "<li>{name}, linked <time datetime=\"{at}\">{} {} UTC</time></li>",
                &at[..10],
                &at[11..16]
            )
        })
        .collect();
    let forms: String = flows
        .iter()
        .filter_map(|flow| {
            let link = flow.link?;
            Some(format!(
                "<form method=\"post\" action=\"{link}\"><button>Link {}</button></form>",
                flow.name
            ))
        })
        .collect();
    let body = format!(
        "<h1>Sign-in methods</h1>\
         <p>Each of these signs in to your account <strong>{username}</strong>.</p><ul>{items}</ul>\
         <p>Link another one: it signs in to this account too. A method of another account is not \
         linked.</p>{forms}{privacy}",
        username = escape(&session.username),
        privacy = sign_in::privacy_policy_link(&state),
    );
    page_of(StatusCode::OK, "Sign-in methods", &body)
}

/// `SignInMethods` in the spec.
#[derive(Serialize)]
struct SignInMethods {
    methods: Vec<SignInMethod>,
}

/// `SignInMethod` in the spec.
#[derive(Serialize)]
struct SignInMethod {
    method: String,
    provider: ProviderKind,
    linked_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    github_login: Option<String>,
    current: bool,
}

/// `GET /account/sign-in-methods`: the sign-in methods of the session's account, web or device.
pub(crate) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    answer(async {
        let session = signed_in(&state.db, &headers)
            .await
            .map_err(|err| Refusal::Database("read a session", err))?
            .ok_or(Refusal::NotSignedIn)?;
        let methods = stored(&state.db, &session.account, &session.token_hash)
            .await
            .map_err(|err| Refusal::Database("read the sign-in methods", err))?;
        let methods = methods
            .into_iter()
            .map(|method| SignInMethod {
                provider: method.kind(),
                method: method.id,
                github_login: method.login.filter(|_| method.provider == GITHUB),
                linked_at: method.linked_at,
                current: method.current,
            })
            .collect();
        Ok(Json(SignInMethods { methods }).into_response())
    })
    .await
}

/// `DELETE /account/sign-in-methods/{method}`: removes that sign-in method of the web session's
/// account, from a page of this server and after a recent sign-in, unless it is the account's last.
/// The web sessions it signed in end at once, as [`sessions`] ends one; device sessions stay.
pub(crate) async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    answer(async {
        let session =
            sessions::web_session_from_page(&state, &headers, "removes a sign-in method").await?;
        let settings = state.sign_in.as_deref().expect("a web session signs in");
        let failed = |err| Refusal::Database("remove a sign-in method", err);
        let mut tx = state.db.begin().await.map_err(failed)?;
        // Of removals on one account at once, each waits for the one before to commit, then counts
        // the methods it left; a session that one ended removes nothing.
        sqlx::query("SELECT 1 FROM accounts WHERE id = $1::uuid FOR UPDATE")
            .bind(&session.account)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        sessions::recent(
            &mut tx,
            &session,
            settings.link_age,
            "remove a sign-in method",
        )
        .await?;
        // As text: an ID that is no UUID is no method.
        let (found, count): (bool, i64) = sqlx::query_as(
            "SELECT coalesce(bool_or(id::text = lower($2)), false), count(*) FROM sign_in_methods
             WHERE account_id = $1::uuid",
        )
        .bind(&session.account)
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(failed)?;
        if !found {
            return Err(Refusal::NoSignInMethod);
        }
        if count < 2 {
            return Err(Refusal::LastSignInMethod);
        }
        let ended: Vec<(Vec<u8>,)> = sqlx::query_as(
            "DELETE FROM sessions s USING sign_in_methods m
             WHERE m.account_id = $1::uuid AND m.id::text = lower($2)
                 AND s.method_provider = m.provider AND s.method_subject = m.subject
             RETURNING s.token_hash",
        )
        .bind(&session.account)
        .bind(&id)
        .fetch_all(&mut *tx)
        .await
        .map_err(failed)?;
        sqlx::query(
            "DELETE FROM sign_in_methods WHERE account_id = $1::uuid AND id::text = lower($2)",
        )
        .bind(&session.account)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
        sessions::tell_ended(&state, &mut tx, &ended)
            .await
            .map_err(failed)?;
        tx.commit().await.map_err(failed)?;
        sessions::close_ended(&state, &ended);
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

/// `POST /auth/github/link` and `POST /auth/google/link`: starts a link of a method of the provider
/// for the web session in the request's cookie, if the request comes from a page of this server and
/// the session's sign-in is recent enough. The link returns to the query's `return_to`, a page of
/// this server, by default the page of the account's sign-in methods.
pub(crate) async fn start<P: Provider>(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let return_to = param(query.as_deref(), "return_to");
    let Some((settings, provider)) = P::of(&state) else {
        return provider_off(P::NAME);
    };
    let return_to = return_to.as_deref().unwrap_or(PAGE_PATH);
    let (state, headers) = (&state, &headers);
    let back = format!("<a href=\"{PAGE_PATH}\">Your sign-in methods</a>");
    // The session cookie is `SameSite=Strict`, and browsers state the origin of every POST.
    if !from_this_server(settings, headers) {
        return page_of(
            StatusCode::FORBIDDEN,
            "Refused",
            &format!("<p>Refused: the link did not come from this server's page. {back}</p>"),
        );
    }
    let found: Result<Option<(Vec<u8>, String, bool)>, _> = sqlx::query_as(
        "SELECT token_hash, account_id::text, created_at > now() - make_interval(secs => $2)
         FROM sessions WHERE kind = 'web' AND token_hash = ANY($1)
             AND created_at > now() - make_interval(secs => $3)
         LIMIT 1",
    )
    .bind(session_hashes(headers))
    .bind(settings.link_age.as_secs() as f64)
    .bind(WEB_LIFETIME.as_secs_f64())
    .fetch_optional(&state.db)
    .await;
    match found {
        Ok(Some((session, _, true))) => {
            start_with(
                &state.db,
                settings,
                provider,
                Some(return_to),
                Some(&session),
            )
            .await
        }
        Ok(Some((_, account, false))) => {
            let methods = match stored(&state.db, &account, &[]).await {
                Ok(methods) => methods,
                Err(err) => return failed("read the sign-in methods", err),
            };
            // Only with a method of the account's own: another one would sign in to another account.
            let mut flows = flows(state);
            flows.retain(|flow| {
                methods
                    .iter()
                    .any(|method| method.provider == flow.provider)
            });
            page_of(
                StatusCode::FORBIDDEN,
                "Sign in again",
                &format!(
                    "<p>To link a sign-in method, sign in again first: your last sign-in is too old. {}</p>",
                    sign_in_links(&flows, " again", return_to)
                ),
            )
        }
        Ok(None) => page_of(
            StatusCode::FORBIDDEN,
            "Not signed in",
            &format!(
                "<p>Not signed in. {}</p>",
                sign_in_links(&flows(state), "", return_to)
            ),
        ),
        Err(err) => failed("read a session", err),
    }
}

/// What a link did.
pub(crate) enum Added {
    Linked,
    /// The account had the method already.
    Already,
}

/// Adds `method` to the account of the web session whose token's SHA-256 is `session`, while the
/// session lasts and has not expired, unless another account has the method.
pub(crate) async fn add(
    db: &PgPool,
    session: &[u8],
    method: &Method<'_>,
) -> Result<Added, Failure> {
    let mut tx = db.begin().await.map_err(Failure::Database)?;
    // The session's row stays until the link commits.
    let account: Option<(String,)> = sqlx::query_as(
        "SELECT account_id::text FROM sessions
         WHERE token_hash = $1 AND kind = 'web' AND created_at > now() - make_interval(secs => $2)
         FOR SHARE",
    )
    .bind(session)
    .bind(WEB_LIFETIME.as_secs_f64())
    .fetch_optional(&mut *tx)
    .await
    .map_err(Failure::Database)?;
    let (account,) = account.ok_or(Failure::Session(
        "the session that started the link has ended.",
    ))?;
    // Of two links of one method at once, the second waits for the first to commit and adds
    // nothing.
    let added = sqlx::query(
        "INSERT INTO sign_in_methods (provider, subject, account_id, login)
         VALUES ($1, $2, $3::uuid, $4) ON CONFLICT (provider, subject) DO NOTHING",
    )
    .bind(method.provider)
    .bind(&method.subject)
    .bind(&account)
    .bind(method.login)
    .execute(&mut *tx)
    .await
    .map_err(Failure::Database)?;
    let outcome = if added.rows_affected() == 1 {
        Added::Linked
    } else {
        let (owner,): (String,) = sqlx::query_as(
            "SELECT account_id::text FROM sign_in_methods WHERE provider = $1 AND subject = $2",
        )
        .bind(method.provider)
        .bind(&method.subject)
        .fetch_one(&mut *tx)
        .await
        .map_err(Failure::Database)?;
        if owner != account {
            return Err(Failure::OtherAccount);
        }
        Added::Already
    };
    tx.commit().await.map_err(Failure::Database)?;
    Ok(outcome)
}

/// The page that ends a link of `flow` that `added` and sends the browser to `return_to`.
pub(crate) fn linked_page(
    settings: &SignInSettings,
    flow: &Flow,
    added: Added,
    return_to: &str,
) -> Response {
    let done = match added {
        Added::Linked => format!(
            "Linked: your {} account signs in to this account.",
            flow.name
        ),
        Added::Already => format!("Your account already has this {} account.", flow.name),
    };
    refresh_page(settings, flow, "Linked", &done, return_to)
}
