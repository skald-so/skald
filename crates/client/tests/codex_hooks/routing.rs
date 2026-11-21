//! Which session gets a message's signal: one of the agent the message is for; of that agent's
//! live sessions, the one whose hook ran last when the message came, whatever hooks run after it;
//! never one whose Codex process ended. A session keeps the helper running while its Codex process
//! runs, also through the npm package's launcher, and the helper exits idle once none does.

use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::codex::{Codex, CodexProcess, Event, QUIET, assert_bare, hook_command};
use crate::helped::{EXIT_LIMIT, IDLE_SETTING};
use crate::kit::RECV;
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
/// and prints nothing, so its hook ran last. api sends web a message, and the test waits [`QUIET`]
/// while the helper takes it in. A third session of web, `newest`, starts; then `newest`'s and
/// `newer`'s `PostToolUse` print nothing, `older`'s gives the signal, and `newer`'s and `newest`'s
/// then print nothing.
#[test]
fn of_two_live_sessions_of_an_agent_the_one_whose_hook_ran_last_gets_the_signal() {
    let codex = Codex::new(4_927_010_001, "ada-codex-two-sessions");
    let (older, newer, newest) = (
        codex.thread_of(WEB),
        codex.thread_of(WEB),
        codex.thread_of(WEB),
    );
    codex.starts(&older);
    codex.starts(&newer);
    codex.busy(&older);
    codex.silent(&older, Event::PostToolUse, "older, with nothing waiting");

    let msg_id = codex.web.own(TO_LATEST);
    thread::sleep(QUIET);
    codex.starts(&newest);
    codex.silent(
        &newest,
        Event::PostToolUse,
        "newest, which hooked after it came",
    );
    codex.silent(
        &newer,
        Event::PostToolUse,
        "newer, which hooked after it came",
    );

    let signal = codex.gives(&older, Event::PostToolUse);
    assert_bare(&signal, &[TO_LATEST], &[msg_id], &[]);
    codex.silent(
        &newer,
        Event::PostToolUse,
        "newer, after older got the signal",
    );
    codex.silent(
        &newest,
        Event::PostToolUse,
        "newest, after older got the signal",
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
    assert_helper_lives_with_the_session(&codex, false, |hook| hook);
}

/// The idle time, in seconds, of the helper that runs before a session starts: room for the
/// session's hooks to run before it, still less than [`LIVE_FOR`].
const FIRST_IDLE: &str = "4";

/// A helper with an idle time of [`FIRST_IDLE`] seconds already runs on Ada's machine, started by
/// the person's `skald recv`. web's session starts in a Codex process of its own and runs
/// `PostToolUse`: that helper, and no other, still runs after [`LIVE_FOR`]; it exits once the
/// Codex process does.
#[test]
fn a_session_that_starts_while_a_helper_runs_keeps_it_running() {
    let codex = Codex::new(4_927_012_003, "ada-codex-helper-first");
    assert_helper_lives_with_the_session(&codex, true, |hook| hook);
}

/// As [`helper_runs_while_a_session_lives_and_exits_once_none_does`], with each hook run as Codex
/// on Windows may run it: through `cmd /c`, `powershell -Command &` and `pwsh -Command &`, each
/// with a machine of its own; a shell not installed is skipped.
#[cfg(windows)]
#[test]
fn a_session_through_a_shell_lives_while_its_codex_process_runs() {
    let shells: [(&str, &[&str]); 3] = [
        ("cmd", &["/c"]),
        ("powershell", &["-NoProfile", "-Command"]),
        ("pwsh", &["-NoProfile", "-Command"]),
    ];
    for (at, (shell, args)) in shells.into_iter().enumerate() {
        if Command::new(shell)
            .args(args)
            .arg("exit 0")
            .output()
            .is_err()
        {
            continue;
        }
        let id = 4_927_012_010 + u64::try_from(at).expect("a small number");
        let codex = Codex::new(id, &format!("ada-codex-through-{shell}"));
        assert_helper_lives_with_the_session(&codex, false, |hook| {
            let mut command = Command::new(shell);
            command.args(args);
            if shell == "cmd" {
                command.arg(hook.get_program()).args(hook.get_args());
            } else {
                let quoted = |text: &std::ffi::OsStr| {
                    format!("'{}'", text.to_string_lossy().replace('\'', "''"))
                };
                let words: Vec<String> = std::iter::once(hook.get_program())
                    .chain(hook.get_args())
                    .map(quoted)
                    .collect();
                command.arg(format!("& {}", words.join(" ")));
            }
            // Codex passes its hooks the system's settings, which the stand-in Codex does not:
            // without them PowerShell returns before `skald` reads the hook's input.
            for (name, value) in std::env::vars_os() {
                if !name
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("SKALD")
                {
                    command.env(name, value);
                }
            }
            settings_of(&hook, &mut command);
            // The shell's search path and temporary directory are the system's, not the machine's.
            for name in [
                "SystemRoot",
                "windir",
                "PATH",
                "TEMP",
                "TMP",
                "PSModulePath",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
            command
        });
    }
}

/// The npm package's launcher's setting of its own process and its parent's.
#[cfg(unix)]
const LAUNCHER: &str = "SKALD_LAUNCHER";

/// As [`helper_runs_while_a_session_lives_and_exits_once_none_does`], with each hook run through a
/// launcher as the npm package's: a process between Codex and `skald` that exits with it and gives
/// it [`LAUNCHER`].
#[cfg(unix)]
#[test]
fn a_session_through_the_npm_launcher_lives_while_its_codex_process_runs() {
    let codex = Codex::new(4_927_012_002, "ada-codex-npm-launcher");
    assert_helper_lives_with_the_session(&codex, false, |hook| {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!("{LAUNCHER}=\"$$ $PPID\" \"$0\" \"$@\"; exit $?"))
            .arg(hook.get_program())
            .args(hook.get_args());
        settings_of(&hook, &mut command);
        command
    });
}

/// Gives `command` the settings and the folder of `hook`.
fn settings_of(hook: &Command, command: &mut Command) {
    for (name, value) in hook.get_envs() {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
    command.current_dir(hook.get_current_dir().expect("the session's folder"));
}

/// With no helper on Ada's machine, or, if `helper_first`, one with an idle time of
/// [`FIRST_IDLE`] seconds that the person's `skald recv` started, web's session starts in a Codex
/// process of its own, its `SessionStart` and then its `PostToolUse` run as `through` makes each of
/// the hook's command, with a helper idle time of [`SHORT_IDLE`] seconds; fails unless one helper
/// then runs, the one that ran before if any, still runs after [`LIVE_FOR`] with no command, and
/// exits once the Codex process does.
#[track_caller]
fn assert_helper_lives_with_the_session(
    codex: &Codex,
    helper_first: bool,
    through: impl Fn(Command) -> Command,
) {
    let web = codex.thread_of(WEB);
    let mut process = CodexProcess::start();
    codex.helped.assert_none();
    let first = helper_first.then(|| {
        let idle = [(IDLE_SETTING, FIRST_IDLE)];
        let received =
            (codex.helped.command_in(&web.dir, &idle, &[RECV]).output()).expect("run skald recv");
        assert!(received.status.success(), "{received:?}");
        codex.helped.one_helper()
    });

    for event in [Event::SessionStart, Event::PostToolUse] {
        let command = through(hook_command(
            &codex.helped,
            &web,
            &[(IDLE_SETTING, SHORT_IDLE)],
        ));
        (process.run(&command, &web.input(event))).assert_silent(event.name());
    }
    let helper = codex.helped.one_helper();
    assert!(
        first.is_none_or(|first| first == helper),
        "another helper runs"
    );
    thread::sleep(LIVE_FOR);
    assert!(
        codex.helped.helpers().contains(&helper),
        "the helper exited while the session lived"
    );

    process.end();
    codex.helped.await_exit(helper, EXIT_LIMIT);
}
