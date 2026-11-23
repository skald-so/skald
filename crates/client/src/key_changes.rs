//! Key changes until the person sees them (`docs/protocol/membership.md`, "Device checks").
//!
//! - The device keeps each key change in its local state as it warns of it ([`keep`]). One that an
//!   SSH signature or a domain's records show is kept with the address of the GitHub API or
//!   resolver that showed it, and counts only for runs with that address, as their results do: the
//!   address the client that started the run has, also through a helper.
//! - Until its person has seen them, each command, as the person or as an agent, and each tool call
//!   of the MCP server says on one line how many wait and that `skald key-changes` at a terminal
//!   shows them ([`say`]); with `--json`, as an object. The line names nothing of any space. A
//!   service agent's runs never say it: no person sees them at a terminal.
//! - `skald key-changes` shows them only to its person at a terminal, and counts them seen once the
//!   person presses Enter, as `skald fingerprint` takes its person's answer ([`run`]); from a pipe
//!   or as an agent it shows none. Killed before the Enter, it counts none seen.

use std::path::Path;
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::devices::text;
use crate::link_code;
use crate::person::Person;
use crate::state::{self, State};

/// The command that shows the key changes that wait, as the person runs it.
pub const SHOWN_BY: &str = "skald key-changes";

/// What the person presses to count them seen.
const ENTER: &str = "Press Enter once you have read them";

/// The step of `skald key-changes` after its person pressed Enter, which its process hands to the
/// helper or runs here holding the local state's lock.
#[derive(Serialize, Deserialize)]
pub struct Step {
    /// The positions of the key changes it showed.
    pub seen: Vec<u64>,
}

/// What found key changes kept apart by its address: the GitHub API at `api`.
pub fn github(api: &str) -> String {
    format!("github {}", api.trim_end_matches('/'))
}

/// What found key changes kept apart by its address: the DNS-over-HTTPS resolver at `resolver`.
pub fn dns(resolver: &str) -> String {
    format!("dns {resolver}")
}

/// The sources whose key changes a run with GitHub's API at `api` and the resolver at `resolver`
/// counts.
pub fn sources(api: &str, resolver: &str) -> Vec<String> {
    vec![String::new(), github(api), dns(resolver)]
}

/// Keeps `line`, the warning of a key change found by no address, in `state` for its person.
pub fn keep(state: &State, line: &str) -> Result<(), String> {
    keep_from(state, "", line)
}

/// Keeps `line`, the warning of a key change found by `source`, in `state` for its person.
pub fn keep_from(state: &State, source: &str, line: &str) -> Result<(), String> {
    (state.keep_key_change((line, source), line, false))
        .map(drop)
        .map_err(text)
}

/// The key of a key change about `bytes`, an object of the kind `kind` names: the kind and the
/// SHA-256 of the bytes, in hex.
pub fn key_of(kind: &str, bytes: &[u8]) -> String {
    let hash: [u8; 32] = Sha256::digest(bytes).into();
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{kind} {hex}")
}

/// Keeps `line`, the warning of the key change `key`, in `state` for its person unless it was ever
/// kept; gives whether it kept it.
pub fn keep_once(state: &State, key: &str, line: &str) -> Result<bool, String> {
    (state.keep_key_change((key, ""), line, true)).map_err(text)
}

/// Says, on one line or as a JSON object if `json`, how many key changes of `sources` wait for the
/// person of the machine whose state is in `dir`, if any; gives whether it said so. A state it
/// cannot read says nothing.
pub fn say(dir: &Path, sources: &[String], json: bool) -> bool {
    if !dir.join(state::FILE).is_file() {
        return false;
    }
    let waiting = State::open(dir).and_then(|state| state.key_changes(sources));
    let count = waiting.map_or(0, |waiting| waiting.len());
    if count == 0 {
        return false;
    }
    if json {
        println!("{}", object(count));
    } else {
        eprintln!("{}", line(count));
    }
    true
}

/// The line that says `count` key changes wait.
fn line(count: usize) -> String {
    match count {
        1 => format!("skald: 1 key change waits for you: run {SHOWN_BY} at a terminal"),
        _ => format!("skald: {count} key changes wait for you: run {SHOWN_BY} at a terminal"),
    }
}

/// The JSON object that says `count` key changes wait.
fn object(count: usize) -> serde_json::Value {
    json!({ "key_changes": { "waiting": count, "show_with": SHOWN_BY } })
}

/// `skald key-changes` on the machine whose state is in `dir`, by its person at a terminal (an
/// agent's run is refused before it): shows the key changes of `sources` that wait, asks the
/// person to press Enter and then has `step` count them seen; gives the exit code.
pub fn run(dir: &Path, sources: &[String], step: impl FnOnce(Step) -> ExitCode) -> ExitCode {
    if !link_code::at_terminal() {
        return crate::exit(Err(format!(
            "{SHOWN_BY} shows key changes only to its person at a terminal, never from a pipe or \
             as an agent; none is counted seen"
        )));
    }
    let waiting = match dir.join(state::FILE).is_file() {
        true => State::open(dir)
            .and_then(|state| state.key_changes(sources))
            .map_err(text),
        false => Ok(Vec::new()),
    };
    let waiting = match waiting {
        Ok(waiting) if waiting.is_empty() => {
            println!("No key change waits.");
            return ExitCode::SUCCESS;
        }
        Ok(waiting) => waiting,
        Err(err) => return crate::exit(Err(err)),
    };
    println!("Key changes this device warned of:");
    for (_, line) in &waiting {
        println!("- {line}");
    }
    match Person::Terminal.enter(ENTER, "") {
        Ok(Some(_)) => step(Step {
            seen: waiting.iter().map(|(position, _)| *position).collect(),
        }),
        Ok(None) => crate::exit(Err(
            "the terminal's input ended; no key change is counted seen".to_owned(),
        )),
        Err(err) => crate::exit(Err(err)),
    }
}

/// Runs `step` of `skald key-changes` on the machine whose state is in `dir`.
pub fn step(dir: &Path, step: &Step) -> Result<(), String> {
    let state = State::open(dir).map_err(text)?;
    state.key_changes_seen(&step.seen).map_err(text)?;
    println!("They no longer wait.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_says_how_many_wait_and_what_shows_them() {
        for (count, said) in [(1, "1 key change waits"), (3, "3 key changes wait")] {
            let line = line(count);
            assert!(line.contains(said), "{line}");
            assert!(line.contains(SHOWN_BY), "{line}");
            assert!(!line.contains('\n'), "{line:?}");
        }
        let object = object(2);
        assert_eq!(object["key_changes"]["waiting"], 2);
        assert_eq!(object["key_changes"]["show_with"], SHOWN_BY);
    }

    #[test]
    fn sources_keep_apart_the_github_api_and_the_resolver() {
        assert_eq!(
            github("https://api.github.com/"),
            "github https://api.github.com"
        );
        assert_eq!(
            sources("https://api.github.com", "https://dns.example/q"),
            [
                "",
                "github https://api.github.com",
                "dns https://dns.example/q"
            ]
        );
    }
}
