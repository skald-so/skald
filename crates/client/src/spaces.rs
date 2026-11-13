//! `skald create-space`, `skald spaces` and `skald members`: a team space the device creates and
//! signs revision 1 of as its first admin, the spaces the device follows and a space's members
//! (`docs/protocol/membership.md`, "Roster"). The local state keeps revision 1 as the space's anchor
//! and last accepted revision; it is written before it is sent and sent again on later runs until
//! the server takes it. Listing spaces or their members first accepts the revisions the server has
//! ([`crate::follow`]).
//!
//! A later revision the device signs is written before it is sent too. Any command that uses the
//! space sends it again first, until the server takes it; only then does it count as accepted, and
//! until then the space stays at its last accepted revision. When another revision came first, the
//! device accepts the new revisions, applies its change to the latest and signs again.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_proto::{
    AccountId, Cbor as _, Cert, Channel, DeviceId, Name, Role, Roster, RosterBody, SpaceId,
    VersionRange,
};

use crate::connection::{self, Connection, Method, printable};
use crate::device;
use crate::devices::{self, signed_in_with_secrets, text};
use crate::follow::{self, Own, SpaceDevices};
use crate::secrets;
use crate::sign_in;
use crate::state::{self, Link, State};

/// What `skald create-space` says on a device without a certificate.
const NOT_LINKED: &str = "this device is not linked yet: it holds no certificate signed by the \
                          account's master key, so it creates no space";

/// What a run says when another run changed the space meanwhile.
pub const CHANGED_MEANWHILE: &str =
    "another skald run changed the space meanwhile; run the command again";

