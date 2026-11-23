use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::fixture::Fixture;
use crate::reporting::{
    LONG_RETENTION, OTHER_REASON, REASON, age_report, receipt, refused, report_rows, settings,
    sign_in, submission, submit,
};
use crate::sessions::Api;

#[test]
fn retries_survive_the_configured_submission_limit_and_total_storage_stays_bounded() {
    let github = 3_305_020_001;
    let mut fixture = Fixture::new(github);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let carol = sign_in(&fixture, &fixture.team.carol, github + 2);
    let server = fixture
        .team
        .flow
        .stand
        .restart(&[], &settings("2", "3", LONG_RETENTION));
    let team = &fixture.team;
    let api = Api::of(&team.flow).on(&server);
    let submit_as = |cookie: &str, request: &Value| submit(&api, team.space, cookie, request);
    let for_bob = submission(&team.bob, REASON);
    let first = receipt(&submit_as(&dave, &for_bob), &[201]);
    receipt(
        &submit_as(&dave, &submission(&team.alice, OTHER_REASON)),
        &[201],
    );

    let hidden = [team.bob.handle.as_str(), &team.bob.account.to_string()];
    let rejected = submit_as(&dave, &submission(&team.bob, OTHER_REASON));
    refused(
        &rejected,
        "the reporting account's submission limit",
        &hidden,
    );
    assert_eq!(receipt(&submit_as(&dave, &for_bob), &[200, 201]), first);
    assert_eq!(
        report_rows(fixture.db()).len(),
        2,
        "only two reports were stored"
    );

    let for_alice = submission(&team.alice, REASON);
    let third = receipt(&submit_as(&carol, &for_alice), &[201]);
    let full = report_rows(fixture.db());
    assert_eq!(full.len(), 3);
    let overflow = submit_as(&carol, &submission(&team.bob, OTHER_REASON));
    refused(&overflow, "the server's report capacity", &hidden);
    assert_eq!(receipt(&submit_as(&carol, &for_alice), &[200, 201]), third);
    assert_eq!(
        report_rows(fixture.db()),
        full,
        "a full store keeps its existing reports"
    );
}

#[test]
fn configured_retention_frees_capacity_and_keeps_newer_reports() {
    let github = 3_305_020_011;
    let mut fixture = Fixture::new(github);
    let dave = sign_in(&fixture, &fixture.team.dave, github + 3);
    let server = fixture
        .team
        .flow
        .stand
        .restart(&[], &settings("5", "2", "86400"));
    let team = &fixture.team;
    let api = Api::of(&team.flow).on(&server);
    let submit_as_dave = |request: &Value| submit(&api, team.space, &dave, request);
    let old = receipt(&submit_as_dave(&submission(&team.bob, REASON)), &[201]);
    let fresh = receipt(
        &submit_as_dave(&submission(&team.alice, OTHER_REASON)),
        &[201],
    );
    assert_ne!(old, fresh, "each submission gets its own receipt");
    let before = report_rows(fixture.db());
    assert_eq!(before.len(), 2);
    age_report(fixture.db(), &old, "25 hours");

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let rows = report_rows(fixture.db());
        if !rows.iter().any(|row| row.contains(&old)) {
            assert_eq!(
                rows.len(),
                1,
                "only the expired report is removed: {rows:?}"
            );
            assert!(rows[0].contains(&fresh), "kept the newer report: {rows:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the expired report remains: {rows:?}"
        );
        thread::sleep(Duration::from_millis(250));
    }

    receipt(&submit_as_dave(&submission(&team.carol, REASON)), &[201]);
    assert_eq!(
        report_rows(fixture.db()).len(),
        2,
        "an expired report frees capacity"
    );
}
