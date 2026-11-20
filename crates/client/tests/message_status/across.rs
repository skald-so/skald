//! A's status of its message to claude across devices: `stored` at each recipient device right
//! after the send, `delivered` with times after their runs, then claude's `read` and `replied`; the
//! command and the tool give the same by `#<ref>`, `<ref>` and `msg_id`, one-shot and through the
//! helper.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::MessageId;

use crate::agents::CLAUDE;
use crate::client::{CLIENT, session_in};
use crate::cloud::WAIT;
use crate::helped::{Helped, START_TIMEOUT, timeout_of};
use crate::kit::received;
use crate::lines::{JSON_FLAG, agent_line_of};
use crate::receipts::{READ, Roles};
use crate::replying::{REPLIED, REPLY, run_in};
use crate::senders::LAPTOP_AGENT;
use crate::statuses::{
    MESSAGE_STATUS, STORED, agent_time, assert_holds, assert_of, assert_refers, assert_words, bare,
    hashed, json_status, one_object, recipient, recipients, shown_status, tool_status,
};
use crate::talk::{ADA_HOST, BOB_HOST, DELIVERED};
use crate::world::{AGENT, now};

/// What A's agent sends claude, and what claude answers; neither is anywhere else.
const ASKED: &str = "message status test: claude, which port does staging listen on";
const ANSWERED: &str = "message status test: staging listens on 8443";

/// A's agent's `skald message-status --json <message>` in its `~/work/acme`.
#[track_caller]
fn a_json(roles: &Roles, message: &str) -> Value {
    let at = (roles.senders.laptop_folder.as_path(), Some(LAPTOP_AGENT));
    json_status(roles.a(), at, message)
}

/// A's agent's `skald recv`, which takes in A's mailbox, then its `skald message-status --json` of
/// `msg_id`, every half second until `done` holds of the object, for up to [`WAIT`]; fails if
/// none does. Gives that object.
#[track_caller]
pub fn a_json_once(
    roles: &Roles,
    msg_id: MessageId,
    what: &str,
    done: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        roles.a_agent_reads(&[]);
        let object = a_json(roles, &msg_id.to_string());
        if done(&object) {
            return object;
        }
        assert!(Instant::now() < deadline, "{what}: {object}");
        thread::sleep(Duration::from_millis(500));
    }
}

