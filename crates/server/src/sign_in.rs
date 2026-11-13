//! Sign-in in the browser and the web session it ends in, the parts every provider shares; and
//! GitHub sign-in: OAuth 2.0's authorization code flow (RFC 6749, §4.1) with a GitHub OAuth App,
//! asking for no scope. Google sign-in is in `google`.

use std::str::FromStr;
use std::time::Duration;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use url::Url;

use crate::accounts::{self, GITHUB, Method};
use crate::github::{GitHubApp, Refusal, http_url};
use crate::oidc::OidcSettings;
use crate::{AppState, database_failed, problem};

/// GitHub sign-in's start's path under the public address.
pub(crate) const START_PATH: &str = "/auth/github";
/// GitHub's callback's path under the public address.
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
    pub(crate) fn at(&self, path: &str) -> String {
        format!("{}{path}", self.origin())
    }

    /// The origin, as a browser's `Origin` header gives it, such as `https://skald.example`.
    pub(crate) fn origin(&self) -> String {
        self.0.origin().ascii_serialization()
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

/// Sign-in's settings: the providers and the device flow.
#[derive(Debug, Clone)]
pub struct SignInSettings {
    /// The server's public address; the providers' callbacks are under it.
    pub public_url: PublicUrl,
    /// GitHub sign-in and the device flow; both are off without it.
    pub github: Option<GitHubApp>,
    /// Google sign-in, with the OIDC provider at Google's issuer; off without it.
    pub google: Option<OidcSettings>,
    /// How long a sign-in's `state` works.
    pub state_lifetime: Duration,
    /// How long a device flow's codes work.
    pub device_code_lifetime: Duration,
    /// The least time between a device flow's polls.
    pub device_poll_interval: Duration,
}

impl SignInSettings {
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

    /// The cookie that binds `state` of a sign-in of `flow`, sent back on the provider's cross-site
    /// redirect to the callback.
    fn state_cookie(&self, flow: &Flow, state: &str, max_age: u64) -> HeaderValue {
        self.cookie(STATE_COOKIE, state, flow.callback, "Lax", Some(max_age))
    }
}

/// A provider's sign-in in the browser.
pub(crate) struct Flow<'a> {
    /// The provider's name for people, such as `GitHub`.
    pub name: &'static str,
    /// The provider of its sign-in methods: [`GITHUB`] or an OIDC provider's issuer.
    pub provider: &'a str,
    /// The sign-in start's path under the public address.
    pub start: &'static str,
    /// The callback's path under the public address.
    pub callback: &'static str,
}

/// GitHub sign-in.
const GITHUB_FLOW: Flow<'static> = Flow {
    name: "GitHub",
    provider: GITHUB,
    start: START_PATH,
    callback: CALLBACK_PATH,
};

/// The sign-in settings and the GitHub OAuth App, while the server has GitHub's settings.
pub(crate) fn github(state: &AppState) -> Option<(&SignInSettings, &GitHubApp)> {
    let settings = state.sign_in.as_deref()?;
    Some((settings, settings.github.as_ref()?))
}

/// What an OIDC sign-in keeps beside its `state` until its callback: its `nonce` and PKCE verifier.
pub(crate) struct Secrets {
    pub nonce: String,
    pub verifier: String,
}

/// Starts a sign-in of `flow` for the start's `query`: stores a new `state`, the return address and
/// an OIDC sign-in's `secrets` until the `state` expires. Gives the `state`.
pub(crate) async fn begin(
    db: &PgPool,
    settings: &SignInSettings,
    flow: &Flow<'_>,
    query: Option<&str>,
    secrets: Option<&Secrets>,
) -> Result<String, Failure> {
    let public = &settings.public_url;
    let return_to = param(query, "return_to")
        .and_then(|to| public.page(&to))
        .unwrap_or_else(|| public.page("/").expect("the root is a page of the server"));
    let token = random_token();
    sqlx::query(
        "WITH expired AS (DELETE FROM sign_in_states WHERE expires_at <= now())
         INSERT INTO sign_in_states (state_hash, provider, return_to, nonce, code_verifier, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))",
    )
    .bind(digest(&token))
    .bind(flow.provider)
    .bind(return_to.as_str())
    .bind(secrets.map(|secrets| secrets.nonce.as_str()))
    .bind(secrets.map(|secrets| secrets.verifier.as_str()))
    .bind(settings.state_lifetime.as_secs() as f64)
    .execute(db)
    .await
    .map_err(Failure::Database)?;
    Ok(token)
}

