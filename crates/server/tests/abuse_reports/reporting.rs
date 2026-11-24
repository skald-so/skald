use serde_json::{Value, json};
use skald_proto::{MessageId, PROTOCOL_VERSION, SpaceId, VERSION_HEADER};

use crate::api::{Person, Team, the_spec};
use crate::common::device_flow::{json, text};
use crate::common::spec::operation_violations;
use crate::common::{Response, Server, TestDb, get_url, send};
use crate::fixture::Fixture;
use crate::sessions::Api;

pub const REPORTS: &str = "/spaces/{space}/abuse-reports";
pub const CAPABILITY: &str = "/spaces/{space}/abuse-reporting";
pub const REASON: &str = "fixture-category-one";
pub const OTHER_REASON: &str = "fixture-category-two";
pub const LONG_RETENTION: &str = "5184000";

pub fn sign_in(fixture: &Fixture, person: &Person, github: u64) -> String {
    let flow = &fixture.team.flow;
    let login = format!("{}-spaces-standin", person.handle);
    flow.web_cookie(&flow.stand.user(github, &login))
}

pub fn settings<'a>(
    limit: &'a str,
    capacity: &'a str,
    retention: &'a str,
) -> [(&'a str, &'a str); 4] {
    [
        (
            "SKALD_ABUSE_REPORT_REASONS",
            "fixture-category-one,fixture-category-two",
        ),
        ("SKALD_ABUSE_REPORT_LIMIT", limit),
        ("SKALD_ABUSE_REPORT_CAPACITY", capacity),
        ("SKALD_ABUSE_REPORT_RETENTION", retention),
    ]
}

pub fn start(fixture: &mut Fixture) -> Server {
    fixture
        .team
        .flow
        .stand
        .restart(&[], &settings("5", "10", LONG_RETENTION))
}

pub fn report_path(space: SpaceId) -> String {
    format!("/spaces/{space}/abuse-reports")
}

/// The member-list snapshot supplies the report selector, even for a refused submission.
pub fn member_revision(team: &Team, server: &Server) -> String {
    let response =
        team.alice
            .session
            .on(server)
            .call("get", &format!("/spaces/{}/members", team.space), None);
    assert_eq!(response.status, 200, "the member list: {}", text(&response));
    response
        .header("skald-roster-revision")
        .expect("the member list's revision")
        .to_owned()
}

pub fn submission(target: &Person, reason: &str, revision: &str) -> Value {
    json!({
        "target": target.handle.as_str(),
        "roster_revision": revision,
        "reason": reason,
        "idempotency_key": MessageId::generate().to_string()
    })
}

pub fn post(api: &Api<'_>, space: SpaceId, headers: &[(&str, &str)], body: &Value) -> Response {
    let version = PROTOCOL_VERSION.to_string();
    let mut all = vec![
        (VERSION_HEADER, version.as_str()),
        ("content-type", "application/json"),
    ];
    all.extend_from_slice(headers);
    send(
        "post",
        &format!("{}{}", api.base, report_path(space)),
        &all,
        &serde_json::to_vec(body).expect("a report request"),
    )
}

pub fn submit(api: &Api<'_>, space: SpaceId, cookie: &str, body: &Value) -> Response {
    post(
        api,
        space,
        &[("cookie", cookie), ("origin", api.public)],
        body,
    )
}

pub fn get(api: &Api<'_>, path: &str, cookie: &str) -> Response {
    let version = PROTOCOL_VERSION.to_string();
    get_url(
        &format!("{}{path}", api.base),
        &[(VERSION_HEADER, &version), ("cookie", cookie)],
    )
}

#[track_caller]
pub fn documented(method: &str, path: &str, response: &Response) {
    let violations = operation_violations(the_spec(), method, path, response);
    assert!(
        violations.is_empty(),
        "{method} {path} {}: {violations:?}",
        response.status,
    );
}

#[track_caller]
pub fn receipt(response: &Response, status: &[u16]) -> String {
    assert!(
        status.contains(&response.status),
        "a report receipt, got {}: {}",
        response.status,
        text(response)
    );
    documented("post", REPORTS, response);
    let body = json(response);
    let receipt = body["receipt"].as_str().expect("an opaque report receipt");
    assert!(!receipt.is_empty(), "an empty receipt");
    assert_eq!(body, json!({ "receipt": receipt }), "only a receipt");
    receipt.to_owned()
}

#[track_caller]
pub fn refused(response: &Response, what: &str, hidden: &[&str]) {
    crate::deletion::assert_refused(response, what);
    documented("post", REPORTS, response);
    let body = text(response);
    for value in hidden {
        assert!(!body.contains(value), "{what} revealed {value}: {body}");
    }
    assert!(json(response).get("receipt").is_none(), "{what}: {body}");
}

/// Only the persistence and clock assertions use the disposable database.
pub fn report_rows(db: &TestDb) -> Vec<String> {
    let mut rows = db.rows();
    rows.retain(|row| row.starts_with("abuse_reports: "));
    rows.sort();
    rows
}

#[track_caller]
pub fn age_report(db: &TestDb, receipt: &str, interval: &str) {
    let changed = db.query(async |conn| {
        sqlx::query(
            "UPDATE abuse_reports SET submitted_at = now() - $2::interval
             WHERE receipt::text = $1
               AND submitted_at BETWEEN now() - interval '5 minutes' AND now()",
        )
        .bind(receipt)
        .bind(interval)
        .execute(conn)
        .await
        .map(|result| result.rows_affected())
    });
    assert_eq!(
        changed.expect("move the report time"),
        1,
        "receipt {receipt} was submitted recently"
    );
}
