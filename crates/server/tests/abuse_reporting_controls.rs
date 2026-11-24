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

use crate::common::Response;
use crate::fixture::Fixture;
use crate::reporting::{
    CAPABILITY, LONG_RETENTION, OTHER_REASON, REASON, age_report, documented, get, post, receipt,
    refused, report_rows, settings, sign_in, submission, submit,
};
use crate::sessions::Api;

#[test]
fn disabling_intake_keeps_the_expiry_sweep_and_web_submissions_require_the_servers_origin() {
    let github = 3_305_040_001;
    let mut fixture = Fixture::new(github);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let team = &fixture.team;
    let path = format!("/spaces/{}/abuse-reporting", team.space);
    let body = submission(&team.bob, REASON);
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
    let deadline = Instant::now() + Duration::from_secs(30);
    while !report_rows(fixture.db()).is_empty() {
        assert!(
            Instant::now() < deadline,
            "expired report remains with intake off"
        );
        thread::sleep(Duration::from_millis(250));
    }
}

#[test]
fn two_instances_share_receipts_and_the_last_storage_slot() {
    let github = 3_305_040_011;
    let fixture = Fixture::new(github);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let carol = sign_in(&fixture, &fixture.team.carol, github + 2);
    let team = &fixture.team;
    let config = settings("5", "2", LONG_RETENTION);
    let one = team.flow.stand.instance(&[], &config);
    let two = team.flow.stand.instance(&[], &config);
    let api = Api::of(&team.flow);
    let first = api.on(&one);
    let second = api.on(&two);
    let report = submission(&team.bob, REASON);

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
        || submit(&first, team.space, &dave, &submission(&team.alice, REASON)),
        || submit(&second, team.space, &carol, &submission(&team.bob, REASON)),
    );
    receipt(&stored, &[201]);
    assert_eq!(full.status, 429);
    refused(&full, "both instances reached capacity", &[]);
    assert_eq!(report_rows(fixture.db()).len(), 2);
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
