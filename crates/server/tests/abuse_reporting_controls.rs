//! Report intake controls exercised against local server instances.

#![allow(dead_code)]

#[path = "erasure_common/mod.rs"]
mod erasure_common;
use erasure_common::*;

#[path = "abuse_reports/reporting.rs"]
mod reporting;

use std::sync::Barrier;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{NonEmpty, Role};

use crate::common::{Response, TestDb};
use crate::fixture::Fixture;
use crate::reporting::{
    CAPABILITY, LONG_RETENTION, OTHER_REASON, REASON, age_report, documented, get, member_revision,
    post, receipt, refused, report_rows, settings, sign_in, submission, submit,
};
use crate::rosters::member;
use crate::sessions::Api;

#[test]
fn disabling_intake_keeps_the_expiry_sweep_and_web_submissions_require_the_servers_origin() {
    let github = 3_305_040_001;
    let mut fixture = Fixture::new(github);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let team = &fixture.team;
    let path = format!("/spaces/{}/abuse-reporting", team.space);
    let revision = member_revision(team, &team.flow.stand.server);
    let body = submission(&team.bob, REASON, &revision);
    let api = Api::of(&team.flow);
    let capability = get(&api, &path, &dave);
    assert_eq!(capability.status, 404);
    documented("get", CAPABILITY, &capability);
    let response = submit(&api, team.space, &dave, &body);
    assert_eq!(response.status, 404);
    refused(&response, "reporting off by default", &[]);

    let mut server = fixture
        .team
        .flow
        .stand
        .restart(&[], &settings("5", "10", "86400"));
    let team = &fixture.team;
    let api = Api::of(&team.flow).on(&server);
    for headers in [
        &[("cookie", dave.as_str())][..],
        &[
            ("cookie", dave.as_str()),
            ("origin", "https://another.example"),
        ],
    ] {
        let response = post(&api, team.space, headers, &body);
        assert_eq!(response.status, 403);
        refused(&response, "wrong web origin", &[]);
    }
    assert!(report_rows(fixture.db()).is_empty());

    let id = receipt(&submit(&api, team.space, &dave, &body), &[201]);
    server.stop();
    age_report(fixture.db(), &id, "25 hours");
    assert_eq!(report_rows(fixture.db()).len(), 1);
    let disabled = team.flow.stand.instance(&[], &[]);
    let response = get(&Api::of(&team.flow).on(&disabled), &path, &dave);
    assert_eq!(response.status, 404);
    wait_for_no_reports(fixture.db());
}

#[test]
fn two_instances_share_receipts_and_the_last_storage_slot() {
    let github = 3_305_040_011;
    let fixture = Fixture::new(github);
    let team = &fixture.team;
    let dave = sign_in(&fixture, &team.dave, github + 3);
    let carol = sign_in(&fixture, &team.carol, github + 2);
    let config = settings("5", "2", LONG_RETENTION);
    let one = team.flow.stand.instance(&[], &config);
    let two = team.flow.stand.instance(&[], &config);
    let api = Api::of(&team.flow);
    let first = api.on(&one);
    let second = api.on(&two);
    let revision = member_revision(team, &one);
    let report = submission(&team.bob, REASON, &revision);

    let [created, retried] = concurrently(
        || submit(&first, team.space, &dave, &report),
        || submit(&second, team.space, &dave, &report),
    );
    let id = receipt(&created, &[201]);
    assert_eq!(receipt(&retried, &[200]), id);
    assert_eq!(report_rows(fixture.db()).len(), 1);

    let mut changed = report;
    changed["reason"] = serde_json::json!(OTHER_REASON);
    let conflict = submit(&second, team.space, &dave, &changed);
    assert_eq!(conflict.status, 409);
    refused(&conflict, "the same key on a different report", &[&id]);

    let [stored, full] = concurrently(
        || {
            submit(
                &first,
                team.space,
                &dave,
                &submission(&team.alice, REASON, &revision),
            )
        },
        || {
            submit(
                &second,
                team.space,
                &carol,
                &submission(&team.bob, REASON, &revision),
            )
        },
    );
    receipt(&stored, &[201]);
    assert_eq!(full.status, 429);
    refused(&full, "both instances reached capacity", &[]);
    assert_eq!(report_rows(fixture.db()).len(), 2);
}

