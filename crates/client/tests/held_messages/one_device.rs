//! On one device, a message for an agent in `hold` from another agent of the device waits for the
//! person too, and the sender's statuses are set with no receipt message.

use skald_proto::MessageId;

use crate::agents::assert_not_handed;
use crate::held::{
    Decision, HELD, HOLD, agent_status_lines, decide, says_none_waiting, says_waiting,
};
use crate::inbound::{REFUSED, set_mode};
use crate::kit::RECV;
use crate::receipts::assert_shows_once;
use crate::solo::{API, Solo, WEB};
use crate::world::AGENT;

/// What api sends web; none is anywhere else.
const ACCEPTED: &str = "hold one device test: web, publish the changelog";
const REFUSED_TEXT: &str = "hold one device test: web, unpublish the docs";

/// What the person types when refusing.
const REASON: &str = "the docs stay up";

/// Fails unless api's next `skald recv` says `status` of each of `msg_ids` on one line; gives
/// those lines.
#[track_caller]
fn api_says(solo: &Solo, msg_ids: &[MessageId], status: &str) -> Vec<String> {
    let api = [AGENT, API, RECV];
    let run = solo.machine().start_in(&solo.dir(API), &api).finish();
    assert!(run.success, "api: {run:?}");
    let shown = run.shown();
    (msg_ids.iter())
        .map(
            |msg_id| match agent_status_lines(&shown, *msg_id, status).as_slice() {
                [line] => line.clone(),
                _ => panic!("{status} of {msg_id} not on one line: {shown}"),
            },
        )
        .collect()
}

/// Ada's machine is alone in `clienta`'s groups. web's session runs, and the person sets web to
/// `hold`. api's `skald send --to ada/web` sends two messages; web's `inbox` shows neither, the
/// person's `skald recv` says two wait, and api's next `skald recv` says `held` of each. The
/// person's `skald held` accepts the first and refuses the second with a reason: web's `inbox`
/// returns the first and not the second, and api's next `skald recv` says `refused` of the second,
/// with the reason. The front saw the two messages in the channel and no other: no receipt.
#[test]
fn on_one_device_held_then_read_or_refused_go_with_no_receipt_message() {
    let solo = Solo::new(4_739_010_001, "ada-web-holds-for-her");
    let mut web = solo.session(WEB);
    web.inbox();
    set_mode(solo.machine(), &solo.folder, WEB, HOLD);
    let mark = solo.mark();

    let accepted = solo.api_sends_web(ACCEPTED);
    let refused = solo.api_sends_web(REFUSED_TEXT);

    let inbox = web.inbox();
    for text in [ACCEPTED, REFUSED_TEXT] {
        assert_not_handed(&inbox, text);
    }
    says_waiting(solo.machine(), 2);
    api_says(&solo, &[accepted, refused], HELD);

    let decisions = [
        (ACCEPTED, Decision::Accept),
        (REFUSED_TEXT, Decision::Refuse(REASON)),
    ];
    decide(solo.machine(), &solo.folder, &decisions);

    let inbox = web.inbox();
    assert_shows_once(&inbox, ACCEPTED);
    assert_not_handed(&inbox, REFUSED_TEXT);
    let lines = api_says(&solo, &[refused], REFUSED);
    assert!(lines[0].contains(REASON), "no reason {REASON:?}: {lines:?}");
    says_none_waiting(solo.machine(), "once both are decided");
    assert_eq!(
        solo.sent_since(mark),
        [accepted, refused],
        "not the messages alone"
    );
}
