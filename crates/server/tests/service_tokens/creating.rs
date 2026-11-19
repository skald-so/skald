//! A token created with an admin's device proof and a valid grant: given once, with the grant's
//! channels and expiry, readable by the token itself, and stored only as a digest.

use serde_json::Value;

use crate::common::assert_holds_none;
use crate::common::device_flow::{json, text};
use crate::tokens::{
    DAY, QUEUES, RECEIVE, SEND, Stage, find, grant, grant_of, seconds, sorted_strings,
};
use crate::wire::now;

/// Checks that `record`, a token's record, is that of the bot's token `id`: `ci/deploy-bot`, with
/// `permissions`, the channel `general` and the expiry `expires`; `what` names the answer.
#[track_caller]
fn assert_record(
    stage: &Stage,
    record: &Value,
    id: &str,
    permissions: &[&str],
    expires: u64,
    what: &str,
) {
    assert_eq!(record["id"], id, "{what}: {record}");
    assert_eq!(
        record["device"],
        stage.bot.id.to_string(),
        "{what}: {record}"
    );
    assert_eq!(record["owner"], "ci", "{what}: {record}");
    assert_eq!(record["agent"], "deploy-bot", "{what}: {record}");
    let mut expected: Vec<String> = permissions.iter().map(|p| (*p).to_owned()).collect();
    expected.sort();
    assert_eq!(
        sorted_strings(&record["permissions"]),
        expected,
        "{what}: {record}"
    );
    let general = vec![stage.f.general.to_string()];
    assert_eq!(
        sorted_strings(&record["channels"]),
        general,
        "{what}: {record}"
    );
    let expires = i64::try_from(expires).expect("a time");
    assert_eq!(
        seconds(stage.db(), &record["expires_at"]),
        expires,
        "{what}: {record}"
    );
}

/// alice's session, proved by her laptop, creates a token of the bot with a grant by her laptop:
/// `201`, with the value, the bot's entry, the permissions and the grant's channels and expiry, and
/// a creation of about now. The token reads its own record and the grant as sent; alice's session
/// lists it. Neither answer holds the value, and no row of the database holds it in the clear.
#[test]
fn a_token_is_given_once_and_reads_its_record_and_grant() {
    let stage = Stage::new(3_535_100_001);
    let expires = now() + 30 * DAY;
    let sent = grant(
        stage.bot.id,
        &[stage.f.general],
        &stage.f.team.alice.laptop,
        expires,
    );
    let before = i64::try_from(now()).expect("a time");
    let created = stage.create_with(&sent);
    assert_record(
        &stage,
        &created.body,
        &created.id,
        &[SEND, RECEIVE],
        expires,
        "the answer",
    );
    let at = seconds(stage.db(), &created.body["created_at"]);
    let after = i64::try_from(now()).expect("a time");
    assert!(
        (before - 1..=after + 1).contains(&at),
        "created at {at}, not between {before} and {after}"
    );

    let own = stage.own(&created.token);
    assert_eq!(own.status, 200, "the token's own record: {}", text(&own));
    let record = json(&own);
    assert_record(
        &stage,
        &record,
        &created.id,
        &[SEND, RECEIVE],
        expires,
        "its own record",
    );
    assert_eq!(grant_of(&record), sent, "the grant as sent");

    let listed = stage.list(&stage.f.team.alice);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_record(
        &stage,
        find(&listed, &created.id),
        &created.id,
        &[SEND, RECEIVE],
        expires,
        "listed",
    );

    for (what, body) in [
        ("its own record", record),
        ("the list", Value::from(listed)),
    ] {
        assert!(
            !body.to_string().contains(&created.token),
            "{what} holds the value: {body}"
        );
    }
    assert_holds_none(&stage.db().rows(), std::slice::from_ref(&created.token));
}

/// A token of the bot with each set of permissions has exactly those; two tokens of one grant are
/// two tokens, each with a value of its own.
#[test]
fn a_token_has_the_permissions_it_was_given() {
    let stage = Stage::new(3_535_100_011);
    let alice = &stage.f.team.alice;
    let sent = stage.bot_grant();
    let mut values = Vec::new();
    for permissions in [&[SEND][..], &[RECEIVE], &[QUEUES], &[SEND, RECEIVE, QUEUES]] {
        let created = stage.create_as(alice, stage.bot.id, &sent, permissions);
        let mut expected: Vec<String> = permissions.iter().map(|p| (*p).to_owned()).collect();
        expected.sort();
        let own = json(&stage.own(&created.token));
        assert_eq!(sorted_strings(&own["permissions"]), expected, "{own}");
        values.push(created.token);
    }
    let mut distinct = values.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), values.len(), "a value given twice");
    assert_eq!(stage.listed(alice).len(), values.len());
}