/// What a command that uses a space says on a machine without a certified device.
const NOT_FOLLOWING: &str = "this device is not linked yet: it holds no certificate signed by the \
                             account's master key, so it follows no space";

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
    let (device, master, cert) = certified(&state)?.ok_or_else(|| NOT_LINKED.to_owned())?;
    let keys = device::known_keys(&stored, Some(&device))
        .map_err(text)?
        .ok_or_else(|| device::Error::MissingKeys.to_string())?;
    let mut connection = Connection::new(&server, speaks, timeout).map_err(text)?;

    let mut existing = None;
    for space in state.spaces().map_err(text)? {
        // A later revision waits for a command that uses its space.
        if let Some(pending) = space
            .pending
            .as_ref()
            .filter(|pending| **pending == space.anchor)
        {
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
        let roster = follow::decode(&space.anchor, &space.name)?;
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

/// This machine's device, with the master key it pinned and its certificate, once the server took
/// them.
fn certified(state: &State) -> Result<Option<(state::Device, [u8; 32], Cert)>, String> {
    let Some(device) = state.device().map_err(text)? else {
        return Ok(None);
    };
    let Link::Certified { master, cert } = device.link.clone() else {
        return Ok(None);
    };
    let cert = Cert::from_cbor(&cert)
        .map_err(|err| state::Error::Malformed(format!("the device's certificate: {err}")))
        .map_err(text)?;
    Ok(Some((device, master, cert)))
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
    match post_revision(connection, token, id, pending) {
        Sent::Taken => state.clear_pending(id, pending).map_err(text),
        Sent::Conflict(detail) | Sent::Refused(detail) => {
            state.remove_space(id).map_err(text)?;
            Err(format!(
                "the server refused revision 1 of the space {name}: {}; this device dropped the \
                 space",
                printable(&detail)
            ))
        }
        Sent::Failed(err) => Err(format!(
            "{err}; the space {name} waits for the server to take its revision 1: run skald \
             create-space {name} again"
        )),
    }
}

/// What the server did with a revision the device sent.
enum Sent {
    Taken,
    /// It holds another revision with its number, or none before it; with the server's detail.
    Conflict(String),
    /// It refuses the revision, saying why.
    Refused(String),
    /// It did not answer, or failed.
    Failed(String),
}

/// `POST /spaces/<id>/revisions`: sends `revision`, in CBOR, as the session `token`.
fn post_revision(connection: &mut Connection, token: &str, id: SpaceId, revision: &[u8]) -> Sent {
    let request = serde_json::json!({ "revision": URL_SAFE_NO_PAD.encode(revision) }).to_string();
    let path = format!("/spaces/{id}/revisions");
    match connection.send_json_as(Method::Post, &path, token, request.as_bytes()) {
        Ok(_) => Sent::Taken,
        Err(connection::Error::Refused {
            status: 409,
            detail,
        }) => Sent::Conflict(detail),
        Err(connection::Error::Refused {
            status: 400 | 403 | 404,
            detail,
        }) => Sent::Refused(detail),
        Err(err) => Sent::Failed(err.to_string()),
    }
}

/// Why a revision the device signed after revision 1 is not on the server.
pub struct Unsent {
    /// Whether it waits for a later command to send it again; otherwise the device dropped it.
    pub waits: bool,
    pub why: String,
}

/// What a signed-in machine with a certified device follows its spaces with: its local state, a
/// connection to the server of its session, the session token and its secrets, its account with
/// the master key it pinned, and its device.
pub struct Follower {
    pub state: State,
    pub connection: Connection,
    pub token: String,
    stored: secrets::Map,
    pub own: Own,
    device: state::Device,
    pub this: DeviceId,
}

impl Follower {
    /// The follower of the machine whose state is in `dir`, its secrets in `storage`; `None` if it
    /// has no certified device.
    fn new(
        dir: &Path,
        storage: secrets::Storage,
        speaks: VersionRange,
        timeout: Duration,
    ) -> Result<Option<Self>, String> {
        let (state, server, token, stored) = signed_in_with_secrets(dir, storage)?;
        let Some((device, master, cert)) = certified(&state)? else {
            return Ok(None);
        };
        let connection = Connection::new(&server, speaks, timeout).map_err(text)?;
        Ok(Some(Follower {
            state,
            connection,
            token,
            stored,
            own: Own {
                account: cert.body().account,
                master,
            },
            this: device.id,
            device,
        }))
    }

    /// The space named `name`, as the state holds it now.
    pub fn space(&self, name: &str) -> Result<state::Space, String> {
        self.state
            .spaces()
            .map_err(text)?
            .into_iter()
            .find(|space| space.name == name)
            .ok_or_else(|| {
                format!(
                    "this device follows no space named \"{}\": it follows the spaces it created",
                    printable(name)
                )
            })
    }

    /// Follows `space`: its last accepted revision and the devices of its accounts. A revision
    /// after revision 1 that the device signed is sent first, warning if it is not stored; while it
    /// waits, the space stays at its last accepted revision.
    pub fn sync(&mut self, space: &state::Space) -> Result<(Roster, SpaceDevices), String> {
        let waiting = space
            .pending
            .as_ref()
            .filter(|pending| **pending != space.anchor);
        if let Some(pending) = waiting
            && let Err(unsent) = self.store(space, pending.clone())
        {
            eprintln!("skald: warning: {}", unsent.why);
            if unsent.waits {
                let held = follow::read(&self.state, space.id, &space.name)?;
                let devices = follow::space_devices(&mut self.connection, &self.token, space.id)?;
                return Ok((follow::decode(&held.last, &space.name)?, devices));
            }
        }
        self.follow(space)
    }

    /// Accepts the revisions the server has of `space`: its last accepted revision and the devices
    /// of its accounts.
    fn follow(&mut self, space: &state::Space) -> Result<(Roster, SpaceDevices), String> {
        follow::sync(
            &mut self.connection,
            &self.state,
            &self.token,
            space,
            &self.own,
        )
    }

    /// Sends `pending`, in CBOR, the pending revision of `space` after revision 1, until the server
    /// takes it; gives the revision it took. When another revision came first, it accepts the new
    /// revisions and signs the channels `pending` adds into the latest, as the next pending one. It
    /// drops the pending revision when the server refuses it, when it accepts no revision that came
    /// first, or when the latest makes this device no admin's.
    pub fn store(&mut self, space: &state::Space, mut pending: Vec<u8>) -> Result<Roster, Unsent> {
        let (id, name) = (space.id, space.name.as_str());
        for _ in 0..follow::ATTEMPTS {
            let roster =
                follow::decode(&pending, name).map_err(|why| Unsent { waits: true, why })?;
            let number = roster.body().revision;
            let waits = |err: String| Unsent {
                waits: true,
                why: format!(
                    "{err}; revision {number} of the space {name}, which this device signed, waits \
                     for the server to take it: a later skald command on the space sends it again"
                ),
            };
            let why = match post_revision(&mut self.connection, &self.token, id, &pending) {
                Sent::Taken => {
                    self.state
                        .clear_pending(id, &pending)
                        .map_err(text)
                        .map_err(waits)?;
                    return Ok(roster);
                }
                Sent::Refused(detail) => {
                    format!("the server refused it: {}", printable(&detail))
                }
                Sent::Failed(err) => return Err(waits(err)),
                Sent::Conflict(_) => {
                    // Another revision came first.
                    let base = follow::read(&self.state, id, name)
                        .and_then(|held| follow::decode(&held.last, name))
                        .map_err(waits)?;
                    let (latest, _) = self.follow(space).map_err(waits)?;
                    if latest.hash() == base.hash() {
                        "the server holds another revision with its number, which this device \
                         does not accept"
                            .to_owned()
                    } else if !admin_device(&latest, self.own.account, self.this) {
                        no_admin(&latest)
                    } else {
                        let added: Vec<Channel> = roster
                            .body()
                            .channels
                            .iter()
                            .filter(|new| !base.body().channels.iter().any(|old| old.id == new.id))
                            .cloned()
                            .collect();
                        let next = self.next_with(&latest, &added).map_err(waits)?.to_cbor();
                        let set = self
                            .state
                            .set_pending(id, &latest.to_cbor(), Some(&pending), &next)
                            .map_err(text)
                            .map_err(waits)?;
                        if !set {
                            return Err(waits(CHANGED_MEANWHILE.to_owned()));
                        }
                        pending = next;
                        continue;
                    }
                }
            };
            self.state
                .clear_pending(id, &pending)
                .map_err(text)
                .map_err(waits)?;
            return Err(Unsent {
                waits: false,
                why: format!(
                    "this device dropped its revision {number} of the space {name}: {why}"
                ),
            });
        }
        Err(Unsent {
            waits: true,
            why: format!(
                "other admins kept changing the space {name} first; a later skald command on the \
                 space sends this device's change again"
            ),
        })
    }

    /// The revision after `latest`, with `channels` added, signed by this device.
    pub fn next_with(&self, latest: &Roster, channels: &[Channel]) -> Result<Roster, String> {
        let keys = device::known_keys(&self.stored, Some(&self.device))
            .map_err(text)?
            .ok_or_else(|| device::Error::MissingKeys.to_string())?;
        let mut body = latest.body().clone();
        body.revision = body
            .revision
            .checked_add(1)
            .ok_or_else(|| format!("the space {} has no revision after", body.name.as_str()))?;
        body.prev = Some(latest.hash());
        body.signer = self.this;
        body.timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        body.channels.extend_from_slice(channels);
        skald_crypto::sign_roster(&body, keys.signing())
            .map_err(|err| format!("revision {} of the space fails: {err}", body.revision))
    }
}

/// A space a command uses, once the device accepted the revisions the server has of it: the
/// follower, the space as the state held it, its last accepted revision and its accounts' devices.
pub struct Synced {
    pub follower: Follower,
    pub space: state::Space,
    pub roster: Roster,
    pub devices: SpaceDevices,
}

/// The space named `name` that the device of the machine whose state is in `dir`, its secrets in
/// `storage`, follows, synced ([`Follower::sync`]).
pub fn synced(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    name: &str,
) -> Result<Synced, String> {
    let mut follower =
        Follower::new(dir, storage, speaks, timeout)?.ok_or_else(|| NOT_FOLLOWING.to_owned())?;
    let space = follower.space(name)?;
    let (roster, devices) = follower.sync(&space)?;
    Ok(Synced {
        follower,
        space,
        roster,
        devices,
    })
}

/// Whether `device` is a device of an admin in `roster`: one of `account`, an admin in it, whose
/// `excluded` does not list it. The device's certificate is checked on accepting.
pub fn admin_device(roster: &Roster, account: AccountId, device: DeviceId) -> bool {
    roster.body().members.iter().any(|member| {
        member.account == account
            && member.role == Role::Admin
            && !member
                .excluded
                .iter()
                .flat_map(|ids| ids.iter())
                .any(|id| *id == device)
    })
}

/// Why this device, no device of an admin in `roster`, changes nothing in its space.
pub fn no_admin(roster: &Roster) -> String {
    let body = roster.body();
    format!(
        "this device is no device of an admin of the space {} in its revision {}, so it changes \
         nothing in it",
        body.name.as_str(),
        body.revision
    )
}

/// Lists the spaces the device of the machine whose state is in `dir`, its secrets in `storage`,
/// follows, each at its last accepted revision. A signed-in machine with a certified device first
/// accepts the revisions the server has of each; a space it fails to follow shows as the local
/// state holds it, saying why.
pub fn list(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<(), String> {
    let lines = if dir.is_dir() {
        let state = State::open(dir).map_err(text)?;
        let mut follower = match state.session().map_err(text)? {
            Some(_) => Follower::new(dir, storage, speaks, timeout).unwrap_or_else(|err| {
                eprintln!("skald: the spaces show their last accepted revisions: {err}");
                None
            }),
            None => None,
        };
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
        let mut lines = Vec::new();
        for space in state.spaces().map_err(text)? {
            let synced = follower.as_mut().map(|follower| follower.sync(&space));
            let roster = match synced {
                Some(Ok((roster, _))) => roster,
                Some(Err(err)) => {
                    eprintln!(
                        "skald: the space {} shows its last accepted revision: {err}",
                        space.name
                    );
                    follow::decode(&space.accepted, &space.name)?
                }
                None => follow::decode(&space.accepted, &space.name)?,
            };
            lines.push(line(&roster, account, space.pending.is_some()));
        }
        lines
    } else {
        Vec::new()
    };
    if lines.is_empty() {
        println!("This device is in no space yet; create one with skald create-space <name>.");
    }
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// Shows the members of `synced`, a space the device follows.
pub fn members(synced: Synced) -> Result<(), String> {
    let Synced {
        follower,
        roster,
        devices,
        ..
    } = synced;
    let (listing, warnings) = members_listing(&roster, &devices, &follower.own, follower.this);
    print!("{listing}");
    for warning in warnings {
        eprintln!("skald: warning: {warning}");
    }
    Ok(())
}

/// What `skald members` shows of `roster`, a space's last accepted revision, with `devices`, those
/// of its accounts: each member's handle and role, and under it each device of its account with a
/// valid certificate, as `skald devices` shows it, `this` device marked; and the warnings of
/// `skald devices` for them. A certificate is checked against the master key `own` pinned for its
/// own account, and against the `master` the revision gives any other.
fn members_listing(
    roster: &Roster,
    devices: &SpaceDevices,
    own: &Own,
    this: DeviceId,
) -> (String, Vec<String>) {
    let body = roster.body();
    let mut warnings: Vec<String> = Vec::new();
    let mut out = format!(
        "The space {} ({}), revision {}:\n",
        body.name.as_str(),
        body.space,
        body.revision
    );
    for member in body.members.iter() {
        out.push_str(&format!(
            "  {}  {}\n",
            member.handle.as_str(),
            role(member.role)
        ));
        let master = if member.account == own.account {
            own.master
        } else {
            member.master
        };
        let checked = devices::check(&master, &devices.ids, &devices.certs);
        for warning in devices::two_bodies_warnings(&checked) {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        let linked: Vec<&Cert> = checked
            .linked
            .iter()
            .copied()
            .filter(|cert| cert.body().account == member.account)
            .collect();
        warnings.extend(devices::shared_name_warnings(&linked));
        if linked.is_empty() {
            out.push_str("    no device with a valid certificate\n");
        }
        for cert in linked {
            out.push_str(&format!("    {}\n", devices::linked_line(cert, this)));
        }
    }
    (out, warnings)
}

/// A role, as the CLI shows it.
fn role(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Member => "member",
    }
}

/// The line of a space whose last accepted revision is `roster`: its name, the role and handle of
/// `account` in it, and its ID; marked if the server has not taken the revision yet.
fn line(roster: &Roster, account: Option<AccountId>, pending: bool) -> String {
    let body = roster.body();
    let member =
        account.and_then(|account| body.members.iter().find(|member| member.account == account));
    let (role, handle) = match member {
        Some(member) => (role(member.role), member.handle.as_str()),
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

    /// The certificate of the device of `key` on `account`, named `name`, signed by `master`.
    fn cert(account: AccountId, master: &SigningKey, key: &SigningKey, name: &str) -> Cert {
        let body = skald_proto::CertBody {
            account,
            device: DeviceId::from_signing_key(&key.public_key()),
            kind: skald_proto::DeviceKind::Client,
            signing: key.public_key(),
            hpke: [4; 32],
            issuer: None,
            issued: 1_800_000_000,
            ssh: None,
            name: name.to_owned(),
            unknown: skald_proto::Unknown::default(),
        };
        skald_crypto::issue(&body, master, key).expect("a certificate")
    }

    /// The accounts of Ada, who created `acme`, and Bob, with their master keys.
    struct Pair {
        ada: AccountId,
        bob: AccountId,
        ada_master: SigningKey,
        bob_master: SigningKey,
    }

    impl Pair {
        fn new() -> Self {
            Pair {
                ada: AccountId::generate(),
                bob: AccountId::generate(),
                ada_master: SigningKey::generate(),
                bob_master: SigningKey::generate(),
            }
        }

        /// Revision 2 of `acme`, which adds Bob as a member. Its revision 1 gives Ada another
        /// master key than the one Ada's device pinned.
        fn roster(&self) -> Roster {
            let mut body = first(self.ada).body().clone();
            body.revision = 2;
            body.members.push(skald_proto::Member {
                account: self.bob,
                handle: "bob".parse().unwrap(),
                role: Role::Member,
                master: self.bob_master.public_key(),
                invite: None,
                excluded: None,
                unknown: skald_proto::Unknown::default(),
            });
            skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap()
        }

        /// What Ada's device trusts revisions by.
        fn own(&self) -> Own {
            Own {
                account: self.ada,
                master: self.ada_master.public_key(),
            }
        }

        /// What `skald members` on Ada's device `this` gives of `roster` with `certs`.
        fn listing(
            &self,
            roster: &Roster,
            certs: &[Cert],
            this: DeviceId,
        ) -> (String, Vec<String>) {
            let devices = SpaceDevices {
                ids: certs.iter().map(|cert| cert.body().device).collect(),
                certs: certs.to_vec(),
            };
            members_listing(roster, &devices, &self.own(), this)
        }
    }

    #[test]
    fn members_show_each_members_devices_with_a_valid_certificate_under_it() {
        let pair = Pair::new();
        let (ada, bob) = (pair.ada, pair.bob);
        let device = |account, master: &SigningKey, name: &str| {
            cert(account, master, &SigningKey::generate(), name)
        };
        let certs = vec![
            device(ada, &pair.ada_master, "ada-laptop"),
            device(bob, &pair.bob_master, "bob-laptop"),
            // On Bob's account, by a key that is not its master key.
            device(bob, &SigningKey::generate(), "bob-forged"),
            // By Bob's master key, on Ada's account.
            device(ada, &pair.bob_master, "ada-forged"),
        ];
        let roster = pair.roster();
        let body = roster.body();
        let this = certs[0].body().device;

        let shown = pair.listing(&roster, &certs, this);

        assert_eq!(
            shown,
            (
                format!(
                    "The space acme ({}), revision 2:\n  ada  admin\n    {}\n  bob  member\n    {}\n",
                    body.space,
                    devices::linked_line(&certs[0], this),
                    devices::linked_line(&certs[1], this)
                ),
                vec![]
            )
        );
        assert_eq!(
            pair.listing(&roster, &[], this),
            (
                format!(
                    "The space acme ({}), revision 2:\n  ada  admin\n    no device with a valid \
                     certificate\n  bob  member\n    no device with a valid certificate\n",
                    body.space
                ),
                vec![]
            )
        );
    }

    #[test]
    fn members_warn_of_a_device_with_two_bodies_and_of_one_accounts_devices_with_one_name() {
        let pair = Pair::new();
        let (ada, bob) = (pair.ada, pair.bob);
        let keys: Vec<SigningKey> = (0..4).map(|_| SigningKey::generate()).collect();
        let certs = vec![
            cert(ada, &pair.ada_master, &keys[0], "laptop"),
            cert(ada, &pair.ada_master, &keys[1], "laptop"),
            // Bob's device of the same name as Ada's: another account's.
            cert(bob, &pair.bob_master, &keys[2], "laptop"),
            // Two bodies of one device of Bob's.
            cert(bob, &pair.bob_master, &keys[3], "phone"),
            cert(bob, &pair.bob_master, &keys[3], "tablet"),
        ];
        let ids: Vec<DeviceId> = certs.iter().map(|cert| cert.body().device).collect();

        let (shown, warnings) = pair.listing(&pair.roster(), &certs, ids[0]);

        assert!(!shown.contains(&ids[3].to_string()), "{shown}");
        assert_eq!(
            warnings,
            vec![
                format!(
                    "the key of device {} signed two different certificates of it, so neither \
                     counts; the device or its key may be compromised",
                    ids[3]
                ),
                format!(
                    "linked devices {}, {} have the same name \"laptop\"; tell them apart by ID",
                    ids[0], ids[1]
                ),
            ]
        );
    }

    #[test]
    fn admin_device_is_one_of_an_admin_account_that_its_excluded_does_not_list() {
        let (ada, bob) = (AccountId::generate(), AccountId::generate());
        let device = |seed| DeviceId::from_signing_key(&[seed; 32]);
        let mut body = first(ada).body().clone();
        body.members.push(skald_proto::Member {
            account: bob,
            handle: "bob".parse().unwrap(),
            role: Role::Member,
            master: [5; 32],
            invite: None,
            excluded: None,
            unknown: skald_proto::Unknown::default(),
        });
        let roster = |body: &RosterBody| skald_crypto::sign_roster(body, &SigningKey::generate());
        let unexcluded = roster(&body).unwrap();
        let mut members = body.members.clone().into_vec();
        for member in &mut members {
            member.excluded = skald_proto::NonEmpty::from_vec(vec![device(1)]);
        }
        body.members = skald_proto::NonEmpty::from_vec(members).unwrap();
        let excluded = roster(&body).unwrap();

        assert!(admin_device(&unexcluded, ada, device(1)));
        assert!(!admin_device(&unexcluded, bob, device(1)), "a member");
        assert!(!admin_device(&unexcluded, AccountId::generate(), device(1)));
        assert!(!admin_device(&excluded, ada, device(1)), "excluded");
        assert!(admin_device(&excluded, ada, device(2)));
        assert!(!admin_device(&excluded, bob, device(2)), "a member");
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
