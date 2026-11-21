//! Hooks never wait on the network: with the helper and the server down, each returns at once and
//! prints nothing.

use std::time::{Duration, Instant};

use crate::codex::{Codex, Event};
use crate::fixtures::TIMEOUT_SETTING;
use crate::helped::ONE_SHOT_SETTING;
use crate::solo::WEB;

/// A request timeout, in seconds, far longer than [`AT_ONCE`]: a hook that waited on the network
/// would wait as long.
const LONG_TIMEOUT: &str = "60";

/// The setting of how long a hook waits for its run, in seconds, here as long as a request may
/// take, so a hook that waited on the network is not cut short.
const PATIENCE_SETTING: (&str, &str) = ("SKALD_HOOK_PATIENCE", LONG_TIMEOUT);

/// How long a hook may take to return at once, however loaded the machine.
const AT_ONCE: Duration = Duration::from_secs(10);

/// web's session starts, which starts the helper; the helper is killed, and the server stops
/// answering: each request to it waits. Each hook of web's session, `SessionStart` too, run with a
/// request timeout of [`LONG_TIMEOUT`] seconds and as long a wait for its run, returns within
/// [`AT_ONCE`], exits with success and prints nothing, run as Codex runs it and in one-shot mode.
#[test]
fn hooks_return_at_once_and_print_nothing_with_the_helper_and_the_server_down() {
    let codex = Codex::new(4_927_013_001, "ada-codex-hooks-offline");
    let web = codex.thread_of(WEB);
    codex.starts(&web);
    let helper = codex.helped.one_helper();
    codex.helped.kill(helper);
    (codex.web.solo.talk.world.cloud.relay).hold("", usize::MAX);

    let waits = [(TIMEOUT_SETTING, LONG_TIMEOUT), PATIENCE_SETTING];
    let one_shot = [waits[0], waits[1], (ONE_SHOT_SETTING, "1")];
    for settings in [&waits[..], &one_shot[..]] {
        for event in Event::ALL {
            let started = Instant::now();
            let hooked = codex.hook_with(&web, event, settings);
            let took = started.elapsed();
            hooked.assert_silent(event.name());
            assert!(
                took <= AT_ONCE,
                "{} took {took:?}: {settings:?}",
                event.name()
            );
        }
    }
}
