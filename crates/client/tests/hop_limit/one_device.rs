//! On one device, alone in the channel's group, two agents replying or sending to each other stop at
//! the same limit, and the person is warned of it once.

use serde_json::json;

use crate::client::{MESSAGE_FIELD, REPLY_TOOL, SEND_TOOL, Session, TEXT_FIELD, TO_FIELD};
use crate::loops::{
    LIMIT, Said, Way, assert_person_warned_once, assert_sent, assert_stopped, reference_of, texts,
};
use crate::solo::{API, Solo, WEB};

/// `session` replies with the reply tool, with `text`, to the message whose line in `shown`, its
/// `inbox`, has the text `asked`.
fn replies(session: &mut Session, shown: &str, asked: &str, text: &str) -> Said {
    let message = format!("#{}", reference_of(shown, asked));
    let called = session.call(
        REPLY_TOOL,
        json!({ MESSAGE_FIELD: message, TEXT_FIELD: text }),
    );
    Said {
        ok: !called.error,
        shown: called.text,
    }
}

/// Ada's machine, alone in `clienta`'s groups, runs MCP sessions `--agent api` and `--agent web`.
/// web sends api a message with the send tool; api and web, in turn, take the other's last message
/// with `inbox` and reply to it with the reply tool, ten replies in all, each sent. api takes web's
/// last reply; its reply to it fails, says why, and web's `inbox` never returns it. Ada's person's
/// next run warns once, with `clienta`'s default channel and web, and the run after it does not.
#[test]
fn agents_of_one_device_replying_to_each_other_stop_at_ten_hops() {
    let solo = Solo::new(4_811_009_001, "ada-loops-on-one-device");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let texts = texts(Way::ReplyTool);
    solo.sends(&mut web, &solo.short(API), &texts[0]);
    let sessions = [&mut api, &mut web];
    for (at, pair) in texts[..=LIMIT as usize].windows(2).enumerate() {
        let session = &mut *sessions[at % 2];
        let shown = session.inbox();
        let said = replies(session, &shown, &pair[0], &pair[1]);
        assert_sent(&said, &pair[1]);
    }
    let last = api.inbox();
    let over = &texts[LIMIT as usize + 1];

    let refused = replies(&mut api, &last, &texts[LIMIT as usize], over);

    assert_stopped(&refused, "the reply at hop 11");
    let got = web.inbox();
    assert!(!got.contains(over.as_str()), "web got {over:?}: {got}");
    assert_person_warned_once(solo.machine(), solo.default_channel(), &solo.short(WEB));
}

/// Ada's machine, alone in `clienta`'s groups, runs MCP sessions `--agent api` and `--agent web`.
/// web sends api a message with the send tool; api and web, in turn, take the other's last message
/// with `inbox` and send the other one with the send tool, ten in all, each sent. api takes web's
/// last one; its next send fails, says why, and web's `inbox` never returns it. Ada's person's next
/// run warns once, with `clienta`'s default channel and web, and the run after it does not.
#[test]
fn agents_of_one_device_sending_to_each_other_stop_at_ten_hops() {
    let solo = Solo::new(4_811_010_001, "ada-sends-in-a-loop-on-one-device");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let texts = texts(Way::SendTool);
    solo.sends(&mut web, &solo.short(API), &texts[0]);
    let mut sessions = [(&mut api, WEB), (&mut web, API)];
    for (at, pair) in texts[..=LIMIT as usize].windows(2).enumerate() {
        let (session, to) = &mut sessions[at % 2];
        let shown = session.inbox();
        assert!(
            shown.contains(pair[0].as_str()),
            "{:?} not taken: {shown}",
            pair[0]
        );
        solo.sends(session, &solo.short(to), &pair[1]);
    }
    let last = api.inbox();
    let (end, over) = (&texts[LIMIT as usize], &texts[LIMIT as usize + 1]);
    assert!(last.contains(end.as_str()), "{end:?} not taken: {last}");

    let called = api.call(
        SEND_TOOL,
        json!({ TO_FIELD: solo.short(WEB), TEXT_FIELD: over }),
    );

    let refused = Said {
        ok: !called.error,
        shown: called.text,
    };
    assert_stopped(&refused, "the send at hop 11");
    assert_person_warned_once(solo.machine(), solo.default_channel(), &solo.short(WEB));
    let inbox = web.inbox();
    assert!(!inbox.contains(over.as_str()), "web got {over:?}: {inbox}");
}
