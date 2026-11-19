//! A local stand-in for GitHub's OAuth web flow and user endpoint, for tests: no test needs a real
//! OAuth App. As GitHub does, it serves
//! - `GET /login/oauth/authorize`: a person signed in to the stand-in in their browser authorizes the
//!   app and goes back to `redirect_uri` with `code` and `state`: at once, as after an earlier
//!   authorization, or by a link on a page that asks first, as at the first one; without that
//!   sign-in, a sign-in page that leads nowhere. A PKCE `code_challenge` (RFC 7636), with
//!   `code_challenge_method` `S256` or `plain`, the default, binds the code to its verifier;
//! - `POST /login/oauth/access_token`: gives a token for a code, once, to the app's client ID and
//!   secret, in the body or by HTTP Basic, and, for a code with a challenge, only with its
//!   `code_verifier`; a refusal is a `200` with `error`;
//! - `GET /user`: the user of a token, `Authorization: Bearer <token>` or `token <token>`, to a
//!   request with a `User-Agent`.
//!
//! Tests control its users and faults through [`GitHub`] in Rust, or from another process, such as a
//! web test running the `skald-github-stand-in` binary, through the control endpoints:
//! - `GET /_stand-in/sign-in?user=<id>`: signs the browser in as that user; with `&ask=1`,
//!   authorize then answers a page whose link `#authorize` goes back with the code and whose link
//!   `#cancel` goes back with `error=access_denied`;
//! - `PUT /_stand-in/users`, a [`User`] in JSON: adds the user or replaces the one with its ID;
//! - `PUT /_stand-in/faults`, a JSON array of [`Fault`]s: the faults from then on, `[]` for none;
//! - `GET /_stand-in/requests`: the [`Request`]s to GitHub's endpoints so far;
//! - `GET /_stand-in/tokens`: the tokens issued so far.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io;
use std::net::TcpListener as StdListener;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Json, Query, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

/// The cookie that holds who is signed in to the stand-in in a browser: `<id>`, or `<id>.ask` when
/// authorize asks first.
const SIGN_IN_COOKIE: &str = "stand_in_user";

/// The OAuth App registered with the stand-in.
#[derive(Clone, Debug)]
pub struct App {
    pub client_id: String,
    pub client_secret: String,
    /// The app's callback URL. When set, as on GitHub, a `redirect_uri` must have its scheme, host
    /// and port and a path under its path, and authorize falls back to it without one. When not,
    /// any `redirect_uri` is taken.
    pub callback: Option<String>,
}

/// A GitHub user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub login: String,
    /// The public email, which `GET /user` gives without any scope.
    pub email: Option<String>,
    pub name: Option<String>,
}

impl User {
    /// A user with no public email and no name.
    pub fn new(id: u64, login: &str) -> Self {
        User {
            id,
            login: login.to_owned(),
            email: None,
            name: None,
        }
    }
}

/// A fault the stand-in plays until the faults are set again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fault", rename_all = "snake_case")]
pub enum Fault {
    /// The person denies the app: authorize goes back with `error=access_denied`.
    AccessDenied,
    /// The token endpoint refuses every code with `bad_verification_code`.
    BadVerificationCode,
    /// The token endpoint answers with this status and no token.
    TokenStatus { status: u16 },
    /// The user endpoint answers with this status and no user.
    UserStatus { status: u16 },
}

/// One of GitHub's endpoints the stand-in serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    Authorize,
    Token,
    User,
}

/// A request to one of GitHub's endpoints, as the stand-in received it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub endpoint: Endpoint,
    /// The query parameters and those of a form or JSON body.
    pub params: BTreeMap<String, String>,
    /// The headers, by lowercase name.
    pub headers: BTreeMap<String, String>,
}

/// A code authorize issued.
struct Grant {
    user: u64,
    redirect_uri: Option<String>,
    /// `code_challenge` and `code_challenge_method`, if the request had a challenge.
    challenge: Option<(String, String)>,
    used: bool,
}

#[derive(Default)]
struct Data {
    users: BTreeMap<u64, User>,
    faults: Vec<Fault>,
    grants: HashMap<String, Grant>,
    /// Issued tokens and their users, in order.
    tokens: Vec<(String, u64)>,
    requests: Vec<Request>,
}