/// A's agent sends a message `--to` claude. Right after, A's status of it gives its `msg_id`, the
/// channel, A's device, its time, and B's and C's devices, by their names and accounts, at
/// `stored`, with no `read` or `replied` yet. After B's person's `skald recv` and C's intake, both
/// are `delivered`, each with a time no earlier than the send, and still no `read`. After claude's
/// `skald recv`, `read`; after claude's reply, `replied`, no earlier than `read`.
///
/// Then A's person's status names the `msg_id`, the channel, both devices and both accounts, and
/// `delivered`, `read` and `replied`. A's agent's status by `#<ref>`, by `<ref>` and by `msg_id`,
/// and its `message_status` tool by `#<ref>` and by `msg_id`, give the same compact text, which
/// refers to the message by its reference and names both devices, `read` and `replied`. Its
/// `--json` by `#<ref>` is the object by `msg_id`, and so is the one a run through the helper
/// gives.
#[test]
fn sender_sees_stored_then_delivered_read_and_replied_by_reference_and_msg_id() {
    let mut roles = Roles::new(
        [4_380_001_001, 4_380_001_002],
        ["ada-follows-her-message", "bob-acks-ada-message"],
    );
    let (b, c) = (
        roles.b_device(),
        roles.senders.agents.talk.played[0].device(),
    );
    let at = (roles.channel(), roles.senders.laptop.device);
    let since = now();

    let msg_id = roles.a_sends_to_claude(ASKED);

    let first = a_json(&roles, &msg_id.to_string());
    let sent = assert_of(&first, msg_id, at, since);
    assert_eq!(recipients(&first).len(), 2, "not B and C: {first}");
    for (device, name) in [(b, ADA_HOST), (c, BOB_HOST)] {
        let one = recipient(&first, device);
        assert_eq!(one.status, STORED, "{one:?}");
        assert_eq!(one.device_name.as_deref(), Some(name), "{one:?}");
        assert!(one.account.is_some(), "no account: {one:?}");
        assert!(one.timestamp.is_some(), "no time: {one:?}");
    }
    assert_eq!(agent_time(&first, READ), None, "{first}");
    assert_eq!(agent_time(&first, REPLIED), None, "{first}");

    assert!(received(roles.b()).success, "B's person's run");
    roles.c_receipts();

    let both = |object: &Value| (recipients(object).iter()).all(|one| one.status == DELIVERED);
    let delivered = a_json_once(&roles, msg_id, "not delivered to both", both);
    for device in [b, c] {
        let one = recipient(&delivered, device);
        let time = one.timestamp.unwrap_or_else(|| panic!("no time: {one:?}"));
        assert!((sent..=now()).contains(&time), "{one:?} before {sent}");
    }
    assert_eq!(
        agent_time(&delivered, READ),
        None,
        "read after the person's run"
    );

    let claude = roles.claude_reads(false).shown();

    let read = a_json_once(&roles, msg_id, "no read", |object| {
        agent_time(object, READ).is_some()
    });
    let read_at = agent_time(&read, READ).unwrap_or_default();
    assert!(
        (sent..=now()).contains(&read_at),
        "read at {read_at}: {read}"
    );
    assert_eq!(agent_time(&read, REPLIED), None, "{read}");
    let short = format!("#{}", agent_line_of(&claude, ASKED).reference);
    let b_at = (roles.senders.agents.folder.as_path(), Some(CLAUDE));
    let answered = run_in(roles.b(), b_at, REPLY, &[&short, ANSWERED]);
    assert!(answered.success, "{answered:?}");

    let replied = a_json_once(&roles, msg_id, "no replied", |object| {
        agent_time(object, REPLIED).is_some()
    });
    let replied_at = agent_time(&replied, REPLIED).unwrap_or_default();
    assert!((read_at..=now()).contains(&replied_at), "{replied}");

    let person = shown_status(
        roles.a(),
        (&roles.senders.laptop_folder, None),
        &msg_id.to_string(),
    );
    let handles = [roles.senders.ada(), roles.senders.bob()];
    let id = roles.channel().to_string();
    assert_holds(&person, &[&msg_id.to_string(), &id, ADA_HOST, BOB_HOST]);
    assert_holds(&person, &[&handles[0], &handles[1]]);
    assert_words(&person, &[DELIVERED, READ, REPLIED]);

    let a_at = (roles.senders.laptop_folder.as_path(), Some(LAPTOP_AGENT));
    let given = [hashed(msg_id), bare(msg_id), msg_id.to_string()];
    let compact = given
        .clone()
        .map(|message| shown_status(roles.a(), a_at, &message));
    assert_refers(&compact[0], msg_id);
    assert_holds(&compact[0], &[ADA_HOST, BOB_HOST]);
    assert_words(&compact[0], &[READ, REPLIED]);
    let mut session = session_in(
        roles.a(),
        &roles.senders.laptop_folder,
        &[AGENT, LAPTOP_AGENT],
        CLIENT,
    );
    let tool = [&given[0], &given[2]].map(|message| tool_status(&mut session, message));
    for (what, other) in [
        ("<ref>", &compact[1]),
        ("msg_id", &compact[2]),
        ("the tool by #<ref>", &tool[0]),
        ("the tool by msg_id", &tool[1]),
    ] {
        assert_eq!(other.trim(), compact[0].trim(), "{what} gives other text");
    }
    assert_eq!(a_json(&roles, &given[0]), replied, "--json by #<ref>");

    drop(session);
    let helped = Helped::new(roles.a());
    let args = [AGENT, LAPTOP_AGENT, MESSAGE_STATUS, JSON_FLAG, &given[2]];
    let through = helped
        .start_in(
            &roles.senders.laptop_folder,
            &timeout_of(START_TIMEOUT),
            &args,
        )
        .finish();

    assert_eq!(
        one_object(&through, &given[2]),
        replied,
        "through the helper"
    );
    helped.one_helper();
}
