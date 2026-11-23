//! `read` after a Codex session confirms it (`docs/protocol/delivery.md`, "Device to agent" and
//! "Receipts"; `docs/dev/agent-delivery.md`, "Read confirmations"; `docs/threat-model.md`, "An
//! agent on the person's own device"):
//!
//! - A message that `inbox` or another path gave a session waits for its confirmation, and no other
//!   call gives it meanwhile. Confirmed, it is handed over and `read` goes. Unconfirmed within a
//!   timeout, it waits again and goes by the next path. Once confirmed, it is never given again.
//! - The confirmation comes from the session file the session registered, its rollout for Codex,
//!   read by the client while the session runs and by its `Stop` hook as the fallback: once the
//!   model's own entry, not an API error, follows the entry of the `inbox` result that gave the
//!   message. For a session whose session file the client does not read, such as a rollout it
//!   cannot parse, or an MCP client without Skald's hooks, `read` goes at the write. A vendor
//!   signal, such as a `codex queue` that succeeded, gives no `read`.
//! - Codex's paths carry only the signal, so the timeout runs from the signal: in a busy session
//!   `Stop` follows `PostToolUse`, then `codex queue` once the turn ends; in an idle one the next
//!   `UserPromptSubmit` follows `codex queue`; after the last, the message waits for `inbox`.
//! - The client reads only the registered session's rollout, only the references in the entries
//!   that gave Skald's messages, and keeps no line of it.
//!
//! As the tests of waking an idle Codex session (`codex_wakes/main.rs`): Ada's machine, alone in
//! `clienta`, her agents web and api in `~/work/clientA`; api sends web messages with `skald send
//! --to`, and the person's `skald recv` shows their `read`; or her second machine's agent review
//! sends them, and that machine's `skald recv` shows it. Each session runs in a Codex process,
//! the stand-in `codex` of an app of its own, which records each `codex queue` the helper runs; its
//! hooks run on inputs recorded from Codex 0.162.1, through a copy of `skald` without the one-shot
//! setting. Its `skald mcp` runs as Codex starts it, with Codex's client name. Its rollout grows as
//! Codex writes it (`rollout.rs`).
//!
//! The CLI, as these tests use it, beside the commands of those tests:
//! - `SKALD_RESEND_AFTER`: how long, in seconds, a message a path gave waits for its confirmation
//!   before it goes by the next path, for tests. The session's hooks, the helper they start and its
//!   `skald mcp` take it.
//!
//! As `codex_wakes`, this binary stands in for `codex`, so it runs its own harness ([`run`]).

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod ada;
mod confirms;
mod fallbacks;
mod laptop;
mod privacy;
mod resending;
mod rollout;
mod unread;

// Codex sessions, the runs of their hooks and the stand-in `codex`, as the tests of Codex's hooks
// and of waking an idle Codex session have them; the runs of a copy of `skald` through a helper, as
// the tests of the helper have them; Ada's and Bob's agents, as the tests of Claude Code's hooks
// have them.
#[path = "../codex_hooks/codex.rs"]
mod codex;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;
#[path = "../codex_wakes/standin.rs"]
mod standin;

include!("../claude_code_hooks/modules.rs");

use std::panic::catch_unwind;
use std::process::ExitCode;

