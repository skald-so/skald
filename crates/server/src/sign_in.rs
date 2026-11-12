//! GitHub sign-in in the browser and the web session it ends in: OAuth 2.0's authorization code
//! flow (RFC 6749, §4.1) with a GitHub OAuth App, asking for no scope.

use std::str::FromStr;
use std::time::Duration;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sha2::{Digest, Sha256};
use skald_proto::AccountId;
use sqlx::PgPool;
use url::Url;

use crate::github::{GitHubApp, Refusal, http_url};
use crate::{AppState, problem};

/// The callback's path under the public address.
pub const CALLBACK_PATH: &str = "/auth/github/callback";
/// The cookie that binds a sign-in's `state` to the browser that started it.
const STATE_COOKIE: &str = "skald_sign_in";
/// The session cookie.
const SESSION_COOKIE: &str = "skald_session";

/// The server's public address: an http or https origin, such as `https://skald.example`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrl(Url);

impl PublicUrl {
    /// Whether browsers reach the server over HTTPS, so that its cookies are `Secure`.
    fn secure(&self) -> bool {
        self.0.scheme() == "https"
    }

    /// `to`, an address relative to this server's, if it is a page of this server.
    fn page(&self, to: &str) -> Option<Url> {
        let url = self.0.join(to).ok()?;
        (url.origin() == self.0.origin()).then_some(url)
    }

    /// The address of `path`, which starts with `/`.
    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.0.origin().ascii_serialization())
    }
}

impl FromStr for PublicUrl {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let url = http_url(text)?;
        let origin = url.origin().ascii_serialization();
        if url.as_str().trim_end_matches('/') != origin {
            return Err(format!(
                "not an origin: give the scheme, host and port only, such as {origin}"
            ));
        }
        Ok(PublicUrl(url))
    }
}

/// GitHub sign-in's settings.
#[derive(Debug, Clone)]
pub struct SignInSettings {
    /// The server's public address; the callback is under it, at [`CALLBACK_PATH`].
    pub public_url: PublicUrl,
    pub github: GitHubApp,
    /// How long a sign-in's `state` works.
    pub state_lifetime: Duration,
}

impl SignInSettings {
    fn callback(&self) -> String {
        self.public_url.at(CALLBACK_PATH)
    }

    /// A cookie of `name` with `value` for `path`, which page scripts cannot read, sent only over
    /// HTTPS under an https public address.
    fn cookie(
        &self,
        name: &str,
        value: &str,
        path: &str,
        same_site: &str,
        max_age: Option<u64>,
    ) -> HeaderValue {
        let mut cookie = format!("{name}={value}; Path={path}; HttpOnly; SameSite={same_site}");
        if let Some(seconds) = max_age {
            cookie.push_str(&format!("; Max-Age={seconds}"));
        }
        if self.public_url.secure() {
            cookie.push_str("; Secure");
        }
        HeaderValue::try_from(cookie).expect("a cookie of tokens and paths is a header value")
    }

    /// The cookie that binds `state`, sent back on GitHub's cross-site redirect to the callback.
    fn state_cookie(&self, state: &str, max_age: u64) -> HeaderValue {
        self.cookie(STATE_COOKIE, state, CALLBACK_PATH, "Lax", Some(max_age))
    }
}

/// A token of 256 random bits from the operating system's cryptographically secure generator, in
/// unpadded base64url.
fn random_token() -> String {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).expect("the operating system's random generator failed");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The SHA-256 of `token`: how the database holds tokens.
fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// The first value of the query parameter `name`.
fn param(query: Option<&str>, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The values of the cookies `name` that a request carries.
fn cookies<'a>(headers: &'a HeaderMap, name: &'a str) -> impl Iterator<Item = &'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(move |pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then_some(value)
        })
}

/// The answer while the server has no GitHub settings.
fn off() -> Response {
    problem(
        StatusCode::NOT_FOUND,
        "github-sign-in-off",
        "GitHub sign-in is off",
        "This server has no GitHub settings.",
    )
}

