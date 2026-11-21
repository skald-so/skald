//! `SessionStart` registers the session with the device, as the agent the session's `skald mcp`
//! registers, and starts the helper when none runs; in a folder in no space nothing registers and
//! the hooks print nothing.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::MessageId;

use crate::client::session_in;
use crate::cloud::WAIT;
use crate::codex::{Codex, Event, QUIET, Thread, assert_bare, codex_client};
use crate::hooks::api_sends;
use crate::lines::{assert_handed_line, reference};
use crate::solo::WEB;
use crate::talk::ADA_HOST;
use crate::world::dir_of;

/// The length of a device's reference in a default name.
const DEVICE_REFERENCE: usize = 6;

/// What api says to web; it is nowhere else.
const TO_WEB: &str = "codex registration test: api asks web for the build log";

/// Waits for up to [`WAIT`] until exactly one helper runs on `codex`'s machine; gives it.
#[track_caller]
fn one_helper_starts(codex: &Codex) -> u32 {
    let deadline = Instant::now() + WAIT;
    loop {
        if let [one] = codex.helped.helpers().as_slice() {
            return *one;
        }
        assert!(Instant::now() < deadline, "no helper started");
        thread::sleep(Duration::from_millis(100));
    }
}

/// No helper runs on Ada's machine. web's session starts: its `SessionStart` prints nothing, and
/// one helper runs. A second session of web starts: still that one helper runs. Its turn starts,
/// and api sends web a message: its `PostToolUse` gives the signal, holding nothing of it.
#[test]
fn session_start_registers_the_session_and_starts_the_helper_when_none_runs() {
    let codex = Codex::new(4_927_003_001, "ada-codex-session-start");
    let (first, second) = (codex.thread_of(WEB), codex.thread_of(WEB));
    codex.helped.assert_none();

    codex.starts(&first);
    let helper = one_helper_starts(&codex);
    codex.starts(&second);
    assert_eq!(codex.helped.helpers(), vec![helper], "not one helper");
    codex.busy(&second);
    let msg_id = codex.web.own(TO_WEB);

    let signal = codex.gives(&second, Event::PostToolUse);
    assert_bare(&signal, &[TO_WEB], &[msg_id], &[]);
}

/// What api says to the unnamed session's agent; it is nowhere else.
const TO_DEFAULT: &str = "codex registration test: api asks the unnamed session for its plan";

/// In `~/work/clientA/session`, `skald mcp` runs without `--agent`, initialized with the client
/// name Codex gives, `codex-mcp-client`; its agent is `codex-mcp-client-ada-workstation-<reference>`.
/// A Codex session there starts without `--agent`, and so does web's, `--agent web`, in its
/// folder; both turns start. api sends the unnamed session's agent a message: web's `PostToolUse`
/// prints nothing, and the unnamed session's `PostToolUse` gives the signal.
#[test]
fn hooks_without_agent_act_as_the_agent_skald_mcp_names_by_default() {
    let codex = Codex::new(4_927_004_001, "ada-codex-unnamed-session");
    let solo = &codex.web.solo;
    let dir = solo.dir("session");
    let client = codex_client();
    let _session = session_in(codex.machine(), &dir, &[], &client);
    let device = reference(MessageId::from_bytes(*solo.talk.ada.device.as_bytes()));
    let name = format!("{client}-{ADA_HOST}-{}", &device[..DEVICE_REFERENCE]);
    let unnamed = Thread::new(codex.machine(), &dir, None);
    let web = codex.thread_of(WEB);
    codex.starts(&unnamed);
    codex.starts(&web);
    codex.busy(&web);
    codex.busy(&unnamed);

    let msg_id = api_sends(solo, &solo.short(&name), TO_DEFAULT);

    codex.silent(&web, Event::PostToolUse, "web's session");
    let signal = codex.gives(&unnamed, Event::PostToolUse);
    assert_bare(&signal, &[TO_DEFAULT], &[msg_id], &[]);
}

/// What api says to web; it is nowhere else.
const OUTSIDE: &str = "codex registration test: api asks web, not the folder outside spaces";

/// In `~/work/elsewhere/web`, a folder in no space, sessions start with `--agent web` and without
/// `--agent`: each `SessionStart` prints nothing, and no helper runs after [`QUIET`]. web's session
/// in `~/work/clientA/web` starts, and its turn too. api sends web a message: each hook of both
/// sessions outside spaces prints nothing; then web's `PostToolUse` gives the signal, and web's
/// `inbox` shows the message.
#[test]
fn nothing_registers_in_a_folder_in_no_space_and_its_hooks_print_nothing() {
    let mut codex = Codex::new(4_927_005_001, "ada-codex-outside-spaces");
    let elsewhere = dir_of(codex.machine(), "work/elsewhere/web");
    let named = Thread::new(codex.machine(), &elsewhere, Some(WEB));
    let unnamed = Thread::new(codex.machine(), &elsewhere, None);

    codex.starts(&named);
    codex.starts(&unnamed);
    thread::sleep(QUIET);
    codex.helped.assert_none();
    let web = codex.thread_of(WEB);
    codex.starts(&web);
    codex.busy(&web);
    let msg_id = codex.web.own(OUTSIDE);
    for session in [&named, &unnamed] {
        for event in Event::ALL {
            codex.silent(session, event, "a session in no space");
        }
    }

    let signal = codex.gives(&web, Event::PostToolUse);
    assert_bare(&signal, &[OUTSIDE], &[msg_id], &[]);
    assert_handed_line(&codex.web.session.inbox(), OUTSIDE, msg_id);
}
