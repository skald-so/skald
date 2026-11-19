//! A one-shot `skald recv --wait` of an agent wakes for a message another agent of its device
//! sends it, which no connection brings.

use std::thread;
use std::time::{Duration, Instant};

use crate::kit::{RECV, received_as_agent};
use crate::lines::assert_says_sent;
use crate::solo::{API, OWN, SEND, Solo, TO, WEB, assert_got};
use crate::talk::{ADA_HOST, one_message_sent};
use crate::world::AGENT;

/// What api sends web while web's wait runs; it is nowhere else.
const ASKED: &str = "local wait test: api asks web, which waits";

/// The flag of `skald recv` that waits.
const WAIT_FLAG: &str = "--wait";

/// The request that opens a WebSocket.
const WEBSOCKET: &str = "GET /ws ";

/// How soon the wait shows the message once api's send ended, whatever the load on the machine:
/// well under the 20 s it waited without waking.
const WOKEN_WITHIN: Duration = Duration::from_secs(10);

/// How often the test looks at the runs.
const TICK: Duration = Duration::from_millis(50);

/// Ada's machine, alone in `clienta`'s groups, has run as web in `~/work/clientA/web`. web's
/// one-shot `skald recv --wait` there connects to the server over WebSocket and waits; api's
/// `skald send --to ada/web` in `~/work/clientA/api` says `sent #<ref>`. Within [`WOKEN_WITHIN`]
/// the wait shows the message from `ada/api` at `own` and exits.
#[test]
fn one_shot_wait_of_an_agent_wakes_for_a_message_of_another_agent_of_its_device() {
    let solo = Solo::new(4_657_012_001, "ada-web-waits-for-api");
    let (api_dir, web_dir) = (solo.dir(API), solo.dir(WEB));
    received_as_agent(solo.machine(), &web_dir, WEB);
    let cloud = solo.talk.cloud();
    let since = cloud.relay.mark();
    let mut waiting = (solo.machine()).start_in(&web_dir, &[AGENT, WEB, RECV, WAIT_FLAG]);
    let deadline = Instant::now() + WOKEN_WITHIN;
    while cloud.count_sent(since, WEBSOCKET) == 0 {
        waiting.assert_running();
        assert!(
            Instant::now() < deadline,
            "no WebSocket: {}",
            waiting.shown()
        );
        thread::sleep(TICK);
    }
    let mark = solo.mark();

    let sent = (solo.machine())
        .start_in(&api_dir, &[AGENT, API, SEND, TO, &solo.short(WEB), ASKED])
        .finish();

    assert!(sent.success, "{sent:?}");
    let asked = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    assert_says_sent(&sent.stdout, asked);
    let deadline = Instant::now() + WOKEN_WITHIN;
    while !waiting.exited() {
        assert!(
            Instant::now() < deadline,
            "still waits: {}",
            waiting.shown()
        );
        thread::sleep(TICK);
    }
    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    assert_got(&got.stdout, ASKED, asked, (&solo.short(API), ADA_HOST, OWN));
}