/// `GET /auth/github`: sends the browser to GitHub with a new `state`, bound to it by a cookie.
pub(crate) async fn start(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let Some(settings) = state.sign_in.as_deref() else {
        return off();
    };
    let public = &settings.public_url;
    let return_to = param(query.as_deref(), "return_to")
        .and_then(|to| public.page(&to))
        .unwrap_or_else(|| public.page("/").expect("the root is a page of the server"));
    let token = random_token();
    let lifetime = settings.state_lifetime.as_secs();
    let stored = sqlx::query(
        "WITH expired AS (DELETE FROM sign_in_states WHERE expires_at <= now())
         INSERT INTO sign_in_states (state_hash, return_to, expires_at)
         VALUES ($1, $2, now() + make_interval(secs => $3))",
    )
    .bind(digest(&token))
    .bind(return_to.as_str())
    .bind(lifetime as f64)
    .execute(&state.db)
    .await;
    if let Err(err) = stored {
        return Failure::Database(err).page();
    }

    let mut authorize = Url::parse(&settings.github.url.at("/login/oauth/authorize"))
        .expect("GitHub's address and a path make a URL");
    authorize
        .query_pairs_mut()
        .append_pair("client_id", &settings.github.client_id)
        .append_pair("redirect_uri", &settings.callback())
        .append_pair("state", &token);
    let mut response = StatusCode::FOUND.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::LOCATION,
        HeaderValue::try_from(authorize.as_str()).expect("a URL is a header value"),
    );
    headers.insert(header::SET_COOKIE, settings.state_cookie(&token, lifetime));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Why a sign-in signed no one in.
#[derive(Debug)]
enum Failure {
    /// The request or the person's answer: a missing, wrong, expired or used `state`, no code, or
    /// one GitHub refused.
    Refused(&'static str),
    GitHub(String),
    Database(sqlx::Error),
}

impl Failure {
    /// The page that says the sign-in failed; failures of GitHub or the database also go to the
    /// server's output.
    fn page(self) -> Response {
        let (status, message) = match self {
            Failure::Refused(message) => (StatusCode::BAD_REQUEST, message),
            Failure::GitHub(reason) => {
                eprintln!("skald-server: GitHub sign-in failed: {reason}");
                (
                    StatusCode::BAD_GATEWAY,
                    "GitHub did not complete the sign-in.",
                )
            }
            Failure::Database(err) => {
                eprintln!("skald-server: GitHub sign-in failed: the database failed: {err}");
                (StatusCode::INTERNAL_SERVER_ERROR, "The server failed.")
            }
        };
        let body =
            format!("<p>Not signed in: {message} <a href=\"/auth/github\">Sign in again</a></p>");
        page(status, "Sign-in failed", &body)
    }
}

/// A page of `status` with `title` and `body`: no scripts, no caching, no referrer.
fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>{title}</title>{body}</html>"
    );
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'"),
        ],
        html,
    )
        .into_response()
}

/// `text` safe in HTML text and attributes.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `GET /auth/github/callback`: checks `state`, reads the GitHub user, finds or creates their
/// account and starts a session.
pub(crate) async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(settings) = state.sign_in.as_deref() else {
        return off();
    };
    match finish(&state.db, settings, &headers, query.as_deref()).await {
        Ok((session, return_to)) => {
            // A page, not a redirect: the browser's navigation to the return page then starts on
            // this site, so it carries the `SameSite=Strict` session cookie.
            let to = escape(&return_to);
            let body = format!(
                "<meta http-equiv=\"refresh\" content=\"0;url={to}\"><p>Signed in. <a href=\"{to}\">Continue</a></p>"
            );
            let mut response = page(StatusCode::OK, "Signed in", &body);
            let headers = response.headers_mut();
            headers.append(
                header::SET_COOKIE,
                settings.cookie(SESSION_COOKIE, &session, "/", "Strict", None),
            );
            headers.append(header::SET_COOKIE, settings.state_cookie("", 0));
            response
        }
        Err(failure) => failure.page(),
    }
}

