//! Which session gets a message's signal: one of the agent the message is for; of that agent's
//! live sessions, the one whose hook ran last when the message came; never one whose Codex process
//! ended. A session keeps the helper running while its Codex process runs, and the helper exits
//! idle once none does.

use std::thread;
use std::time::Duration;

use crate::codex::{Codex, CodexProcess, Event, assert_bare, hook_command};
use crate::helped::{EXIT_LIMIT, IDLE_SETTING};
use crate::solo::{API, WEB};

/// What web says to api, and api to web; neither is anywhere else.
const TO_API: &str = "codex routing test: web asks api for the schema";
const TO_WEB: &str = "codex routing test: api asks web for the page";

/// Sessions of api, `--agent api`, and of web, `--agent web`, start in their folders, each with its
/// MCP session, and their turns start. web sends api a message: web's `PostToolUse` prints nothing,
/// and api's gives the signal. api sends web a message: api's `PostToolUse` prints nothing, and
/// web's gives the signal.
#[test]
fn only_a_session_of_the_agent_a_message_is_for_gets_its_signal() {
    let mut codex = Codex::new(4_927_009_001, "ada-codex-two-agents");
    let solo = &codex.web.solo;
    let _api_mcp = solo.session(API);
    let (api, web) = (codex.thread_of(API), codex.thread_of(WEB));
    codex.starts(&api);
    codex.starts(&web);
    codex.busy(&api);
    codex.busy(&web);

    let to_api = solo.sends(&mut codex.web.session, &solo.short(API), TO_API);
    codex.silent(&web, Event::PostToolUse, "web, for api's message");
    let for_api = codex.gives(&api, Event::PostToolUse);
    let to_web = codex.web.own(TO_WEB);
    codex.silent(&api, Event::PostToolUse, "api, for web's message");
    let for_web = codex.gives(&web, Event::PostToolUse);

    assert_bare(&for_api, &[TO_API], &[to_api], &[]);
    assert_bare(&for_web, &[TO_WEB], &[to_web], &[]);
}

/// What api says to web while both of web's sessions live; it is nowhere else.
const TO_LATEST: &str = "codex routing test: api asks the web session that hooked last";

/// Two sessions of web start, `older` first; then `older`'s turn starts and its `PostToolUse` runs
/// and prints nothing, so its hook ran last. api sends web a message: `older`'s `PostToolUse` gives
/// the signal, and `newer`'s `PostToolUse` then prints nothing.
#[test]
fn of_two_live_sessions_of_an_agent_the_one_whose_hook_ran_last_gets_the_signal() {
    let codex = Codex::new(4_927_010_001, "ada-codex-two-sessions");
    let (older, newer) = (codex.thread_of(WEB), codex.thread_of(WEB));
    codex.starts(&older);
    codex.starts(&newer);
    codex.busy(&older);
    codex.silent(&older, Event::PostToolUse, "older, with nothing waiting");

    let msg_id = codex.web.own(TO_LATEST);

    let signal = codex.gives(&older, Event::PostToolUse);
    assert_bare(&signal, &[TO_LATEST], &[msg_id], &[]);
    codex.silent(
        &newer,
        Event::PostToolUse,
        "newer, after older got the signal",
    );
}

/// What api says to web once the Codex process of its session that hooked last ended; it is
/// nowhere else.
const AFTER_EXIT: &str = "codex routing test: api asks web after one Codex exited";

/// Two sessions of web start: `staying`, run by Codex in the test's process, whose turn starts,
/// then `leaving`, run by a Codex process of its own, whose `PostToolUse` then runs and prints
/// nothing, so its hook ran last. That Codex process exits. api sends web a message: `staying`'s
/// `PostToolUse` gives the signal.
#[test]
fn a_session_whose_codex_process_ended_gets_no_signal() {
    let codex = Codex::new(4_927_011_001, "ada-codex-process-ended");
    let (staying, leaving) = (codex.thread_of(WEB), codex.thread_of(WEB));
    let mut process = CodexProcess::start();
    codex.starts(&staying);
    codex.busy(&staying);
    (codex.hook_by(&mut process, &leaving, Event::SessionStart))
        .assert_silent("leaving's SessionStart");
    (codex.hook_by(&mut process, &leaving, Event::PostToolUse))
        .assert_silent("leaving's PostToolUse");
    process.end();

    let msg_id = codex.web.own(AFTER_EXIT);

    let signal = codex.gives(&staying, Event::PostToolUse);
    assert_bare(&signal, &[AFTER_EXIT], &[msg_id], &[]);
}

/// The helper's idle time in this test, in seconds.
const SHORT_IDLE: &str = "2";

/// How long the test watches the helper of a live session: several of its idle times.
const LIVE_FOR: Duration = Duration::from_secs(8);

/// No helper runs on Ada's machine. web's session starts in a Codex process of its own, its
/// `SessionStart` run with a helper idle time of [`SHORT_IDLE`] seconds, and runs `PostToolUse`:
/// one helper runs, and it still runs after [`LIVE_FOR`] with no command. The Codex process exits:
/// the helper exits too.
#[test]
fn helper_runs_while_a_session_lives_and_exits_once_none_does() {
    let codex = Codex::new(4_927_012_001, "ada-codex-helper-lifetime");
    let web = codex.thread_of(WEB);
    let mut process = CodexProcess::start();
    codex.helped.assert_none();

    let command = hook_command(&codex.helped, &web, &[(IDLE_SETTING, SHORT_IDLE)]);
    (process.run(&command, &web.input(Event::SessionStart))).assert_silent("SessionStart");
    (codex.hook_by(&mut process, &web, Event::PostToolUse)).assert_silent("PostToolUse");
    let helper = codex.helped.one_helper();
    thread::sleep(LIVE_FOR);
    assert!(
        codex.helped.helpers().contains(&helper),
        "the helper exited while the session lived"
    );

    process.end();
    codex.helped.await_exit(helper, EXIT_LIMIT);
}
