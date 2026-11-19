//! `skald set-inbound-mode`, `skald allow-sender` and `skald inbound-modes`: each agent of the
//! device in a space has an inbound mode, `accept` by default or `refuse`, and an allowlist of
//! senders it takes messages from whatever the mode (`docs/protocol/delivery.md`, "Device to
//! agent"). The person sets and lists them in the space of the working directory; they are kept in
//! the local state. An agent cannot run the commands ([`crate::agents::person_only`]).
//!
//! The mode decides when the device takes a message in ([`refusing`]); a message taken in keeps
//! that decision when the mode changes.

use std::path::Path;

use serde::{Deserialize, Serialize};
use skald_proto::{Address, Name};

use crate::connection::printable;
use crate::devices::text;
use crate::folders;
use crate::state::State;

/// The reason of a `refused` the mode gives, for the message's sender.
pub const REASON: &str = "the agent takes messages only from the senders on its allowlist";

/// An agent's inbound mode, as the person sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
pub enum Mode {
    /// Hand every message over.
    Accept,
    /// Hand over only the messages of the senders on the allowlist; issue `refused` of the others.
    Refuse,
}

impl Mode {
    /// Its name, as the local state keeps it and the CLI shows it.
    fn name(self) -> &'static str {
        match self {
            Mode::Accept => "accept",
            Mode::Refuse => "refuse",
        }
    }
}

/// `skald set-inbound-mode`: sets the mode of the device's agent `agent` in the space of the
/// working directory to `mode`, by the state in `dir`.
pub fn set_mode(dir: &Path, agent: &str, mode: Mode) -> Result<(), String> {
    let agent = crate::agents::name(agent)?;
    let (state, space) = opened(dir)?;
    (state.set_inbound_mode(space.id, agent.as_str(), mode.name())).map_err(text)?;
    println!(
        "The agent {agent} in the space {}: {}.",
        space.name,
        mode.name()
    );
    Ok(())
}

/// `skald allow-sender`: puts `sender`, an address in a form `--to` takes or a member's handle, on
/// the allowlist of the device's agent `agent` in the space of the working directory, by the state
/// in `dir`.
pub fn allow(dir: &Path, agent: &str, sender: &str) -> Result<(), String> {
    let agent = crate::agents::name(agent)?;
    let (state, space) = opened(dir)?;
    let sender = allowed(sender, &space.name)?;
    let added = (state.allow_sender(space.id, agent.as_str(), &sender)).map_err(text)?;
    let already = if added { "" } else { " already" };
    println!(
        "The agent {agent} in the space {}{already} takes messages from {sender} whatever its mode.",
        space.name
    );
    Ok(())
}

/// `skald inbound-modes`: lists each agent of the device in the space of the working directory
/// with its mode and the senders on its allowlist, by the state in `dir`.
pub fn list(dir: &Path) -> Result<(), String> {
    let (state, space) = opened(dir)?;
    let agents = state.inbound(space.id).map_err(text)?;
    if agents.is_empty() {
        println!("No agent of this device in the space {}.", space.name);
    }
    for inbound in agents {
        let allowed = if inbound.allowed.is_empty() {
            String::new()
        } else {
            format!(", allows {}", inbound.allowed.join(", "))
        };
        println!("{}: {}{allowed}", inbound.agent, inbound.mode);
    }
    Ok(())
}

/// The state in `dir` and the space of the working directory.
fn opened(dir: &Path) -> Result<(State, crate::state::Space), String> {
    let space = folders::working_space(dir)?;
    let state = State::open(dir).map_err(text)?;
    Ok((state, space))
}