/// Sends the browser to `authorize`, the provider's authorization request of a sign-in of `flow`,
/// with the cookie that binds its `state` to the browser.
pub(crate) fn redirect(
    settings: &SignInSettings,
    flow: &Flow,
    authorize: &str,
    state: &str,
) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::LOCATION,
        HeaderValue::try_from(authorize).expect("a URL is a header value"),
    );
    headers.insert(
        header::SET_COOKIE,
        settings.state_cookie(flow, state, settings.state_lifetime.as_secs()),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The return address and the secrets of the sign-in of `flow` whose `state` the callback's
/// `query` gives, if the browser's cookie binds that `state` and it works; it works no more.
pub(crate) async fn take(
    db: &PgPool,
    flow: &Flow<'_>,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<(String, Option<Secrets>), Failure> {
    let given = param(query, "state")
        .filter(|given| !given.is_empty())
        .ok_or(Failure::Refused("the sign-in has no state."))?;
    let hash = digest(&given);
    if !cookies(headers, STATE_COOKIE).any(|cookie| digest(cookie) == hash) {
        return Err(Failure::Refused("this browser did not start this sign-in."));
    }
    let (return_to, nonce, verifier): (String, Option<String>, Option<String>) = sqlx::query_as(
        "DELETE FROM sign_in_states WHERE state_hash = $1 AND provider = $2 AND expires_at > now()
         RETURNING return_to, nonce, code_verifier",
    )
    .bind(&hash)
    .bind(flow.provider)
    .fetch_optional(db)
    .await
    .map_err(Failure::Database)?
    .ok_or(Failure::Refused("the sign-in expired or was used."))?;
    let secrets = nonce
        .zip(verifier)
        .map(|(nonce, verifier)| Secrets { nonce, verifier });
    Ok((return_to, secrets))
}

/// The page that ends a sign-in of `flow` in the web session `session` and sends the browser to
/// `return_to`.
pub(crate) fn signed_in_page(
    settings: &SignInSettings,
    flow: &Flow,
    session: &str,
    return_to: &str,
) -> Response {
    // A page, not a redirect: the browser's navigation to the return page then starts on this
    // site, so it carries the `SameSite=Strict` session cookie.
    let to = escape(return_to);
    let body = format!(
        "<meta http-equiv=\"refresh\" content=\"0;url={to}\"><p>Signed in. <a href=\"{to}\">Continue</a></p>"
    );
    let mut response = page(StatusCode::OK, "Signed in", &body);
    let headers = response.headers_mut();
    headers.append(
        header::SET_COOKIE,
        settings.cookie(SESSION_COOKIE, session, "/", "Strict", None),
    );
    headers.append(header::SET_COOKIE, settings.state_cookie(flow, "", 0));
    response
}

/// A token of 256 random bits from the operating system's cryptographically secure generator, in
/// unpadded base64url.
pub(crate) fn random_token() -> String {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).expect("the operating system's random generator failed");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The SHA-256 of `token`: how the database holds tokens.
pub(crate) fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// The first value of the query parameter `name`.
pub(crate) fn param(query: Option<&str>, name: &str) -> Option<String> {
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
pub(crate) fn off() -> Response {
    problem(
        StatusCode::NOT_FOUND,
        "github-sign-in-off",
        "GitHub sign-in is off",
        "This server has no GitHub settings.",
    )
}

/// `GET /auth/github`: sends the browser to GitHub with a new `state`, bound to it by a cookie.
pub(crate) async fn start(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let Some((settings, github)) = self::github(&state) else {
        return off();
    };
    let flow = &GITHUB_FLOW;
    let token = match begin(&state.db, settings, flow, query.as_deref(), None).await {
        Ok(token) => token,
        Err(failure) => return failure.page(flow),
    };
    let mut authorize = Url::parse(&github.url.at("/login/oauth/authorize"))
        .expect("GitHub's address and a path make a URL");
    authorize
        .query_pairs_mut()
        .append_pair("client_id", &github.client_id)
        .append_pair("redirect_uri", &settings.public_url.at(CALLBACK_PATH))
        .append_pair("state", &token);
    redirect(settings, flow, authorize.as_str(), &token)
}

/// Why a sign-in signed no one in.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The request or the person's answer: a missing, wrong, expired or used `state`, no code, or
    /// a refused one.
    Refused(&'static str),
    /// The provider failed, is unreachable or refused the server's client.
    Provider(String),
    Database(sqlx::Error),
}

impl Failure {
    /// The page that says the sign-in of `flow` failed; failures of the provider or the database
    /// also go to the server's output.
    pub(crate) fn page(self, flow: &Flow) -> Response {
        let name = flow.name;
        let (status, message) = match self {
            Failure::Refused(message) => (StatusCode::BAD_REQUEST, message.to_owned()),
            Failure::Provider(reason) => {
                eprintln!("skald-server: {name} sign-in failed: {reason}");
                (
                    StatusCode::BAD_GATEWAY,
                    format!("{name} did not complete the sign-in."),
                )
            }
            Failure::Database(err) => {
                eprintln!("skald-server: {name} sign-in failed: the database failed: {err}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The server failed.".to_owned(),
                )
            }
        };
        let body = format!(
            "<p>Not signed in: {message} <a href=\"{}\">Sign in again</a></p>",
            flow.start
        );
        page(status, "Sign-in failed", &body)
    }
}

/// A page of `status` with `title` and `body`: no scripts, no caching, no referrer.
pub(crate) fn page(status: StatusCode, title: &str, body: &str) -> Response {
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
pub(crate) fn escape(text: &str) -> String {
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
    let Some((settings, github)) = self::github(&state) else {
        return off();
    };
    let flow = &GITHUB_FLOW;
    match finish(&state.db, settings, github, &headers, query.as_deref()).await {
        Ok((session, return_to)) => signed_in_page(settings, flow, &session, &return_to),
        Err(failure) => failure.page(flow),
    }
}

/// The session token and the return address of a sign-in that GitHub's callback with `query`
/// completes.
async fn finish(
    db: &PgPool,
    settings: &SignInSettings,
    github: &GitHubApp,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<(String, String), Failure> {
    let (return_to, _) = take(db, &GITHUB_FLOW, headers, query).await?;
    if param(query, "error").is_some() {
        return Err(Failure::Refused("GitHub did not authorize it."));
    }
    let code = param(query, "code")
        .filter(|code| !code.is_empty())
        .ok_or(Failure::Refused("GitHub gave no code."))?;

    let github = github.clone();
    let callback = settings.public_url.at(CALLBACK_PATH);
    let user = tokio::task::spawn_blocking(move || github.user(&code, &callback))
        .await
        .map_err(|err| Failure::Provider(format!("the request to GitHub failed: {err}")))?
        .map_err(|refusal| match refusal {
            Refusal::Code => Failure::Refused("GitHub refused the code."),
            Refusal::GitHub(reason) => Failure::Provider(reason),
        })?;
    let method = Method {
        provider: GITHUB,
        subject: user.id.to_string(),
        login: Some(&user.login),
    };
    let session = accounts::sign_in(db, &method, &user.login)
        .await
        .map_err(Failure::Database)?;
    Ok((session, return_to))
}

/// `Session` in the spec: a session's account.
#[derive(Serialize)]
pub(crate) struct Session {
    pub account: String,
    pub username: String,
    /// The login of the account's GitHub method, while it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_login: Option<String>,
}

/// The account of the session of `kind`, `web` or `device`, whose token has one of `hashes`.
async fn find(
    db: &PgPool,
    kind: &str,
    hashes: Vec<Vec<u8>>,
) -> Result<Option<Session>, sqlx::Error> {
    let found: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT a.id::text, a.username, m.login FROM sessions s
         JOIN accounts a ON a.id = s.account_id
         LEFT JOIN LATERAL (
             SELECT login FROM sign_in_methods
             WHERE account_id = a.id AND provider = $3 ORDER BY created_at LIMIT 1
         ) m ON true
         WHERE s.kind = $1 AND s.token_hash = ANY($2) LIMIT 1",
    )
    .bind(kind)
    .bind(hashes)
    .bind(GITHUB)
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(account, username, github_login)| Session {
        account,
        username,
        github_login,
    }))
}

/// The account of the web session in the request's cookie.
pub(crate) async fn web_session(
    db: &PgPool,
    headers: &HeaderMap,
) -> Result<Option<Session>, sqlx::Error> {
    find(
        db,
        "web",
        cookies(headers, SESSION_COOKIE).map(digest).collect(),
    )
    .await
}

/// The token of the request's only `Authorization` header, `Bearer <token>`; `None` without one,
/// `Some(None)` for any other.
fn bearer(headers: &HeaderMap) -> Option<Option<&str>> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let first = values.next()?;
    if values.next().is_some() {
        return Some(None);
    }
    Some(first.to_str().ok().and_then(|value| {
        let (scheme, token) = value.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then_some(token)
    }))
}

