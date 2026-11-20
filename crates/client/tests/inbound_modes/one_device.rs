//! On one device, an agent in `refuse` gets no message from another agent of the device, and the
//! sender's status is `refused` with no receipt message.

use crate::agents::assert_not_handed;
use crate::inbound::{REFUSE, agent_refused_lines, marked_lines, set_mode};
use crate::kit::RECV;
use crate::solo::{API, Solo, WEB};
use crate::world::AGENT;

/// What api sends web; it is nowhere else.
const TEXT: &str = "inbound one device test: web, take the site offline";

/// Ada's machine is alone in `clienta`'s groups. web's session runs in `~/work/clientA/web`, and
/// the person sets web to `refuse` in `~/work/clientA`. api's `skald send --to ada/web` sends one
/// message; web's `inbox` does not show it. The person's next `skald recv` shows it once, marked
/// refused for web. api's next `skald recv` says `refused` of it on one line, and the next one does
/// not. The front saw that message in the channel and no other: no receipt.
#[test]
fn on_one_device_refused_is_set_with_no_receipt_message() {
    let solo = Solo::new(4_731_016_001, "ada-web-turns-api-away");
    let mut web = solo.session(WEB);
    web.inbox();
    set_mode(solo.machine(), &solo.folder, WEB, REFUSE);
    let mark = solo.mark();

    let msg_id = solo.api_sends_web(TEXT);

    assert_not_handed(&web.inbox(), TEXT);
    let person = solo.person_reads();
    assert_eq!(person.matches(TEXT).count(), 1, "{person}");
    assert!(
        marked_lines(&person, msg_id, WEB) >= 1,
        "not marked: {person}"
    );
    let api_reads = || {
        let read = (solo.machine())
            .start_in(&solo.dir(API), &[AGENT, API, RECV])
            .finish();
        assert!(read.success, "{read:?}");
        read.shown()
    };
    let said = api_reads();
    assert_eq!(agent_refused_lines(&said, msg_id).len(), 1, "{said}");
    let again = api_reads();
    assert!(agent_refused_lines(&again, msg_id).is_empty(), "{again}");
    assert_not_handed(&web.inbox(), TEXT);
    assert_eq!(solo.sent_since(mark), [msg_id], "not the message alone");
}
