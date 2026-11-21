//! `skald create-space`, `skald spaces` and `skald members`: a team space the device creates and
//! signs revision 1 of as its first admin, the spaces the device follows and a space's members
//! (`docs/protocol/membership.md`, "Roster"). The local state keeps revision 1 as the space's anchor
//! and last accepted revision; it is written before it is sent and sent again on later runs until
//! the server takes it. Listing spaces or their members first accepts the revisions the server has
//! ([`crate::follow`]).
//!
//! The account's first device signs the revision 1 of the account's personal space the same way,
//! once the server has taken its certificate: at its sign-in, or at any later command that uses
//! spaces. It pins that space as its personal space when it signs it, and never takes another from
//! the server.
//!
//! A later revision the device signs is written before it is sent too. Any command that uses the
//! space sends it again first, until the server takes it; only then does it count as accepted, and
//! until then the space stays at its last accepted revision. When another revision came first, the
//! device accepts the new revisions, applies its change to the latest and signs again.
//!
//! Each revision the device signs after revision 1 drops the `service` devices the revision before
//! it lists that are revoked or on the account of no member of the new one.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_proto::{
    AccountId, Anchors, Cbor as _, Cert, Channel, ChannelId, ChannelMember, DeviceId, DeviceKind,
    Member, Name, NonEmpty, Role, Roster, RosterBody, ServiceAgent, SpaceId, Unknown, VersionRange,
};

use crate::anchors;
use crate::connection::{self, Connection, Method, printable};
use crate::device;
use crate::devices::{self, signed_in_with_secrets, text, to_session};
use crate::folders;
use crate::follow::{self, Own, SpaceDevices};
use crate::groups;
use crate::key_packages;
use crate::mailbox::{Mailbox, Transport};
use crate::revocations;
use crate::secrets;
use crate::service;
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

/// What a run within [`Scope::Only`] says of another space, alike for every space it is given.
pub const OUT_OF_SCOPE: &str = "as an agent, skald shows only the space of the agent's folder; \
                                skald --agent <name> spaces lists it";

/// The account's personal space, `POST`.
const PERSONAL_PATH: &str = "/account/personal-space";

/// The problem of a creation beyond the spaces an account may own.
const OWNED_SPACES_LIMIT: &str = "https://skald.so/problems/owned-spaces-limit";

/// What a run says of the server's answer to a creation or a request for the personal space that
/// gives no space.
const NO_SPACE_GIVEN: &str = "unexpected answer from the server: a space without its ID or name";

/// When the personal space's revision 1 is sent again.
const PERSONAL_AGAIN: &str = "a later skald command sends it again";

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
    let mut follower =
        Follower::new(dir, storage, speaks, timeout)?.ok_or_else(|| NOT_LINKED.to_owned())?;
    let keys = follower.keys()?;
    let again = format!("run skald create-space {} again", name.as_str());

    let mut existing = None;
    for space in follower.state.spaces().map_err(text)? {
        // A later revision waits for a command that uses its space.
        if let Some(pending) = space.pending.as_ref().filter(|_| space.first_waits()) {
            follower.send_first((space.id, &space.name), pending, &again)?;
        }
        if space.name == name.as_str() {
            existing = Some(space);
        }
    }
    if let Some(space) = existing {
        let roster = follow::decode(&space.anchor, &space.name)?;
        println!("{}", created(&roster, !space.first_waits()));
        finish(&mut follower, &space.name);
        return Ok(());
    }

    let handle = handle(
        &mut follower.connection,
        &follower.token,
        follower.own.account,
    )?;
    let space = create_on_server(&mut follower.connection, &follower.token, &name)?;
    let roster = follower.sign_first(&keys, space, name.clone(), handle)?;
    let bytes = roster.to_cbor();
    follower
        .state
        .add_space(space, name.as_str(), &bytes)
        .map_err(text)?;
    follower.send_first((space, name.as_str()), &bytes, &again)?;
    println!("{}", created(&roster, false));
    finish(&mut follower, name.as_str());
    Ok(())
}

/// Accepts the revisions of the space named `name`, which signs its default channel, and brings
/// the groups of its channels in line ([`crate::groups`]); warns of what fails.
pub fn finish(follower: &mut Follower, name: &str) {
    match follower.space(name).and_then(|space| follower.sync(&space)) {
        Ok(_) => groups::align_accepted(follower),
        Err(err) => eprintln!("skald: warning: {err}"),
    }
}

/// After `skald sign-in` on the machine whose state is in `dir`, its secrets in `storage`: takes
/// the account's personal space if the device holds its certificate
/// ([`Follower::take_personal`]), saying so, or warns why it did not.
pub fn take_personal(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
) {
    let taken = Follower::open(dir, storage, speaks, timeout).and_then(|follower| match follower {
        Some(mut follower) => follower.take_personal(),
        None => Ok(None),
    });
    match taken {
        Ok(Some(first)) => println!("{}", personal_taken(&first)),
        Ok(None) => {}
        Err(err) => personal_warning(&err),
    }
}

/// Warns that the account's personal space is not set up, for the reason `err`.
fn personal_warning(err: &str) {
    eprintln!("skald: warning: the account's personal space is not set up yet: {err}");
}

/// What the device says when it signed `first`, revision 1 of the account's personal space.
fn personal_taken(first: &Roster) -> String {
    let body = first.body();
    let handle = creator_handle(first);
    format!(
        "This device signed revision 1 of the account's personal space {} ({}) as its admin, with \
         the handle {handle}.",
        body.name.as_str(),
        body.space
    )
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
    let handle = creator_handle(first);
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

/// The handle of the creator of the space whose revision 1 is `first`.
fn creator_handle(first: &Roster) -> &str {
    first
        .body()
        .members
        .first()
        .map_or("", |member| member.handle.as_str())
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
        Err(connection::Error::Refused { kind, .. }) if kind == OWNED_SPACES_LIMIT => {
            return Err(
                "the account owns as many spaces as it may on this server; no space is created"
                    .to_owned(),
            );
        }
        Err(connection::Error::Refused {
            status: 400 | 409,
            detail,
            ..
        }) => {
            return Err(format!(
                "the server refused the space name \"{}\": {}; no space is created",
                name.as_str(),
                printable(&detail)
            ));
        }
        Err(err) => return Err(err.to_string()),
    };
    let created: Created = serde_json::from_slice(&body).map_err(|_| NO_SPACE_GIVEN)?;
    if created.name != name.as_str() {
        return Err(NO_SPACE_GIVEN.to_owned());
    }
    created.space.parse().map_err(|_| NO_SPACE_GIVEN.to_owned())
}