/// The session token and the return address of a sign-in that GitHub's callback with `query`
/// completes.
async fn finish(
    db: &PgPool,
    settings: &SignInSettings,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<(String, String), Failure> {
    let given = param(query, "state")
        .filter(|given| !given.is_empty())
        .ok_or(Failure::Refused("the sign-in has no state."))?;
    let hash = digest(&given);
    if !cookies(headers, STATE_COOKIE).any(|cookie| digest(cookie) == hash) {
        return Err(Failure::Refused("this browser did not start this sign-in."));
    }
    let (return_to,): (String,) = sqlx::query_as(
        "DELETE FROM sign_in_states WHERE state_hash = $1 AND expires_at > now() RETURNING return_to",
    )
    .bind(&hash)
    .fetch_optional(db)
    .await
    .map_err(Failure::Database)?
    .ok_or(Failure::Refused("the sign-in expired or was used."))?;
    if param(query, "error").is_some() {
        return Err(Failure::Refused("GitHub did not authorize it."));
    }
    let code = param(query, "code")
        .filter(|code| !code.is_empty())
        .ok_or(Failure::Refused("GitHub gave no code."))?;

    let github = settings.github.clone();
    let callback = settings.callback();
    let user = tokio::task::spawn_blocking(move || github.user(&code, &callback))
        .await
        .map_err(|err| Failure::GitHub(format!("the request to GitHub failed: {err}")))?
        .map_err(|refusal| match refusal {
            Refusal::Code => Failure::Refused("GitHub refused the code."),
            Refusal::GitHub(reason) => Failure::GitHub(reason),
        })?;
    let github_id = i64::try_from(user.id)
        .map_err(|_| Failure::GitHub(format!("GitHub gave the user ID {}", user.id)))?;

    let session = random_token();
    sqlx::query(
        "WITH account AS (
             INSERT INTO accounts (id, github_id, github_login) VALUES ($1::uuid, $2, $3)
             ON CONFLICT (github_id) DO UPDATE SET github_login = EXCLUDED.github_login
             RETURNING id
         )
         INSERT INTO sessions (token_hash, account_id) SELECT $4, id FROM account",
    )
    .bind(AccountId::generate().to_string())
    .bind(github_id)
    .bind(&user.login)
    .bind(digest(&session))
    .execute(db)
    .await
    .map_err(Failure::Database)?;
    Ok((session, return_to))
}

/// `Session` in the spec.
#[derive(Serialize)]
struct Session {
    account: String,
    github_login: String,
}

/// `GET /session`: the account of the request's session.
pub(crate) async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let hashes: Vec<Vec<u8>> = cookies(&headers, SESSION_COOKIE).map(digest).collect();
    let found: Result<Option<(String, String)>, _> = sqlx::query_as(
        "SELECT a.id::text, a.github_login FROM sessions s JOIN accounts a ON a.id = s.account_id
         WHERE s.token_hash = ANY($1) LIMIT 1",
    )
    .bind(hashes)
    .fetch_optional(&state.db)
    .await;
    match found {
        Ok(Some((account, github_login))) => axum::Json(Session {
            account,
            github_login,
        })
        .into_response(),
        Ok(None) => problem(
            StatusCode::UNAUTHORIZED,
            "not-signed-in",
            "Not signed in",
            "The request has no session, or an unknown one: sign in at /auth/github.",
        ),
        Err(err) => {
            eprintln!("skald-server: cannot read a session: {err}");
            problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server-error",
                "The server failed",
                "The database failed.",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_url_is_an_origin() {
        for text in [
            "https://skald.example",
            "https://skald.example/",
            "http://127.0.0.1:8080",
        ] {
            assert!(text.parse::<PublicUrl>().is_ok(), "{text}");
        }
        for text in [
            "not a URL",
            "skald.example",
            "ftp://skald.example",
            "https://skald.example/skald",
            "https://skald.example/?q=1",
            "https://skald.example/#top",
            "https://user@skald.example",
        ] {
            assert!(text.parse::<PublicUrl>().is_err(), "{text}");
        }
    }

    #[test]
    fn public_url_is_secure_only_over_https() {
        assert!(
            "https://skald.example"
                .parse::<PublicUrl>()
                .unwrap()
                .secure()
        );
        assert!(
            !"http://skald.example"
                .parse::<PublicUrl>()
                .unwrap()
                .secure()
        );
    }

    #[test]
    fn escape_leaves_no_markup() {
        assert_eq!(
            escape(r#"<a href="x">'&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn tokens_are_256_random_bits() {
        let token = random_token();
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).expect("base64url").len(), 32);
        assert_ne!(token, random_token());
    }
}
