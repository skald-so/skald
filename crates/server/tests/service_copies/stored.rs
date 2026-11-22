//! The server stores an `hpke` envelope to a `service` device only while a live token of it with
//! the receive permission lists the envelope's channel; otherwise `unknown-recipient`, with nothing
//! stored.

use crate::bot::ops;
use crate::copies::{
    alice, assert_stored, assert_unknown_recipient, drop_entries, expire, grant_for, token_with,
};
use crate::tokens::{QUEUES, RECEIVE, SEND, Stage};

/// Envelopes from alice's laptop to the bot in `general` are refused with `unknown-recipient` while
/// the bot has no token, a token that only sends, one that only claims from queues, and one that
/// receives in `ops` alone; once a token that receives in `general` is created, the next one is
/// stored, and the bot's mailbox holds it alone.
#[test]
fn an_envelope_to_a_service_device_needs_a_receiving_token_for_its_channel() {
    let mut stage = Stage::new(3_555_100_001);
    let ops = ops(&mut stage);
    let general = stage.f.general;
    let mut socket = alice(&stage);

    assert_unknown_recipient(&stage, &mut socket, general, "no token");
    token_with(&stage, &grant_for(&stage, &[general], 0), &[SEND]);
    assert_unknown_recipient(&stage, &mut socket, general, "a token that only sends");
    token_with(&stage, &grant_for(&stage, &[general], 1), &[QUEUES]);
    assert_unknown_recipient(&stage, &mut socket, general, "a token of queues alone");
    token_with(&stage, &grant_for(&stage, &[ops], 2), &[SEND, RECEIVE]);
    assert_unknown_recipient(&stage, &mut socket, general, "a token of another channel");

    token_with(&stage, &grant_for(&stage, &[general], 3), &[RECEIVE]);
    assert_stored(
        &stage,
        &mut socket,
        general,
        &[],
        "a token that receives in general",
    );
}

/// A token that receives in `general` lets alice's laptop's envelopes to the bot be stored until
/// it ends: after its revocation, its expiry, which the test moves back in the database, and a
/// revision that drops the bot's entry, the next envelope is refused with `unknown-recipient`;
/// a new token stores again in between.
#[test]
fn an_envelope_to_a_service_device_is_refused_once_its_receiving_token_ends() {
    let mut stage = Stage::new(3_555_100_011);
    let general = stage.f.general;
    let mut socket = alice(&stage);
    let alice_person = &stage.f.team.alice;

    let revoked = token_with(&stage, &grant_for(&stage, &[general], 0), &[SEND, RECEIVE]);
    let held = assert_stored(&stage, &mut socket, general, &[], "a live token");
    let response = stage.revoke_as(alice_person, &revoked.id);
    assert_eq!(response.status, 204, "revoked");
    assert_unknown_recipient(&stage, &mut socket, general, "a revoked token");

    let expired = token_with(&stage, &grant_for(&stage, &[general], 1), &[RECEIVE]);
    let held = assert_stored(&stage, &mut socket, general, &held, "a new token");
    expire(&stage, &expired.id);
    assert_unknown_recipient(&stage, &mut socket, general, "an expired token");

    token_with(&stage, &grant_for(&stage, &[general], 2), &[RECEIVE]);
    assert_stored(&stage, &mut socket, general, &held, "another new token");
    drop_entries(&mut stage);
    assert_unknown_recipient(&stage, &mut socket, general, "a dropped entry");
}
