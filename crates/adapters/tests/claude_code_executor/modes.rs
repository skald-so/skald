//! The executor's modes, and what a stand-in `claude` sees of a run: `read-only` runs only on the
//! Claude Code version the live checks confirmed or a later one, and a session runs in its folder
//! without the device's other variables.

use skald_adapters::claude_code::Executor;
use skald_adapters::executor::Mode;

use crate::fixtures::{
    CHECKED, MARKERS, Scratch, StandIn, device_environment, run, set_in_this_process, token,
};

const PROMPT: &str = "Say done.";

/// A run in `mode` with a stand-in whose version run prints `version` and exits with `code`: the
/// stand-in, and the reason if the executor refused.
fn run_with(
    scratch: &Scratch,
    version: Option<&str>,
    code: u8,
    mode: Mode,
) -> (StandIn, Option<String>) {
    let stand_in = StandIn::new(scratch, version, code);
    let environment = device_environment(
        &scratch.dir("home"),
        &scratch.dir("config"),
        "http://127.0.0.1:9",
        token,
    );
    let executor = Executor::new(stand_in.path.clone()).environment(environment);
    let refused = run(executor, &scratch.dir("folder"), mode, PROMPT).err();
    (stand_in, refused)
}

#[test]
fn the_modes_are_read_only_and_full() {
    assert!(matches!("read-only".parse::<Mode>(), Ok(Mode::ReadOnly)));
    assert!(matches!("full".parse::<Mode>(), Ok(Mode::Full)));
}

/// Any other mode is refused, and the refusal names it.
#[test]
fn any_other_mode_is_refused() {
    for mode in [
        "approve",
        "off",
        "read_only",
        "readonly",
        "Read-Only",
        "FULL",
        "write",
        "auto",
    ] {
        match mode.parse::<Mode>() {
            Ok(_) => panic!("the mode {mode:?} is taken"),
            Err(refused) => assert!(
                refused.to_string().contains(mode),
                "the refusal of {mode:?} does not name it: {refused}"
            ),
        }
    }
    assert!("".parse::<Mode>().is_err(), "an empty mode is taken");
}

/// `read-only` on a Claude Code older than the one the live checks ran on is refused before any
/// session starts, saying which version it found.
#[test]
fn read_only_is_refused_on_a_claude_code_older_than_the_checked_one() {
    for version in ["2.1.327", "2.1.32", "2.0.999", "1.9.400"] {
        let scratch = Scratch::new();
        let line = format!("{version} (Claude Code)");
        let (stand_in, refused) = run_with(&scratch, Some(&line), 0, Mode::ReadOnly);

        let reason = refused.unwrap_or_else(|| panic!("read-only runs on Claude Code {version}"));
        assert!(
            reason.contains(version),
            "the refusal on {version} does not say the version: {reason}"
        );
        assert!(
            !stand_in.started(),
            "a session started on Claude Code {version}"
        );
    }
}

/// `read-only` on a Claude Code whose version is unknown is refused before any session starts.
#[test]
fn read_only_is_refused_on_a_claude_code_of_unknown_version() {
    let checked = format!("{CHECKED} (Claude Code)");
    for (version, code) in [
        (Some("Claude Code"), 0),
        (Some("version unknown"), 0),
        (None, 0),
        (None, 1),
        (Some(checked.as_str()), 1),
    ] {
        let scratch = Scratch::new();
        let (stand_in, refused) = run_with(&scratch, version, code, Mode::ReadOnly);

        assert!(
            refused.is_some(),
            "read-only runs on a version run printing {version:?} with exit code {code}"
        );
        assert!(
            !stand_in.started(),
            "a session started on a version run printing {version:?} with exit code {code}"
        );
    }
}

/// `read-only` starts a session on the checked Claude Code and on later ones, versions compared by
/// their numbers.
#[test]
fn read_only_runs_on_the_checked_claude_code_and_later_ones() {
    for version in [CHECKED, "2.1.329", "2.1.1000", "2.10.0", "3.0.0"] {
        let scratch = Scratch::new();
        let line = format!("{version} (Claude Code)");
        let (stand_in, refused) = run_with(&scratch, Some(&line), 0, Mode::ReadOnly);

        assert!(
            stand_in.started(),
            "no session started on Claude Code {version}: {refused:?}"
        );
    }
}

/// A session runs in the given folder and gets none of the device's variables that Claude Code
/// needs neither to run nor to sign in.
#[test]
fn a_session_runs_in_its_folder_without_the_device_s_other_variables() {
    set_in_this_process(&MARKERS);
    for mode in [Mode::Full, Mode::ReadOnly] {
        let scratch = Scratch::new();
        let line = format!("{CHECKED} (Claude Code)");
        let (stand_in, refused) = run_with(&scratch, Some(&line), 0, mode);

        assert!(stand_in.started(), "no session started: {refused:?}");
        assert_eq!(stand_in.directory(), scratch.path().join("folder"));
        for name in MARKERS {
            assert!(!stand_in.got(name), "the session got {name}");
        }
    }
}
