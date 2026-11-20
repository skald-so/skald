//! A recipient device whose entry expired before it got the message shows `expired`.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::Role;
use skald_server::Settings;

use crate::agents::{Agents, CLAUDE};
use crate::cloud::WAIT;
use crate::fixtures::assert_deleted;
use crate::said::sent_as;
use crate::statuses::{
    EXPIRED, STORED, assert_holds, assert_refers, assert_words, hashed, json_status, recipient,
    recipients, shown_status,
};
use crate::talk::{BOB_HOST, Talk};
use crate::times::{DAY, MAILBOX, WEEK, moved_back};
use crate::world::{ACME, now};

/// What claude sends while Bob's device is away; it is nowhere else.
const AWAY: &str = "message status test: claude's note that bob's device never takes";

/// Gives the server a mailbox retention of [`WEEK`].
fn week_long(settings: &mut Settings) {
    settings.mailbox_retention = Some(Duration::from_secs(WEEK));
}

/// With a mailbox retention of a week, claude sends a message to the channel from `~/work/acme`,
/// which Bob's device does not take; claude's status shows Bob's device at `stored`. The server
/// expires the entry once its storing time is moved back past the retention. claude's runs then
/// take in the server's `expired`, and claude's status shows Bob's device at `expired`, with a
/// time no earlier than the send; its compact status refers to the message and says `expired` and
/// Bob's device's name, and the person's says `expired` with the `msg_id` and Bob's device's name.
#[test]
fn recipient_device_whose_entry_expired_shows_expired() {
    let talk = Talk::with(
        ACME,
        &[4_380_003_001, 4_380_003_002],
        &["ada-outlives-the-retention", "bob-stays-away-a-week"],
        false,
        (Role::Member, week_long),
    );
    let agents = Agents::bound(talk);
    agents.run_as(CLAUDE);
    let bob = agents.talk.played[0].device();
    let since = now();
    let envelope = sent_as(&agents, CLAUDE, AWAY);
    let msg_id = envelope.envelope().header.msg_id;
    let id = msg_id.to_string();
    let machine = &agents.talk.ada.machine;
    let at = (agents.folder.as_path(), Some(CLAUDE));
    let before = json_status(machine, at, &id);
    assert_eq!(
        recipients(&before).len(),
        1,
        "not Bob's device alone: {before}"
    );
    assert_eq!(recipient(&before, bob).status, STORED, "{before}");
    let cloud = agents.talk.cloud();
    cloud.execute(&moved_back(&MAILBOX, &format!("{} seconds", WEEK + DAY)));
    assert_deleted(cloud, &envelope);

    let deadline = Instant::now() + WAIT;
    let expired = loop {
        agents.run_as(CLAUDE);
        let object = json_status(machine, at, &id);
        if recipient(&object, bob).status == EXPIRED {
            break object;
        }
        assert!(Instant::now() < deadline, "not expired: {object}");
        thread::sleep(Duration::from_millis(500));
    };

    let time = recipient(&expired, bob).timestamp;
    assert!(
        time.is_some_and(|time| (since..=now()).contains(&time)),
        "{expired}"
    );
    let compact = shown_status(machine, at, &hashed(msg_id));
    assert_refers(&compact, msg_id);
    assert_words(&compact, &[EXPIRED]);
    assert_holds(&compact, &[BOB_HOST]);
    let person = shown_status(machine, (agents.folder.as_path(), None), &id);
    assert_words(&person, &[EXPIRED]);
    assert_holds(&person, &[&id, BOB_HOST]);
}
