//! Links of a sign-in method: a web session from a recent sign-in links another GitHub or Google
//! method to its account, through the provider's sign-in callback; and the account's sign-in
//! methods, on a page and in the API. Accounts never merge or move a method.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{GITHUB, Method};
use crate::refusal::{Refusal, account, answer};
use crate::sign_in::{
    self, Failure, Flow, Provider, SignInSettings, escape, flows, form_page, form_page_failed,
    from_this_server, off, provider_off, refresh_page, session_hashes, sign_in_links, start_with,
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
    provider: String,
    login: Option<String>,
    /// When it was linked, as an RFC 3339 date-time in UTC.
    linked_at: String,
}

impl Stored {
    /// The provider's name in the API. The server's only OIDC provider is Google.
    fn kind(&self) -> &'static str {
        if self.provider == GITHUB {
            "github"
        } else {
            "google"
        }
    }
}

/// The sign-in methods of `account`, oldest first.
async fn stored(db: &PgPool, account: &str) -> Result<Vec<Stored>, sqlx::Error> {
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT provider, login,
                to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')
         FROM sign_in_methods WHERE account_id = $1::uuid ORDER BY created_at, provider, subject",
    )
    .bind(account)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(provider, login, linked_at)| Stored {
            provider,
            login,
            linked_at,
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
                    "<h1>Sign-in methods</h1><p>Sign in to see your account's sign-in methods. {}</p>",
                    sign_in_links(&flows, "", PAGE_PATH)
                ),
            );
        }
        Err(err) => return failed("read a session", err),
    };
    let methods = match stored(&state.db, &session.account).await {
        Ok(methods) => methods,
        Err(err) => return failed("read the sign-in methods", err),
    };
    let items: String = methods
        .iter()
        .map(|method| {
            let name = match (method.kind(), &method.login) {
                ("github", Some(login)) => format!("GitHub <strong>{}</strong>", escape(login)),
                ("github", None) => "GitHub".to_owned(),
                _ => "Google".to_owned(),
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
        .map(|flow| {
            format!(
                "<form method=\"post\" action=\"{}\"><button>Link {}</button></form>",
                flow.link, flow.name
            )
        })
        .collect();
    let body = format!(
        "<h1>Sign-in methods</h1>\
         <p>Each of these signs in to your account <strong>{username}</strong>.</p><ul>{items}</ul>\
         <p>Link another one: it signs in to this account too. A method of another account is not \
         linked.</p>{forms}",
        username = escape(&session.username),
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
    provider: &'static str,
    linked_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    github_login: Option<String>,
}

/// `GET /account/sign-in-methods`: the sign-in methods of the session's account, web or device.
pub(crate) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let methods = stored(&state.db, &account)
            .await
            .map_err(|err| Refusal::Database("read the sign-in methods", err))?;
        let methods = methods
            .into_iter()
            .map(|method| SignInMethod {
                provider: method.kind(),
                github_login: method.login.filter(|_| method.provider == GITHUB),
                linked_at: method.linked_at,
            })
            .collect();
        Ok(Json(SignInMethods { methods }).into_response())
    })
    .await
}

/// `POST /auth/github/link` and `POST /auth/google/link`: starts a link of a method of the provider
/// for the web session in the request's cookie, if the request comes from a page of this server and
/// the session's sign-in is recent enough.
pub(crate) async fn start<P: Provider>(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let Some((settings, provider)) = P::of(&state) else {
        return provider_off(P::NAME);
    };
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
         FROM sessions WHERE kind = 'web' AND token_hash = ANY($1) LIMIT 1",
    )
    .bind(session_hashes(headers))
    .bind(settings.link_age.as_secs() as f64)
    .fetch_optional(&state.db)
    .await;
    match found {
        Ok(Some((session, _, true))) => {
            start_with(
                &state.db,
                settings,
                provider,
                Some(PAGE_PATH),
                Some(&session),
            )
            .await
        }
        Ok(Some((_, account, false))) => {
            let methods = match stored(&state.db, &account).await {
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
                    sign_in_links(&flows, " again", PAGE_PATH)
                ),
            )
        }
        Ok(None) => page_of(
            StatusCode::FORBIDDEN,
            "Not signed in",
            &format!(
                "<p>Not signed in. {}</p>",
                sign_in_links(&flows(state), "", PAGE_PATH)
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
/// session lasts, unless another account has the method.
pub(crate) async fn add(
    db: &PgPool,
    session: &[u8],
    method: &Method<'_>,
) -> Result<Added, Failure> {
    let mut tx = db.begin().await.map_err(Failure::Database)?;
    // The session's row stays until the link commits.
    let account: Option<(String,)> = sqlx::query_as(
        "SELECT account_id::text FROM sessions WHERE token_hash = $1 AND kind = 'web' FOR SHARE",
    )
    .bind(session)
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
