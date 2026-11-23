//! The link comes only from the person at a terminal: given as an argument, through a pipe, with no
//! terminal for input, or in the environment, it is not taken, and run as an agent the command
//! refuses; none of them sends anything. A machine that has never signed in still needs a valid
//! invite link before it makes a request.

use std::io::{Read as _, Write as _};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use crate::machine::{Machine, RUN_LIMIT};
use crate::terminal::Terminal;
use crate::world::{ADD_RULE, AGENT, CREATE_SPACE, GLOBEX, dir_of, path, run};
use crate::{ACCEPT, Scene, asked_for_link, assert_refused_saying, refused_at_terminal, start};

/// The agent the tests run the command as.
const AGENT_NAME: &str = "builder";

/// Settings a command could read a link from; it must read none of them.
const LINK_SETTINGS: [&str; 4] = [
    "SKALD_INVITE",
    "SKALD_INVITE_LINK",
    "SKALD_LINK",
    "INVITE_LINK",
];

/// Fails unless `text` says `terminal`.
#[track_caller]
fn assert_says_terminal(text: &str) {
    assert!(
        text.to_lowercase().contains("terminal"),
        "it does not say why: {text}"
    );
}

/// `skald accept-invite <link>` at a terminal on Bob's machine fails, saying it takes the link only
/// at the terminal, and sends nothing at all.
#[test]
fn link_given_as_an_argument_is_refused_and_nothing_is_sent() {
    let scene = Scene::new(
        (7_305_404_001, "ada-whose-link-is-an-argument"),
        (7_305_404_002, "bob-passes-the-link"),
    );
    let made = scene.invite();
    let since = scene.cloud().relay.mark();

    let refused = Terminal::start(&scene.bob.machine, &[ACCEPT, &made.link]).finish();

    assert!(!refused.success, "{}", refused.text);
    assert_says_terminal(&refused.text);
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}

/// `skald accept-invite` on Bob's machine with the link on its input through a pipe, its output a
/// pipe too, fails, saying it needs a terminal, and sends nothing at all.
#[test]
fn link_through_a_pipe_is_refused_and_nothing_is_sent() {
    let scene = Scene::new(
        (7_305_404_101, "ada-whose-link-is-piped"),
        (7_305_404_102, "bob-pipes-the-link"),
    );
    let made = scene.invite();
    let since = scene.cloud().relay.mark();

    let (success, shown) = piped(&scene.bob.machine, &format!("{}\n", made.link));

    assert!(!success, "{shown}");
    assert_says_terminal(&shown);
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}

/// `skald accept-invite` on Bob's machine, its output a terminal and its input empty, as a shell
/// redirects it from `/dev/null`, fails, saying it needs a terminal, and sends nothing at all.
#[cfg(unix)]
#[test]
fn input_that_is_no_terminal_is_refused_and_nothing_is_sent() {
    let scene = Scene::new(
        (7_305_404_201, "ada-whose-link-is-not-typed"),
        (7_305_404_202, "bob-without-a-keyboard"),
    );
    let made = scene.invite();
    let since = scene.cloud().relay.mark();

    let refused = Terminal::start_without_input(&scene.bob.machine, &[ACCEPT]).finish();

    assert!(!refused.success, "{}", refused.text);
    assert_says_terminal(&refused.text);
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}

/// Bob's machine runs with the link in each of [`LINK_SETTINGS`]. `skald accept-invite` at a
/// terminal still asks for the link; once Bob ends the terminal's input without typing one, it
/// fails and has sent nothing at all.
#[test]
fn link_in_the_environment_is_not_taken_and_nothing_is_sent() {
    let mut scene = Scene::new(
        (7_305_404_301, "ada-whose-link-is-in-the-env"),
        (7_305_404_302, "bob-sets-the-link"),
    );
    let made = scene.invite();
    for name in LINK_SETTINGS {
        scene.bob.machine = scene.bob.machine.with_env(name, &made.link);
    }
    let since = scene.cloud().relay.mark();

    let mut terminal = start(&scene.bob.machine, &[]);
    asked_for_link(&mut terminal);
    terminal.end_input();
    let ended = terminal.finish();

    assert!(!ended.success, "{}", ended.text);
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}

/// Bob's machine creates `globex` and binds `~/work/globex` to it. There, at a terminal, `skald
/// --agent builder accept-invite` fails, saying it is the person's command, which an agent cannot
/// run, and sends nothing at all.
#[test]
fn as_an_agent_at_a_terminal_it_refuses_and_sends_nothing() {
    let scene = Scene::new(
        (7_305_404_401, "ada-whose-link-reaches-an-agent"),
        (7_305_404_402, "bob-s-agent-accepts"),
    );
    let made = scene.invite();
    let machine = &scene.bob.machine;
    let created = run(machine, &[CREATE_SPACE, GLOBEX]);
    assert!(created.success, "{created:?}");
    let folder = dir_of(machine, "work/globex");
    let bound = run(machine, &[ADD_RULE, path(&folder), GLOBEX]);
    assert!(bound.success, "{bound:?}");
    let since = scene.cloud().relay.mark();

    let refused = Terminal::start_in(machine, &folder, &[AGENT, AGENT_NAME, ACCEPT]).finish();

    assert!(!refused.success, "{}", refused.text);
    let said = refused.text.to_lowercase();
    for part in [
        "skald accept-invite",
        "person's command",
        "an agent cannot run",
    ] {
        assert!(
            said.contains(part),
            "it does not say {part:?}: {}",
            refused.text
        );
    }
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}

/// A machine that never signed in rejects an invite link missing its secret without sending
/// anything.
#[test]
fn a_signed_out_machine_refuses_a_malformed_link_before_sending_anything() {
    let scene = Scene::new(
        (7_305_404_501, "ada-invites-a-newcomer"),
        (7_305_404_502, "bob-on-a-new-machine"),
    );
    let made = scene.invite();
    let fresh = Machine::new().with_host_name("bob-new-machine");
    let (without_secret, _) = made.link.split_once('#').expect("an invite fragment");
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&fresh, without_secret);

    assert_refused_saying(&refused, &["link"]);
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}

/// Runs `skald accept-invite` on `machine` with `input` on its input through a pipe and its outputs
/// piped, for up to [`RUN_LIMIT`]: whether it succeeded, and both its outputs.
fn piped(machine: &Machine, input: &str) -> (bool, String) {
    let home = machine.root().join("home");
    let mut child = machine
        .command_in(&home, &[ACCEPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run skald");
    // A run that refuses at once may close its input before the link is written.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for skald") {
            break status;
        }
        if started.elapsed() > RUN_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("skald ran for more than {RUN_LIMIT:?}");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let mut shown = String::new();
    for output in [
        child
            .stdout
            .take()
            .map(|out| Box::new(out) as Box<dyn std::io::Read>),
        child
            .stderr
            .take()
            .map(|err| Box::new(err) as Box<dyn std::io::Read>),
    ]
    .into_iter()
    .flatten()
    {
        let mut output = output;
        let _ = output.read_to_string(&mut shown);
        shown.push('\n');
    }
    (status.success(), shown)
}
