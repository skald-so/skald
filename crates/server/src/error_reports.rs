//! Error reports to Sentry (`docs/dev/infrastructure.md`, "Monitoring"), only when the setting
//! `--sentry-dsn` gives a DSN. A report says what failed and, for a request, its method and route;
//! never its headers, query or body, nor a database's message or rows (`docs/threat-model.md`, "The
//! server sees").

use std::fmt;
use std::panic::{self, PanicHookInfo};
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{MatchedPath, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use serde_json::{Value, json};
use url::Url;

use crate::http_client::agent;

/// The reports waiting to be sent; the server drops those that find it full.
const QUEUE: usize = 64;
/// How long the server sends nothing after Sentry answers `429` without a `Retry-After`.
const RATE_LIMITED: Duration = Duration::from_secs(60);
/// The longest pause a `Retry-After` sets.
const LONGEST_PAUSE: Duration = Duration::from_secs(3_600);
/// The client that sends the reports.
const CLIENT: &str = concat!("skald-server/", env!("CARGO_PKG_VERSION"));

/// The reports to send, once [`report_errors_to`] has a DSN.
static REPORTS: OnceLock<SyncSender<Value>> = OnceLock::new();

/// A Sentry DSN, `<scheme>://<public key>@<host>[:<port>][/<path>]/<project ID>`: https, or http on
/// a loopback address. A secret key in it is never sent.
#[derive(Clone)]
pub struct SentryDsn {
    /// The DSN without a secret key.
    dsn: String,
    key: String,
    /// The project's envelope endpoint, `[/<path>]/api/<project ID>/envelope/`.
    envelope: Url,
}

impl FromStr for SentryDsn {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let url = crate::oidc::provider_url(text).map_err(|err| format!("a Sentry DSN: {err}"))?;
        let key = url.username().to_owned();
        if key.is_empty() {
            return Err("a Sentry DSN names its public key: <key>@<host>".to_owned());
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("a Sentry DSN has no query or fragment".to_owned());
        }
        let (path, project) = url
            .path()
            .trim_end_matches('/')
            .rsplit_once('/')
            .filter(|(_, project)| {
                !project.is_empty() && project.bytes().all(|b| b.is_ascii_digit())
            })
            .ok_or("a Sentry DSN ends with its project ID")?;
        let mut origin = url.clone();
        origin.set_path("");
        // Infallible on an http or https URL.
        let _ = origin.set_username("");
        let _ = origin.set_password(None);
        let origin = origin.as_str().trim_end_matches('/');
        let envelope = Url::parse(&format!("{origin}{path}/api/{project}/envelope/"))
            .map_err(|err| format!("a Sentry DSN: {err}"))?;
        let scheme = format!("{}://", url.scheme());
        let host = origin.strip_prefix(&scheme).unwrap_or(origin);
        Ok(SentryDsn {
            dsn: format!("{scheme}{key}@{host}{path}/{project}"),
            key,
            envelope,
        })
    }
}

/// Sends the server's error reports, its panics' included, to `dsn` from now on, from a thread of
/// their own; takes the first DSN of the process.
pub fn report_errors_to(dsn: SentryDsn) {
    let (reports, queue) = mpsc::sync_channel(QUEUE);
    if REPORTS.set(reports).is_err() {
        return;
    }
    thread::Builder::new()
        .name("error-reports".to_owned())
        .spawn(move || send_all(&dsn, queue))
        .expect("cannot start the error reports' thread");
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        report(&panicked(info), None);
        previous(info);
    }));
}

/// Reports `message`, of a request to `route` such as `PUT /account/username` if given. Without a
/// DSN, or with the queue full, it does nothing.
pub(crate) fn report(message: &str, route: Option<&str>) {
    if let Some(reports) = REPORTS.get() {
        // A full queue drops the report: reports never hold the server up.
        let _ = reports.try_send(event(message, route));
    }
}

/// What failed, as a report says it: nothing a request, a database row or a message holds.
pub(crate) trait Cause: fmt::Display {
    fn reported(&self) -> String;
}

