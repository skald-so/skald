//! Sign-in in the browser and the web session it ends in, the parts every provider shares, links of
//! a sign-in method among them; and GitHub sign-in: OAuth 2.0's authorization code flow (RFC 6749,
//! §4.1) with a GitHub OAuth App and PKCE (RFC 7636), asking for no scope. Google sign-in is in
//! `google`; starting a link and the account's sign-in methods in `links`.

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
use crate::google::Google;
use crate::oidc::OidcSettings;
use crate::{AppState, database_failed, links, problem};

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
    /// GitHub sign-in; off without it. The device flow is off without any provider.
    pub github: Option<GitHubApp>,
    /// Google sign-in, with the OIDC provider at Google's issuer; off without it.
    pub google: Option<OidcSettings>,
    /// How long a sign-in's `state` works.
    pub state_lifetime: Duration,
    /// How recent a web session's sign-in has to be to start a link.
    pub link_age: Duration,
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
    /// The link start's path under the public address.
    pub link: &'static str,
}

/// The sign-in settings, while the server has the settings of any provider.
pub(crate) fn settings(state: &AppState) -> Option<&SignInSettings> {
    state
        .sign_in
        .as_deref()
        .filter(|settings| settings.github.is_some() || settings.google.is_some())
}

/// The flows of the providers the server has settings for.
pub(crate) fn flows(state: &AppState) -> Vec<Flow<'_>> {
    let mut flows = Vec::new();
    if let Some((_, github)) = GitHubApp::of(state) {
        flows.push(github.flow());
    }
    if let Some((_, google)) = Google::of(state) {
        flows.push(google.flow());
    }
    flows
}

/// Links that sign in with each of `flows` and return to `return_to`, a page of this server.
pub(crate) fn sign_in_links(flows: &[Flow], again: &str, return_to: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("return_to", return_to)
        .finish();
    let links: Vec<String> = flows
        .iter()
        .map(|flow| {
            format!(
                "<a href=\"{}?{}\">Sign in{again} with {}</a>",
                flow.start,
                escape(&query),
                flow.name
            )
        })
        .collect();
    links.join(" ")
}

/// What a sign-in keeps beside its `state` until its callback: its PKCE verifier and an OIDC
/// sign-in's `nonce`.
pub(crate) struct Secrets {
    pub verifier: String,
    pub nonce: Option<String>,
}

impl Secrets {
    /// New secrets of a sign-in, with a `nonce` if it is an OIDC one.
    fn new(oidc: bool) -> Self {
        Secrets {
            verifier: random_token(),
            nonce: oidc.then(random_token),
        }
    }

    /// The S256 PKCE challenge of the verifier (RFC 7636, §4.2).
    pub(crate) fn challenge(&self) -> String {
        s256(&self.verifier)
    }
}

/// The S256 PKCE challenge of `verifier`.
fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A person a provider brought back.
pub(crate) struct Identity {
    /// The subject the provider gives the person: for GitHub, the user's ID in decimal.
    pub subject: String,
    /// A GitHub user's login.
    pub login: Option<String>,
    /// What a sign-up makes the username from, such as the GitHub login.
    pub source: String,
}

/// A provider people sign in with in the browser.
pub(crate) trait Provider: Sized {
    /// The provider's name for people, such as `GitHub`.
    const NAME: &'static str;

    /// The sign-in settings and the provider, while the server has the provider's settings.
    fn of(state: &AppState) -> Option<(&SignInSettings, &Self)>;

    fn flow(&self) -> Flow<'_>;

    /// The address of the provider's authorization request of a sign-in, or of a link if `link`,
    /// with `state` and the challenge, and an OIDC sign-in's `nonce`, of `secrets`.
    async fn authorize(
        &self,
        settings: &SignInSettings,
        state: &str,
        secrets: &Secrets,
        link: bool,
    ) -> Result<String, Failure>;

