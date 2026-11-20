//! Values refreshed before they expire, with `expires` shortened; the helper kept while an idle MCP
//! session runs, and its agents uploaded `offline` before it exits.

use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::AgentState;

use crate::agents::{Agents, CLAUDE};
use crate::helped::{EXIT_LIMIT, Helped, IDLE_SETTING};
use crate::values::{
    EXPIRES_SETTING, ada_s_value, assert_between, await_available, body_of, claude_session_with,
    helpers_beside, listed, one_helper_beside, shows,
};
use crate::world::now;

/// The shortened span from an upload to its value's `expires`, in seconds, and how long the test
/// watches the values.
const EXPIRES: u64 = 8;
const WATCHED: Duration = Duration::from_secs(3 * EXPIRES + 2);

/// The helper's shortened idle time, in seconds, and how long the session stays idle: well past it.
const IDLE: u64 = 2;
const IDLE_FOR: Duration = Duration::from_secs(4 * IDLE);

/// A value as the test first saw it: its revision, `expires`, when the test saw it, and claude's
/// `since`.
#[derive(Debug, Clone, Copy)]
struct Seen {
    revision: u64,
    expires: u64,
    at: u64,
    since: u64,
}

/// Through a helper started with `expires` 8 seconds after each upload, claude runs an MCP session
/// of `claude-code` in `~/work/acme`, and nothing changes. For 26 seconds the test reads Ada's
/// value in each channel of `acme`: none it reads has expired; each states `expires` at most 8
/// seconds, and a second of rounding, after the test first saw it; each next revision comes before the previous one expires,
/// at least twice in each channel; claude stays `available`, its `since` unchanged.
#[test]
fn values_are_refreshed_before_they_expire() {
    let mut agents = Agents::new(
        [4_731_021_001, 4_731_021_002],
        ["ada-keeps-her-presence-fresh", "bob-watches-it-refresh"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let _session = claude_session_with(&helped, &agents, (EXPIRES_SETTING, EXPIRES));
    let (claude, acme) = await_available(&mut agents, CLAUDE);
    let mut seen: Vec<BTreeMap<u64, Seen>> = vec![BTreeMap::new(); acme.len()];

    let until = Instant::now() + WATCHED;
    while Instant::now() < until {
        for (at, seen) in acme.iter().zip(&mut seen) {
            let value = ada_s_value(&agents.talk, *at).expect("a value of Ada's");
            let body = body_of(&agents.talk, &value);
            let at_time = now();
            let status = listed(&body, &claude);
            assert_eq!(status.state, AgentState::Available, "{body:?}");
            seen.entry(body.revision).or_insert(Seen {
                revision: body.revision,
                expires: body.expires,
                at: at_time,
                since: status.since,
            });
        }
        thread::sleep(Duration::from_millis(250));
    }

    for (at, seen) in acme.iter().zip(&seen) {
        let seen: Vec<&Seen> = seen.values().collect();
        assert!(seen.len() >= 3, "not refreshed twice in {}: {seen:?}", at.1);
        for one in &seen {
            // A second for a value that states its expiry rounded up.
            assert!(one.expires <= one.at + EXPIRES + 1, "{one:?}");
        }
        for pair in seen.windows(2) {
            assert!(
                pair[1].at <= pair[0].expires,
                "refreshed after it expired: {pair:?}"
            );
            assert_eq!(pair[1].since, pair[0].since, "since moved: {pair:?}");
        }
    }
}

/// Through a helper started with an idle time of 2 seconds, claude runs an MCP session of
/// `claude-code` in `~/work/acme` that calls nothing for 8 seconds: the helper still runs, and
/// Ada's values still list claude `available`. The session ends: the helper exits, and by then
/// Ada's value in each channel of `acme` lists claude `offline`, since the session ended.
#[test]
fn helper_stays_while_an_idle_session_runs_and_leaves_its_agents_offline() {
    let mut agents = Agents::new(
        [4_731_022_001, 4_731_022_002],
        ["ada-leaves-claude-idle", "bob-sees-it-stay"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let session = claude_session_with(&helped, &agents, (IDLE_SETTING, IDLE));
    let (claude, acme) = await_available(&mut agents, CLAUDE);
    let helper = one_helper_beside(&helped, &[&session]);

    thread::sleep(IDLE_FOR);

    let beside = helpers_beside(&helped, &[&session]);
    assert_eq!(beside, [helper], "the helper exited under an idle session");
    for at in acme {
        let value = ada_s_value(&agents.talk, at).expect("a value of Ada's");
        let body = body_of(&agents.talk, &value);
        assert!(shows(&body, &claude, AgentState::Available), "{body:?}");
    }
    let ended = now();

    drop(session);

    helped.await_exit(helper, EXIT_LIMIT);
    let exited = now();
    for at in acme {
        let value = ada_s_value(&agents.talk, at).expect("a value of Ada's");
        let body = body_of(&agents.talk, &value);
        let status = listed(&body, &claude);
        assert_eq!(status.state, AgentState::Offline, "{body:?}");
        assert_between(status.since, (ended, exited), "offline");
    }
}
