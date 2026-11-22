//! A message held at an agent in `hold`: the sender's status says `held`, then `read` once the
//! person accepts it, or `refused` with the reason the person gives; across devices and on one.

use skald_proto::MessageId;

use crate::across::a_json_once;
use crate::held::{Decision, HELD, HOLD, decide, holding, says_waiting};
use crate::inbound::{REFUSED, set_mode};
use crate::receipts::READ;
use crate::senders::LAPTOP_AGENT;
use crate::solo::{API, Solo, WEB};
use crate::statuses::{agent_time, assert_holds, hashed, json_status, shown_status};

/// What A's agent sends claude; neither is anywhere else.
const ACCEPTED: &str = "message status test: claude, rerun the nightly import";
const DECLINED: &str = "message status test: claude, drop the audit table";

/// The reason B's person refuses [`DECLINED`] with.
const REASON: &str = "not without a ticket";

/// B's person sets claude to `hold`. A's agent sends claude two messages, and B's person's
/// `skald recv` takes them in: A's agent's `skald message-status --json` of each gives `held`, and
/// neither `read` nor `refused`. B's person's `skald held` accepts the first and refuses the second
/// with a reason; claude's `skald recv` follows. The first then gives `read`, no earlier than
/// `held`; the second `refused`, no earlier than `held`, with that reason. A's agent's status of
/// each says `held` and then `read`, or `refused` with the reason.
#[test]
fn held_then_read_once_accepted_or_refused_with_the_reason() {
    let roles = holding(
        [4_380_012_001, 4_380_012_002],
        ["ada-waits-on-a-decision", "bob-decides-for-claude"],
    );
    let accepted = roles.a_sends_to_claude(ACCEPTED);
    let declined = roles.a_sends_to_claude(DECLINED);
    says_waiting(roles.b(), 2);

    let held: Vec<u64> = [accepted, declined]
        .into_iter()
        .map(|msg_id| {
            let object = a_json_once(&roles, msg_id, "no held", |object| {
                agent_time(object, HELD).is_some()
            });
            assert_eq!(agent_time(&object, READ), None, "{object}");
            assert_eq!(agent_time(&object, REFUSED), None, "{object}");
            agent_time(&object, HELD).unwrap_or_default()
        })
        .collect();

    let folder = roles.senders.agents.folder.clone();
    let decisions = [
        (ACCEPTED, Decision::Accept),
        (DECLINED, Decision::Refuse(REASON)),
    ];
    decide(roles.b(), &folder, &decisions);
    assert!(roles.claude_reads(false).success, "claude's recv failed");

    let read = a_json_once(&roles, accepted, "no read", |object| {
        agent_time(object, READ).is_some()
    });
    assert!(agent_time(&read, READ) >= Some(held[0]), "{read}");
    assert_eq!(agent_time(&read, HELD), Some(held[0]), "{read}");
    let refused = a_json_once(&roles, declined, "no refused", |object| {
        agent_time(object, REFUSED).is_some()
    });
    assert!(agent_time(&refused, REFUSED) >= Some(held[1]), "{refused}");
    assert_eq!(agent_time(&refused, READ), None, "{refused}");
    assert_eq!(refused["agent"]["reason"], REASON, "{refused}");
    let at = (roles.senders.laptop_folder.as_path(), Some(LAPTOP_AGENT));
    assert_holds(
        &shown_status(roles.a(), at, &hashed(accepted)),
        &[HELD, READ],
    );
    let said = shown_status(roles.a(), at, &hashed(declined));
    assert_holds(&said, &[HELD, REFUSED, REASON]);
}

/// Ada's machine is alone in `clienta`'s groups; web's session runs, and the person sets web to
/// `hold`. api's `skald send --to ada/web` sends two messages: api's
/// `skald message-status --json` of each gives `held`, and neither `read` nor `refused`. The
/// person's `skald held` accepts the first and refuses the second with a reason, and web's `inbox`
/// gets the first. The first then gives `read`, the second `refused` with that reason and no
/// `read`, each still `held`, and api's status says so; the front saw the two messages and no
/// other: no receipt.
#[test]
fn on_one_device_held_then_read_or_refused_with_the_reason() {
    let solo = Solo::new(4_380_013_001, "ada-web-holds-api");
    let mut web = solo.session(WEB);
    web.inbox();
    set_mode(solo.machine(), &solo.folder, WEB, HOLD);
    let mark = solo.mark();
    let [accepted, declined] = [ACCEPTED, DECLINED].map(|text| solo.api_sends_web(text));
    let dir = solo.dir(API);
    let at = (dir.as_path(), Some(API));
    let json = |msg_id: MessageId| json_status(solo.machine(), at, &msg_id.to_string());
    for msg_id in [accepted, declined] {
        let object = json(msg_id);
        assert!(agent_time(&object, HELD).is_some(), "no held: {object}");
        assert_eq!(agent_time(&object, READ), None, "{object}");
        assert_eq!(agent_time(&object, REFUSED), None, "{object}");
    }

    let decisions = [
        (ACCEPTED, Decision::Accept),
        (DECLINED, Decision::Refuse(REASON)),
    ];
    decide(solo.machine(), &solo.folder, &decisions);
    assert!(web.inbox().contains(ACCEPTED), "web got no message");

    let read = json(accepted);
    assert!(agent_time(&read, READ).is_some(), "no read: {read}");
    assert!(agent_time(&read, HELD).is_some(), "no held: {read}");
    let refused = json(declined);
    assert!(
        agent_time(&refused, REFUSED).is_some(),
        "no refused: {refused}"
    );
    assert!(agent_time(&refused, HELD).is_some(), "no held: {refused}");
    assert_eq!(agent_time(&refused, READ), None, "{refused}");
    assert_eq!(refused["agent"]["reason"], REASON, "{refused}");
    let said = shown_status(solo.machine(), at, &hashed(declined));
    assert_holds(&said, &[HELD, REFUSED, REASON]);
    assert_eq!(
        solo.sent_since(mark),
        [accepted, declined],
        "not the messages alone"
    );
}