#[derive(Clone)]
struct Shared {
    app: Arc<App>,
    data: Arc<Mutex<Data>>,
}

impl Shared {
    fn new(app: App) -> Self {
        Shared {
            app: Arc::new(app),
            data: Arc::default(),
        }
    }

    fn data(&self) -> MutexGuard<'_, Data> {
        self.data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record(&self, endpoint: Endpoint, params: &BTreeMap<String, String>, headers: &HeaderMap) {
        let headers = headers
            .iter()
            .map(|(name, value)| {
                let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
                (name.as_str().to_owned(), value)
            })
            .collect();
        self.data().requests.push(Request {
            endpoint,
            params: params.clone(),
            headers,
        });
    }

    fn fault(&self, matches: impl Fn(&Fault) -> bool) -> Option<Fault> {
        self.data().faults.iter().copied().find(matches)
    }
}

/// A running stand-in on a free local port; stopped on drop.
pub struct GitHub {
    url: String,
    shared: Shared,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl GitHub {
    /// Starts a stand-in for `app` with no users and no faults.
    pub fn start(app: App) -> Self {
        let listener = StdListener::bind("127.0.0.1:0").expect("bind a local port");
        listener
            .set_nonblocking(true)
            .expect("make the listener non-blocking");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let shared = Shared::new(app);
        let router = router(shared.clone());
        let (stop, stopped) = oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("start a tokio runtime");
            runtime.block_on(async move {
                let listener =
                    tokio::net::TcpListener::from_std(listener).expect("listen with tokio");
                tokio::select! {
                    served = axum::serve(listener, router) => served.expect("serve the stand-in"),
                    _ = stopped => {}
                }
            });
        });
        GitHub {
            url,
            shared,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    /// The base URL, e.g. `http://127.0.0.1:41234`: GitHub's address and its API's address both.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Adds `user`, or replaces the user with its ID: a login or an email change.
    pub fn put_user(&self, user: User) {
        self.shared.data().users.insert(user.id, user);
    }

    /// The page that signs a browser in to the stand-in as the user `id`, as signing in to GitHub.
    /// Authorize then goes back at once.
    pub fn sign_in_url(&self, id: u64) -> String {
        format!("{}/_stand-in/sign-in?user={id}", self.url)
    }

    /// As [`GitHub::sign_in_url`], but authorize then asks first: its page's link `#authorize`
    /// goes back with the code, and `#cancel` with `error=access_denied`.
    pub fn asking_sign_in_url(&self, id: u64) -> String {
        format!("{}/_stand-in/sign-in?user={id}&ask=1", self.url)
    }

    /// Plays `faults` from now on; none for `&[]`.
    pub fn set_faults(&self, faults: &[Fault]) {
        self.shared.data().faults = faults.to_vec();
    }

    /// The requests to GitHub's endpoints so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.shared.data().requests.clone()
    }

    /// The requests to `endpoint` so far, in order.
    pub fn requests_to(&self, endpoint: Endpoint) -> Vec<Request> {
        let mut requests = self.requests();
        requests.retain(|request| request.endpoint == endpoint);
        requests
    }

    /// The tokens issued so far, in order.
    pub fn tokens(&self) -> Vec<String> {
        let data = self.shared.data();
        data.tokens.iter().map(|(token, _)| token.clone()).collect()
    }
}

