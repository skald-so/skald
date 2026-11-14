//! `skald --agent <name>`: the CLI acts as an agent, a named endpoint on this device, and `skald
//! --agent <name> address` shows its address (`docs/protocol/README.md`, "Agent address") in the
//! space of the working directory. The agent's name stays on the machine: no request carries it
//! (`docs/threat-model.md`, "The server does not see").
//!
//! As an agent, the CLI covers only the space of the working directory: it lists no other space
//! and refuses a command that names one, alike for every other name, and it refuses the person's
//! commands. A folder in no space gives it none.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use skald_proto::{Address, Name, PERSON, ServerName, VersionRange};

use crate::connection::printable;
use crate::devices::{text, to_session};
use crate::spaces::{OUT_OF_SCOPE, Scope};
use crate::{folders, follow, secrets, spaces};

/// The server's name, `GET`.
const SERVER_PATH: &str = "/server";

/// `Server` in the OpenAPI spec.
#[derive(Deserialize)]
struct Server {
    name: Option<String>,
}

/// The agent name `text`; fails unless it is a valid `name` other than [`PERSON`].
pub fn name(text: &str) -> Result<Name, String> {
    let name: Name = text
        .parse()
        .map_err(|err| format!("the agent name \"{}\" {err}", printable(text)))?;
    if name.as_str() == PERSON {
        return Err(format!(
            "the agent name {PERSON} is reserved for a person's address"
        ));
    }
    Ok(name)
}

/// Why an agent cannot run `command`, the CLI's name of one of the person's commands.
pub fn person_only(command: &str) -> String {
    format!("skald {command} is the person's command, which an agent cannot run; nothing changed")
}

/// `skald --agent <name> spaces`: lists the space of the working directory, by the state in `dir`,
/// its secrets in `storage`, as `skald spaces` lists it; or says why the folder is in none.
pub fn spaces(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<(), String> {
    match folders::working_space(dir) {
        Ok((_, space)) => spaces::list(dir, storage, speaks, timeout, Scope::Only(space.id)),
        Err(why) => {
            println!("This agent sees no space: {why}.");
            Ok(())
        }
    }
}

/// The scope of a command that names the space `name`, by the state in `dir`: every space for
/// the person; for an `agent`, the space of the working directory if it is named so. Any other
/// name, of a space of the device's or not, is refused alike.
pub fn scope(dir: &Path, name: &str, agent: bool) -> Result<Scope, String> {
    if !agent {
        return Ok(Scope::All);
    }
    match folders::working_space(dir) {
        Ok((_, space)) if space.name == name => Ok(Scope::Only(space.id)),
        _ => Err(OUT_OF_SCOPE.to_owned()),
    }
}

/// `skald --agent <agent> address`: shows the address of `agent` in the space of the working
/// directory, by the state in `dir`, with the name of the server of the machine's session.
pub fn address(
    dir: &Path,
    agent: &Name,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<(), String> {
    let no_address = |why: String| format!("the agent {agent} has no address: {why}");
    let (state, space) = folders::working_space(dir).map_err(no_address)?;
    let roster = follow::decode(&space.accepted, &space.name)?;
    let body = roster.body();
    let member = spaces::own_account(&state)?
        .and_then(|account| body.members.iter().find(|member| member.account == account));
    let Some(member) = member else {
        return Err(no_address(format!(
            "this account is no member of the space {} in its last accepted revision",
            body.name
        )));
    };
    let session = state.session().map_err(text)?.ok_or_else(|| {
        no_address("this machine is signed in to no server; run skald sign-in".to_owned())
    })?;
    let server = server_name(&session.server, speaks, timeout)?.ok_or_else(|| {
        no_address(format!(
            "the server {} has no valid name for addresses, such as when its public address has \
             an IP address as its host",
            session.server
        ))
    })?;
    let address = Address::new(
        body.name.clone(),
        member.handle.clone(),
        agent.clone(),
        server,
    );
    println!("{address}");
    Ok(())
}

/// The name of the server at `url`, if it has a valid one.
fn server_name(
    url: &str,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<Option<ServerName>, String> {
    let body = to_session(url, speaks, timeout)?
        .get(SERVER_PATH)
        .map_err(text)?;
    let unexpected = |err: String| format!("unexpected answer from the server to its name: {err}");
    let server: Server =
        serde_json::from_slice(&body).map_err(|err| unexpected(err.to_string()))?;
    server
        .name
        .map(|name| {
            name.parse()
                .map_err(|err| unexpected(format!("\"{}\": {err}", printable(&name))))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_name_is_a_name_other_than_user() {
        assert_eq!(
            name("claude-api").map(|name| name.to_string()),
            Ok("claude-api".to_owned())
        );
        let err = name(PERSON).expect_err("reserved");
        assert_eq!(
            err,
            "the agent name user is reserved for a person's address"
        );
        let err = name("Claude\u{1b}").expect_err("not a name");
        assert!(
            err.starts_with("the agent name \"Claude\\u{1b}\" "),
            "{err}"
        );
    }
}