/// `given`, a sender for an allowlist in the space named `space`, as the state keeps it,
/// lowercased: a full address of that space, `owner/agent` or a member's handle; fails for any
/// other.
fn allowed(given: &str, space: &str) -> Result<String, String> {
    let lowered = given.to_lowercase();
    let invalid = |err: &dyn std::fmt::Display| {
        format!(
            "the sender \"{}\": {err}; nothing changed",
            printable(given)
        )
    };
    if lowered.contains('@') {
        let address: Address = lowered.parse().map_err(|err| invalid(&err))?;
        if address.space().as_str() != space {
            return Err(invalid(&format!(
                "it is an address of another space than {space}"
            )));
        }
        return Ok(address.to_string());
    }
    let names: Vec<&str> = lowered.split('/').collect();
    let parsed = match names.as_slice() {
        [handle] => handle.parse::<Name>().map(|handle| handle.to_string()),
        [owner, agent] => (owner.parse::<Name>())
            .and_then(|owner| Ok(format!("{owner}/{}", agent.parse::<Name>()?))),
        _ => {
            return Err(invalid(
                &"use space/owner/agent@server, owner/agent or a member's handle",
            ));
        }
    };
    parsed.map_err(|err| invalid(&err))
}

/// Whether `entry`, a sender on an allowlist as the state keeps it, admits a message from `from`:
/// the same full address, the same owner and agent, or its owner's handle.
fn admits(entry: &str, from: &Address) -> bool {
    if entry.contains('@') {
        return entry == from.to_string();
    }
    match entry.split_once('/') {
        Some((owner, agent)) => from.owner().as_str() == owner && from.agent().as_str() == agent,
        None => from.owner().as_str() == entry,
    }
}

/// The agents of `agents`, the device's agents in the space `space` a message from `from` is for,
/// whose inbound mode refuses it: those in `refuse` without a sender on their allowlist that admits
/// `from`, by the state `state`.
pub fn refusing(
    state: &State,
    space: skald_proto::SpaceId,
    agents: &[String],
    from: &Address,
) -> Result<Vec<String>, String> {
    if agents.is_empty() {
        return Ok(Vec::new());
    }
    let inbound = state.inbound(space).map_err(text)?;
    Ok((inbound.into_iter())
        .filter(|inbound| agents.contains(&inbound.agent))
        .filter(|inbound| inbound.mode == Mode::Refuse.name())
        .filter(|inbound| !inbound.allowed.iter().any(|entry| admits(entry, from)))
        .map(|inbound| inbound.agent)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> Address {
        text.parse().unwrap()
    }

    /// A sender is kept lowercased as a full address of the space, `owner/agent` or a handle; an
    /// address of another space or of no such form is refused.
    #[test]
    fn allowlist_takes_an_address_of_the_space_or_a_handle() {
        let acme = "acme";
        assert_eq!(
            allowed("ACME/Bob/Claude@skald.example", acme),
            Ok("acme/bob/claude@skald.example".to_owned())
        );
        assert_eq!(allowed("Bob/claude", acme), Ok("bob/claude".to_owned()));
        assert_eq!(allowed("bob", acme), Ok("bob".to_owned()));
        for wrong in ["other/bob/claude@skald.example", "a/b/c", "bob/", "b ob"] {
            let err = allowed(wrong, acme).expect_err(wrong);
            assert!(err.ends_with("; nothing changed"), "{err}");
        }
    }

    /// A full address admits only itself; `owner/agent` that agent of the owner on any server; a
    /// handle each agent of its member and its person.
    #[test]
    fn entry_admits_its_address_its_agent_or_its_member() {
        let claude = address("acme/bob/claude@skald.example");
        let person = address("acme/bob/user@skald.example");
        let codex = address("acme/bob/codex@skald.example");
        assert!(admits("acme/bob/claude@skald.example", &claude));
        assert!(!admits("acme/bob/claude@skald.example", &codex));
        assert!(admits("bob/claude", &claude));
        assert!(!admits("bob/claude", &codex));
        assert!(!admits("ada/claude", &claude));
        for from in [&claude, &person, &codex] {
            assert!(admits("bob", from), "{from}");
            assert!(!admits("ada", from), "{from}");
        }
    }
}
