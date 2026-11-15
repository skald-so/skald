//! `skald --agent <name>`: the CLI acts as an agent, a named endpoint on this device, and `skald
//! --agent <name> address` shows its address (`docs/protocol/README.md`, "Agent address") in the
//! space of the working directory. The agent's name stays on the machine: no request carries it
//! (`docs/threat-model.md`, "The server does not see").
//!
//! As an agent, the CLI covers only the space of the working directory: it lists no other space
//! and refuses a command that names one, alike for every other name, and it refuses the person's
//! commands of rules, `.skald.toml` approval and creating spaces and channels. A folder in no space
//! gives it none.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use skald_proto::{Address, Name, PERSON, ServerName, VersionRange};

use crate::connection::{Connection, printable};
use crate::devices::text;
use crate::spaces::{OUT_OF_SCOPE, Scope};
use crate::{folders, secrets, spaces};

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
        Ok(space) => spaces::list(dir, storage, speaks, timeout, Scope::Only(space.id)),
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
        Ok(space) if space.name == name => Ok(Scope::Only(space.id)),
        _ => Err(OUT_OF_SCOPE.to_owned()),
    }
}

/// `skald --agent <agent> address`: shows the address of `agent` in the space of the working
/// directory, by the state in `dir`, its secrets in `storage`, once the space is synced
/// ([`spaces::synced`]), with the name of the server of the machine's session.
pub fn address(
    dir: &Path,
    storage: secrets::Storage,
    agent: &Name,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<(), String> {
    let no_address = |why: String| format!("the agent {agent} has no address: {why}");
    let space = folders::working_space(dir).map_err(no_address)?;
    let mut synced = spaces::synced(
        dir,
        storage,
        speaks,
        timeout,
        &space.name,
        Scope::Only(space.id),
    )
    .map_err(no_address)?;
    let body = synced.roster.body();
    let account = synced.follower.own.account;
    let Some(member) = body.member(account) else {
        return Err(no_address(format!(
            "this account is no member of the space {} in its last accepted revision",
            body.name
        )));
    };
    let server = server_name(&mut synced.follower.connection)?
        .ok_or_else(|| no_address(NO_SERVER_NAME.to_owned()))?;
    let address = Address::new(
        body.name.clone(),
        member.handle.clone(),
        agent.clone(),
        server,
    );
    println!("{address}");
    Ok(())
}

/// What a run says when the server of the machine's session has no valid name.
pub const NO_SERVER_NAME: &str = "the server of this machine's session has no valid name for \
                                  addresses, such as when its public address has an IP address \
                                  as its host";

/// The name of the server `connection` reaches, if it has a valid one.
pub fn server_name(connection: &mut Connection) -> Result<Option<ServerName>, String> {
    let body = connection.get(SERVER_PATH).map_err(text)?;
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
