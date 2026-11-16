//! Error reports to Sentry (`docs/dev/infrastructure.md`, "Monitoring"), only when the setting
//! `--sentry-dsn` gives a DSN. A report says what failed and, for a request, its method and route;
//! never its headers, query or body, nor a database's message or rows (`docs/threat-model.md`, "The
//! server sees").

use std::any::Any;
use std::fmt;
use std::panic::{self, Location};
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
        report(&panicked(info.location(), info.payload()), None);
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
            sqlx::Error::Io(err) => format!("the database connection failed: {:?}", err.kind()),
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

/// Writes that a background task cannot do `doing` for `err` to the server's output, and reports it.
pub(crate) fn background_failed(doing: &str, err: &impl Cause) {
    report(&logged(doing, err), None);
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

/// What a report says of a panic at `location` with `payload`: where, and its message only if it is
/// a literal, as a formatted one could hold a request's data.
fn panicked(location: Option<&Location<'_>>, payload: &(dyn Any + Send)) -> String {
    let at = location.map_or_else(String::new, |location| {
        format!(" at {}:{}", location.file(), location.line())
    });
    match payload.downcast_ref::<&'static str>() {
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

/// What became of a report Sentry was sent.
#[derive(Debug, PartialEq)]
enum Sent {
    Taken,
    /// Sentry answered `429`: nothing is sent for that long.
    Paused(Duration),
    /// Why it was not taken.
    Failed(String),
}

/// Sends the events of `queue` to `dsn` one by one, until the queue closes; drops those that come
/// while Sentry asks to wait. A failure goes to the server's output.
fn send_all(dsn: &SentryDsn, queue: Receiver<Value>) {
    let mut paused_until = None;
    for event in queue {
        if paused_until.is_some_and(|until| Instant::now() < until) {
            continue;
        }
        match send(dsn, &event) {
            Sent::Taken => {}
            Sent::Paused(pause) => paused_until = Some(Instant::now() + pause),
            Sent::Failed(why) => eprintln!("skald-server: cannot send an error report: {why}"),
        }
    }
}

/// Sends `event` to `dsn`.
fn send(dsn: &SentryDsn, event: &Value) -> Sent {
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
        Ok(response) if response.status() == 429 => Sent::Paused(pause(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
        )),
        Ok(response) if response.status().is_success() => Sent::Taken,
        Ok(response) => Sent::Failed(format!("Sentry answered {}", response.status())),
        Err(err) => Sent::Failed(err.to_string()),
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
    use std::borrow::Cow;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use sqlx::error::{DatabaseError, ErrorKind};

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

    /// A database's refusal whose message quotes a request's data.
    #[derive(Debug)]
    struct Refused;

    const QUOTED: &str = "duplicate key value (username)=(secret-ada)";

    impl fmt::Display for Refused {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(QUOTED)
        }
    }

    impl std::error::Error for Refused {}

    impl DatabaseError for Refused {
        fn message(&self) -> &str {
            QUOTED
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some("23505".into())
        }
        fn constraint(&self) -> Option<&str> {
            Some("accounts_username_key")
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> ErrorKind {
            ErrorKind::UniqueViolation
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
        assert_eq!(protocol.reported(), "the database failed");
        let refused = sqlx::Error::Database(Box::new(Refused));
        let reported = "the database answered SQLSTATE 23505 on accounts_username_key";
        assert_eq!(refused.reported(), reported);
        assert_eq!(
            logged("set a username", &refused),
            format!("cannot set a username: {reported}")
        );
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

    #[test]
    fn a_panic_is_reported_by_its_place_and_only_a_literal_message() {
        let here = Location::caller();
        assert_eq!(
            panicked(Some(here), &"a literal"),
            format!("panicked at {}:{}: a literal", here.file(), here.line())
        );
        let formatted = format!("{} of a request", "the body");
        assert_eq!(panicked(None, &formatted), "panicked");
    }

    /// A stand-in for Sentry that answers each request with `answer`; gives its DSN and the number
    /// of requests it got.
    fn stand_in(answer: &'static str) -> (SentryDsn, Arc<Mutex<usize>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("local address").port();
        let got = Arc::new(Mutex::new(0));
        let counted = Arc::clone(&got);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("a header");
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().expect("a length");
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).expect("the body");
                *counted.lock().expect("the count") += 1;
                (&stream).write_all(answer.as_bytes()).expect("answer");
            }
        });
        let dsn = format!("http://key@127.0.0.1:{port}/1");
        (dsn.parse().expect("a DSN"), got)
    }

    #[test]
    fn a_report_is_taken_paused_or_failed_by_the_answer() {
        let event = event("x", None);
        let (dsn, _) =
            stand_in("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}");
        assert_eq!(send(&dsn, &event), Sent::Taken);
        let (dsn, _) = stand_in(
            "HTTP/1.1 429 Too Many Requests\r\nretry-after: 7\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        );
        assert_eq!(send(&dsn, &event), Sent::Paused(Duration::from_secs(7)));
        let (dsn, _) = stand_in(
            "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        );
        assert_eq!(
            send(&dsn, &event),
            Sent::Failed("Sentry answered 500 Internal Server Error".to_owned())
        );
    }

    #[test]
    fn nothing_is_sent_while_sentry_asks_to_wait() {
        let (dsn, got) = stand_in(
            "HTTP/1.1 429 Too Many Requests\r\nretry-after: 60\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        );
        let (reports, queue) = mpsc::sync_channel(QUEUE);
        for _ in 0..3 {
            reports.try_send(event("x", None)).expect("queued");
        }
        drop(reports);
        send_all(&dsn, queue);
        assert_eq!(*got.lock().expect("the count"), 1);
    }
}