/// `GET /session`: the account of the request's session.
pub(crate) async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match signed_in(&state.db, &headers).await {
        Ok(Some(session)) => axum::Json(session).into_response(),
        Ok(None) => not_signed_in(),
        Err(err) => database_failed("read a session", err),
    }
}

/// `DELETE /session`: ends the device session of the request's bearer token.
pub(crate) async fn end(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(Some(token)) = bearer(&headers) else {
        return not_signed_in();
    };
    let ended = sqlx::query("DELETE FROM sessions WHERE kind = 'device' AND token_hash = $1")
        .bind(digest(token))
        .execute(&state.db)
        .await;
    match ended {
        Ok(done) if done.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => not_signed_in(),
        Err(err) => database_failed("end a session", err),
    }
}

/// The account of the request's session: the device session of its bearer token if it has an
/// `Authorization` header, else the web session of its cookie.
pub(crate) async fn signed_in(
    db: &PgPool,
    headers: &HeaderMap,
) -> Result<Option<Session>, sqlx::Error> {
    match bearer(headers) {
        Some(Some(token)) => find(db, "device", vec![digest(token)]).await,
        Some(None) => Ok(None),
        None => web_session(db, headers).await,
    }
}

/// The answer to a request without a session.
pub(crate) fn not_signed_in() -> Response {
    problem(
        StatusCode::UNAUTHORIZED,
        "not-signed-in",
        "Not signed in",
        "The request has no session, or an unknown one: sign in first.",
    )
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
