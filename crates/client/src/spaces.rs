//! `skald create-space` and `skald spaces`: a team space the device creates and signs revision 1 of
//! as its first admin, and the spaces the device accepted a revision of
//! (`docs/protocol/membership.md`, "Roster"). The local state keeps revision 1 as the space's anchor
//! and last accepted revision; it is written before it is sent and sent again on later runs until
//! the server takes it.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_proto::{
    AccountId, Cbor as _, Cert, Name, Role, Roster, RosterBody, SpaceId, VersionRange,
};

use crate::connection::{self, Connection, Method, printable};
use crate::device;
use crate::devices::{signed_in_with_secrets, text, to_session};
use crate::secrets;
use crate::sign_in;
use crate::state::{self, Link, State};

/// What `skald create-space` says on a device without a certificate.
const NOT_LINKED: &str = "this device is not linked yet: it holds no certificate signed by the \
                          account's master key, so it creates no space";

/// `Space` in the API spec.
#[derive(Deserialize)]
struct Created {
    space: String,
    name: String,
}

/// Creates the team space `name` on the server of the session of the machine whose state is in
/// `dir`, its secrets in `storage`, and stores its revision 1, signed by the device. Revisions the
/// device signed that the server has not taken yet are sent first; a space the device created
/// already is not created again.
pub fn create(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    name: &str,
) -> Result<(), String> {
    let name: Name = name.parse().map_err(|err| {
        format!(
            "the space name \"{}\" {err}; no space is created",
            printable(name)
        )
    })?;
    let (state, server, token, stored) = signed_in_with_secrets(dir, storage)?;
    let device = state
        .device()
        .map_err(text)?
        .ok_or_else(|| NOT_LINKED.to_owned())?;
    let Link::Certified { master, cert } = device.link.clone() else {
        return Err(NOT_LINKED.to_owned());
    };
    let cert = Cert::from_cbor(&cert)
        .map_err(|err| state::Error::Malformed(format!("the device's certificate: {err}")))
        .map_err(text)?;
    let keys = device::known_keys(&stored, Some(&device))
        .map_err(text)?
        .ok_or_else(|| device::Error::MissingKeys.to_string())?;
    let mut connection = to_session(&server, speaks, timeout)?;

    let mut existing = None;
    for space in state.spaces().map_err(text)? {
        if let Some(pending) = &space.pending {
            send(
                &mut connection,
                &state,
                &token,
                (space.id, &space.name),
                pending,
            )?;
        }
        if space.name == name.as_str() {
            existing = Some(space);
        }
    }
    if let Some(space) = existing {
        let roster = Roster::from_cbor(&space.anchor)
            .map_err(|err| state::Error::Malformed(format!("a space's revision: {err}")))
            .map_err(text)?;
        println!("{}", created(&roster, space.pending.is_none()));
        return Ok(());
    }

    let account = cert.body().account;
    let handle = handle(&mut connection, &token, account)?;
    let space = create_on_server(&mut connection, &token, &name)?;
    let body = RosterBody::first(
        space,
        name.clone(),
        account,
        handle,
        master,
        device.id,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    );
    let roster = skald_crypto::sign_roster(&body, keys.signing())
        .map_err(|err| format!("revision 1 of the space fails: {err}"))?;
    skald_crypto::check_roster(&roster, std::slice::from_ref(&cert))
        .map_err(|err| format!("revision 1 of the space fails: {err}"))?;
    let bytes = roster.to_cbor();
    state
        .add_space(space, name.as_str(), &bytes)
        .map_err(text)?;
    send(
        &mut connection,
        &state,
        &token,
        (space, name.as_str()),
        &bytes,
    )?;
    println!("{}", created(&roster, false));
    Ok(())
}

/// What `skald create-space` says of the space whose revision 1 is `first`: that it created it,
/// or, `already`, that it had created it on an earlier run.
fn created(first: &Roster, already: bool) -> String {
    let body = first.body();
    let handle = body
        .members
        .first()
        .map_or("", |member| member.handle.as_str());
    let (name, id) = (body.name.as_str(), body.space);
    if already {
        format!(
            "This device created the space {name} ({id}) already, as its first admin, with the \
             handle {handle}."
        )
    } else {
        format!(
            "Created the space {name} ({id}) as its first admin, with the handle {handle}: this \
             device signed its revision 1."
        )
    }
}

/// The handle of `account`, the account of the session `token`: its username, as the server gives
/// it.
fn handle(connection: &mut Connection, token: &str, account: AccountId) -> Result<Name, String> {
    let body = connection.get_as("/session", token).map_err(text)?;
    let session: sign_in::Account = serde_json::from_slice(&body)
        .map_err(|_| "unexpected answer from the server: a session without its account")?;
    if session.account.parse::<AccountId>() != Ok(account) {
        return Err(
            "the session is of another account than this device's; run skald sign-in".to_owned(),
        );
    }
    session.username.parse().map_err(|err| {
        format!(
            "unexpected answer from the server: the account's username \"{}\" {err}",
            printable(&session.username)
        )
    })
}