impl Drop for GitHub {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Serves a stand-in for `app` on `listener` until the future is dropped.
pub async fn serve(listener: tokio::net::TcpListener, app: App) -> io::Result<()> {
    axum::serve(listener, router(Shared::new(app))).await
}

fn router(shared: Shared) -> Router {
    Router::new()
        .route("/login/oauth/authorize", get(authorize))
        .route("/login/oauth/access_token", post(access_token))
        .route("/user", get(user))
        .route("/_stand-in/sign-in", get(sign_in))
        .route("/_stand-in/users", put(put_user))
        .route("/_stand-in/faults", put(put_faults))
        .route("/_stand-in/requests", get(requests))
        .route("/_stand-in/tokens", get(tokens))
        .with_state(shared)
}

async fn authorize(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    shared.record(Endpoint::Authorize, &params, &headers);
    if params.get("client_id") != Some(&shared.app.client_id) {
        return (
            StatusCode::NOT_FOUND,
            Html("<h1>404: unknown client_id</h1>"),
        )
            .into_response();
    }
    let state = params.get("state").map(String::as_str);
    let redirect_uri = match (params.get("redirect_uri"), &shared.app.callback) {
        (Some(uri), Some(callback)) if !under(uri, callback) => {
            let description =
                "The redirect_uri MUST match the registered callback URL for this application.";
            return back(
                callback,
                &[
                    ("error", "redirect_uri_mismatch"),
                    ("error_description", description),
                ],
                state,
            );
        }
        (Some(uri), _) => uri.clone(),
        (None, Some(callback)) => callback.clone(),
        (None, None) => {
            return (StatusCode::BAD_REQUEST, Html("<h1>No redirect_uri</h1>")).into_response();
        }
    };
    let challenge = match (
        params.get("code_challenge"),
        params.get("code_challenge_method").map(String::as_str),
    ) {
        (None, None) => None,
        (Some(challenge), method @ (None | Some("S256" | "plain"))) => {
            Some((challenge.clone(), method.unwrap_or("plain").to_owned()))
        }
        _ => {
            return back(
                &redirect_uri,
                &[
                    ("error", "invalid_request"),
                    ("error_description", "Invalid code_challenge_method."),
                ],
                state,
            );
        }
    };
    let Some((user, asks)) = signed_in(&shared, &headers) else {
        return Html("<h1>Sign in to GitHub</h1>").into_response();
    };
    let denied = back_url(
        &redirect_uri,
        &[
            ("error", "access_denied"),
            (
                "error_description",
                "The user has denied your application access.",
            ),
        ],
        state,
    );
    if shared
        .fault(|fault| *fault == Fault::AccessDenied)
        .is_some()
    {
        return redirect(denied);
    }
    let code = random_hex(10);
    shared.data().grants.insert(
        code.clone(),
        Grant {
            user,
            redirect_uri: params.get("redirect_uri").cloned(),
            challenge,
            used: false,
        },
    );
    let authorized = back_url(&redirect_uri, &[("code", &code)], state);
    if asks {
        let page = format!(
            "<!doctype html><title>Authorize application</title>\
             <a id=\"authorize\" href=\"{}\">Authorize</a> <a id=\"cancel\" href=\"{}\">Cancel</a>",
            escape(&authorized),
            escape(&denied),
        );
        return Html(page).into_response();
    }
    redirect(authorized)
}

/// The user signed in to the stand-in in the browser that sent `headers`, and whether authorize
/// asks that browser first.
fn signed_in(shared: &Shared, headers: &HeaderMap) -> Option<(u64, bool)> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    let value = cookies.split(';').find_map(|cookie| {
        let (name, value) = cookie.trim().split_once('=')?;
        (name == SIGN_IN_COOKIE).then_some(value)
    })?;
    let (id, asks) = match value.strip_suffix(".ask") {
        Some(id) => (id, true),
        None => (value, false),
    };
    let id = id.parse().ok()?;
    shared.data().users.contains_key(&id).then_some((id, asks))
}

/// A `302` to `uri` with `params` and `state`, if any, added to its query.
fn back(uri: &str, params: &[(&str, &str)], state: Option<&str>) -> Response {
    redirect(back_url(uri, params, state))
}

/// `uri` with `params` and `state`, if any, added to its query.
fn back_url(uri: &str, params: &[(&str, &str)], state: Option<&str>) -> String {
    let mut pairs = params.to_vec();
    if let Some(state) = state {
        pairs.push(("state", state));
    }
    let query = serde_urlencoded::to_string(pairs).expect("encode a query");
    let separator = if uri.contains('?') { '&' } else { '?' };
    format!("{uri}{separator}{query}")
}

fn redirect(location: String) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

/// `text` safe in an HTML attribute.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Whether `uri` is under the callback URL `callback` as GitHub has it: the same scheme, host and
/// port, and the same path or one below it.
fn under(uri: &str, callback: &str) -> bool {
    let (origin, path) = split_url(uri);
    let (callback_origin, callback_path) = split_url(callback);
    let callback_path = callback_path.trim_end_matches('/');
    origin == callback_origin
        && (path == callback_path
            || path
                .strip_prefix(callback_path)
                .is_some_and(|rest| rest.starts_with('/')))
}

