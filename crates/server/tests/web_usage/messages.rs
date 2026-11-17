//! A space's messages this month, as the monthly messages limit counts them: each message once,
//! whatever its envelopes; no retry and no refused message; in the current calendar month in UTC.

use skald_proto::{DeviceId, Header, MessageId};

use crate::limits::{
    MONTHLY, QUOTA_EXCEEDED, answer_to, assert_refused_with, assert_stored, clear_of_the_month_end,
    to_the_month_before,
};
use crate::usage::{Web, of, space_usage};
use crate::wire::{Socket, UNKNOWN_RECIPIENT, envelope};

/// The envelopes of one new message of `web`'s fixture, in `general` of `acme`: one per recipient
/// of `to`, all with one `msg_id`, each with its header.
fn one_message(web: &Web, to: &[DeviceId]) -> Vec<(Header, Vec<u8>)> {
    let msg_id = MessageId::generate();
    let mut sent = Vec::new();
    for &recipient in to {
        let mut header = web.fixture.to(recipient);
        header.msg_id = msg_id;
        let bytes = envelope(&header);
        sent.push((header, bytes));
    }
    sent
}

/// Sends each envelope of `message` on `socket`; the server must store each.
#[track_caller]
fn stored(socket: &mut Socket, message: &[(Header, Vec<u8>)]) {
    for (header, bytes) in message {
        assert_stored(socket, header, bytes);
    }
}

/// With the setting at 3: `acme` starts at 0 of 3. Alice's message to bob and carol counts once,
/// and its retry not at all: 1 of 3, as alice's, bob's and carol's web sessions each get it. Her
/// send to eve, outside the space, is refused and does not count. With bob's message and alice's
/// next, 3 of 3: the next message is `quota-exceeded`, and the count stays at 3. Once the month's
/// messages are moved to the month before, `acme` is at 0 of 3 again.
#[test]
fn the_count_follows_the_messages_sent_this_month_up_to_the_limit() {
    let web = Web::with(3_226_100_001, &[MONTHLY, "3"]);
    let (fixture, api) = (&web.fixture, web.api());
    clear_of_the_month_end(fixture.db());
    let [alice_laptop, bob_laptop, carol_laptop, _, eve_laptop] = fixture.laptops();
    let space = fixture.team.space;
    let messages = |cookie: &str| space_usage(&api, space, cookie).messages;
    assert_eq!(messages(&web.alice), of(0, 3), "before any message");

    let mut alice = Socket::of(&fixture.team.flow, &fixture.team.alice);
    let first = one_message(&web, &[bob_laptop, carol_laptop]);
    stored(&mut alice, &first);
    let (header, bytes) = &first[0];
    answer_to(&mut alice, header, bytes);
    for (cookie, who) in [
        (&web.alice, "alice"),
        (&web.bob, "bob"),
        (&web.carol, "carol"),
    ] {
        assert_eq!(messages(cookie), of(1, 3), "one message, {who}");
    }

    let (to_eve, refused) = one_message(&web, &[eve_laptop]).remove(0);
    let db = fixture.db();
    assert_refused_with(db, &mut alice, &to_eve, &refused, UNKNOWN_RECIPIENT, "eve");
    assert_eq!(messages(&web.alice), of(1, 3), "a refused message");

    let mut bob = Socket::of(&fixture.team.flow, &fixture.team.bob);
    stored(&mut bob, &one_message(&web, &[alice_laptop]));
    stored(&mut alice, &one_message(&web, &[bob_laptop]));
    assert_eq!(messages(&web.dave), of(3, 3), "three messages");

    let (over, bytes) = one_message(&web, &[bob_laptop]).remove(0);
    assert_refused_with(db, &mut alice, &over, &bytes, QUOTA_EXCEEDED, "a fourth");
    assert_eq!(messages(&web.alice), of(3, 3), "at the limit");

    to_the_month_before(db);
    assert_eq!(messages(&web.bob), of(0, 3), "the month before");
}