/// `POST /spaces`: the space `name` the server gives the session `token`, new or, after a lost
/// answer, the one it created before.
fn create_on_server(
    connection: &mut Connection,
    token: &str,
    name: &Name,
) -> Result<SpaceId, String> {
    let request = serde_json::json!({ "name": name.as_str() }).to_string();
    let body = match connection.send_json_as(Method::Post, "/spaces", token, request.as_bytes()) {
        Ok(body) => body,
        Err(connection::Error::Refused {
            status: 400 | 409,
            detail,
        }) => {
            return Err(format!(
                "the server refused the space name \"{}\": {}; no space is created",
                name.as_str(),
                printable(&detail)
            ));
        }
        Err(err) => return Err(err.to_string()),
    };
    let unexpected = || "unexpected answer from the server: a space without its ID or name";
    let created: Created = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    if created.name != name.as_str() {
        return Err(unexpected().to_owned());
    }
    created.space.parse().map_err(|_| unexpected().to_owned())
}

/// `POST /spaces/<id>/revisions`: sends `pending`, the revision of `space`, its ID and name, that
/// the device signed, as the session `token`, and records that the server took it. A revision the
/// server refuses is dropped with its space, which the server never took a revision of.
fn send(
    connection: &mut Connection,
    state: &State,
    token: &str,
    (id, name): (SpaceId, &str),
    pending: &[u8],
) -> Result<(), String> {
    let request = serde_json::json!({ "revision": URL_SAFE_NO_PAD.encode(pending) }).to_string();
    let path = format!("/spaces/{id}/revisions");
    match connection.send_json_as(Method::Post, &path, token, request.as_bytes()) {
        Ok(_) => state.set_taken(id).map_err(text),
        Err(connection::Error::Refused {
            status: 400 | 403 | 404 | 409,
            detail,
        }) => {
            state.remove_space(id).map_err(text)?;
            Err(format!(
                "the server refused revision 1 of the space {name}: {}; this device dropped the \
                 space",
                printable(&detail)
            ))
        }
        Err(err) => Err(format!(
            "{err}; the space {name} waits for the server to take its revision 1: run skald \
             create-space {name} again"
        )),
    }
}

/// Lists the spaces the device of the machine whose state is in `dir` accepted a revision of, from
/// its local state only.
pub fn list(dir: &Path) -> Result<(), String> {
    let spaces = if dir.is_dir() {
        let state = State::open(dir).map_err(text)?;
        let account = match state
            .device()
            .map_err(text)?
            .and_then(|device| device.account)
        {
            Some(account) => Some(
                account
                    .parse::<AccountId>()
                    .map_err(|_| {
                        state::Error::Malformed(format!("the device's account {account:?}"))
                    })
                    .map_err(text)?,
            ),
            None => None,
        };
        state
            .spaces()
            .map_err(text)?
            .iter()
            .map(|space| {
                let roster = Roster::from_cbor(&space.accepted).map_err(|err| {
                    text(state::Error::Malformed(format!(
                        "the revision of the space {}: {err}",
                        space.name
                    )))
                })?;
                Ok(line(&roster, account, space.pending.is_some()))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        Vec::new()
    };
    if spaces.is_empty() {
        println!("This device is in no space yet; create one with skald create-space <name>.");
    }
    for line in spaces {
        println!("{line}");
    }
    Ok(())
}

/// The line of a space whose last accepted revision is `roster`: its name, the role and handle of
/// `account` in it, and its ID; marked if the server has not taken the revision yet.
fn line(roster: &Roster, account: Option<AccountId>, pending: bool) -> String {
    let body = roster.body();
    let member =
        account.and_then(|account| body.members.iter().find(|member| member.account == account));
    let (role, handle) = match member {
        Some(member) => (
            match member.role {
                Role::Admin => "admin",
                Role::Member => "member",
            },
            member.handle.as_str(),
        ),
        None => ("no member", "-"),
    };
    let mut line = format!("{}  {role}  {handle}  {}", body.name.as_str(), body.space);
    if pending {
        line.push_str("  (not on the server yet)");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use skald_crypto::SigningKey;
    use skald_proto::DeviceId;

    /// Revision 1 of the space `acme`, created by `account` with the handle `ada`.
    fn first(account: AccountId) -> Roster {
        let key = SigningKey::generate();
        let body = RosterBody::first(
            SpaceId::generate(),
            "acme".parse().unwrap(),
            account,
            "ada".parse().unwrap(),
            [3; 32],
            DeviceId::from_signing_key(&key.public_key()),
            1_800_000_000,
        );
        skald_crypto::sign_roster(&body, &key).expect("a revision")
    }

    #[test]
    fn line_shows_the_name_the_accounts_role_and_handle_and_the_id() {
        let account = AccountId::generate();
        let roster = first(account);
        let id = roster.body().space;
        assert_eq!(
            line(&roster, Some(account), false),
            format!("acme  admin  ada  {id}")
        );
        assert_eq!(
            line(&roster, Some(account), true),
            format!("acme  admin  ada  {id}  (not on the server yet)")
        );
        assert_eq!(
            line(&roster, Some(AccountId::generate()), false),
            format!("acme  no member  -  {id}")
        );
        assert_eq!(
            line(&roster, None, false),
            format!("acme  no member  -  {id}")
        );
    }

    #[test]
    fn created_names_the_space_and_the_handle() {
        let roster = first(AccountId::generate());
        let id = roster.body().space;
        assert_eq!(
            created(&roster, false),
            format!(
                "Created the space acme ({id}) as its first admin, with the handle ada: this \
                 device signed its revision 1."
            )
        );
        assert_eq!(
            created(&roster, true),
            format!(
                "This device created the space acme ({id}) already, as its first admin, with the \
                 handle ada."
            )
        );
    }
}