    /// The person that the provider brings back from the sign-in of `secrets` with `code`, the code
    /// its callback gave.
    async fn identify(
        &self,
        settings: &SignInSettings,
        code: String,
        secrets: Secrets,
    ) -> Result<Identity, Failure>;
}

/// The result of `request`, which blocks on the provider `name`, run off the async runtime.
pub(crate) async fn blocking<T: Send + 'static>(
    name: &str,
    request: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(request)
        .await
        .map_err(|err| Failure::Provider(format!("the request to {name} failed: {err}")))
}

/// `GET /auth/github` and `GET /auth/google`: sends the browser to the provider with a new `state`,
/// bound to it by a cookie, a PKCE challenge and an OIDC sign-in's `nonce`.
pub(crate) async fn start<P: Provider>(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some((settings, provider)) = P::of(&state) else {
        return provider_off(P::NAME);
    };
    let return_to = param(query.as_deref(), "return_to");
    start_with(&state.db, settings, provider, return_to.as_deref(), None).await
}

/// Starts a sign-in with `provider`, or a link of the web session whose token's SHA-256 is `link`:
/// stores a new `state`, the return address `return_to`, if a page of this server, else `/`, and
/// the sign-in's secrets until the `state` expires; sends the browser to the provider.
pub(crate) async fn start_with<P: Provider>(
    db: &PgPool,
    settings: &SignInSettings,
    provider: &P,
    return_to: Option<&str>,
    link: Option<&[u8]>,
) -> Response {
    let flow = provider.flow();
    let public = &settings.public_url;
    let return_to = return_to
        .and_then(|to| public.page(to))
        .unwrap_or_else(|| public.page("/").expect("the root is a page of the server"));
    let secrets = Secrets::new(flow.provider != GITHUB);
    let token = random_token();
    let stored = sqlx::query(
        "WITH expired AS (DELETE FROM sign_in_states WHERE expires_at <= now())
         INSERT INTO sign_in_states
             (state_hash, provider, return_to, nonce, code_verifier, link_session, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7))",
    )
    .bind(digest(&token))
    .bind(flow.provider)
    .bind(return_to.as_str())
    .bind(secrets.nonce.as_deref())
    .bind(&secrets.verifier)
    .bind(link)
    .bind(settings.state_lifetime.as_secs() as f64)
    .execute(db)
    .await;
    let authorize = match stored {
        Ok(_) => {
            provider
                .authorize(settings, &token, &secrets, link.is_some())
                .await
        }
        Err(err) => Err(Failure::Database(err)),
    };
    match authorize {
        Ok(authorize) => redirect(settings, &flow, &authorize, &token),
        Err(failure) => failure.page(&flow, link.is_some()),
    }
}

