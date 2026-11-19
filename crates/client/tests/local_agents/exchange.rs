//! Two agents of one device, alone in its channel's group, exchange a message and its reply through
//! the MCP tools; the person sees `read` and `replied`, and no receipt leaves the device.

use crate::receipts::read_lines;
use crate::replying::replied_lines;
use crate::solo::{API, OWN, Solo, WEB, assert_got, assert_not_shown, replies};
use crate::talk::{ADA_HOST, one_message_sent};

/// What api asks web and web answers; neither is anywhere else.
const ASKED: &str = "local agents test: api asks web for the build's port";
const ANSWERED: &str = "local agents test: web says the build listens on 8080";

/// Sessions `--agent api` in `~/work/clientA/api` and `--agent web` in `~/work/clientA/web` run on
/// Ada's machine, alone in `clienta`'s groups. api's send tool `to` web, with no channel, sends one
/// message in `clienta`'s default channel. The person's next `skald recv` shows it once and no
/// `read` of it. web's `inbox` shows it from `ada/api`, Ada's machine, at the level `own`; api's
/// shows nothing of it. web's reply tool answers it by its `#<ref>`: one message in the channel,
/// which api's next `inbox` shows from `ada/web` at `own`, and web's does not. The person's next
/// `skald recv` shows `read` and `replied` of api's message once each, the next one neither. The
/// front saw the two messages in the channel and no other: no receipt.
#[test]
fn send_tool_to_web_reaches_web_only_and_its_reply_gives_read_and_replied() {
    let solo = Solo::new(4_657_001_001, "ada-runs-api-and-web");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let from = solo.mark();

    let asked = solo.sends(&mut api, &solo.short(WEB), ASKED);

    let before = solo.person_reads();
    assert_eq!(before.matches(ASKED).count(), 1, "{before}");
    assert_eq!(
        read_lines(&before, asked),
        0,
        "read before web has it: {before}"
    );
    let got = web.inbox();
    assert_got(&got, ASKED, asked, (&solo.short(API), ADA_HOST, OWN));
    assert_not_shown(&api.inbox(), ASKED, API);

    let mark = solo.mark();
    replies(&mut web, &got, ASKED, ANSWERED);

    let answered = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    let back = api.inbox();
    assert_got(&back, ANSWERED, answered, (&solo.short(WEB), ADA_HOST, OWN));
    assert_not_shown(&web.inbox(), ANSWERED, WEB);
    let person = solo.person_reads();
    assert_eq!(read_lines(&person, asked), 1, "not read once: {person}");
    let replied = replied_lines(&person, asked, &[ANSWERED]);
    assert_eq!(replied.len(), 1, "not replied once: {person}");
    let next = solo.person_reads();
    assert_eq!(read_lines(&next, asked), 0, "read again: {next}");
    assert!(
        replied_lines(&next, asked, &[ANSWERED]).is_empty(),
        "{next}"
    );
    assert_eq!(
        solo.sent_since(from),
        [asked, answered],
        "not the message and the reply alone"
    );
}