#[test]
fn a_stale_member_selection_cannot_report_the_new_holder_of_a_handle() {
    let github = 3_305_040_021;
    let mut fixture = Fixture::new(github);
    let team = &fixture.team;
    let carol = sign_in(&fixture, &team.carol, github + 2);
    // Keep the original instance for signing in and storing roster revisions.
    let server = team
        .flow
        .stand
        .instance(&[], &settings("5", "10", LONG_RETENTION));
    let revision = member_revision(team, &server);
    let report = submission(&team.dave, REASON, &revision);
    let stale = submission(&team.dave, REASON, &revision);
    let api = Api::of(&team.flow).on(&server);
    let original = receipt(&submit(&api, team.space, &carol, &report), &[201]);
    let previous = team.dave.account;
    let mut replacement = member(&fixture.eve, Role::Member);
    replacement.handle = team.dave.handle.clone();

    fixture.revise(|body| {
        let mut members = body.members.clone().into_vec();
        members.retain(|member| member.account != previous);
        body.members = NonEmpty::from_vec(members).expect("members remain");
    });
    fixture.revise(|body| body.members.push(replacement));

    let team = &fixture.team;
    let api = Api::of(&team.flow).on(&server);
    let rejected = submit(&api, team.space, &carol, &stale);
    assert_eq!(rejected.status, 409);
    refused(&rejected, "the roster changed before submission", &[]);
    let retried = submit(&api, team.space, &carol, &report);
    assert_eq!(receipt(&retried, &[200]), original);
    let rows = report_rows(fixture.db());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains(&previous.to_string()));
    assert!(!rows[0].contains(&fixture.eve.account.to_string()));

    let latest = member_revision(team, &server);
    assert_ne!(latest, revision);
    let fresh = submission(&team.dave, REASON, &latest);
    let new = receipt(&submit(&api, team.space, &carol, &fresh), &[201]);
    assert!(
        report_rows(fixture.db())
            .iter()
            .any(|row| { row.contains(&new) && row.contains(&fixture.eve.account.to_string()) })
    );
}

#[test]
fn an_expired_report_still_counts_until_the_submission_window_passes() {
    let github = 3_305_040_031;
    let mut fixture = Fixture::new(github);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let server = fixture
        .team
        .flow
        .stand
        .restart(&[], &settings("1", "1", "600"));
    let team = &fixture.team;
    let revision = member_revision(team, &server);
    let api = Api::of(&team.flow).on(&server);
    let submit_as_dave = |body| submit(&api, team.space, &dave, body);
    let first = submission(&team.bob, REASON, &revision);
    let id = receipt(&submit_as_dave(&first), &[201]);
    assert_eq!(receipt(&submit_as_dave(&first), &[200]), id);

    let db = fixture.db();
    age_report(db, &id, "11 minutes");
    wait_for_no_reports(db);
    let second = submission(&team.alice, OTHER_REASON, &revision);
    let refused_in_window = submit_as_dave(&second);
    assert_eq!(refused_in_window.status, 429);
    refused(
        &refused_in_window,
        "the submission window remains open",
        &[],
    );
    assert!(report_rows(db).is_empty());

    let reporter = team.dave.account.to_string();
    let aged = db.query(async |conn| {
        sqlx::query(
            "UPDATE abuse_report_submissions SET submitted_at = now() - interval '25 hours'
             WHERE reporter_id = $1::uuid",
        )
        .bind(&reporter)
        .execute(conn)
        .await
        .map(|result| result.rows_affected())
    });
    assert_eq!(aged.expect("move the submission outside its window"), 1);
    receipt(&submit_as_dave(&second), &[201]);
}

fn wait_for_no_reports(db: &TestDb) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !report_rows(db).is_empty() {
        assert!(Instant::now() < deadline, "expired reports remain");
        thread::sleep(Duration::from_millis(250));
    }
}

fn concurrently(
    first: impl FnOnce() -> Response + Send,
    second: impl FnOnce() -> Response + Send,
) -> [Response; 2] {
    let start = Barrier::new(2);
    let mut answers = thread::scope(|scope| {
        let first = scope.spawn(|| {
            start.wait();
            first()
        });
        start.wait();
        let second = second();
        [first.join().expect("first request"), second]
    });
    if answers[1].status == 201 {
        answers.swap(0, 1);
    }
    answers
}
