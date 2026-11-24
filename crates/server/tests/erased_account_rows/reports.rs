use crate::deletion::deletes;
use crate::erasure::{ERASED_AFTER, Kept, Traces, erased_within, leftovers, move_deletion_back};
use crate::fixture::Fixture;
use crate::reporting::{
    OTHER_REASON, REASON, age_report, member_revision, receipt, report_rows, sign_in, start,
    submission, submit,
};
use crate::sessions::Api;

#[test]
fn a_report_loses_its_reporters_account_data_after_thirty_days() {
    erases_report_account(3_305_030_001, true);
}

#[test]
fn a_report_loses_its_targets_account_data_after_thirty_days() {
    erases_report_account(3_305_030_011, false);
}

fn erases_report_account(github: u64, alice_reported: bool) {
    let mut fixture = Fixture::new(github);
    let team = &fixture.team;
    let devices = [team.alice.laptop.id, team.alice.uncertified.id];
    let traces = Traces::of(&team.alice, github, &devices);
    let kept = Kept::of_space(&team.bob.session, team.space, &devices);
    let (reporter, reporter_github) = if alice_reported {
        (&team.alice, github)
    } else {
        (&team.dave, github + 3)
    };
    let affected_cookie = sign_in(&fixture, reporter, reporter_github);
    let bob = sign_in(&fixture, &team.bob, github + 1);

    let server = start(&mut fixture);
    let team = &fixture.team;
    let api = Api::of(&team.flow).on(&server);
    let revision = member_revision(team, &server);
    let target = if alice_reported {
        &team.dave
    } else {
        &team.alice
    };
    let affected = receipt(
        &submit(
            &api,
            team.space,
            &affected_cookie,
            &submission(target, REASON, &revision),
        ),
        &[201],
    );
    let unrelated = receipt(
        &submit(
            &api,
            team.space,
            &bob,
            &submission(&team.carol, OTHER_REASON, &revision),
        ),
        &[201],
    );
    let db = fixture.db();

    age_report(db, &affected, "31 days");
    // Both reports are within the configured 60 days, and the space survives deletion.
    deletes(&api, &team.alice);
    assert!(
        leftovers(db, &traces, &kept)
            .iter()
            .any(|row| row.contains(&affected)),
        "normal deletion leaves the synthetic report for the erasure sweep"
    );
    move_deletion_back(db, team.alice.account, ERASED_AFTER);

    erased_within(db, &traces, &kept);
    assert!(
        report_rows(db).iter().any(|row| row.contains(&unrelated)),
        "the unrelated report remains"
    );
    assert!(
        team.bob
            .session
            .on(&server)
            .spaces()
            .contains_key(&team.space.to_string()),
        "the shared space remains"
    );
}
