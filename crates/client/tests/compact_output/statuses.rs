//! The statuses and warnings an agent sees: only `expired` and `refused` of its own messages, and
//! warnings in one line without IDs; the person's `skald recv` shows every one in full.

use std::time::Duration;

use skald_proto::{MessageId, Role};
use skald_server::Settings;

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::fixtures::assert_deleted;
use crate::kit::{received, warnings_naming};
use crate::lines::{agent_line, agent_notes, assert_no_uuid, lines_referring_to};
use crate::said::sent_as;
use crate::talk::{BOB_HOST, DELIVERED, Line, Talk, status_lines};
use crate::times::{DAY, MAILBOX, WEEK, moved_back};
use crate::world::ACME;

/// A server status as the CLI shows it.
const EXPIRED: &str = "expired";

/// What Ada's agents send; none is anywhere else.
const EXPIRES: &str = "status test: claude's message that expires";
const REACHES: &str = "status test: claude's message that reaches bob";
const OF_CODEX: &str = "status test: codex's message that reaches bob";

/// What Bob's device says with a `messageId` other than its `msg_id`; it is nowhere else.
const FORGED: &str = "status test: a message whose messageId is not its msg_id";

/// Gives the server a mailbox retention of [`WEEK`].
fn retained(settings: &mut Settings) {
    settings.mailbox_retention = Some(Duration::from_secs(WEEK));
}

/// The lines of `shown`, an agent's output, that refer to `msg_id` and are no message.
fn statuses_referring_to(shown: &str, msg_id: MessageId) -> Vec<String> {
    let lines = lines_referring_to(shown, msg_id).into_iter();
    lines.filter(|line| agent_line(line).is_none()).collect()
}

/// Fails unless `shown`, an agent's output, says `expired` on one line, which is no message, by a
/// reference of `msg_id`.
#[track_caller]
fn assert_says_expired(shown: &str, msg_id: MessageId) {
    let lines = lines_referring_to(shown, msg_id);
    assert!(
        matches!(
            lines.as_slice(),
            [line] if line.to_lowercase().contains(EXPIRED) && agent_line(line).is_none()
        ),
        "not one line of {EXPIRED} of {msg_id}: {shown}"
    );
}

/// With a mailbox retention of a week, claude sends a message to the channel from `~/work/acme`
/// while Bob's device is offline, and the server expires it once its storing time is moved back
/// past the retention. claude then sends another message, and codex one; Bob's device reads both.
/// claude's next `skald recv` says `expired` of its first message, by its reference, and no status
/// of its second or of codex's message; codex's says no status of any of them; neither holds a
/// UUID. Each shows the other's messages, as the device hands its own messages to its agents.
/// The person's next `skald recv` then shows each of the three statuses once, in full: `expired`
/// of claude's first message and `delivered` of the other two, each with Bob's device.
#[test]
fn agent_sees_only_expired_of_its_own_messages_and_the_person_sees_every_status() {
    let talk = Talk::with(
        ACME,
        &[4_149_021_001, 4_149_021_002],
        &["ada-agents-get-statuses", "bob-goes-offline-awhile"],
        false,
        (Role::Member, retained),
    );
    let mut agents = Agents::bound(talk);
    let expires = sent_as(&agents, CLAUDE, EXPIRES);
    let cloud = agents.talk.cloud();
    cloud.execute(&moved_back(&MAILBOX, &format!("{} seconds", WEEK + DAY)));
    assert_deleted(cloud, &expires);
    let reaches = sent_as(&agents, CLAUDE, REACHES);
    let of_codex = sent_as(&agents, CODEX, OF_CODEX);
    let bob = agents.talk.played[0].device();
    let cloud = &agents.talk.world.cloud;
    agents.talk.played[0].take_in(cloud);
    for envelope in [&reaches, &of_codex] {
        assert_deleted(cloud, envelope);
    }
    let [expires, reaches, of_codex] =
        [&expires, &reaches, &of_codex].map(|envelope| envelope.envelope().header.msg_id);

    let claude = agents.run_as(CLAUDE).shown();
    let codex = agents.run_as(CODEX).shown();

    assert_says_expired(&claude, expires);
    for shown in [&claude, &codex] {
        for id in [reaches, of_codex] {
            let lines = statuses_referring_to(shown, id);
            assert!(lines.is_empty(), "a status of {id}: {lines:?}");
        }
        assert!(!shown.to_lowercase().contains(DELIVERED), "{shown}");
        assert_no_uuid(shown);
    }
    let lines = statuses_referring_to(&codex, expires);
    assert!(lines.is_empty(), "codex is told of claude's: {lines:?}");

    let person = received(&agents.talk.ada.machine).shown();

    for (status, id) in [
        (EXPIRED, expires),
        (DELIVERED, reaches),
        (DELIVERED, of_codex),
    ] {
        assert_eq!(
            status_lines(&person, status, id, bob),
            1,
            "{status} of {id} is not shown once: {person}"
        );
    }
}

/// Ada's device has run as claude in `~/work/acme`. A proxy puts into Ada's catch-up Bob's content
/// in the channel whose `messageId` is not its `msg_id`. claude's next `skald recv` warns of it on
/// one line that names Bob's device by its name, shows no other line, no UUID and not the text; the
/// person's next run warns of it once, naming Bob's device by its ID, and shows not the text.
#[test]
fn agent_is_warned_of_a_dropped_message_in_one_line_without_ids_and_the_person_in_full() {
    let mut agents = Agents::new(
        [4_149_022_001, 4_149_022_002],
        ["ada-agent-warned-briefly", "bob-forged-by-a-proxy"],
    );
    agents.run_as(CLAUDE);
    let mut line = Line::new(&agents.bob(), 1, FORGED);
    line.message_id = Some(MessageId::generate());
    let forged = agents.bob_says(&line);
    let talk = &agents.talk;
    talk.front.inject(talk.ada.device, vec![forged]);

    let claude = agents.run_as(CLAUDE).shown();

    assert_eq!(talk.front.injected(), 1, "not put into catch-up: {claude}");
    let notes = agent_notes(&claude);
    assert!(
        matches!(notes.as_slice(), [warning] if warning.contains(BOB_HOST)),
        "not one warning naming {BOB_HOST}: {claude}"
    );
    assert_no_uuid(&claude);
    assert!(!claude.contains(FORGED), "{claude}");

    let person = received(&talk.ada.machine);

    let warned = warnings_naming(&person, talk.played[0].device());
    assert_eq!(warned.len(), 1, "not warned once: {person:?}");
    assert!(!person.shown().contains(FORGED), "{person:?}");
}