impl Cause for sqlx::Error {
    fn reported(&self) -> String {
        match self {
            sqlx::Error::Database(err) => {
                let code = err.code().unwrap_or_default();
                match err.constraint() {
                    Some(constraint) => {
                        format!("the database answered SQLSTATE {code} on {constraint}")
                    }
                    None => format!("the database answered SQLSTATE {code}"),
                }
            }
            sqlx::Error::PoolTimedOut => "no database connection in time".to_owned(),
            sqlx::Error::PoolClosed => "the database pool is closed".to_owned(),
            sqlx::Error::Io(err) => format!("the database connection failed: {:?}", err.kind()),
            sqlx::Error::Tls(_) => "the database's TLS failed".to_owned(),
            sqlx::Error::Protocol(_) => "the database broke its protocol".to_owned(),
            sqlx::Error::RowNotFound => "no database row".to_owned(),
            sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => {
                "a database value does not decode".to_owned()
            }
            _ => "the database failed".to_owned(),
        }
    }
}

impl Cause for &'static str {
    fn reported(&self) -> String {
        (*self).to_owned()
    }
}

/// Writes that the server cannot do `doing` for `err` to its output; gives what a report says of it.
pub(crate) fn logged(doing: &str, err: &impl Cause) -> String {
    eprintln!("skald-server: cannot {doing}: {err}");
    format!("cannot {doing}: {}", err.reported())
}

/// The cause of a `500` answer that [`report_failures`] reports.
#[derive(Clone)]
struct Failure(String);

/// `response` with `cause`, which its report gives.
pub(crate) fn with_cause(mut response: Response, cause: String) -> Response {
    response.extensions_mut().insert(Failure(cause));
    response
}

/// Reports each `500` answer with the request's method and route and the cause [`with_cause`] gave.
pub(crate) async fn report_failures(request: Request, next: Next) -> Response {
    if REPORTS.get().is_none() {
        return next.run(request).await;
    }
    let route = format!(
        "{} {}",
        request.method(),
        request
            .extensions()
            .get::<MatchedPath>()
            .map_or("an unknown route", MatchedPath::as_str)
    );
    let response = next.run(request).await;
    if response.status() == StatusCode::INTERNAL_SERVER_ERROR {
        let cause = response
            .extensions()
            .get::<Failure>()
            .map_or("no cause given", |failure| &failure.0);
        report(&format!("{route} answered 500: {cause}"), Some(&route));
    }
    response
}

/// What a report says of a panic: where, and its message only if it is a literal, as a formatted
/// one could hold a request's data.
fn panicked(info: &PanicHookInfo<'_>) -> String {
    let at = info.location().map_or_else(String::new, |location| {
        format!(" at {}:{}", location.file(), location.line())
    });
    match info.payload().downcast_ref::<&'static str>() {
        Some(message) => format!("panicked{at}: {message}"),
        None => format!("panicked{at}"),
    }
}

/// A Sentry event of the level `error` with `message`, of a request to `route` if given.
fn event(message: &str, route: Option<&str>) -> Value {
    let mut id = [0u8; 16];
    getrandom::fill(&mut id).expect("the operating system's random generator failed");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64());
    let mut event = json!({
        "event_id": id.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "timestamp": timestamp,
        "platform": "other",
        "level": "error",
        "logger": "skald-server",
        "release": CLIENT.replace('/', "@"),
        "message": { "formatted": message },
    });
    if let Some(route) = route {
        event["transaction"] = route.into();
    }
    event
}

/// `event` in an envelope to `dsn`.
fn envelope(dsn: &SentryDsn, event: &Value) -> Vec<u8> {
    let payload = event.to_string();
    let header = json!({ "event_id": event["event_id"], "dsn": dsn.dsn });
    let item = json!({ "type": "event", "length": payload.len() });
    format!("{header}\n{item}\n{payload}\n").into_bytes()
}

/// Sends the events of `queue` to `dsn` one by one, until the process ends; drops those that come
/// while Sentry asks to wait.
fn send_all(dsn: &SentryDsn, queue: Receiver<Value>) {
    let mut paused_until = None;
    for event in queue {
        if paused_until.is_some_and(|until| Instant::now() < until) {
            continue;
        }
        paused_until = send(dsn, &event).map(|pause| Instant::now() + pause);
    }
}

/// Sends `event` to `dsn`; gives how long to wait if Sentry answers `429`. A failure goes to the
/// server's output.
fn send(dsn: &SentryDsn, event: &Value) -> Option<Duration> {
    let auth = format!(
        "Sentry sentry_version=7, sentry_key={}, sentry_client={CLIENT}",
        dsn.key
    );
    let sent = agent()
        .post(dsn.envelope.as_str())
        .header("content-type", "application/x-sentry-envelope")
        .header("x-sentry-auth", auth)
        .send(&envelope(dsn, event)[..]);
    match sent {
        Ok(response) if response.status() == 429 => Some(pause(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
        )),
        Ok(response) if response.status().is_success() => None,
        Ok(response) => {
            eprintln!(
                "skald-server: Sentry answered {} to an error report",
                response.status()
            );
            None
        }
        Err(err) => {
            eprintln!("skald-server: cannot send an error report: {err}");
            None
        }
    }
}