/// The tests, each with the name nextest and cargo give it.
const TESTS: [(&str, fn()); 11] = [
    (
        "confirms::read_follows_once_the_model_s_own_entry_follows_inbox_s_result_in_the_rollout",
        confirms::read_follows_once_the_model_s_own_entry_follows_inbox_s_result_in_the_rollout,
    ),
    (
        "confirms::a_reference_in_another_tool_s_result_confirms_nothing",
        confirms::a_reference_in_another_tool_s_result_confirms_nothing,
    ),
    (
        "resending::unconfirmed_past_the_timeout_it_goes_by_the_next_path_and_once_confirmed_never_again",
        resending::unconfirmed_past_the_timeout_it_goes_by_the_next_path_and_once_confirmed_never_again,
    ),
    (
        "resending::busy_the_signal_goes_by_stop_then_codex_queue_then_user_prompt_submit_then_waits",
        resending::busy_the_signal_goes_by_stop_then_codex_queue_then_user_prompt_submit_then_waits,
    ),
    (
        "resending::idle_the_signal_goes_by_codex_queue_then_user_prompt_submit_then_waits",
        resending::idle_the_signal_goes_by_codex_queue_then_user_prompt_submit_then_waits,
    ),
    (
        "fallbacks::with_the_helper_killed_stop_confirms_it_and_read_goes_once",
        fallbacks::with_the_helper_killed_stop_confirms_it_and_read_goes_once,
    ),
    (
        "fallbacks::a_restarted_helper_confirms_it_from_the_rollout_while_the_session_runs",
        fallbacks::a_restarted_helper_confirms_it_from_the_rollout_while_the_session_runs,
    ),
    (
        "laptop::the_helper_s_confirmation_sends_read_to_the_second_machine_with_no_hook_run",
        laptop::the_helper_s_confirmation_sends_read_to_the_second_machine_with_no_hook_run,
    ),
    (
        "laptop::a_confirmation_by_stop_sends_read_to_the_second_machine_once_the_helper_runs_on",
        laptop::a_confirmation_by_stop_sends_read_to_the_second_machine_once_the_helper_runs_on,
    ),
    (
        "privacy::the_state_directory_keeps_nothing_of_the_rollout",
        privacy::the_state_directory_keeps_nothing_of_the_rollout,
    ),
    (
        "unread::an_unparsable_rollout_gives_read_at_the_write_and_a_readable_one_holds_it_back",
        unread::an_unparsable_rollout_gives_read_at_the_write_and_a_readable_one_holds_it_back,
    ),
];

fn main() -> ExitCode {
    match standin::acts() {
        Some(code) => code,
        None => run(&TESTS),
    }
}

/// libtest's command line, as cargo and cargo-nextest give it: `--list` lists the chosen tests as
/// `<name>: test`; otherwise each chosen test runs in turn, and a panic fails the run with libtest's
/// exit code, 101. No test is ignored, so `--ignored` chooses none. A name is chosen if no filter is
/// given or it holds one, or equals one with `--exact`, and no `--skip` matches it.
fn run(tests: &[(&str, fn())]) -> ExitCode {
    let given: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| given.iter().any(|arg| arg == flag);
    let exact = has("--exact");
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let mut rest = given.iter();
    while let Some(arg) = rest.next() {
        if arg == "--skip" {
            skips.extend(rest.next().cloned());
        } else if [
            "--format",
            "--color",
            "--test-threads",
            "--logfile",
            "--shuffle-seed",
            "-Z",
        ]
        .contains(&arg.as_str())
        {
            rest.next();
        } else if !arg.starts_with('-') {
            filters.push(arg.clone());
        }
    }
    let fits = |name: &str, pattern: &String| match exact {
        true => name == pattern,
        false => name.contains(pattern.as_str()),
    };
    let mut chosen = Vec::new();
    for &(name, test) in tests {
        let wanted = filters.is_empty() || filters.iter().any(|pattern| fits(name, pattern));
        let skipped = skips.iter().any(|pattern| fits(name, pattern));
        if wanted && !skipped && !has("--ignored") {
            chosen.push((name, test));
        }
    }
    if has("--list") {
        chosen.iter().for_each(|(name, _)| println!("{name}: test"));
        return ExitCode::SUCCESS;
    }
    let failures = chosen
        .iter()
        .filter(|&&(name, test)| {
            println!("test {name} ...");
            let failed = catch_unwind(test).is_err();
            println!("test {name} ... {}", if failed { "FAILED" } else { "ok" });
            failed
        })
        .count();
    println!(
        "test result: {}. {} passed; {failures} failed",
        if failures == 0 { "ok" } else { "FAILED" },
        chosen.len() - failures
    );
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(101)
    }
}