/// `POST /account/personal-space`: the account's personal space, as the server gives it to the
/// session `token`.
fn personal_on_server(connection: &mut Connection, token: &str) -> Result<(SpaceId, Name), String> {
    let body = connection.post_as(PERSONAL_PATH, token).map_err(text)?;
    let given: Created = serde_json::from_slice(&body).map_err(|_| NO_SPACE_GIVEN)?;
    let id = given.space.parse().map_err(|_| NO_SPACE_GIVEN)?;
    let name = given.name.parse().map_err(|err| {
        format!(
            "unexpected answer from the server: the space name \"{}\" {err}",
            printable(&given.name)
        )
    })?;
    Ok((id, name))
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
            ..
        }) => Sent::Conflict(detail),
        Err(connection::Error::Refused {
            status: 400 | 403 | 404,
            detail,
            ..
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
    pub secrets: secrets::Secrets,
    pub own: Own,
    device: state::Device,
    pub this: DeviceId,
    /// The device's certificate.
    pub cert: Cert,
    /// How long a run sends an envelope again while the server has not answered it.
    pub timeout: Duration,
    /// Each space the run accepted the revisions of, as it last did ([`Follower::sync`]).
    pub accepted: groups::Rosters,
    /// The transport the device's next connection tries first ([`crate::mailbox`]).
    pub transport: Transport,
    /// Whether the run has kept the device's KeyPackages on the server.
    kept: bool,
    /// Whether a send of the device's anchors object failed in this run ([`anchors::keep`]).
    anchors_unsent: bool,
    /// The revisions the server last gave of each space the run followed or synced.
    pub given: BTreeMap<SpaceId, Vec<Roster>>,
    /// For a service agent's run ([`crate::service`]), its space and the key secret's anchor.
    pub service: Option<service::Member>,
    /// Whether the run covers every space the device follows ([`Scope::All`]), so its warnings of
    /// master keys compare them ([`follow::master_warnings`]); otherwise only the synced one's.
    every_space: bool,
}

impl Follower {
    /// The follower of the machine whose state is in `dir`, its secrets in `storage`; `None` if it
    /// has no certified device. It takes the account's personal space first
    /// ([`Follower::take_personal`]), warning if it cannot.
    fn new(
        dir: &Path,
        storage: secrets::Storage,
        speaks: VersionRange,
        timeout: Duration,
    ) -> Result<Option<Self>, String> {
        let mut follower = Self::open(dir, storage, speaks, timeout)?;
        if let Some(follower) = follower.as_mut() {
            follower.every_space = true;
            if let Err(err) = follower.take_personal() {
                personal_warning(&err);
            }
        }
        Ok(follower)
    }

    /// As [`Follower::new`], without taking the personal space. Once the revocations the device
    /// signed are sent, it fails if the device is revoked ([`revocations::refuse_if_revoked`]), so
    /// a revoked device signs and sends nothing; otherwise it keeps its anchors object
    /// ([`Follower::keep_anchors`]).
    fn open(
        dir: &Path,
        storage: secrets::Storage,
        speaks: VersionRange,
        timeout: Duration,
    ) -> Result<Option<Self>, String> {
        let mut follower = Self::unchecked(dir, storage, speaks, timeout)?;
        if let Some(follower) = follower.as_mut() {
            revocations::refuse_if_revoked(
                &mut follower.connection,
                &follower.token,
                &follower.state,
                (&follower.own.master, &follower.cert),
            )?;
            follower.keep_anchors();
        }
        Ok(follower)
    }

    /// Keeps the device's anchors object of what the state holds on the server
    /// ([`anchors::keep`]), unless it is a `service` device, which has none; warns of what fails.
    fn keep_anchors(&mut self) {
        if self.service.is_some() {
            return;
        }
        match self.keys() {
            Ok(keys) => anchors::keep(
                &mut self.connection,
                &self.token,
                &self.state,
                &anchors::Signer {
                    account: self.own.account,
                    device: self.this,
                    keys: &keys,
                },
                &mut self.anchors_unsent,
            ),
            Err(err) => eprintln!("skald: warning: {err}"),
        }
    }

    /// As [`Follower::open`], without checking whether the device is revoked.
    pub(crate) fn unchecked(
        dir: &Path,
        storage: secrets::Storage,
        speaks: VersionRange,
        timeout: Duration,
    ) -> Result<Option<Self>, String> {
        let (state, server, token, stored, secrets) = signed_in_with_secrets(dir, storage)?;
        let Some((device, master, cert)) = certified(&state)? else {
            return Ok(None);
        };
        let mut connection = to_session(&server, speaks, timeout)?;
        revocations::send_pending(&mut connection, &token, &state);
        Ok(Some(Follower {
            state,
            connection,
            token,
            stored,
            secrets,
            own: Own {
                account: cert.body().account,
                master,
            },
            this: device.id,
            device,
            cert,
            timeout,
            accepted: groups::Rosters::new(),
            transport: Transport::default(),
            kept: false,
            anchors_unsent: false,
            given: BTreeMap::new(),
            service: None,
            every_space: false,
        }))
    }

    /// The follower of `agent`, a service agent's run, whose state is in `dir`, which it creates
    /// if it is missing, or in memory only for a stateless run ([`crate::service`]): its keys and
    /// state key come from the key secret, its token is its session, and it follows the key
    /// secret's space from the key secret's anchor. Fails if the device's certificate is not valid
    /// against the pinned master key, if the state is of another device, and, for a state the
    /// device has not run with yet, if the device has been in a group
    /// ([`service::refuse_without_state`]), recording nothing.
    fn service(
        dir: &Path,
        agent: &service::Agent,
        speaks: VersionRange,
        timeout: Duration,
    ) -> Result<Self, String> {
        let key = &agent.key;
        let state = if agent.stateless {
            State::in_memory().map_err(text)?
        } else {
            let unusable = |err: &dyn std::fmt::Display| {
                format!("cannot use the state directory {}: {err}", dir.display())
            };
            state::create_dir(dir).map_err(|err| unusable(&err))?;
            let dir = dir.canonicalize().map_err(|err| unusable(&err))?;
            State::open(&dir).map_err(text)?
        };
        let keys = agent.keys();
        let this = keys.id();
        let held = state.device().map_err(text)?;
        if held.as_ref().is_some_and(|held| held.id != this) {
            return Err(format!(
                "the state directory {} holds the state of another device than the key secret's: \
                 give each service agent a state directory of its own",
                dir.display()
            ));
        }
        let mut connection = to_session(&key.server, speaks, timeout)?;
        let token = agent.token.to_string();
        let devices = follow::space_devices(&mut connection, &token, &state, key.space)?;
        let cert = skald_crypto::check_valid_device(
            &key.master,
            &devices.certs,
            &devices.revocations,
            this,
        )
        .map_err(|err| {
            format!(
                "the service device {this} of the key secret has no valid certificate: {err}; \
                 nothing is sent"
            )
        })?
        .clone();
        if cert.body().kind != DeviceKind::Service || cert.body().account != key.account {
            return Err(format!(
                "the device {this} of the key secret is no service device of its account; \
                 nothing is sent"
            ));
        }
        if held.is_none() {
            let given = follow::given(&mut connection, &token, key.space)?;
            let anchor = (given.iter())
                .find(|roster| roster.hash() == key.anchor)
                .ok_or_else(|| {
                    format!(
                        "the server gives no revision of the space {} that is the key secret's \
                         anchor; nothing is sent",
                        key.space
                    )
                })?;
            if let Some(latest) = given.iter().max_by_key(|roster| roster.body().revision) {
                service::refuse_without_state(&mut connection, &token, latest, this)?;
            }
            state.start_session(&key.server).map_err(text)?;
            let account = key.account.to_string();
            state
                .add_device(this, &cert.body().name, &account)
                .map_err(text)?;
            state.set_registered().map_err(text)?;
            state
                .set_link(&Link::Certified {
                    master: key.master,
                    cert: cert.to_cbor(),
                })
                .map_err(text)?;
            let name = anchor.body().name.as_str();
            state
                .add_linked_space(key.space, name, &anchor.to_cbor(), false)
                .map_err(text)?;
        }
        let device = state
            .device()
            .map_err(text)?
            .ok_or_else(|| "the local state holds no device".to_owned())?;
        let (stored, secrets) = agent.secrets(&keys);
        Ok(Follower {
            state,
            connection,
            token,
            stored,
            secrets,
            own: agent.own(),
            this,
            device,
            cert,
            timeout,
            accepted: groups::Rosters::new(),
            transport: Transport::default(),
            kept: false,
            anchors_unsent: false,
            given: BTreeMap::new(),
            service: Some(service::Member {
                space: key.space,
                anchor: key.anchor,
                stateless: agent.stateless,
            }),
            every_space: false,
        })
    }

    /// Whether the run is a stateless service agent's ([`crate::service`]).
    pub fn stateless(&self) -> bool {
        self.service.is_some_and(|member| member.stateless)
    }

    /// A connection to the server as the device, proving its `keys` ([`Mailbox::open`]), once the
    /// device kept its KeyPackages on the server, warning if it cannot; a run keeps them once. A
    /// stateless run keeps none: it would lose their private keys.
    pub fn connect(&mut self, keys: &device::Keys) -> Result<Mailbox, String> {
        if !self.kept && !self.stateless() {
            let kept = key_packages::keep(
                &mut self.connection,
                &self.token,
                (&self.state, &self.secrets),
                keys,
                self.service.is_some(),
            );
            match kept {
                Ok(()) => self.kept = true,
                Err(err) => key_packages::warn(&err),
            }
        }
        Mailbox::open(self, keys)
    }

    /// `GET /account/anchors` with the device's proof: the anchors objects of the account's
    /// devices, as a service token reads them.
    fn anchors_objects(&mut self) -> Result<Vec<Anchors>, String> {
        let keys = self.keys()?;
        let proof = self.proof(&keys)?;
        let body = (self.connection)
            .get_proved(anchors::PATH, (&self.token, &proof))
            .map_err(|err| format!("the account's anchors objects are not read: {}", text(err)))?;
        anchors::objects(&body)
    }

    /// The proof of an HTTPS request of the device, whose keys are `keys`, on a new challenge of
    /// its session: the value of its `Skald-Device-Proof` header.
    pub fn proof(&mut self, keys: &device::Keys) -> Result<String, String> {
        (self.connection)
            .request_proof(&self.token, keys.id(), keys.signing())
            .map_err(text)
    }

    /// The device's keys.
    pub fn keys(&self) -> Result<device::Keys, String> {
        device::known_keys(&self.stored, Some(&self.device))
            .map_err(text)?
            .ok_or_else(|| device::Error::MissingKeys.to_string())
    }

    /// Revision 1 of the new space `space` named `name`, signed by this device with `keys`, its
    /// account the only member and admin, with `handle`, checked as a new space's revision.
    fn sign_first(
        &self,
        keys: &device::Keys,
        space: SpaceId,
        name: Name,
        handle: Name,
    ) -> Result<Roster, String> {
        let body = RosterBody::first(
            space,
            name,
            self.own.account,
            handle,
            self.own.master,
            self.this,
            now(),
        );
        let fails = |err: String| format!("revision 1 of the space fails: {err}");
        let roster = skald_crypto::sign_roster(&body, keys.signing())
            .map_err(|err| fails(err.to_string()))?;
        skald_crypto::check_roster(&roster, std::slice::from_ref(&self.cert))
            .map_err(|err| fails(err.to_string()))?;
        Ok(roster)
    }

    /// `POST /spaces/<id>/revisions`: sends `pending`, revision 1 of `space`, its ID and name,
    /// which the device signed, and records that the server took it; while it waits, `again` says
    /// when it is sent again. A revision the server refuses is dropped with its space, which the
    /// server never took a revision of.
    fn send_first(
        &mut self,
        (id, name): (SpaceId, &str),
        pending: &[u8],
        again: &str,
    ) -> Result<(), String> {
        match post_revision(&mut self.connection, &self.token, id, pending) {
            Sent::Taken => self.state.clear_pending(id, pending).map_err(text),
            Sent::Conflict(detail) | Sent::Refused(detail) => {
                self.state.remove_space(id).map_err(text)?;
                Err(format!(
                    "the server refused revision 1 of the space {name}: {}; this device dropped \
                     the space",
                    printable(&detail)
                ))
            }
            Sent::Failed(err) => Err(format!(
                "{err}; the space {name} waits for the server to take its revision 1: {again}"
            )),
        }
    }

    /// Takes the account's personal space, unless the device pinned one already, whose revision 1
    /// it sends again while the server has not taken it. Otherwise it gets the space from the
    /// server and, unless another device stored its revision 1, signs that revision, pins the
    /// space, stores the revision as `skald create-space` does and keeps the device's anchors
    /// object ([`Follower::keep_anchors`]). Gives the revision 1 it signed.
    fn take_personal(&mut self) -> Result<Option<Roster>, String> {
        let spaces = self.state.spaces().map_err(text)?;
        if let Some(space) = spaces.iter().find(|space| space.personal) {
            if space.first_waits() {
                self.send_first((space.id, &space.name), &space.anchor, PERSONAL_AGAIN)?;
            }
            return Ok(None);
        }
        let keys = self.keys()?;
        let (id, name) = personal_on_server(&mut self.connection, &self.token)?;
        if spaces.iter().any(|space| space.id == id) {
            return Err(format!(
                "the server gives the space {} ({id}), which this device follows already, as the \
                 account's personal space",
                name.as_str()
            ));
        }
        // A device that signed none of its revisions gets the space from one that did.
        if !follow::listed(&mut self.connection, &self.token, id)?.is_empty() {
            return Ok(None);
        }
        let handle = handle(&mut self.connection, &self.token, self.own.account)?;
        let first = self.sign_first(&keys, id, name.clone(), handle)?;
        let bytes = first.to_cbor();
        self.state
            .add_personal_space(id, name.as_str(), &bytes)
            .map_err(text)?;
        self.send_first((id, name.as_str()), &bytes, PERSONAL_AGAIN)?;
        self.keep_anchors();
        Ok(Some(first))
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
        let synced = self.sync_once(space)?;
        self.accepted.insert(space.id, synced.clone());
        Ok(synced)
    }

    /// As [`Follower::sync`], recording nothing in [`Follower::accepted`].
    fn sync_once(&mut self, space: &state::Space) -> Result<(Roster, SpaceDevices), String> {
        let waiting = space.pending.as_ref().filter(|_| !space.first_waits());
        if let Some(pending) = waiting
            && let Err(unsent) = self.store(space, pending.clone())
        {
            eprintln!("skald: warning: {}", unsent.why);
            if unsent.waits {
                let held = follow::read(&self.state, space.id, &space.name)?;
                let devices = follow::space_devices(
                    &mut self.connection,
                    &self.token,
                    &self.state,
                    space.id,
                )?;
                let given = follow::given(&mut self.connection, &self.token, space.id)?;
                self.given.insert(space.id, given);
                return Ok((follow::decode(&held.last, &space.name)?, devices));
            }
        }
        let (roster, devices, _) = self.follow(space)?;
        match self.add_default(space, &roster, &devices) {
            Ok(true) => self
                .follow(space)
                .map(|(roster, devices, _)| (roster, devices)),
            Ok(false) => Ok((roster, devices)),
            Err(why) => {
                eprintln!("skald: warning: {why}");
                Ok((roster, devices))
            }
        }
    }

    /// On a device of an admin, signs and stores the revision after `latest`, the last accepted
    /// revision of `space`, with a new default channel first, if `latest` needs one
    /// ([`needs_default`]) and no other revision of this device waits; gives whether the server
    /// took it.
    fn add_default(
        &mut self,
        space: &state::Space,
        latest: &Roster,
        devices: &SpaceDevices,
    ) -> Result<bool, String> {
        if !admin_device(latest, self.own.account, self.this) || !needs_default(latest) {
            return Ok(false);
        }
        let waits = self
            .state
            .spaces()
            .map_err(text)?
            .into_iter()
            .any(|held| held.id == space.id && held.pending.is_some());
        if waits {
            return Ok(false);
        }
        let name = &space.name;
        let id = issue_channel(&mut self.connection, &self.token, space.id)
            .map_err(|err| format!("the space {name} gets no default channel yet: {err}"))?;
        let default = default_channel(id, latest.body());
        let next = self.next_with((latest, devices), Some(default), &[])?;
        self.submit(space, latest, &next)?;
        Ok(true)
    }

    /// Accepts the revisions the server has of `space`: its last accepted revision, the devices of
    /// its accounts and the revisions the server gives, which it records in [`Follower::given`].
    /// Then it keeps the device's anchors object ([`Follower::keep_anchors`]).
    fn follow(
        &mut self,
        space: &state::Space,
    ) -> Result<(Roster, SpaceDevices, Vec<Roster>), String> {
        let start = match self.service {
            Some(member) if member.space == space.id => Some(follow::Start {
                anchor: member.anchor,
                objects: self.anchors_objects()?,
            }),
            _ => None,
        };
        let followed = follow::sync(
            &mut self.connection,
            (&self.state, &self.token),
            space,
            (&self.own, self.every_space),
            start.as_ref(),
        )?;
        self.given.insert(space.id, followed.2.clone());
        self.keep_anchors();
        Ok(followed)
    }

    /// Sends `pending`, in CBOR, the pending revision of `space` after revision 1, until the server
    /// takes it; gives the revision it took. When another revision came first, it accepts the new
    /// revisions and signs what `pending` changes into the latest, as the next pending one
    /// ([`Change`]). It drops the pending revision when the server refuses it, when it accepts no
    /// revision that came first, when the latest makes this device no admin's or lacks a member
    /// whose role the change sets, or when the change signed into the latest fails the checks of
    /// accepting it; and, giving the latest, when nothing of it is left to change.
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
                    // Another revision came first. The change is what `pending` makes in the
                    // revision it was signed on, which the state may have accepted others after.
                    let (latest, devices, given) = self.follow(space).map_err(waits)?;
                    let prev = roster.body().prev;
                    let base = given
                        .into_iter()
                        .find(|given| Some(given.hash()) == prev)
                        .unwrap_or_else(|| latest.clone());
                    let change = Change::between(base.body(), roster.body());
                    if latest.hash() == base.hash() {
                        "the server gives no revision this device accepts in its place, or not \
                         the one it was signed on"
                            .to_owned()
                    } else if !admin_device(&latest, self.own.account, self.this) {
                        no_admin(&latest)
                    } else if let Some(why) = change.role_gone(roster.body(), &latest) {
                        why
                    } else {
                        let mut body = latest.body().clone();
                        change.onto(&mut body, needs_default(&latest));
                        if body == *latest.body() {
                            self.state
                                .clear_pending(id, &pending)
                                .map_err(text)
                                .map_err(waits)?;
                            return Ok(latest);
                        }
                        let next = self
                            .next_by((&latest, &devices), |next| {
                                next.members = body.members;
                                next.service_agents = body.service_agents;
                                next.channels = body.channels;
                                next.policy = body.policy;
                            })
                            .map_err(waits)?;
                        match self.accepts(&latest, &next, &devices.certs) {
                            Ok(()) => {
                                let next = next.to_cbor();
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
                            Err(err) => format!("signed into the latest revision, {err}"),
                        }
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

    /// Fails unless this device may sign a change into `latest`, the last accepted revision of
    /// `space`: no revision it signed waits for the server, and it is a device of an admin.
    pub fn may_change(&self, space: &state::Space, latest: &Roster) -> Result<(), String> {
        if self.space(&space.name)?.pending.is_some() {
            return Err(format!(
                "the space {} waits for the server to take a revision this device signed; this \
                 device changes nothing in it until then",
                space.name
            ));
        }
        if !admin_device(latest, self.own.account, self.this) {
            return Err(no_admin(latest));
        }
        Ok(())
    }

    /// Writes `next`, the revision after `latest` that this device signed, as the pending revision
    /// of `space`, then sends it until the server takes it ([`Follower::store`]); gives the
    /// revision the server took.
    pub fn submit(
        &mut self,
        space: &state::Space,
        latest: &Roster,
        next: &Roster,
    ) -> Result<Roster, String> {
        self.submitted(space, latest, next)
            .map_err(|unsent| unsent.why)
    }

    /// As [`Follower::submit`]; when the server took no revision, says whether one waits.
    pub fn submitted(
        &mut self,
        space: &state::Space,
        latest: &Roster,
        next: &Roster,
    ) -> Result<Roster, Unsent> {
        let next = next.to_cbor();
        let dropped = |why: String| Unsent { waits: false, why };
        if !self
            .state
            .set_pending(space.id, &latest.to_cbor(), None, &next)
            .map_err(text)
            .map_err(dropped)?
        {
            return Err(dropped(CHANGED_MEANWHILE.to_owned()));
        }
        self.store(space, next)
    }

    /// Checks `next`, the revision after `latest` that this device signed, as the members of
    /// `space` accept it with `certs` ([`Follower::accepts`]), then sends it as
    /// [`Follower::submit`] does; gives the revision the server took.
    pub fn checked_submit(
        &mut self,
        space: &state::Space,
        (latest, next): (&Roster, &Roster),
        certs: &[Cert],
    ) -> Result<Roster, String> {
        self.accepts(latest, next, certs)
            .map_err(|err| format!("{err}; nothing is sent"))?;
        self.submit(space, latest, next)
    }

    /// The revision after `latest`, with `first` added before its channels and `channels` after
    /// them, signed by this device, as [`Follower::next_by`] signs it with `devices`.
    pub fn next_with(
        &self,
        latest: (&Roster, &SpaceDevices),
        first: Option<Channel>,
        channels: &[Channel],
    ) -> Result<Roster, String> {
        self.next_by(latest, |body| {
            body.channels.splice(0..0, first);
            body.channels.extend_from_slice(channels);
        })
    }

    /// The revision after `latest` with what `edit` changes in its body, signed by this device,
    /// without the `service` devices that `devices`, those of the space's accounts, show as stale
    /// ([`drop_stale_services`]).
    pub fn next_by(
        &self,
        (latest, devices): (&Roster, &SpaceDevices),
        edit: impl FnOnce(&mut RosterBody),
    ) -> Result<Roster, String> {
        let keys = self.keys()?;
        let mut body = latest.body().clone();
        body.revision = body
            .revision
            .checked_add(1)
            .ok_or_else(|| format!("the space {} has no revision after", body.name.as_str()))?;
        body.prev = Some(latest.hash());
        body.signer = self.this;
        body.timestamp = now();
        edit(&mut body);
        drop_stale_services(&mut body, latest, &self.own, devices);
        skald_crypto::sign_roster(&body, keys.signing())
            .map_err(|err| format!("revision {} of the space fails: {err}", body.revision))
    }

    /// Whether this device accepts `next` after `latest`, with `certs`, the certificates of the
    /// space's devices, as its members do ([`skald_crypto::accept_next`]); why not otherwise.
    pub fn accepts(&self, latest: &Roster, next: &Roster, certs: &[Cert]) -> Result<(), String> {
        skald_crypto::accept_next(latest, next, certs, self.own.account, &self.own.master).map_err(
            |err| {
                format!(
                    "revision {} of the space fails its checks: {err}",
                    next.body().revision
                )
            },
        )
    }
}

/// What a revision this device signed changes in the one it was signed on, as a run signs it into
/// a later one: the channels it adds, a default channel among them, the members it removes, the
/// devices it bars, the roles it sets, the members it adds to and removes from channels, the
/// policy's `history` it sets, the service agents it removes and adds, and the `service` devices
/// it adds to channels.
#[derive(Debug, Default, PartialEq)]
struct Change {
    default: Option<ChannelId>,
    channels: Vec<Channel>,
    removed: Vec<AccountId>,
    excluded: Vec<(AccountId, DeviceId)>,
    roles: Vec<(AccountId, Role)>,
    joined: Vec<(ChannelId, ChannelMember)>,
    left: Vec<(ChannelId, AccountId)>,
    history: Option<u64>,
    dropped: Vec<DeviceId>,
    agents: Vec<ServiceAgent>,
    services: Vec<(ChannelId, DeviceId)>,
}

impl Change {
    /// Why the change does not apply to `latest`, a later revision than `signed`, the one it made:
    /// a member whose role it sets is in `latest` no more.
    fn role_gone(&self, signed: &RosterBody, latest: &Roster) -> Option<String> {
        let (account, _) =
            (self.roles.iter()).find(|(account, _)| latest.body().member(*account).is_none())?;
        let handle = signed.member(*account).map_or_else(
            || account.to_string(),
            |member| member.handle.as_str().to_owned(),
        );
        Some(format!(
            "{handle}, whose role it sets, is gone from the space {} in its revision {}",
            latest.body().name.as_str(),
            latest.body().revision
        ))
    }

    /// What `next` changes in `base`.
    fn between(base: &RosterBody, next: &RosterBody) -> Self {
        let mut change = Change::default();
        for (at, channel) in next.channels.iter().enumerate() {
            match base.channels.iter().find(|old| old.id == channel.id) {
                None if at == 0 => change.default = Some(channel.id),
                None => change.channels.push(channel.clone()),
                Some(old) => {
                    let has = |channel: &Channel, account| {
                        channel.members.iter().any(|entry| entry.account == account)
                    };
                    for entry in channel
                        .members
                        .iter()
                        .filter(|entry| !has(old, entry.account))
                    {
                        change.joined.push((channel.id, *entry));
                    }
                    for entry in old
                        .members
                        .iter()
                        .filter(|entry| !has(channel, entry.account))
                    {
                        change.left.push((channel.id, entry.account));
                    }
                    for device in (channel.services.iter()).filter(|it| !old.services.contains(it))
                    {
                        change.services.push((channel.id, *device));
                    }
                }
            }
        }
        for member in base.members.iter() {
            let Some(kept) = next.member(member.account) else {
                change.removed.push(member.account);
                continue;
            };
            let barred = |member: &Member| member.excluded.as_deref().unwrap_or_default().to_vec();
            let before = barred(member);
            for device in barred(kept).into_iter().filter(|id| !before.contains(id)) {
                change.excluded.push((member.account, device));
            }
            if kept.role != member.role {
                change.roles.push((member.account, kept.role));
            }
        }
        change
            .left
            .retain(|(_, account)| !change.removed.contains(account));
        change.history = Some(next.policy.history).filter(|&set| set != base.policy.history);
        change.dropped = (base.service_agents.iter())
            .map(|agent| agent.device)
            .filter(|device| {
                !next
                    .service_agents
                    .iter()
                    .any(|kept| kept.device == *device)
            })
            .collect();
        change.agents = (next.service_agents.iter())
            .filter(|agent| {
                !base
                    .service_agents
                    .iter()
                    .any(|old| old.device == agent.device)
            })
            .cloned()
            .collect();
        change
    }

    /// Makes the change in `body`, so far as it still applies: a member it removes leaves every
    /// channel; it bars a device, sets a role and adds a member to a channel only of a member of
    /// `body`; it removes no member from the default channel, the first one, which keeps every
    /// member; and it puts its default channel first, with every member, only if `needs_default`;
    /// it sets the policy's `history`; it removes a service agent's entry and its device from every
    /// channel's `services`; it adds a service agent only while `body` has neither its device nor
    /// its address, and its owner is no member's handle; and it adds a `service` device to a
    /// channel's `services` only while `body` has the device's service agent entry.
    fn onto(&self, body: &mut RosterBody, needs_default: bool) {
        if let Some(history) = self.history {
            body.policy.history = history;
        }
        // A change that would leave no member removes none.
        let _ = remove_members(body, &self.removed);
        let mut members = body.members.clone().into_vec();
        for member in &mut members {
            let account = member.account;
            for (_, device) in self.excluded.iter().filter(|(of, _)| *of == account) {
                exclude(member, *device);
            }
            if let Some((_, role)) = self.roles.iter().find(|(of, _)| *of == account) {
                member.role = *role;
            }
        }
        if let Some(members) = NonEmpty::from_vec(members) {
            body.members = members;
        }
        let is_member = |account: AccountId| body.members.iter().any(|m| m.account == account);
        let mut channels = body.channels.clone();
        for (at, channel) in channels.iter_mut().enumerate() {
            channel.members.retain(|entry| {
                is_member(entry.account)
                    && (at == 0 || !self.left.contains(&(channel.id, entry.account)))
            });
            for (_, entry) in self.joined.iter().filter(|(id, _)| *id == channel.id) {
                let listed = channel.members.iter().any(|it| it.account == entry.account);
                if is_member(entry.account) && !listed {
                    channel.members.push(*entry);
                }
            }
        }
        channels.extend(
            (self.channels.iter())
                .filter(|new| !body.channels.iter().any(|old| old.id == new.id))
                .cloned(),
        );
        if let Some(id) = self.default.filter(|_| needs_default) {
            channels.insert(0, default_channel(id, body));
        }
        body.channels = channels;
        body.service_agents
            .retain(|agent| !self.dropped.contains(&agent.device));
        for channel in &mut body.channels {
            channel
                .services
                .retain(|device| !self.dropped.contains(device));
        }
        for agent in &self.agents {
            if !address_taken(body, &agent.owner, &agent.agent)
                && !body
                    .service_agents
                    .iter()
                    .any(|old| old.device == agent.device)
            {
                body.service_agents.push(agent.clone());
            }
        }
        for (id, device) in &self.services {
            let listed = body
                .service_agents
                .iter()
                .any(|agent| agent.device == *device);
            if let Some(channel) = body.channels.iter_mut().find(|channel| channel.id == *id)
                && listed
                && !channel.services.contains(device)
            {
                channel.services.push(*device);
            }
        }
    }
}

/// Drops from `body`, a revision after `latest`, each `service` device that `latest` lists, in a
/// service agent entry or a channel's `services`, and that is revoked or on the account of no member
/// of `body`: by `devices`, the certificates and revocations of the space's accounts, against the
/// master keys `own` and `latest` give (`docs/protocol/membership.md`, "Roster"). A device whose
/// certificate is not among `devices` stays.
pub fn drop_stale_services(
    body: &mut RosterBody,
    latest: &Roster,
    own: &Own,
    devices: &SpaceDevices,
) {
    let before = latest.body();
    let listed = (before.service_agents.iter().map(|entry| entry.device)).chain(
        before
            .channels
            .iter()
            .flat_map(|it| it.services.iter().copied()),
    );
    let stale: Vec<DeviceId> = listed
        .filter(|&device| {
            let owner = before.members.iter().find_map(|member| {
                let master = follow::master_of(latest, own, member.account)?;
                let cert = skald_crypto::check_device(&master, &devices.certs, device).ok()?;
                let body = cert.body();
                (body.account == member.account && body.kind == DeviceKind::Service)
                    .then_some((member.account, master))
            });
            owner.is_some_and(|(account, master)| {
                body.member(account).is_none()
                    || skald_crypto::check_valid_device(
                        &master,
                        &devices.certs,
                        &devices.revocations,
                        device,
                    )
                    .is_err()
            })
        })
        .collect();
    body.service_agents
        .retain(|entry| !stale.contains(&entry.device));
    for channel in &mut body.channels {
        channel.services.retain(|device| !stale.contains(device));
    }
}

/// Whether `body`, a revision's body, has the address `owner/agent`: a service agent's, or one of
/// a member's agents, `owner` its handle.
pub fn address_taken(body: &RosterBody, owner: &Name, agent: &Name) -> bool {
    body.members.iter().any(|member| member.handle == *owner)
        || (body.service_agents.iter()).any(|it| it.owner == *owner && it.agent == *agent)
}

/// Removes the members of `accounts` from `body`, a revision's body, and from each of its channels;
/// fails, changing nothing, if no member would be left.
pub fn remove_members(body: &mut RosterBody, accounts: &[AccountId]) -> Result<(), String> {
    let members: Vec<Member> = (body.members.iter())
        .filter(|member| !accounts.contains(&member.account))
        .cloned()
        .collect();
    body.members = NonEmpty::from_vec(members).ok_or("the space keeps a member")?;
    for channel in &mut body.channels {
        channel
            .members
            .retain(|entry| !accounts.contains(&entry.account));
    }
    Ok(())
}

/// Adds `device` to `member`'s `excluded`; false if it lists the device already.
pub fn exclude(member: &mut Member, device: DeviceId) -> bool {
    let mut barred = member
        .excluded
        .clone()
        .map_or_else(Vec::new, NonEmpty::into_vec);
    if barred.contains(&device) {
        return false;
    }
    barred.push(device);
    member.excluded = NonEmpty::from_vec(barred);
    true
}

/// Whether `roster` needs a default channel (`architecture.md`, "Membership"), the first channel
/// it lists: it has no channel, or its first channel lacks a member of the space.
pub fn needs_default(roster: &Roster) -> bool {
    let body = roster.body();
    body.channels.first().is_none_or(|first| {
        body.members.iter().any(|member| {
            !first
                .members
                .iter()
                .any(|entry| entry.account == member.account)
        })
    })
}

/// The default channel of `roster`, its first channel, unless it needs one ([`needs_default`]).
pub fn default_of(roster: &Roster) -> Option<ChannelId> {
    let first = roster.body().channels.first()?;
    (!needs_default(roster)).then_some(first.id)
}

/// The default channel `id` of the revision `body`: every member of the space, with `history`
/// true.
fn default_channel(id: ChannelId, body: &RosterBody) -> Channel {
    Channel {
        id,
        members: body
            .members
            .iter()
            .map(|member| ChannelMember {
                account: member.account,
                history: true,
            })
            .collect(),
        services: Vec::new(),
        unknown: Unknown::default(),
    }
}

/// `NewChannel` in the API spec.
#[derive(Deserialize)]
struct NewChannel {
    channel: String,
}

/// `POST /spaces/<id>/channels`: a channel ID the server issues for the space `id` to the session
/// `token`.
pub fn issue_channel(
    connection: &mut Connection,
    token: &str,
    id: SpaceId,
) -> Result<ChannelId, String> {
    let path = format!("/spaces/{id}/channels");
    let body = connection.post_as(&path, token).map_err(text)?;
    let unexpected = || "unexpected answer from the server: a channel without its ID".to_owned();
    let issued: NewChannel = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    issued.channel.parse().map_err(|_| unexpected())
}

/// Seconds since the Unix epoch, a revision's `timestamp`; 0 with a clock before it.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// A space a command uses, once the device accepted the revisions the server has of it: the
/// follower, the space as the state held it, its last accepted revision and its accounts' devices.
pub struct Synced {
    pub follower: Follower,
    pub space: state::Space,
    pub roster: Roster,
    pub devices: SpaceDevices,
}

/// The spaces a run covers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every space the device follows: a person's run.
    All,
    /// Only this one, the space of an agent's folder. The run takes no personal space and sends
    /// nothing of another space.
    Only(SpaceId),
}

impl Scope {
    /// The scope of a run in `dir`: the space of its folder for an `agent`, otherwise every space.
    pub fn of_run(dir: &Path, agent: bool) -> Result<Self, String> {
        Ok(if agent {
            Scope::Only(folders::working_space(dir)?.id)
        } else {
            Scope::All
        })
    }

    /// Whether the space `id` is outside the scope.
    pub fn excludes(self, id: SpaceId) -> bool {
        matches!(self, Scope::Only(only) if only != id)
    }
}

/// The follower of the machine whose state is in `dir`, its secrets in `storage`, for `scope`:
/// [`Follower::new`] for all spaces, [`Follower::open`] for one.
fn follower(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    scope: Scope,
) -> Result<Option<Follower>, String> {
    if let Some(agent) = service::from_env()? {
        return Follower::service(dir, &agent, speaks, timeout).map(Some);
    }
    match scope {
        Scope::All => Follower::new(dir, storage, speaks, timeout),
        Scope::Only(_) => Follower::open(dir, storage, speaks, timeout),
    }
}

/// As [`follower`]; fails if the machine has no certified device.
pub fn following(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    scope: Scope,
) -> Result<Follower, String> {
    follower(dir, storage, speaks, timeout, scope)?.ok_or_else(|| NOT_FOLLOWING.to_owned())
}

/// The space named `name` that the device of the machine whose state is in `dir`, its secrets in
/// `storage`, follows, synced ([`Follower::sync`]), the groups of its channels brought in line
/// ([`groups::align_accepted`]); within `scope`.
pub fn synced(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    name: &str,
    scope: Scope,
) -> Result<Synced, String> {
    let mut follower = following(dir, storage, speaks, timeout, scope)?;
    let space = follower.space(name)?;
    if scope.excludes(space.id) {
        return Err(OUT_OF_SCOPE.to_owned());
    }
    let (roster, devices) = follower.sync(&space)?;
    groups::align_accepted(&mut follower);
    Ok(Synced {
        follower,
        space,
        roster,
        devices,
    })
}

/// The space of the channel `channel` that the device of the machine whose state is in `dir`, its
/// secrets in `storage`, follows, synced ([`Follower::sync`]); within `scope`. The spaces whose
/// last accepted revision has the channel are synced first, then, while none has it, the others.
pub fn of_channel(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    channel: ChannelId,
    scope: Scope,
) -> Result<Synced, String> {
    let mut follower = following(dir, storage, speaks, timeout, scope)?;
    let mut spaces: Vec<state::Space> = follower
        .state
        .spaces()
        .map_err(text)?
        .into_iter()
        .filter(|space| !scope.excludes(space.id))
        .collect();
    let has = |roster: &Roster| roster.body().channels.iter().any(|it| it.id == channel);
    spaces.sort_by_key(|space| {
        !follow::decode(&space.accepted, &space.name).is_ok_and(|roster| has(&roster))
    });
    for space in spaces {
        let (roster, devices) = follower.sync(&space)?;
        if has(&roster) {
            return Ok(Synced {
                follower,
                space,
                roster,
                devices,
            });
        }
    }
    Err(match scope {
        Scope::All => format!("no space this device follows has the channel {channel}"),
        // The agent's output names nothing of another space, not even the channel it gave.
        Scope::Only(_) => "the space of the agent's folder has no such channel: as an agent, \
                           skald uses only its channels"
            .to_owned(),
    })
}

/// The working directory's space ([`folders::working_space`]) that the device of the machine whose
/// state is in `dir`, its secrets in `storage`, follows, synced ([`Follower::sync`]), and its
/// default channel; within `scope`, which for an agent is that space. Fails while the space has no
/// default channel ([`needs_default`]).
pub fn of_working_dir(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    scope: Scope,
) -> Result<(Synced, ChannelId), String> {
    let mut follower = following(dir, storage, speaks, timeout, scope)?;
    // A service agent's space is its key secret's, whatever the working directory.
    let id = match (follower.service, scope) {
        (Some(member), _) => member.space,
        (None, Scope::Only(id)) => id,
        (None, Scope::All) => folders::working_space(dir)?.id,
    };
    let space = (follower.state.spaces().map_err(text)?.into_iter())
        .find(|space| space.id == id)
        .ok_or_else(|| "this device follows no space of the working directory".to_owned())?;
    let (roster, devices) = follower.sync(&space)?;
    let channel = default_of(&roster).ok_or_else(|| {
        format!(
            "the space {} has no default channel yet; nothing is sent",
            space.name
        )
    })?;
    let synced = Synced {
        follower,
        space,
        roster,
        devices,
    };
    Ok((synced, channel))
}

/// Whether `device` is a device of an admin in `roster`: one of `account`, an admin in it, whose
/// `excluded` does not list it. The device's certificate is checked on accepting.
pub fn admin_device(roster: &Roster, account: AccountId, device: DeviceId) -> bool {
    roster.body().member(account).is_some_and(|member| {
        member.role == Role::Admin
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
/// accepts the revisions the server has of each, and brings the groups of their channels in line
/// ([`groups::align_accepted`]); a space it fails to follow shows as the local state holds it,
/// saying why. Only the spaces within `scope`. Warns of each it is the only admin of
/// ([`only_admin`]).
pub fn list(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    scope: Scope,
) -> Result<(), String> {
    let (lines, warnings) = if dir.is_dir() {
        let state = State::open(dir).map_err(text)?;
        let mut follower = match state.session().map_err(text)? {
            Some(_) => follower(dir, storage, speaks, timeout, scope).unwrap_or_else(|err| {
                eprintln!("skald: the spaces show their last accepted revisions: {err}");
                None
            }),
            None => None,
        };
        let account = own_account(&state)?;
        let mut lines = Vec::new();
        let mut warnings = Vec::new();
        let spaces = state.spaces().map_err(text)?;
        for space in spaces.into_iter().filter(|space| !scope.excludes(space.id)) {
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
            lines.push(line(&roster, account, space.first_waits(), space.personal));
            warnings.extend(account.and_then(|account| only_admin(&roster, account)));
        }
        if let Some(follower) = follower.as_mut() {
            groups::align_accepted(follower);
        }
        (lines, warnings)
    } else {
        (Vec::new(), Vec::new())
    };
    if lines.is_empty() {
        println!("This device is in no space yet; create one with skald create-space <name>.");
    }
    for line in lines {
        println!("{line}");
    }
    for warning in warnings {
        eprintln!("skald: warning: {warning}");
    }
    Ok(())
}

/// The warning for `account` if it is the only admin of `roster`'s space and the space has another
/// member: nobody manages the space once the account loses all its devices and its recovery key.
pub fn only_admin(roster: &Roster, account: AccountId) -> Option<String> {
    let body = roster.body();
    let mut admins = body
        .members
        .iter()
        .filter(|member| member.role == Role::Admin);
    let only =
        admins.next().is_some_and(|admin| admin.account == account) && admins.next().is_none();
    (only && body.members.len() > 1).then(|| {
        let name = body.name.as_str();
        format!(
            "this account is the only admin of the space {name}; if it loses all its devices and \
             its recovery key, nobody can manage the space. Add an admin: skald make-admin {name} \
             <handle>"
        )
    })
}

/// The account of the device the state `state` holds, if it knows it.
pub fn own_account(state: &State) -> Result<Option<AccountId>, String> {
    let Some(account) = state
        .device()
        .map_err(text)?
        .and_then(|device| device.account)
    else {
        return Ok(None);
    };
    account
        .parse()
        .map(Some)
        .map_err(|_| state::Error::Malformed(format!("the device's account {account:?}")))
        .map_err(text)
}

/// Shows the members of `synced`, a space the device follows; warns if the device's account is its
/// only admin ([`only_admin`]).
pub fn members(synced: Synced) -> Result<(), String> {
    let Synced {
        follower,
        roster,
        devices,
        ..
    } = synced;
    let (listing, mut warnings) = members_listing(&roster, &devices, &follower.own, follower.this);
    warnings.extend(only_admin(&roster, follower.own.account));
    print!("{listing}");
    for warning in warnings {
        eprintln!("skald: warning: {warning}");
    }
    Ok(())
}

/// What `skald members` shows of `roster`, a space's last accepted revision, with `devices`, those
/// of its accounts: each member's handle and role, and under it each device of its account with a
/// valid certificate, then each revoked one, as `skald devices` shows them, `this` device marked;
/// and the warnings of `skald devices` for them. A certificate is checked against the master key
/// `own` pinned for its own account, and against the `master` the revision gives any other.
fn members_listing(
    roster: &Roster,
    devices: &SpaceDevices,
    own: &Own,
    this: DeviceId,
) -> (String, Vec<String>) {
    let body = roster.body();
    let mut warnings: Vec<String> = Vec::new();
    let mut out = heading(roster);
    for member in body.members.iter() {
        out.push_str(&format!(
            "  {}  {}\n",
            member.handle.as_str(),
            role(member.role)
        ));
        let Some(master) = follow::master_of(roster, own, member.account) else {
            continue;
        };
        let checked = devices::check(&master, devices);
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
        for cert in (checked.revoked.iter()).filter(|cert| cert.body().account == member.account) {
            out.push_str(&format!("    {}\n", devices::revoked_line(cert, this)));
        }
    }
    (out, warnings)
}

/// The first line of what `skald members` and `skald channels` show of `roster`, a space's last
/// accepted revision: the space's name and ID and the revision's number.
pub fn heading(roster: &Roster) -> String {
    let body = roster.body();
    format!(
        "The space {} ({}), revision {}:\n",
        body.name.as_str(),
        body.space,
        body.revision
    )
}

/// A role, as the CLI shows it.
fn role(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Member => "member",
    }
}

/// The line of a space whose last accepted revision is `roster`: its name, the role and handle of
/// `account` in it, and its ID; marked if it is the device's `personal` space, and if the server
/// has not taken the revision yet.
fn line(roster: &Roster, account: Option<AccountId>, pending: bool, personal: bool) -> String {
    let body = roster.body();
    let member = account.and_then(|account| body.member(account));
    let (role, handle) = match member {
        Some(member) => (role(member.role), member.handle.as_str()),
        None => ("no member", "-"),
    };
    let mut line = format!("{}  {role}  {handle}  {}", body.name.as_str(), body.space);
    if personal {
        line.push_str("  personal");
    }
    if pending {
        line.push_str("  (not on the server yet)");
    }
    line
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use skald_crypto::SigningKey;
    use skald_proto::{DeviceId, HISTORY_MIN};

    /// Revision 1 of the space `acme`, created by `account` with the handle `ada`.
    pub(crate) fn first(account: AccountId) -> Roster {
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

    /// The entry of `account` as a member with `handle` and the master key `master`.
    pub(crate) fn member(
        account: AccountId,
        handle: &str,
        master: [u8; 32],
    ) -> skald_proto::Member {
        skald_proto::Member {
            account,
            handle: handle.parse().unwrap(),
            role: Role::Member,
            master,
            invite: None,
            excluded: None,
            unknown: skald_proto::Unknown::default(),
        }
    }

    #[test]
    fn now_is_the_seconds_since_the_unix_epoch() {
        let since = |time: SystemTime| time.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let before = since(SystemTime::now());
        let now = now();
        assert!(before <= now && now <= since(SystemTime::now()), "{now}");
    }

    #[test]
    fn line_shows_the_name_the_accounts_role_and_handle_and_the_id() {
        let account = AccountId::generate();
        let roster = first(account);
        let id = roster.body().space;
        assert_eq!(
            line(&roster, Some(account), false, false),
            format!("acme  admin  ada  {id}")
        );
        assert_eq!(
            line(&roster, Some(account), true, false),
            format!("acme  admin  ada  {id}  (not on the server yet)")
        );
        assert_eq!(
            line(&roster, Some(account), true, true),
            format!("acme  admin  ada  {id}  personal  (not on the server yet)")
        );
        assert_eq!(
            line(&roster, Some(AccountId::generate()), false, false),
            format!("acme  no member  -  {id}")
        );
        assert_eq!(
            line(&roster, None, false, false),
            format!("acme  no member  -  {id}")
        );
    }

    /// The certificate of the device of `key` on `account`, named `name`, signed by `master`.
    pub(crate) fn cert(
        account: AccountId,
        master: &SigningKey,
        key: &SigningKey,
        name: &str,
    ) -> Cert {
        let body = skald_proto::CertBody {
            account,
            device: DeviceId::from_signing_key(&key.public_key()),
            kind: skald_proto::DeviceKind::Client,
            signing: key.public_key(),
            hpke: [4; 32],
            issuer: None,
            issued: 1_800_000_000,
            ssh: None,
            github: None,
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
            body.members
                .push(member(self.bob, "bob", self.bob_master.public_key()));
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
            self.listing_with(roster, (certs, Vec::new()), this)
        }

        /// As [`Pair::listing`], with `revocations`.
        fn listing_with(
            &self,
            roster: &Roster,
            (certs, revocations): (&[Cert], Vec<skald_proto::Revocation>),
            this: DeviceId,
        ) -> (String, Vec<String>) {
            let devices = SpaceDevices {
                ids: certs.iter().map(|cert| cert.body().device).collect(),
                certs: certs.to_vec(),
                revocations,
                records: Vec::new(),
            };
            members_listing(roster, &devices, &self.own(), this)
        }
    }

    #[test]
    fn members_show_each_members_revoked_devices_after_its_valid_ones() {
        let pair = Pair::new();
        let keys: Vec<SigningKey> = (0..3).map(|_| SigningKey::generate()).collect();
        let certs = vec![
            cert(pair.bob, &pair.bob_master, &keys[0], "bob-old"),
            cert(pair.bob, &pair.bob_master, &keys[1], "bob-new"),
            cert(pair.ada, &pair.ada_master, &keys[2], "ada-laptop"),
        ];
        let (old, new) = (certs[0].body().device, certs[1].body().device);
        let body = skald_proto::RevocationBody {
            account: pair.bob,
            device: old,
            keep: None,
            signer: Some(new),
            timestamp: 1_800_000_000,
            also: None,
            unknown: skald_proto::Unknown::default(),
        };
        let revocation = skald_crypto::sign_revocation(&body, &keys[1]).unwrap();
        let roster = pair.roster();
        let this = certs[2].body().device;

        let (shown, _) = pair.listing_with(&roster, (&certs, vec![revocation]), this);

        assert!(
            shown.ends_with(&format!(
                "  bob  member\n    {}\n    {}\n",
                devices::linked_line(&certs[1], this),
                devices::revoked_line(&certs[0], this)
            )),
            "{shown}"
        );
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

    /// A next revision drops the `service` devices the revision before it lists, in entries and in
    /// `services`, that are revoked or of an account it removes; it keeps the others, and a device
    /// whose certificate the space's devices lack.
    #[test]
    fn next_revisions_drop_revoked_and_removed_accounts_service_devices() {
        let pair = Pair::new();
        let keys: Vec<SigningKey> = (0..5).map(|_| SigningKey::generate()).collect();
        let laptops = [
            cert(pair.ada, &pair.ada_master, &keys[0], "ada-laptop"),
            cert(pair.bob, &pair.bob_master, &keys[1], "bob-laptop"),
        ];
        let service = |account, issuer: &SigningKey, key: &SigningKey| {
            let body = skald_proto::CertBody {
                account,
                device: DeviceId::from_signing_key(&key.public_key()),
                kind: DeviceKind::Service,
                signing: key.public_key(),
                hpke: [4; 32],
                issuer: Some(DeviceId::from_signing_key(&issuer.public_key())),
                issued: 1_800_000_000,
                ssh: None,
                github: None,
                name: "ci/bot".to_owned(),
                unknown: Unknown::default(),
            };
            skald_crypto::issue(&body, issuer, key).expect("a certificate")
        };
        let services = [
            service(pair.ada, &keys[0], &keys[2]),
            service(pair.ada, &keys[0], &keys[3]),
            service(pair.bob, &keys[1], &keys[4]),
        ];
        let [revoked, kept, bobs] = services.each_ref().map(|cert| cert.body().device);
        let uncertified = DeviceId::from_signing_key(&[7; 32]);
        let revocation = skald_crypto::sign_revocation(
            &skald_proto::RevocationBody {
                account: pair.ada,
                device: revoked,
                keep: None,
                signer: Some(laptops[0].body().device),
                timestamp: 1_800_000_000,
                also: None,
                unknown: Unknown::default(),
            },
            &keys[0],
        )
        .unwrap();
        let devices = SpaceDevices {
            ids: Vec::new(),
            certs: laptops.iter().chain(&services).cloned().collect(),
            revocations: vec![revocation],
            records: Vec::new(),
        };
        let mut body = pair.roster().body().clone();
        let entry = |device, name: &str| ServiceAgent {
            device,
            owner: "ci".parse().unwrap(),
            agent: name.parse().unwrap(),
            unknown: Unknown::default(),
        };
        body.service_agents = vec![
            entry(revoked, "old-bot"),
            entry(kept, "deploy-bot"),
            entry(bobs, "bob-bot"),
            entry(uncertified, "lost-bot"),
        ];
        let mut general = channel(ChannelId::generate(), &[pair.ada, pair.bob]);
        general.services = vec![revoked, bobs, uncertified];
        body.channels = vec![general];
        let latest = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let listed = |body: &RosterBody| {
            let entries: Vec<DeviceId> = body.service_agents.iter().map(|it| it.device).collect();
            (entries, body.channels[0].services.clone())
        };

        let mut next = latest.body().clone();
        drop_stale_services(&mut next, &latest, &pair.own(), &devices);
        assert_eq!(
            listed(&next),
            (vec![kept, bobs, uncertified], vec![bobs, uncertified])
        );

        let mut without_bob = latest.body().clone();
        remove_members(&mut without_bob, &[pair.bob]).unwrap();
        drop_stale_services(&mut without_bob, &latest, &pair.own(), &devices);
        assert_eq!(
            listed(&without_bob),
            (vec![kept, uncertified], vec![uncertified])
        );
    }

    /// A space needs a default channel while it has no channel or its first channel lacks a
    /// member; a later channel with every member does not count. A default channel holds every
    /// member with `history` true; once there, it is the space's default channel.
    #[test]
    fn space_needs_a_default_channel_until_its_first_channel_has_every_member() {
        let (ada, bob) = (AccountId::generate(), AccountId::generate());
        let mut body = first(ada).body().clone();
        let sign = |body: &RosterBody| {
            skald_crypto::sign_roster(body, &SigningKey::generate()).expect("a revision")
        };
        assert!(needs_default(&sign(&body)), "no channel");
        assert_eq!(default_of(&sign(&body)), None, "no channel");
        body.channels = vec![default_channel(ChannelId::generate(), &body)];
        let ada_in = ChannelMember {
            account: ada,
            history: true,
        };
        assert_eq!(body.channels[0].members, [ada_in]);
        assert!(!needs_default(&sign(&body)));

        body.members.push(member(bob, "bob", [4; 32]));
        assert!(needs_default(&sign(&body)), "bob outside the first channel");
        body.channels
            .push(default_channel(ChannelId::generate(), &body));
        assert!(
            needs_default(&sign(&body)),
            "every member in a later channel"
        );
        assert_eq!(default_of(&sign(&body)), None, "a later channel");
        let default = ChannelId::generate();
        body.channels.insert(0, default_channel(default, &body));
        let bob_in = ChannelMember {
            account: bob,
            history: true,
        };
        assert_eq!(body.channels[0].members, [ada_in, bob_in]);
        assert!(body.channels[0].services.is_empty());
        assert!(!needs_default(&sign(&body)));
        assert_eq!(default_of(&sign(&body)), Some(default));
    }

    #[test]
    fn admin_device_is_one_of_an_admin_account_that_its_excluded_does_not_list() {
        let (ada, bob) = (AccountId::generate(), AccountId::generate());
        let device = |seed| DeviceId::from_signing_key(&[seed; 32]);
        let mut body = first(ada).body().clone();
        body.members.push(member(bob, "bob", [5; 32]));
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

    /// The body of a revision of `acme` with `members`, the first its admin, and `channels`, each
    /// with its members' accounts, every one with `history` true.
    fn acme(members: &[AccountId], channels: &[(ChannelId, &[AccountId])]) -> RosterBody {
        let mut body = first(members[0]).body().clone();
        for account in &members[1..] {
            // The random end of the account's ID.
            let handle = format!("m{}", &account.to_string()[24..]);
            body.members.push(member(*account, &handle, [4; 32]));
        }
        body.channels = (channels.iter())
            .map(|(id, accounts)| channel(*id, accounts))
            .collect();
        body
    }

    /// The channel `id` with `accounts` as its members, each with `history` true.
    pub(crate) fn channel(id: ChannelId, accounts: &[AccountId]) -> Channel {
        Channel {
            id,
            members: (accounts.iter())
                .map(|&account| ChannelMember {
                    account,
                    history: true,
                })
                .collect(),
            services: Vec::new(),
            unknown: Unknown::default(),
        }
    }

    /// The service agent `owner/agent` on the device of the signing key of `byte` repeated.
    pub(crate) fn agent(owner: &str, agent: &str, byte: u8) -> ServiceAgent {
        ServiceAgent {
            device: DeviceId::from_signing_key(&[byte; 32]),
            owner: owner.parse().unwrap(),
            agent: agent.parse().unwrap(),
            unknown: Unknown::default(),
        }
    }

    /// Gives `account` the role `role` in `body`.
    fn make(body: &mut RosterBody, account: AccountId, role: Role) {
        let mut members = body.members.clone().into_vec();
        for member in members
            .iter_mut()
            .filter(|member| member.account == account)
        {
            member.role = role;
        }
        body.members = NonEmpty::from_vec(members).unwrap();
    }

    /// Bars `device` of `account` in `body`.
    fn bar(body: &mut RosterBody, account: AccountId, device: DeviceId) {
        let mut members = body.members.clone().into_vec();
        for member in members
            .iter_mut()
            .filter(|member| member.account == account)
        {
            member.excluded = NonEmpty::from_vec(vec![device]);
        }
        body.members = NonEmpty::from_vec(members).unwrap();
    }

    /// A revision that removes Carol from the space, bars a device of Bob's, removes Bob from a
    /// channel, adds Dave to it, makes Dave an admin and Ada a member, adds a channel, sets the
    /// history period and adds a service agent, listed in a channel's `services`, changes the same
    /// in a later revision that added Eve, who stays in every channel she is in, and another
    /// service agent; a service agent all of them list stays once.
    #[test]
    fn change_is_made_again_in_a_later_revision() {
        let [ada, bob, carol, dave, eve] = [(); 5].map(|()| AccountId::generate());
        let [default, team, new] = [(); 3].map(|()| ChannelId::generate());
        let barred = DeviceId::from_signing_key(&[9; 32]);
        // A service agent every revision lists, and one the change removes.
        let watch = agent("ops", "watch-bot", 5);
        let old = agent("ci", "old-bot", 6);
        let mut base = acme(
            &[ada, bob, carol, dave],
            &[
                (default, &[ada, bob, carol, dave]),
                (team, &[ada, bob, carol]),
            ],
        );
        base.service_agents = vec![watch.clone(), old.clone()];
        base.channels[1].services.push(old.device);
        let mut next = acme(
            &[ada, bob, dave],
            &[
                (default, &[ada, bob, dave]),
                (team, &[ada, dave]),
                (new, &[ada]),
            ],
        );
        next.channels[1].members[1].history = false;
        next.policy.history = 2 * HISTORY_MIN;
        bar(&mut next, bob, barred);
        make(&mut next, dave, Role::Admin);
        make(&mut next, ada, Role::Member);
        let (deploy, release) = (agent("ci", "deploy-bot", 7), agent("ci", "release-bot", 8));
        next.service_agents = vec![watch.clone(), deploy.clone()];
        next.channels[1].services.push(deploy.device);

        let change = Change::between(&base, &next);

        let dave_in = ChannelMember {
            account: dave,
            history: false,
        };
        assert_eq!(
            change,
            Change {
                default: None,
                channels: vec![next.channels[2].clone()],
                removed: vec![carol],
                excluded: vec![(bob, barred)],
                roles: vec![(ada, Role::Member), (dave, Role::Admin)],
                joined: vec![(team, dave_in)],
                left: vec![(team, bob)],
                history: Some(2 * HISTORY_MIN),
                dropped: vec![old.device],
                agents: vec![deploy.clone()],
                services: vec![(team, deploy.device)],
            }
        );
        let mut latest = acme(
            &[ada, bob, carol, dave, eve],
            &[
                (default, &[ada, bob, carol, dave, eve]),
                (team, &[ada, bob, carol, eve]),
            ],
        );
        latest.service_agents = vec![watch.clone(), release.clone(), old.clone()];
        latest.channels[1].services.push(old.device);
        let (space, signer) = (latest.space, latest.signer);
        change.onto(&mut latest, false);
        let mut expected = acme(
            &[ada, bob, dave, eve],
            &[
                (default, &[ada, bob, dave, eve]),
                (team, &[ada, eve]),
                (new, &[ada]),
            ],
        );
        expected.channels[1].members.push(dave_in);
        expected.channels[1].services.push(deploy.device);
        expected.policy.history = 2 * HISTORY_MIN;
        bar(&mut expected, bob, barred);
        make(&mut expected, dave, Role::Admin);
        make(&mut expected, ada, Role::Member);
        expected.service_agents = vec![watch, release, deploy];
        (expected.space, expected.signer) = (space, signer);
        assert_eq!(latest, expected);
    }

    /// Of a change, what no longer applies to a later revision is left out: a removed or barred
    /// member that is gone, a member added to a channel who left the space or is in it already, a
    /// member removed from the default channel, a default channel the revision does not need, and a
    /// service agent whose device or address the revision has or whose owner is a member's handle,
    /// a removed service agent the revision does not list, and a `service` device in a channel the
    /// revision lacks, or without the device's entry. One that needs it gets it first, with every
    /// member.
    #[test]
    fn change_leaves_out_what_no_longer_applies() {
        let [ada, bob, carol] = [(); 3].map(|()| AccountId::generate());
        let [default, team, added] = [(); 3].map(|()| ChannelId::generate());
        let entry = |account| ChannelMember {
            account,
            history: false,
        };
        let change = Change {
            default: Some(added),
            channels: Vec::new(),
            removed: vec![carol],
            excluded: vec![(carol, DeviceId::from_signing_key(&[9; 32]))],
            roles: Vec::new(),
            joined: vec![(team, entry(carol)), (team, entry(ada))],
            left: vec![(default, bob)],
            history: None,
            // An agent the revision does not list.
            dropped: vec![agent("ops", "gone-bot", 10).device],
            agents: vec![
                agent("ci", "deploy-bot", 7),
                agent("ops", "other-bot", 8),
                agent("ada", "deploy-bot", 9),
            ],
            services: vec![
                (ChannelId::generate(), agent("ops", "bot", 8).device),
                (team, agent("ada", "deploy-bot", 9).device),
            ],
        };
        let mut latest = acme(&[ada, bob], &[(default, &[ada, bob]), (team, &[ada])]);
        latest.service_agents = vec![agent("ci", "deploy-bot", 6), agent("ops", "bot", 8)];

        let mut kept = latest.clone();
        change.onto(&mut kept, false);
        assert_eq!(kept, latest);

        let mut needing = latest.clone();
        change.onto(&mut needing, true);
        let mut expected = latest.clone();
        expected.channels.insert(0, default_channel(added, &latest));
        assert_eq!(needing, expected);
    }

    /// A change that sets the role of a member a later revision lacks does not apply to it, saying
    /// the member is gone, by its handle, and naming that revision; one whose members are all in
    /// it does.
    #[test]
    fn role_change_of_a_member_gone_from_a_later_revision_does_not_apply() {
        let [ada, bob, carol] = [(); 3].map(|()| AccountId::generate());
        let base = acme(&[ada, bob, carol], &[]);
        let mut handed = base.clone();
        make(&mut handed, bob, Role::Admin);
        make(&mut handed, ada, Role::Member);
        let change = Change::between(&base, &handed);
        let later = |removed: AccountId| {
            let mut body = base.clone();
            remove_members(&mut body, &[removed]).unwrap();
            body.revision = 7;
            skald_crypto::sign_roster(&body, &SigningKey::generate()).expect("a revision")
        };

        let why = change.role_gone(&handed, &later(bob)).expect("refused");

        let handle = handed.member(bob).unwrap().handle.as_str();
        assert!(
            why.contains(handle) && why.contains("gone") && why.contains("revision 7"),
            "{why}"
        );
        assert_eq!(change.role_gone(&handed, &later(carol)), None);
    }

    /// Only the only admin of a space with another member is warned, naming the command that makes
    /// a member an admin; not a member, nor an admin alone or with another admin.
    #[test]
    fn only_admin_of_a_space_with_another_member_is_warned() {
        let [ada, bob] = [(); 2].map(|()| AccountId::generate());
        let signed = |body: &RosterBody| {
            skald_crypto::sign_roster(body, &SigningKey::generate()).expect("a revision")
        };
        let alone = first(ada);
        let with_bob = signed(&acme(&[ada, bob], &[]));
        let mut two = acme(&[ada, bob], &[]);
        make(&mut two, bob, Role::Admin);
        let two = signed(&two);

        let warning = only_admin(&with_bob, ada).expect("a warning");

        assert!(
            warning.contains("recovery key") && warning.contains("skald make-admin acme <handle>"),
            "{warning}"
        );
        assert_eq!(only_admin(&with_bob, bob), None);
        assert_eq!(only_admin(&alone, ada), None);
        assert_eq!(only_admin(&two, ada), None);
        assert_eq!(only_admin(&two, bob), None);
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

    /// A machine signed in to `server` whose device is linked as `link`.
    fn machine(dir: &Path, server: &str, link: &Link) {
        let keys = device::Keys::generate();
        let state = State::open(dir).unwrap();
        state.start_session(server).unwrap();
        state.add_device(keys.id(), "Laptop", "account-1").unwrap();
        state.set_link(link).unwrap();
        let mut stored = secrets::Map::from([
            (sign_in::SESSION_TOKEN.to_owned(), "token-1".to_owned()),
            (sign_in::SESSION_SERVER.to_owned(), server.to_owned()),
        ]);
        keys.store(&mut stored);
        secrets::Secrets::new(dir, secrets::Storage::File)
            .save(&stored)
            .unwrap();
    }

    /// A certified device's follower keeps its secrets where sign-in does: in the key storage entry
    /// and the file of the state directory's canonical path, also from a path that is not.
    #[test]
    fn follower_keeps_its_secrets_where_sign_in_does() {
        let dir = crate::test_dir::TestDir::new("follow-secrets");
        let keys = device::Keys::generate();
        let body = skald_proto::CertBody {
            account: AccountId::generate(),
            device: keys.id(),
            kind: skald_proto::DeviceKind::Client,
            signing: keys.signing().public_key(),
            hpke: keys.hpke().public_key(),
            issuer: None,
            issued: 0,
            ssh: None,
            github: None,
            name: "Laptop".to_owned(),
            unknown: skald_proto::Unknown::default(),
        };
        let master = skald_crypto::SigningKey::generate();
        let cert = skald_crypto::issue(&body, &master, keys.signing()).unwrap();
        let state = State::open(&dir).unwrap();
        state.start_session("http://127.0.0.1:9").unwrap();
        state.add_device(keys.id(), "Laptop", "account-1").unwrap();
        state
            .set_link(&Link::Certified {
                master: master.public_key(),
                cert: cert.to_cbor(),
            })
            .unwrap();
        let mut stored = secrets::Map::from([
            (sign_in::SESSION_TOKEN.to_owned(), "token-1".to_owned()),
            (
                sign_in::SESSION_SERVER.to_owned(),
                "http://127.0.0.1:9".to_owned(),
            ),
        ]);
        keys.store(&mut stored);
        secrets::Secrets::new(&dir, secrets::Storage::File)
            .save(&stored)
            .unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let follower = Follower::unchecked(
            &dir.join("sub").join(".."),
            secrets::Storage::File,
            VersionRange::default(),
            Duration::from_secs(10),
        )
        .unwrap()
        .expect("a certified device");

        let canonical = std::fs::canonicalize(&*dir).unwrap();
        assert_eq!(follower.secrets.file(), canonical.join(secrets::FILE));
    }

    /// A device without a certificate follows no space and says it is not linked yet, reaching no
    /// server, for a person's run and an agent's.
    #[test]
    fn device_without_a_certificate_follows_none() {
        for (name, link) in [
            ("new", Link::New),
            ("waiting", Link::Waiting),
            (
                "sent",
                Link::Sent {
                    master: [7; 32],
                    cert: vec![0x80],
                    record: vec![0x81],
                    backup: vec![0x82],
                },
            ),
        ] {
            for scope in [Scope::All, Scope::Only(SpaceId::generate())] {
                let dir = crate::test_dir::TestDir::new(&format!("follow-{name}"));
                let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
                listener.set_nonblocking(true).unwrap();
                let server = format!("http://{}", listener.local_addr().unwrap());
                machine(&dir, &server, &link);

                let refused = following(
                    &dir,
                    secrets::Storage::File,
                    VersionRange::default(),
                    Duration::from_secs(10),
                    scope,
                );

                assert_eq!(refused.err().as_deref(), Some(NOT_FOLLOWING), "{name}");
                assert!(listener.accept().is_err(), "{name}: the server was reached");
            }
        }
    }
}
