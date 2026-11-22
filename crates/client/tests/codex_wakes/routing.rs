//! Which session's thread gets `codex queue`: only one of the agent the message is for, each with
//! the `codex` of the process that runs it, such as an app's bundled one, and its own config home;
//! never one whose Codex process ended.

use crate::codex::Codex;
use crate::solo::{API, WEB};
use crate::standin::{StandIn, assert_queued, assert_quiet, session_of};

/// What web says to api, and api to web; neither is anywhere else.
const TO_API: &str = "codex wake test: web asks api to regenerate the client";
const TO_WEB: &str = "codex wake test: api asks web to refresh the screenshots";

/// api's session starts in a desktop app whose bundled `codex` is its own, with its config home;
/// web's in an IDE extension with another, with another config home; api's MCP session runs. Both
/// sessions are idle. web sends api a message: the desktop app's `codex` runs `codex queue` once,
/// for api's thread with api's config home; the extension's does not run. api sends web a message:
/// the extension's `codex` runs `codex queue` once, for web's thread with web's config home; the
/// desktop app's does not run again. The `codex` on the search path never runs.
pub fn only_a_session_of_the_agent_gets_the_queue_with_its_own_codex_and_config_home() {
    let mut codex = Codex::new(4_928_004_001, "ada-codex-wake-two-agents");
    let _api_mcp = codex.web.solo.session(API);
    let on_path = StandIn::on_path(&codex);
    let (desktop, extension) = (
        StandIn::app(&codex, "desktop"),
        StandIn::app(&codex, "extension"),
    );
    let (api, web) = (
        session_of(&codex, API, "desktop"),
        session_of(&codex, WEB, "extension"),
    );
    let mut in_desktop = desktop.start();
    let mut in_extension = extension.start();
    in_desktop.starts(&codex, &api);
    in_extension.starts(&codex, &web);

    let solo = &codex.web.solo;
    let to = solo.short(API);
    let to_api = solo.sends(&mut codex.web.session, &to, TO_API);
    let ran = desktop.await_runs(1);
    assert_queued(&ran[0], (&api, &desktop), &[TO_API], &[to_api], &[]);
    assert_quiet(&[(&extension, 0, "web's codex, for api's message")]);

    let to_web = codex.web.own(TO_WEB);
    let ran = extension.await_runs(1);
    assert_queued(&ran[0], (&web, &extension), &[TO_WEB], &[to_web], &[]);
    assert_quiet(&[
        (&desktop, 1, "api's codex, for web's message"),
        (&on_path, 0, "the codex on the search path"),
    ]);
}

/// What api says to web once one Codex process ended, and once both did; neither is anywhere else.
const ONE_ENDED: &str = "codex wake test: api asks web after one Codex app exited";
const BOTH_ENDED: &str = "codex wake test: api asks web after both Codex apps exited";

/// Two sessions of web start, each in a Codex app of its own with its config home: `staying`, then
/// `leaving`, whose hook so ran last. `leaving`'s app exits. api sends web a message: `staying`'s
/// app's `codex` runs `codex queue` once, for `staying`'s thread; `leaving`'s does not run.
/// `staying`'s app exits too. api sends web another message: neither app's `codex` runs again.
pub fn a_session_whose_codex_process_ended_gets_no_codex_queue() {
    let codex = Codex::new(4_928_005_001, "ada-codex-wake-ended");
    let (stays, leaves) = (
        StandIn::app(&codex, "stays"),
        StandIn::app(&codex, "leaves"),
    );
    let (staying, leaving) = (
        session_of(&codex, WEB, "stays"),
        session_of(&codex, WEB, "leaves"),
    );
    let mut staying_app = stays.start();
    let mut leaving_app = leaves.start();
    staying_app.starts(&codex, &staying);
    leaving_app.starts(&codex, &leaving);
    leaving_app.end();

    let one_ended = codex.web.own(ONE_ENDED);
    let ran = stays.await_runs(1);
    assert_queued(&ran[0], (&staying, &stays), &[ONE_ENDED], &[one_ended], &[]);
    assert_quiet(&[(&leaves, 0, "the codex of the app that exited")]);

    staying_app.end();
    codex.web.own(BOTH_ENDED);
    assert_quiet(&[
        (&stays, 1, "the codex of the app that exited last"),
        (&leaves, 0, "the codex of the app that exited first"),
    ]);
}
