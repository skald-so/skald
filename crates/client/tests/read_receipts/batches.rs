//! One handover of messages from two sending devices and two `from`s gets a receipt per sending
//! device and `from`, each `to` its messages' `from`.

use std::collections::BTreeSet;

use skald_proto::MessageId;

use crate::agents::CLAUDE;
use crate::fixtures::{SEND, person_address};
use crate::receipts::{CHANNEL, Got, Roles, TO};
use crate::senders::LAPTOP_AGENT;
use crate::talk::{Line, one_message_sent};
use crate::world::{ACME, HOST};

/// What A's agent, A's person and Bob's agent send; none is anywhere else.
const BY_AGENT: &str = "receipt batch test: claude, tag the release";
const BY_PERSON: &str = "receipt batch test: claude, write the changelog";
const BY_BOB: &str = "receipt batch test: claude, bump the version";

/// Bob's agent that sends to claude.
const BOB_AGENT: &str = "codex-p4n7vd";

/// A's agent and A's person each send a message `--to` claude, and Bob's device, C, one from his
/// agent; claude's one `skald recv` gets all three. C reads three receipts of B's device in the
/// channel from claude: one of each message, `to` that message's `from`.
#[test]
fn one_handover_gets_a_receipt_per_sending_device_and_from() {
    let mut roles = Roles::new(
        [4_155_013_001, 4_155_013_002],
        ["ada-agent-hears-three", "bob-adds-a-third"],
    );
    let by_agent = roles.a_sends_to_claude(BY_AGENT);
    let by_person = a_person_sends_to_claude(&roles, BY_PERSON);
    let bob_agent = format!("{ACME}/{}/{BOB_AGENT}@{HOST}", roles.senders.bob());
    let mut line = Line::new(&bob_agent, 1, BY_BOB);
    line.to = Some(roles.full(CLAUDE));
    let by_bob = roles.senders.agents.bob_sends(&line);

    let got = roles.claude_reads(false).stdout;

    for text in [BY_AGENT, BY_PERSON, BY_BOB] {
        assert!(got.contains(text), "claude did not get {text:?}: {got}");
    }
    let person = person_address(ACME, &roles.senders.laptop);
    let mut receipts = Vec::new();
    for msg_id in [by_agent, by_person, by_bob] {
        receipts = roles.c_gets_receipt_of(msg_id);
    }
    assert_eq!(receipts.len(), 3, "{receipts:#?}");
    for (msg_id, from) in [
        (by_agent, roles.full(LAPTOP_AGENT)),
        (by_person, person),
        (by_bob, bob_agent),
    ] {
        let of_it: Vec<&Got> = receipts.iter().filter(|got| got.lists(msg_id)).collect();
        let [receipt] = of_it.as_slice() else {
            panic!("not one receipt of {msg_id}: {receipts:#?}");
        };
        // What it lists, the device that sent it, its `from` and its `to`.
        let seen = (
            receipt
                .receipt
                .messages
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            receipt.sender,
            receipt.content.from.to_string(),
            receipt.content.to.as_ref().map(ToString::to_string),
        );
        let wanted = (
            BTreeSet::from([msg_id]),
            roles.b_device(),
            roles.full(CLAUDE),
            Some(from),
        );
        assert_eq!(seen, wanted, "{receipt:#?}");
    }
}

/// Runs `skald send --to <claude> --channel <channel> <text>` as the person on A's machine; fails
/// unless it succeeds and the front saw one message sent in the channel. Gives its `msg_id`.
#[track_caller]
fn a_person_sends_to_claude(roles: &Roles, text: &str) -> MessageId {
    let talk = &roles.senders.agents.talk;
    let (claude, channel) = (roles.claude(), talk.channel.to_string());
    let from = talk.front.sent().len();
    let sent = (roles.a())
        .start(&[SEND, TO, &claude, CHANNEL, &channel, text])
        .finish();
    assert!(sent.success, "A's person: {sent:?}");
    one_message_sent(&talk.front, talk.channel, from)
}