/// The pause of a `429` answer with the `Retry-After` value `retry_after`, in seconds.
fn pause(retry_after: Option<&str>) -> Duration {
    retry_after
        .and_then(|seconds| seconds.trim().parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map_or(RATE_LIMITED, |seconds| {
            Duration::from_secs_f64(seconds.min(LONGEST_PAUSE.as_secs_f64()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dsn(text: &str) -> Result<(String, String, String), String> {
        text.parse::<SentryDsn>()
            .map(|dsn| (dsn.dsn, dsn.key, dsn.envelope.to_string()))
    }

    #[test]
    fn a_dsn_gives_its_key_and_envelope_endpoint_without_a_secret() {
        assert_eq!(
            dsn("https://abc@o1.ingest.sentry.example/42"),
            Ok((
                "https://abc@o1.ingest.sentry.example/42".to_owned(),
                "abc".to_owned(),
                "https://o1.ingest.sentry.example/api/42/envelope/".to_owned(),
            ))
        );
        assert_eq!(
            dsn("http://abc:secret@127.0.0.1:9000/sentry/7/"),
            Ok((
                "http://abc@127.0.0.1:9000/sentry/7".to_owned(),
                "abc".to_owned(),
                "http://127.0.0.1:9000/sentry/api/7/envelope/".to_owned(),
            ))
        );
    }

    #[test]
    fn a_dsn_is_https_or_loopback_http_with_a_key_and_a_project() {
        for text in [
            "http://abc@sentry.example/1",
            "ftp://abc@127.0.0.1/1",
            "https://sentry.example/1",
            "https://abc@sentry.example/",
            "https://abc@sentry.example/project",
            "https://abc@sentry.example/1?a=b",
            "https://abc@sentry.example/1#a",
            "abc@sentry.example/1",
        ] {
            assert!(text.parse::<SentryDsn>().is_err(), "{text}");
        }
    }

    #[test]
    fn a_database_error_is_reported_by_its_kind_only() {
        let io = sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "secret detail",
        ));
        assert_eq!(
            io.reported(),
            "the database connection failed: ConnectionRefused"
        );
        let protocol = sqlx::Error::Protocol("secret detail".to_owned());
        assert_eq!(protocol.reported(), "the database broke its protocol");
        assert_eq!(
            sqlx::Error::PoolTimedOut.reported(),
            "no database connection in time"
        );
    }

    #[test]
    fn an_envelope_holds_its_dsn_and_one_event_of_its_length() {
        let dsn: SentryDsn = "http://abc:secret@127.0.0.1:9000/7".parse().expect("a DSN");
        let event = event(
            "cannot read a session: no database row",
            Some("GET /session"),
        );
        let bytes = envelope(&dsn, &event);
        let text = String::from_utf8(bytes).expect("UTF-8");
        let lines: Vec<&str> = text.lines().collect();
        let header: Value = serde_json::from_str(lines[0]).expect("a header");
        assert_eq!(header["dsn"], "http://abc@127.0.0.1:9000/7");
        assert_eq!(header["event_id"], event["event_id"]);
        let item: Value = serde_json::from_str(lines[1]).expect("an item header");
        assert_eq!(item["type"], "event");
        assert_eq!(item["length"], lines[2].len());
        let sent: Value = serde_json::from_str(lines[2]).expect("an event");
        assert_eq!(sent, event);
        assert_eq!(sent["level"], "error");
        assert_eq!(sent["transaction"], "GET /session");
        assert!(!text.contains("secret"));
        assert_eq!(super::event("x", None).get("transaction"), None);
    }

    #[test]
    fn a_rate_limit_pauses_for_its_retry_after() {
        assert_eq!(pause(Some("30")), Duration::from_secs(30));
        assert_eq!(pause(Some(" 1.5 ")), Duration::from_millis(1_500));
        assert_eq!(pause(Some("1000000")), LONGEST_PAUSE);
        assert_eq!(pause(Some("soon")), RATE_LIMITED);
        assert_eq!(pause(Some("-1")), RATE_LIMITED);
        assert_eq!(pause(None), RATE_LIMITED);
    }
}