/// Sends the browser to `authorize`, the provider's authorization request of a sign-in of `flow`,
/// with the cookie that binds its `state` to the browser.
fn redirect(settings: &SignInSettings, flow: &Flow, authorize: &str, state: &str) -> Response {
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

/// A sign-in in progress, as its callback takes it.
struct Taken {
    return_to: String,
    secrets: Secrets,
    /// A link's web session, by its token's SHA-256; `None` for a sign-in.
    link: Option<Vec<u8>>,
}

/// The SHA-256 of the `state` that the callback's `query` gives, if the browser's cookie binds it.
fn bound_state(headers: &HeaderMap, query: Option<&str>) -> Result<Vec<u8>, Failure> {
    let given = param(query, "state")
        .filter(|given| !given.is_empty())
        .ok_or(Failure::Refused("the sign-in has no state."))?;
    let hash = digest(&given);
    if !cookies(headers, STATE_COOKIE).any(|cookie| digest(cookie) == hash) {
        return Err(Failure::Refused("this browser did not start this sign-in."));
    }
    Ok(hash)
}

/// The sign-in of `flow` whose `state` the callback's `query` gives, if the browser's cookie binds
/// that `state` and it works; it works no more.
async fn take(
    db: &PgPool,
    flow: &Flow<'_>,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<Taken, Failure> {
    let hash = bound_state(headers, query)?;
    let (return_to, verifier, nonce, link): (String, String, Option<String>, Option<Vec<u8>>) =
        sqlx::query_as(
            "DELETE FROM sign_in_states
             WHERE state_hash = $1 AND provider = $2 AND expires_at > now()
             RETURNING return_to, code_verifier, nonce, link_session",
        )
        .bind(&hash)
        .bind(flow.provider)
        .fetch_optional(db)
        .await
        .map_err(Failure::Database)?
        .ok_or(Failure::Refused("the sign-in expired or was used."))?;
    Ok(Taken {
        return_to,
        secrets: Secrets { verifier, nonce },
        link,
    })
}

/// The page that opens a link's callback of `flow` with `query` again from this server, once, if
/// the request lacks the cookie of the link's session: so does the provider's redirect from another
/// site, as browsers send `SameSite=Strict` cookies only from this server's pages. `None` for any
/// other request.
async fn hop(
    db: &PgPool,
    settings: &SignInSettings,
    flow: &Flow<'_>,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Option<Response> {
    let hash = bound_state(headers, query).ok()?;
    // Once: a browser without that cookie then meets the refusal, not a loop.
    let reopened = sqlx::query(
        "UPDATE sign_in_states SET reopened = true
         WHERE state_hash = $1 AND provider = $2 AND expires_at > now() AND NOT reopened
             AND link_session IS NOT NULL AND link_session <> ALL($3)",
    )
    .bind(&hash)
    .bind(flow.provider)
    .bind(session_hashes(headers))
    .execute(db)
    .await
    .ok()?;
    if reopened.rows_affected() == 0 {
        return None;
    }
    let again = escape(&format!(
        "{}?{}",
        settings.public_url.at(flow.callback),
        query.unwrap_or_default()
    ));
    let body = format!(
        "<meta http-equiv=\"refresh\" content=\"0;url={again}\"><p>Linking. <a href=\"{again}\">Continue</a></p>"
    );
    Some(page(StatusCode::OK, "Linking", &body))
}

/// `GET /auth/github/callback` and `GET /auth/google/callback`, the provider's callback: checks
/// `state`, then signs the person it brings back in, finding or creating their account, or links
/// their method to the account of the link's session.
pub(crate) async fn callback<P: Provider>(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let Some((settings, provider)) = P::of(&state) else {
        return provider_off(P::NAME);
    };
    let (db, query) = (&state.db, query.as_deref());
    let flow = provider.flow();
    if let Some(hop) = hop(db, settings, &flow, &headers, query).await {
        return hop;
    }
    let taken = match take(db, &flow, &headers, query).await {
        Ok(taken) => taken,
        Err(failure) => return failure.page(&flow, false),
    };
    let link = taken.link.is_some();
    finish(db, settings, provider, &flow, &headers, query, taken)
        .await
        .unwrap_or_else(|failure| failure.page(&flow, link))
}

/// The page that ends the sign-in or the link `taken`, whose provider's callback has `query`.
async fn finish<P: Provider>(
    db: &PgPool,
    settings: &SignInSettings,
    provider: &P,
    flow: &Flow<'_>,
    headers: &HeaderMap,
    query: Option<&str>,
    taken: Taken,
) -> Result<Response, Failure> {
    let Taken {
        return_to,
        secrets,
        link,
    } = taken;
    if let Some(session) = &link
        && !session_hashes(headers).contains(session)
    {
        return Err(Failure::Session(
            "this browser is not under the session that started the link.",
        ));
    }
    // The authorization response (RFC 6749, §4.1.2).
    if param(query, "error").is_some() {
        return Err(Failure::ProviderRefused("did not authorize it."));
    }
    let code = param(query, "code")
        .filter(|code| !code.is_empty())
        .ok_or(Failure::ProviderRefused("gave no code."))?;
    let identity = provider.identify(settings, code, secrets).await?;
    let method = Method {
        provider: flow.provider,
        subject: identity.subject,
        login: identity.login.as_deref(),
    };
    match link {
        Some(session) => {
            let added = links::add(db, &session, &method).await?;
            Ok(links::linked_page(settings, flow, added, &return_to))
        }
        None => {
            let session = accounts::sign_in(db, &method, &identity.source)
                .await
                .map_err(Failure::Database)?;
            Ok(signed_in_page(settings, flow, &session, &return_to))
        }
    }
}

/// The page that ends a sign-in of `flow` in the web session `session` and sends the browser to
/// `return_to`.
fn signed_in_page(
    settings: &SignInSettings,
    flow: &Flow,
    session: &str,
    return_to: &str,
) -> Response {
    let mut response = refresh_page(settings, flow, "Signed in", "Signed in.", return_to);
    response.headers_mut().append(
        header::SET_COOKIE,
        settings.cookie(SESSION_COOKIE, session, "/", "Strict", None),
    );
    response
}

/// The page of `title` that says `done` of a sign-in or link of `flow`, sends the browser to
/// `return_to` and removes the cookie that bound its `state`.
pub(crate) fn refresh_page(
    settings: &SignInSettings,
    flow: &Flow,
    title: &str,
    done: &str,
    return_to: &str,
) -> Response {
    // A page, not a redirect: the browser's navigation to the return page then starts on this
    // site, so it carries the `SameSite=Strict` session cookie.
    let to = escape(return_to);
    let body = format!(
        "<meta http-equiv=\"refresh\" content=\"0;url={to}\"><p>{done} <a href=\"{to}\">Continue</a></p>"
    );
    let mut response = page(StatusCode::OK, title, &body);
    response
        .headers_mut()
        .append(header::SET_COOKIE, settings.state_cookie(flow, "", 0));
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

/// The SHA-256 of each session token in the request's cookies.
pub(crate) fn session_hashes(headers: &HeaderMap) -> Vec<Vec<u8>> {
    cookies(headers, SESSION_COOKIE).map(digest).collect()
}

/// The answer while the server has no settings of the provider `name`, such as `GitHub`.
pub(crate) fn provider_off(name: &str) -> Response {
    problem(
        StatusCode::NOT_FOUND,
        &format!("{}-sign-in-off", name.to_ascii_lowercase()),
        &format!("{name} sign-in is off"),
        &format!("This server has no {name} settings."),
    )
}

/// The answer while the server has no settings of any provider.
pub(crate) fn off() -> Response {
    problem(
        StatusCode::NOT_FOUND,
        "sign-in-off",
        "Sign-in is off",
        "This server has no sign-in settings.",
    )
}

impl Provider for GitHubApp {
    const NAME: &'static str = "GitHub";

    fn of(state: &AppState) -> Option<(&SignInSettings, &Self)> {
        let settings = state.sign_in.as_deref()?;
        Some((settings, settings.github.as_ref()?))
    }

    fn flow(&self) -> Flow<'_> {
        Flow {
            name: Self::NAME,
            provider: GITHUB,
            start: START_PATH,
            callback: CALLBACK_PATH,
            link: links::GITHUB_PATH,
        }
    }

    async fn authorize(
        &self,
        settings: &SignInSettings,
        state: &str,
        secrets: &Secrets,
        _link: bool,
    ) -> Result<String, Failure> {
        let mut authorize = Url::parse(&self.url.at("/login/oauth/authorize"))
            .expect("GitHub's address and a path make a URL");
        authorize
            .query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &settings.public_url.at(CALLBACK_PATH))
            .append_pair("state", state)
            .append_pair("code_challenge", &secrets.challenge())
            .append_pair("code_challenge_method", "S256");
        Ok(authorize.into())
    }

    async fn identify(
        &self,
        settings: &SignInSettings,
        code: String,
        secrets: Secrets,
    ) -> Result<Identity, Failure> {
        let github = self.clone();
        let callback = settings.public_url.at(CALLBACK_PATH);
        let user = blocking(Self::NAME, move || {
            github.user(&code, &callback, &secrets.verifier)
        })
        .await?
        .map_err(|refusal| match refusal {
            Refusal::Code => Failure::ProviderRefused("refused the code."),
            Refusal::GitHub(reason) => Failure::Provider(reason),
        })?;
        Ok(Identity {
            subject: user.id.to_string(),
            source: user.login.clone(),
            login: Some(user.login),
        })
    }
}