/// `url` as its origin and its path, without query or fragment.
fn split_url(url: &str) -> (&str, &str) {
    let url = url.split(['?', '#']).next().unwrap_or_default();
    let host_start = url.find("://").map_or(0, |at| at + 3);
    match url[host_start..].find('/') {
        Some(at) => url.split_at(host_start + at),
        None => (url, ""),
    }
}

async fn access_token(
    State(shared): State<Shared>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut params: BTreeMap<String, String> =
        serde_urlencoded::from_str(query.as_deref().unwrap_or_default()).unwrap_or_default();
    params.extend(body_params(&headers, &body));
    shared.record(Endpoint::Token, &params, &headers);
    let json = header_contains(&headers, header::ACCEPT, "json");
    if let Some(Fault::TokenStatus { status }) =
        shared.fault(|fault| matches!(fault, Fault::TokenStatus { .. }))
    {
        return status_response(status, "Server Error");
    }

    let (client_id, client_secret) = basic_credentials(&headers).unwrap_or_else(|| {
        let param = |name| params.get(name).cloned().unwrap_or_default();
        (param("client_id"), param("client_secret"))
    });
    if client_id != shared.app.client_id || client_secret != shared.app.client_secret {
        return token_error(
            json,
            "incorrect_client_credentials",
            "The client_id and/or client_secret passed are incorrect.",
        );
    }
    let bad_code = || {
        token_error(
            json,
            "bad_verification_code",
            "The code passed is incorrect or expired.",
        )
    };
    if shared
        .fault(|fault| *fault == Fault::BadVerificationCode)
        .is_some()
    {
        return bad_code();
    }
    let mut data = shared.data();
    let Some(grant) = params
        .get("code")
        .and_then(|code| data.grants.get_mut(code))
        .filter(|grant| !grant.used)
    else {
        return bad_code();
    };
    if let (Some(sent), Some(authorized)) = (params.get("redirect_uri"), &grant.redirect_uri)
        && sent != authorized
    {
        return token_error(
            json,
            "redirect_uri_mismatch",
            "The redirect_uri MUST match the registered callback URL for this application.",
        );
    }
    if let Some((challenge, method)) = &grant.challenge {
        let verifier = params.get("code_verifier").map(String::as_str);
        if !verifier.is_some_and(|verifier| verifies(verifier, challenge, method)) {
            return token_error(
                json,
                "bad_verification_code",
                "The code_verifier does not match the code_challenge.",
            );
        }
    }
    grant.used = true;
    let user = grant.user;
    let token = format!("gho_{}", random_hex(18));
    data.tokens.push((token.clone(), user));
    let fields = [
        ("access_token", token.as_str()),
        ("token_type", "bearer"),
        ("scope", ""),
    ];
    if json {
        Json(json!({ "access_token": token, "token_type": "bearer", "scope": "" })).into_response()
    } else {
        form_response(&fields)
    }
}

/// Whether `verifier` is the PKCE verifier of `challenge` under `method` (RFC 7636, §4.6).
fn verifies(verifier: &str, challenge: &str, method: &str) -> bool {
    match method {
        "S256" => URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == challenge,
        _ => verifier == challenge,
    }
}

/// The parameters of a form or JSON body; none for another body.
fn body_params(headers: &HeaderMap, body: &[u8]) -> BTreeMap<String, String> {
    if header_contains(headers, header::CONTENT_TYPE, "json") {
        let fields: BTreeMap<String, Value> = serde_json::from_slice(body).unwrap_or_default();
        fields
            .into_iter()
            .map(|(name, value)| match value {
                Value::String(text) => (name, text),
                other => (name, other.to_string()),
            })
            .collect()
    } else {
        serde_urlencoded::from_bytes(body).unwrap_or_default()
    }
}

/// The client ID and secret of an `Authorization: Basic` header, as RFC 6749, §2.3.1, encodes them.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let (id, secret) = decoded.split_once(':')?;
    let pairs: BTreeMap<String, String> =
        serde_urlencoded::from_str(&format!("id={id}&secret={secret}")).ok()?;
    Some((pairs.get("id")?.clone(), pairs.get("secret")?.clone()))
}

