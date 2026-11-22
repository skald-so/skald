//! Waking an idle Codex session with `codex queue` (`docs/dev/architecture.md`, "Delivery to
//! agents"; `docs/dev/agent-delivery.md`, "`codex queue`" and "Results"; `docs/protocol/delivery.md`,
//! "Device to agent"):
//!
//! - When a message for an agent reaches the helper and the agent's Codex session is idle, the
//!   helper runs `codex queue --thread <thread> --message <signal>` for the session's thread, with
//!   the session's config home in `CODEX_HOME`, and with the `codex` of the process that runs the
//!   session, so a desktop app's or an IDE extension's bundled Codex serves its own sessions. A busy
//!   session gets the signal from its hooks instead.
//! - A session has one queued signal at a time: messages that come before the turn it starts runs
//!   add none.
//! - What `codex queue` gets holds only the signal, whatever the sender, its trust level or the
//!   `text` setting: no message's text, in its arguments, its settings or its input.
//! - A `codex queue` that fails leaves the messages to the session's hooks and `inbox`.
//!
//! As the tests of Codex's hooks (`codex_hooks/main.rs`): Ada's machine, alone in `clienta`, her
//! agents web and api in `~/work/clientA`, Bob joining by a fixture revision and her second machine
//! linked by hand; the hooks run on inputs recorded from Codex 0.162.1, a copy of `skald` without
//! the one-shot setting, so messages from other devices reach Ada's machine through the helper.
//!
//! A stand-in plays the `codex` command (`standin.rs`): a copy of this test binary named `codex`,
//! which plays a Codex process that runs a session's hooks as its children, and, run by the helper,
//! records each run, its arguments, settings and input, instead of queueing anything. So this
//! binary runs its own harness, as libtest's (`harness`): libtest would refuse the arguments of
//! `codex queue`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod busy;
mod failing;
mod once;
mod queued;
mod routing;
mod standin;

// Codex sessions and the runs of their hooks, as the tests of Codex's hooks have them; the runs of a
// copy of `skald` through a helper, as the tests of the helper have them; Ada's and Bob's agents,
// as the tests of Claude Code's hooks have them; the person's delivery settings, as the tests of
// the delivery settings have them.
#[path = "../codex_hooks/codex.rs"]
mod codex;
#[path = "../claude_code_e2e/delivery.rs"]
mod delivery;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;

include!("../claude_code_hooks/modules.rs");

use std::panic;
use std::process::ExitCode;

/// Each test, by the name the harness gives it.
const TESTS: &[(&str, fn())] = &[
    (
        "queued::an_idle_session_gets_one_codex_queue_with_only_the_signal_within_a_second",
        queued::an_idle_session_gets_one_codex_queue_with_only_the_signal_within_a_second,
    ),
    (
        "busy::a_busy_session_gets_the_signal_from_its_hooks_and_a_queue_once_its_turn_ends",
        busy::a_busy_session_gets_the_signal_from_its_hooks_and_a_queue_once_its_turn_ends,
    ),
    (
        "once::messages_while_a_queued_signal_waits_add_no_codex_queue",
        once::messages_while_a_queued_signal_waits_add_no_codex_queue,
    ),
    (
        "routing::only_a_session_of_the_agent_gets_the_queue_with_its_own_codex_and_config_home",
        routing::only_a_session_of_the_agent_gets_the_queue_with_its_own_codex_and_config_home,
    ),
    (
        "routing::a_session_whose_codex_process_ended_gets_no_codex_queue",
        routing::a_session_whose_codex_process_ended_gets_no_codex_queue,
    ),
    (
        "failing::with_codex_queue_failing_the_next_hook_gives_the_signal_and_inbox_every_message",
        failing::with_codex_queue_failing_the_next_hook_gives_the_signal_and_inbox_every_message,
    ),
];

fn main() -> ExitCode {
    if let Some(code) = standin::acts() {
        return code;
    }
    harness(TESTS)
}

/// Runs `tests` as libtest does for cargo and cargo-nextest: with `--list`, prints `<name>: test`
/// for each chosen test and runs none; otherwise runs each chosen test, one after another, and fails
/// with libtest's exit code if one panics. A test is chosen unless `--ignored` is given, as none is
/// ignored, or a filter is given that its name does not hold, or, with `--exact`, is not.
fn harness(tests: &[(&str, fn())]) -> ExitCode {
    let mut filters: Vec<String> = Vec::new();
    let mut skips: Vec<String> = Vec::new();
    let (mut list, mut exact, mut ignored) = (false, false, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--exact" => exact = true,
            "--ignored" => ignored = true,
            "--skip" => skips.extend(args.next()),
            // libtest's options that take a value.
            "--format" | "--color" | "--test-threads" | "--logfile" | "--shuffle-seed" | "-Z" => {
                args.next();
            }
            flag if flag.starts_with('-') => {}
            _ => filters.push(arg),
        }
    }
    let matches = |name: &str, filter: &String| {
        if exact {
            name == filter
        } else {
            name.contains(filter.as_str())
        }
    };
    let chosen: Vec<&(&str, fn())> = (tests.iter())
        .filter(|_| !ignored)
        .filter(|(name, _)| filters.is_empty() || filters.iter().any(|f| matches(name, f)))
        .filter(|(name, _)| !skips.iter().any(|skip| matches(name, skip)))
        .collect();
    if list {
        for (name, _) in &chosen {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    let mut failed = Vec::new();
    for (name, test) in &chosen {
        println!("test {name} ...");
        match panic::catch_unwind(*test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
            }
        }
    }
    println!(
        "test result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        chosen.len() - failed.len(),
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        // libtest's exit code when a test fails.
        ExitCode::from(101)
    }
}