/// Why a sign-in signed no one in, or a link linked nothing.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The request or its sign-in, as its message says: a missing, wrong, expired or used `state`,
    /// or an ID token that failed a check.
    Refused(&'static str),
    /// The provider's answer, as its message after the provider's name says, such as
    /// `gave no code.`: no authorization, no code, or a refused one.
    ProviderRefused(&'static str),
    /// A link's callback is not under the web session that started it, or that session ended.
    Session(&'static str),
    /// A link of a method that another account has.
    OtherAccount,
    /// The provider failed, is unreachable or refused the server's client.
    Provider(String),
    Database(sqlx::Error),
}

impl Failure {
    /// The page that says the sign-in of `flow`, or its link if `link`, failed; failures of the
    /// provider or the database also go to the server's output.
    pub(crate) fn page(self, flow: &Flow, link: bool) -> Response {
        let name = flow.name;
        let what = if link { "link" } else { "sign-in" };
        let (status, message) = match self {
            Failure::Refused(message) => (StatusCode::BAD_REQUEST, message.to_owned()),
            Failure::ProviderRefused(message) => {
                (StatusCode::BAD_REQUEST, format!("{name} {message}"))
            }
            Failure::Session(message) => (StatusCode::FORBIDDEN, message.to_owned()),
            Failure::OtherAccount => (
                StatusCode::CONFLICT,
                format!(
                    "this {name} account signs in to another Skald account. Accounts never merge."
                ),
            ),
            Failure::Provider(reason) => {
                eprintln!("skald-server: {name} {what} failed: {reason}");
                (
                    StatusCode::BAD_GATEWAY,
                    format!("{name} did not complete the {what}."),
                )
            }
            Failure::Database(err) => {
                eprintln!("skald-server: {name} {what} failed: the database failed: {err}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The server failed.".to_owned(),
                )
            }
        };
        let body = if link {
            format!(
                "<p>Not linked: {message} <a href=\"{}\">Your sign-in methods</a></p>",
                links::PAGE_PATH
            )
        } else {
            format!(
                "<p>Not signed in: {message} <a href=\"{}\">Sign in again</a></p>",
                flow.start
            )
        };
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

/// A page of a form that posts to this server, as [`page`] serves pages, under the content security
/// policy `policy`. Its referrer goes only to this server: under `no-referrer`, browsers state the
/// origin of its form's submissions as `null`.
pub(crate) fn form_page(
    status: StatusCode,
    title: &str,
    body: &str,
    policy: &'static str,
) -> Response {
    let mut response = page(status, title, body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(policy),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    response
}

/// The answer of a form page under `policy`, as [`form_page`] serves it, when the database fails at
/// `doing`, such as `read a session`; the failure also goes to the server's output.
pub(crate) fn form_page_failed(doing: &str, err: sqlx::Error, policy: &'static str) -> Response {
    eprintln!("skald-server: cannot {doing}: {err}");
    form_page(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed",
        "<p>The server failed. Try again later.</p>",
        policy,
    )
}

/// Whether the request comes from a page of this server, as its `Origin` says: browsers state the
/// origin of every `POST`.
pub(crate) fn from_this_server(settings: &SignInSettings, headers: &HeaderMap) -> bool {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    origin == Some(settings.public_url.origin().as_str())
}

/// `text` safe in HTML text and attributes.
pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
    find(db, "web", session_hashes(headers)).await
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
    fn challenge_is_s256_of_the_verifier() {
        // RFC 7636, Appendix B.
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn tokens_are_256_random_bits() {
        let token = random_token();
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).expect("base64url").len(), 32);
        assert_ne!(token, random_token());
    }
}