fn header_contains(headers: &HeaderMap, name: header::HeaderName, part: &str) -> bool {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains(part))
}

/// A refusal of the token endpoint: `200` with `error`, as on GitHub.
fn token_error(json: bool, error: &str, description: &str) -> Response {
    let uri = "https://docs.github.com/apps/managing-oauth-apps/troubleshooting-oauth-app-access-token-request-errors";
    if json {
        Json(json!({ "error": error, "error_description": description, "error_uri": uri }))
            .into_response()
    } else {
        form_response(&[
            ("error", error),
            ("error_description", description),
            ("error_uri", uri),
        ])
    }
}

fn form_response(fields: &[(&str, &str)]) -> Response {
    let body = serde_urlencoded::to_string(fields).expect("encode a form");
    (
        [(header::CONTENT_TYPE, "application/x-www-form-urlencoded")],
        body,
    )
        .into_response()
}

/// A GitHub API error with `status`.
fn status_response(status: u16, message: &str) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = json!({
        "message": message,
        "documentation_url": "https://docs.github.com/rest",
        "status": status.as_u16().to_string(),
    });
    (status, Json(body)).into_response()
}

async fn user(State(shared): State<Shared>, headers: HeaderMap) -> Response {
    shared.record(Endpoint::User, &BTreeMap::new(), &headers);
    if !headers.contains_key(header::USER_AGENT) {
        return status_response(
            403,
            "Request forbidden by administrative rules. Please make sure your request has a User-Agent header.",
        );
    }
    if let Some(Fault::UserStatus { status }) =
        shared.fault(|fault| matches!(fault, Fault::UserStatus { .. }))
    {
        return status_response(status, "Server Error");
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("token")
        })
        .map(|(_, token)| token.trim());
    let data = shared.data();
    let user = token
        .and_then(|token| data.tokens.iter().find(|(issued, _)| issued == token))
        .and_then(|(_, id)| data.users.get(id));
    let Some(user) = user else {
        return status_response(401, "Bad credentials");
    };
    Json(json!({
        "login": user.login,
        "id": user.id,
        "node_id": format!("U_standin{}", user.id),
        "avatar_url": format!("https://avatars.githubusercontent.com/u/{}?v=4", user.id),
        "html_url": format!("https://github.com/{}", user.login),
        "type": "User",
        "site_admin": false,
        "name": user.name,
        "email": user.email,
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }))
    .into_response()
}

async fn sign_in(
    State(shared): State<Shared>,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    let id = params.get("user").and_then(|id| id.parse::<u64>().ok());
    let Some(id) = id.filter(|id| shared.data().users.contains_key(id)) else {
        return (StatusCode::NOT_FOUND, Html("<h1>No such user</h1>")).into_response();
    };
    let asks = if params.get("ask").is_some_and(|ask| ask == "1") {
        ".ask"
    } else {
        ""
    };
    (
        [(
            header::SET_COOKIE,
            format!("{SIGN_IN_COOKIE}={id}{asks}; Path=/; HttpOnly; SameSite=Lax"),
        )],
        Html(format!("<h1>Signed in to GitHub as user {id}</h1>")),
    )
        .into_response()
}

async fn put_user(State(shared): State<Shared>, Json(user): Json<User>) -> StatusCode {
    shared.data().users.insert(user.id, user);
    StatusCode::NO_CONTENT
}

async fn put_faults(State(shared): State<Shared>, Json(faults): Json<Vec<Fault>>) -> StatusCode {
    shared.data().faults = faults;
    StatusCode::NO_CONTENT
}

async fn requests(State(shared): State<Shared>) -> Json<Vec<Request>> {
    Json(shared.data().requests.clone())
}

async fn tokens(State(shared): State<Shared>) -> Json<Vec<String>> {
    let data = shared.data();
    Json(data.tokens.iter().map(|(token, _)| token.clone()).collect())
}

/// `bytes` random bytes in hex. Distinct across calls and processes; not for real secrets.
fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    while out.len() < bytes * 2 {
        let word = RandomState::new().build_hasher().finish();
        write!(out, "{word:016x}").expect("write to a string");
    }
    out.truncate(bytes * 2);
    out
}
