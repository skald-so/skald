//! One-shot runs with an agent: a value uploaded once, again only after a change or once the last
//! one has expired.

use std::thread;
use std::time::Duration;

use skald_proto::AgentState;

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::kit::RECV;
use crate::values::{
    EXPIRES_SETTING, acme_channels, ada_s_body, ada_s_value, addresses, listed, opened, ran_as,
    run_with, uploads,
};
use crate::world::{AGENT, now};

/// The shortened span from an upload to its value's `expires`, in seconds.
const EXPIRES: u64 = 5;

/// Claude runs `skald recv` one-shot in `~/work/acme`: one value goes up in each channel of
/// `acme`, listing claude `offline`, as no session of it runs. Claude's next run uploads nothing,
/// and the server holds the same values. Codex's run, registering codex, uploads one more in each
/// channel, listing both.
#[test]
fn one_shot_run_with_an_agent_uploads_once_and_again_only_after_a_change() {
    let agents = Agents::new(
        [4_731_041_001, 4_731_041_002],
        ["ada-runs-claude-twice", "bob-sees-one-value"],
    );
    let acme = acme_channels(&agents);
    let (claude, codex) = (agents.address(CLAUDE), agents.address(CODEX));
    let since = agents.talk.cloud().relay.mark();

    ran_as(&agents, CLAUDE, &[RECV]);

    let mut first = Vec::new();
    for at in acme {
        assert_eq!(
            uploads(&agents.talk, at, since),
            1,
            "claude's run in {}",
            at.1
        );
        let body = ada_s_body(&agents.talk, at).expect("a value of Ada's");
        assert_eq!(
            listed(&body, &claude).state,
            AgentState::Offline,
            "{body:?}"
        );
        assert_eq!(addresses(&body), std::slice::from_ref(&claude), "{body:?}");
        first.push(ada_s_value(&agents.talk, at).expect("a value of Ada's"));
    }
    let since = agents.talk.cloud().relay.mark();

    ran_as(&agents, CLAUDE, &[RECV]);

    for (at, first) in acme.iter().zip(&first) {
        assert_eq!(
            uploads(&agents.talk, *at, since),
            0,
            "claude's next run in {}",
            at.1
        );
        let held = ada_s_value(&agents.talk, *at).expect("a value of Ada's");
        assert_eq!(&held, first, "another value in {}", at.1);
    }
    let since = agents.talk.cloud().relay.mark();

    ran_as(&agents, CODEX, &[RECV]);

    for at in acme {
        assert_eq!(
            uploads(&agents.talk, at, since),
            1,
            "codex's run in {}",
            at.1
        );
        let body = ada_s_body(&agents.talk, at).expect("a value of Ada's");
        let mut names = addresses(&body);
        names.sort();
        let mut expected = vec![claude.clone(), codex.clone()];
        expected.sort();
        assert_eq!(names, expected, "{body:?}");
    }
}

/// Claude runs `skald recv` one-shot in `~/work/acme` with `expires` 5 seconds after each upload.
/// Once Ada's values have expired, claude's next run, though nothing changed, uploads one value in
/// each channel of `acme`, one revision on, that has not expired.
#[test]
fn one_shot_run_uploads_again_once_its_last_value_has_expired() {
    let agents = Agents::new(
        [4_731_042_001, 4_731_042_002],
        ["ada-lets-her-value-expire", "bob-sees-it-renewed"],
    );
    let acme = acme_channels(&agents);
    let expires = EXPIRES.to_string();
    let settings = [(EXPIRES_SETTING, expires.as_str())];
    let as_claude = [AGENT, CLAUDE, RECV];
    let machine = &agents.talk.ada.machine;
    let ran = run_with(machine, &agents.folder, &settings, &as_claude);
    assert!(ran.success, "{ran:?}");
    let mut last = 0;
    let mut revisions = Vec::new();
    for at in acme {
        let body = ada_s_body(&agents.talk, at).expect("a value of Ada's");
        assert!(body.expires <= now() + EXPIRES + 1, "{body:?}");
        last = last.max(body.expires);
        revisions.push(body.revision);
    }
    while now() <= last + 1 {
        thread::sleep(Duration::from_millis(200));
    }
    for at in acme {
        let value = ada_s_value(&agents.talk, at).expect("a value of Ada's");
        let expired = opened(&agents.talk, &value, now());
        assert!(expired.is_err(), "not expired: {expired:?}");
    }
    let since = agents.talk.cloud().relay.mark();

    let ran = run_with(machine, &agents.folder, &settings, &as_claude);

    assert!(ran.success, "{ran:?}");
    for (at, revision) in acme.iter().zip(revisions) {
        assert_eq!(
            uploads(&agents.talk, *at, since),
            1,
            "claude's next run in {}",
            at.1
        );
        let body = ada_s_body(&agents.talk, *at).expect("a value of Ada's, not expired");
        assert_eq!(body.revision, revision + 1, "{body:?}");
    }
}
