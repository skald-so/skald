//! `skald create-service-agent`, `skald create-service-token`, `skald service-tokens`, `skald
//! revoke-service-token`, `skald replace-service-key` and `skald remove-service-agent`: an admin's
//! `client` device creates a service agent of a space and its tokens, replaces its key and removes
//! it (`docs/protocol/membership.md`, "Service device"; `docs/protocol/README.md`, "Service
//! grant"). A creation generates the service device's keys in memory, registers the device on the
//! admin's account with a certificate this device issues, signs the space's next roster revision
//! with the agent's entry as [`crate::spaces`] says, and for a member agent with its device in the
//! `services` of the grant's channels, signs the device's grant and creates its first token. It
//! writes the key secret and the token to stdout once, and keeps nothing of them. A key change
//! makes a new device the same way, puts it in the old one's place and revokes the old one.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::SigningKey;
use skald_proto::{
    CERT_NAME_MAX, Cbor as _, Cert, CertBody, ChannelId, DeviceId, DeviceKind, GRANT_LABEL, Grant,
    GrantBody, Name, NonEmpty, Roster, ServiceAgent, ServiceKey, SpaceId, Unknown,
};
use zeroize::Zeroizing;

use crate::connection::{Method, printable};
use crate::device::Keys;
use crate::devices::text;
use crate::follow::SpaceDevices;
use crate::revocations;
use crate::spaces::{Follower, Synced, address_taken, admin_device, finish, no_admin, now};

/// The environment variables of a service agent's two secrets, as stdout names them.
pub const KEY_VARIABLE: &str = "SKALD_SERVICE_KEY";
pub const TOKEN_VARIABLE: &str = "SKALD_SERVICE_TOKEN";

/// A grant's expiry when none is given, in days.
const DEFAULT_DAYS: u64 = 90;
/// A day, in seconds.
const DAY: u64 = 86_400;

/// A permission of a service token.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, clap::ValueEnum, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    Send,
    Receive,
    Queues,
}

/// What a grant and its token give, as a command takes it.
#[derive(clap::Args, Serialize, Deserialize)]
pub struct Granted {
    /// A channel the service agent may use, by its ID as skald channels shows it; once per
    /// channel.
    #[arg(long = "channel", value_name = "CHANNEL", required = true)]
    pub channels: Vec<String>,
    /// When the grant and the token expire: whole days from now, such as 30d, or an RFC 3339
    /// date-time, such as 2027-01-31T00:00:00Z [default: 90d].
    #[arg(long, value_name = "WHEN")]
    pub expires: Option<String>,
    /// A permission of the token; once per permission [default: send and receive].
    #[arg(long = "permission", value_name = "PERMISSION", value_enum)]
    pub permissions: Vec<Permission>,
}

/// A grant's channels and expiry and a token's permissions, checked against a space's latest
/// revision.
struct Checked {
    channels: NonEmpty<ChannelId>,
    expires: u64,
    permissions: Vec<Permission>,
}

impl Granted {
    /// Checks this against `latest`, the space's last accepted revision, at `now`: each channel is
    /// one of the space's, given once or more, and the expiry has not passed.
    fn checked(&self, latest: &Roster, now: u64) -> Result<Checked, String> {
        let body = latest.body();
        let mut channels = Vec::new();
        for given in &self.channels {
            let id: ChannelId = given
                .parse()
                .map_err(|_| format!("\"{}\" is no channel ID", printable(given)))?;
            if !body.channels.iter().any(|channel| channel.id == id) {
                return Err(format!(
                    "the space {} has no channel {id}",
                    body.name.as_str()
                ));
            }
            if !channels.contains(&id) {
                channels.push(id);
            }
        }
        let channels = NonEmpty::from_vec(channels).ok_or("give a channel with --channel")?;
        let expires = match &self.expires {
            Some(given) => expiry(given, now)?,
            None => now + DEFAULT_DAYS * DAY,
        };
        if expires <= now {
            return Err(format!(
                "the expiry {} has passed; give a later one",
                printable(self.expires.as_deref().unwrap_or_default())
            ));
        }
        let mut permissions = self.permissions.clone();
        if permissions.is_empty() {
            permissions = vec![Permission::Send, Permission::Receive];
        }
        permissions.sort();
        permissions.dedup();
        Ok(Checked {
            channels,
            expires,
            permissions,
        })
    }
}

/// The expiry `given` as seconds since the Unix epoch: whole days from `now`, `<n>d`, or an RFC
/// 3339 date-time.
fn expiry(given: &str, now: u64) -> Result<u64, String> {
    let invalid = || {
        format!(
            "the expiry \"{}\" is neither whole days, such as 30d, nor an RFC 3339 date-time, such \
             as 2027-01-31T00:00:00Z",
            printable(given)
        )
    };
    if let Some(days) = given.strip_suffix('d') {
        return days
            .parse::<u64>()
            .ok()
            .and_then(|days| days.checked_mul(DAY))
            .and_then(|seconds| now.checked_add(seconds))
            .ok_or_else(invalid);
    }
    rfc3339(given).ok_or_else(invalid)
}

/// The seconds since the Unix epoch of the RFC 3339 date-time `text`, such as
/// `2027-01-31T00:00:00Z` or `2027-01-31T09:30:00+07:00`, or of an ISO 8601 form of one that jiff
/// reads; its fraction of a second dropped. `None` for a text without a date, a time and an
/// offset, or before the epoch.
fn rfc3339(text: &str) -> Option<u64> {
    let time: jiff::Timestamp = text.parse().ok()?;
    u64::try_from(time.as_second()).ok()
}

/// The owner and agent of `address`, `owner/agent`.
fn address(address: &str) -> Result<(Name, Name), String> {
    let invalid = || {
        format!(
            "\"{}\" is no service agent's address: give it as owner/agent, such as ci/deploy-bot",
            printable(address)
        )
    };
    let (owner, agent) = address.split_once('/').ok_or_else(invalid)?;
    let owner: Name = owner.parse().map_err(|_| invalid())?;
    let agent: Name = agent.parse().map_err(|_| invalid())?;
    Ok((owner, agent))
}

/// The `name` of the certificate of the service agent `owner/agent`: its address, cut to the
/// longest `name`.
fn cert_name(owner: &Name, agent: &Name) -> String {
    let mut name = format!("{owner}/{agent}");
    // Names are ASCII, so any cut is at a character's boundary.
    name.truncate(CERT_NAME_MAX);
    name
}

/// `DeviceRegistration` in the API spec.
#[derive(Serialize)]
struct Registration<'a> {
    device: String,
    kind: &'a str,
    signing: String,
    hpke: String,
    name: &'a str,
}

/// `ServiceTokenRequest` in the API spec.
#[derive(Serialize)]
struct TokenRequest {
    device: String,
    grant: String,
    permissions: Vec<Permission>,
}

/// The fields of `ServiceToken` and `NewServiceToken` in the API spec the CLI uses: those a token's
/// line shows, in its order, then the device and the token's value.
#[derive(Deserialize)]
struct Record {
    id: String,
    owner: String,
    agent: String,
    permissions: Vec<String>,
    channels: Vec<String>,
    created_at: String,
    expires_at: String,
    last_used_at: Option<String>,
    device: String,
    #[serde(default)]
    token: Option<String>,
}

/// `ServiceTokens` in the API spec.
#[derive(Deserialize)]
struct Records {
    tokens: Vec<Record>,
}

/// Fails, saying why, if `permissions`, those of a token of a `member` agent, lack the receive
/// permission: such an agent reads its Commits and Welcomes from its mailbox.
fn receives_if_member(member: bool, permissions: &[Permission]) -> Result<(), String> {
    if member && !permissions.contains(&Permission::Receive) {
        return Err(
            "a member agent reads its Commits and Welcomes from its mailbox, so its token needs \
             the receive permission: add --permission receive; nothing changed"
                .to_owned(),
        );
    }
    Ok(())
}

/// What a command reads of `SpaceUsage` in the API spec: the limit on the space's service agents,
/// absent while none applies.
#[derive(Deserialize)]
struct Usage {
    service_agents: Option<Limited>,
}

/// What a command reads of `UsageCount` in the API spec.
#[derive(Deserialize)]
struct Limited {
    limit: Option<u64>,
}

/// `skald create-service-agent`: creates the service agent `address` in `synced`, a space whose
/// admin's device this is, with `granted`, as a member of the grant's channels if `member`: the
/// revision then lists its device in their `services`. Writes its key secret and token to stdout,
/// or its key secret alone when the space lists the agent and its token fails. Refused before
/// anything is registered or signed: on a device that is no admin's, for an address the space's
/// last accepted revision has, for channels or an expiry that `granted` gets wrong, for a member
/// without the receive permission, and when that revision has as many service agents as the
/// server allows.
pub fn create(
    synced: Synced,
    address_given: &str,
    granted: &Granted,
    member: bool,
) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster: latest,
        devices,
    } = synced;
    follower.may_change(&space, &latest)?;
    let (owner, agent) = address(address_given)?;
    let name = latest.body().name.as_str().to_owned();
    if address_taken(latest.body(), &owner, &agent) {
        return Err(format!(
            "the space {name} has the address {owner}/{agent} already, as a service agent's or as \
             one of a member's agents; nothing changed"
        ));
    }
    let checked = granted.checked(&latest, now())?;
    receives_if_member(member, &checked.permissions)?;
    if let Some(limit) = agents_limit(&mut follower, space.id)?
        && latest.body().service_agents.len() as u64 >= limit
    {
        return Err(format!(
            "the space {name} has as many service agents as it may on this server: {limit}; \
             nothing changed"
        ));
    }
    let admin = follower.keys()?;
    let (keys, cert) = made(&mut follower, &admin, (&owner, &agent))?;
    let device = keys.id();
    let entry = ServiceAgent {
        device,
        owner: owner.clone(),
        agent: agent.clone(),
        unknown: Unknown::default(),
    };
    let unlisted = |why: String| {
        format!(
            "{why}; the service device {device} is registered and certified, but the space has no \
             entry of it: create the service agent again"
        )
    };
    let next = follower
        .next_by((&latest, &devices), |body| {
            body.service_agents.push(entry);
            for channel in (body.channels.iter_mut())
                .filter(|channel| member && checked.channels.contains(&channel.id))
            {
                channel.services.push(device);
            }
        })
        .map_err(unlisted)?;
    let mut certs = devices.certs;
    certs.push(cert);
    follower.accepts(&latest, &next, &certs).map_err(unlisted)?;
    let stored = follower
        .submitted(&space, &latest, &next)
        .map_err(|unsent| {
            if unsent.waits {
                format!(
                    "{}; the service device {device} is registered and certified: once the server \
                     takes that revision, the space lists {owner}/{agent} without a key secret, \
                     which nothing holds",
                    unsent.why
                )
            } else {
                unlisted(unsent.why)
            }
        })?;
    if entry_of(&stored, &owner, &agent) != Some(device) {
        return Err(unlisted(format!(
            "another revision of the space {name} took the address {owner}/{agent} first"
        )));
    }
    let key = key_secret(&follower, &keys, (space.id, stored.hash()));
    hand_over(
        &mut follower,
        (space.id, &admin),
        (&owner, &agent, device),
        &checked,
        &key,
    )
}

/// `skald replace-service-key`: replaces the key of the service agent `address` in `synced`, a
/// space whose admin's device this is, its device one of this device's account, which alone
/// revokes it. A new `service` device, made as [`create`] makes one, takes the old one's place in
/// the agent's entry and in `services` in the space's next revision, which ends the old device's
/// tokens on the server; this device then revokes the old device. Writes the new key secret, whose
/// anchor is that revision, and a token with `granted`, to stdout. Refused before anything is
/// registered or signed: on a device that is no admin's, for an agent the space's last accepted
/// revision does not list or of another account, for channels or an expiry that `granted` gets
/// wrong, and for a member without the receive permission.
pub fn replace(synced: Synced, address_given: &str, granted: &Granted) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster: latest,
        devices,
    } = synced;
    follower.may_change(&space, &latest)?;
    let (owner, agent) = address(address_given)?;
    let old = listed(&latest, &owner, &agent)?;
    if !own_device(&follower, &devices, old) {
        return Err(format!(
            "the service agent {owner}/{agent} is a device of another account, and only that \
             account revokes its devices: an admin's device of that account replaces its key; \
             nothing changed"
        ));
    }
    let checked = granted.checked(&latest, now())?;
    let member = (latest.body().channels.iter()).any(|channel| channel.services.contains(&old));
    receives_if_member(member, &checked.permissions)?;
    let admin = follower.keys()?;
    let (keys, cert) = made(&mut follower, &admin, (&owner, &agent))?;
    let device = keys.id();
    let unlisted = |why: String| {
        format!(
            "{why}; the service device {device} is registered and certified, but the space lists \
             the old device {old} as {owner}/{agent} still: replace its key again"
        )
    };
    let next = follower
        .next_by((&latest, &devices), |body| {
            for entry in (body.service_agents.iter_mut()).filter(|entry| entry.device == old) {
                entry.device = device;
            }
            for listed in (body.channels.iter_mut())
                .flat_map(|channel| channel.services.iter_mut())
                .filter(|listed| **listed == old)
            {
                *listed = device;
            }
        })
        .map_err(unlisted)?;
    let mut certs = devices.certs;
    certs.push(cert);
    follower.accepts(&latest, &next, &certs).map_err(unlisted)?;
    let stored = follower
        .submitted(&space, &latest, &next)
        .map_err(|unsent| {
            if unsent.waits {
                format!(
                    "{}; the service device {device} is registered and certified: once the server \
                     takes that revision, the space lists {owner}/{agent} without a key secret, \
                     which nothing holds, and the old device {old} is not revoked",
                    unsent.why
                )
            } else {
                unlisted(unsent.why)
            }
        })?;
    if entry_of(&stored, &owner, &agent) != Some(device) {
        return Err(unlisted(format!(
            "another revision of the space {} changed {owner}/{agent} first",
            space.name
        )));
    }
    let key = key_secret(&follower, &keys, (space.id, stored.hash()));
    let handed = hand_over(
        &mut follower,
        (space.id, &admin),
        (&owner, &agent, device),
        &checked,
        &key,
    );
    drop(key);
    let revoked = revoke_device(&mut follower, &admin, old);
    finish(&mut follower, &space.name);
    handed.and(revoked)
}

/// `skald remove-service-agent`: removes the service agent `address` from `synced`, a space whose
/// admin's device this is. The space's next revision lists its device neither in an entry nor in
/// a channel's `services`, which ends its tokens on the server. This device then revokes the device
/// if it is of this device's account; otherwise that account does.
pub fn remove(synced: Synced, address_given: &str) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster: latest,
        devices,
    } = synced;
    follower.may_change(&space, &latest)?;
    let (owner, agent) = address(address_given)?;
    let device = listed(&latest, &owner, &agent)?;
    let next = follower.next_by((&latest, &devices), |body| {
        body.service_agents.retain(|entry| entry.device != device);
        for channel in &mut body.channels {
            channel.services.retain(|listed| *listed != device);
        }
    })?;
    follower
        .accepts(&latest, &next, &devices.certs)
        .map_err(|err| format!("{err}; nothing is sent"))?;
    let stored = follower.submit(&space, &latest, &next)?;
    let body = stored.body();
    if (body.service_agents.iter()).any(|entry| entry.device == device)
        || (body.channels.iter()).any(|channel| channel.services.contains(&device))
    {
        return Err(format!(
            "another revision of the space {} came first and lists {owner}/{agent} still; run \
             the command again",
            space.name
        ));
    }
    println!(
        "Removed the service agent {owner}/{agent} from the space {}: this device signed its \
         revision {}.",
        space.name, body.revision
    );
    let revoked = if own_device(&follower, &devices, device) {
        let admin = follower.keys()?;
        revoke_device(&mut follower, &admin, device)
    } else {
        println!(
            "Its device {device} is of another account, which revokes it with skald revoke-device."
        );
        Ok(())
    };
    finish(&mut follower, &space.name);
    revoked
}

/// The device of the service agent `owner/agent` in `latest`, a space's last accepted revision;
/// why not if it has no such entry.
fn listed(latest: &Roster, owner: &Name, agent: &Name) -> Result<DeviceId, String> {
    entry_of(latest, owner, agent).ok_or_else(|| {
        format!(
            "the space {} has no service agent {owner}/{agent}; nothing changed",
            latest.body().name.as_str()
        )
    })
}

/// Whether `device` is of the account of `follower`: its certificate among `devices` passes checks
/// 1 and 2 of "Device checks" against the master key the device pinned, revoked or not.
fn own_device(follower: &Follower, devices: &SpaceDevices, device: DeviceId) -> bool {
    skald_crypto::check_device(&follower.own.master, &devices.certs, device)
        .is_ok_and(|cert| cert.body().account == follower.own.account)
}

/// Revokes `device`, a `service` device of this device's account, by a revocation this device,
/// whose keys are `admin`, signs; a `service` device certifies no other device.
fn revoke_device(follower: &mut Follower, admin: &Keys, device: DeviceId) -> Result<(), String> {
    let (account, this) = (follower.own.account, follower.this);
    revocations::signed_and_stored(
        (&mut follower.connection, &follower.token, &follower.state),
        (account, this, admin),
        device,
        &[],
    )
    .map_err(|err| format!("{err}; the service device {device} is not revoked yet"))?;
    eprintln!("skald: revoked the service device {device}");
    Ok(())
}

/// A new `service` device of the service agent `owner/agent` on this device's account, whose keys
/// are `admin`: its keys, generated in memory, and the certificate this device issues, registered
/// and stored on the server.
fn made(
    follower: &mut Follower,
    admin: &Keys,
    (owner, agent): (&Name, &Name),
) -> Result<(Keys, Cert), String> {
    let keys = Keys::generate();
    register(follower, &keys, &cert_name(owner, agent))?;
    let cert = certify(follower, admin, &keys, &cert_name(owner, agent))?;
    // This device made it, so its next device checks do not warn of it as new.
    follower
        .state
        .device_statuses(&[(keys.id(), false)])
        .map_err(text)?;
    Ok((keys, cert))
}

/// The key secret of the `service` device of `keys` on `follower`'s account, in `space`, whose
/// revision `anchor` lists it, in base64url without padding.
fn key_secret(
    follower: &Follower,
    keys: &Keys,
    (space, anchor): (SpaceId, [u8; 32]),
) -> Zeroizing<String> {
    let key = ServiceKey {
        signing: *keys.signing().to_bytes(),
        hpke: *keys.hpke().to_bytes(),
        account: follower.own.account,
        master: follower.own.master,
        space,
        anchor,
        server: follower.connection.server().to_owned(),
        unknown: Unknown::default(),
    };
    Zeroizing::new(URL_SAFE_NO_PAD.encode(Zeroizing::new(key.to_cbor())))
}

/// Creates a token of `device`, the service agent `owner/agent` of `space` that the space lists
/// now, with `checked`, its grant signed by this device, whose keys are `admin`, and writes `key`,
/// its key secret, and the token to stdout; without a token, the key secret still goes out.
fn hand_over(
    follower: &mut Follower,
    (space, admin): (SpaceId, &Keys),
    (owner, agent, device): (&Name, &Name, DeviceId),
    checked: &Checked,
    key: &str,
) -> Result<(), String> {
    let tokenless = |why: String| {
        println!("{KEY_VARIABLE}={key}");
        format!(
            "{why}; the space lists {owner}/{agent}, and {KEY_VARIABLE} above is its key secret: \
             create its token with skald create-service-token"
        )
    };
    let token = new_token(follower, (space, admin), device, checked).map_err(tokenless)?;
    let token = Zeroizing::new(token.token.ok_or_else(|| {
        tokenless(
            "the server's new token has no value; revoke it with skald revoke-service-token"
                .to_owned(),
        )
    })?);
    println!("{KEY_VARIABLE}={key}\n{TOKEN_VARIABLE}={}", *token);
    Ok(())
}

/// `skald create-service-token`: a new token of the service agent `address` in `synced`, a space
/// whose admin's device this is, with a new grant as `granted` says; revokes the agent's earlier
/// tokens unless `keep_earlier`. Writes the token to stdout. Refused for an agent the space's last
/// accepted revision lists in a channel's `services` without the receive permission.
pub fn renew(
    synced: Synced,
    address_given: &str,
    granted: &Granted,
    keep_earlier: bool,
) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster: latest,
        ..
    } = synced;
    if !admin_device(&latest, follower.own.account, follower.this) {
        return Err(no_admin(&latest));
    }
    let (owner, agent) = address(address_given)?;
    let device = entry_of(&latest, &owner, &agent).ok_or_else(|| {
        format!(
            "the space {} has no service agent {owner}/{agent}",
            latest.body().name.as_str()
        )
    })?;
    let checked = granted.checked(&latest, now())?;
    let member = (latest.body().channels.iter()).any(|channel| channel.services.contains(&device));
    receives_if_member(member, &checked.permissions)?;
    let admin = follower.keys()?;
    let mut token = new_token(&mut follower, (space.id, &admin), device, &checked)?;
    let value =
        Zeroizing::new(token.token.take().ok_or(
            "the server's new token has no value; revoke it with skald revoke-service-token",
        )?);
    println!("{TOKEN_VARIABLE}={}", *value);
    drop(value);
    if keep_earlier {
        return Ok(());
    }
    let device = device.to_string();
    for earlier in records(&mut follower, space.id)?
        .into_iter()
        .filter(|record| record.device == device && record.id != token.id)
    {
        revoke_one(&mut follower, space.id, &earlier.id).map_err(|err| {
            format!(
                "{err}; the earlier token {} still works: revoke it with skald \
                 revoke-service-token",
                earlier.id
            )
        })?;
        eprintln!("skald: revoked the earlier token {}", earlier.id);
    }
    Ok(())
}

/// `skald service-tokens`: one line per live token of `synced`'s space, as the server lists them
/// to an admin's session.
pub fn list(synced: Synced) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        ..
    } = synced;
    let records = records(&mut follower, space.id)?;
    if records.is_empty() {
        println!("The space {} has no live service token.", space.name);
    }
    for record in records {
        println!("{}", line(&record));
    }
    Ok(())
}

/// `skald revoke-service-token`: revokes the token `id` of `synced`'s space.
pub fn revoke(synced: Synced, id: &str) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        ..
    } = synced;
    revoke_one(&mut follower, space.id, id)?;
    println!("Revoked the service token {}.", printable(id));
    Ok(())
}

/// A token's line in `skald service-tokens`.
fn line(record: &Record) -> String {
    format!(
        "{}  {}/{}  {}  channels {}  created {}  expires {}  last used {}",
        printable(&record.id),
        printable(&record.owner),
        printable(&record.agent),
        printable(&record.permissions.join(",")),
        printable(&record.channels.join(",")),
        printable(&record.created_at),
        printable(&record.expires_at),
        record
            .last_used_at
            .as_deref()
            .map_or("never".to_owned(), printable),
    )
}

/// The device of the service agent `owner/agent` in `roster`, if it has the entry.
fn entry_of(roster: &Roster, owner: &Name, agent: &Name) -> Option<DeviceId> {
    (roster.body().service_agents.iter())
        .find(|entry| entry.owner == *owner && entry.agent == *agent)
        .map(|entry| entry.device)
}

/// `POST /devices`: registers the `service` device of `keys`, named `name`, on the account of
/// `follower`'s session.
fn register(follower: &mut Follower, keys: &Keys, name: &str) -> Result<(), String> {
    let registration = Registration {
        device: keys.id().to_string(),
        kind: "service",
        signing: URL_SAFE_NO_PAD.encode(keys.signing().public_key()),
        hpke: URL_SAFE_NO_PAD.encode(keys.hpke().public_key()),
        name,
    };
    let body = serde_json::to_vec(&registration).expect("a registration is JSON");
    follower
        .connection
        .send_json_as(Method::Post, "/devices", &follower.token, &body)
        .map(drop)
        .map_err(|err| format!("{err}; the service device is not registered, nothing changed"))
}

/// `PUT /devices/<device>/cert`: the certificate of the `service` device of `keys`, named `name`,
/// on `follower`'s account, issued now by this device, whose keys are `admin`.
fn certify(follower: &mut Follower, admin: &Keys, keys: &Keys, name: &str) -> Result<Cert, String> {
    let device = keys.id();
    let body = CertBody {
        account: follower.own.account,
        device,
        kind: DeviceKind::Service,
        signing: keys.signing().public_key(),
        hpke: keys.hpke().public_key(),
        issuer: Some(follower.this),
        issued: now(),
        ssh: None,
        name: name.to_owned(),
        unknown: Unknown::default(),
    };
    let cert = skald_crypto::issue(&body, admin.signing(), keys.signing())
        .map_err(|err| format!("the service device's certificate fails: {err}"))?;
    let upload = serde_json::json!({ "cert": URL_SAFE_NO_PAD.encode(cert.to_cbor()) });
    let path = format!("/devices/{device}/cert");
    follower
        .connection
        .send_json_as(
            Method::Put,
            &path,
            &follower.token,
            upload.to_string().as_bytes(),
        )
        .map_err(|err| {
            format!(
                "{err}; the service device {device} is registered without a certificate, and the \
                 space has no entry of it: create the service agent again"
            )
        })?;
    Ok(cert)
}

/// `POST /spaces/<space>/service-tokens`: a token of the service device `device` with `checked`,
/// its grant signed by this device, whose keys are `admin`, and the request proved by them.
fn new_token(
    follower: &mut Follower,
    (space, admin): (SpaceId, &Keys),
    device: DeviceId,
    checked: &Checked,
) -> Result<Record, String> {
    let grant = sign_grant(
        admin.signing(),
        GrantBody {
            service: device,
            channels: checked.channels.clone(),
            signer: follower.this,
            expires: checked.expires,
            unknown: Unknown::default(),
        },
    );
    let request = TokenRequest {
        device: device.to_string(),
        grant: URL_SAFE_NO_PAD.encode(grant.to_cbor()),
        permissions: checked.permissions.clone(),
    };
    let body = serde_json::to_vec(&request).expect("a token request is JSON");
    let failed = |err: String| format!("{err}; no token of {device} was created");
    let proof = follower
        .connection
        .request_proof(&follower.token, follower.this, admin.signing())
        .map_err(|err| failed(err.to_string()))?;
    let path = format!("/spaces/{space}/service-tokens");
    let answer = follower
        .connection
        .post_json_proved(&path, (&follower.token, &proof), &body)
        .map_err(|err| failed(err.to_string()))?;
    let answer = Zeroizing::new(answer);
    serde_json::from_slice(&answer).map_err(|_| {
        "the server's answer holds no new token; list the tokens with skald service-tokens"
            .to_owned()
    })
}

/// The grant of `body`, signed with `key`.
fn sign_grant(key: &SigningKey, body: GrantBody) -> Grant {
    let signature = key.sign(GRANT_LABEL, &[&body.to_cbor()]);
    Grant { body, signature }
}

/// `GET /spaces/<space>/service-tokens`: the live tokens of `space`.
fn records(follower: &mut Follower, space: SpaceId) -> Result<Vec<Record>, String> {
    let path = format!("/spaces/{space}/service-tokens");
    let answer = follower
        .connection
        .get_as(&path, &follower.token)
        .map_err(text)?;
    serde_json::from_slice::<Records>(&answer)
        .map(|records| records.tokens)
        .map_err(|_| "the server's answer holds no list of tokens".to_owned())
}

/// `GET /spaces/<space>/usage`: the server's limit on the service agents of `space`, if it has one.
fn agents_limit(follower: &mut Follower, space: SpaceId) -> Result<Option<u64>, String> {
    let path = format!("/spaces/{space}/usage");
    let answer = follower
        .connection
        .get_as(&path, &follower.token)
        .map_err(text)?;
    serde_json::from_slice::<Usage>(&answer)
        .map(|usage| usage.service_agents.and_then(|agents| agents.limit))
        .map_err(|_| "the server's answer holds no usage of the space".to_owned())
}

/// `DELETE /spaces/<space>/service-tokens/<id>`.
fn revoke_one(follower: &mut Follower, space: SpaceId, id: &str) -> Result<(), String> {
    // A path segment: the server takes only an ID, whose characters need no escaping.
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("\"{}\" is no token ID", printable(id)));
    }
    let path = format!("/spaces/{space}/service-tokens/{id}");
    follower
        .connection
        .delete_as(&path, &follower.token)
        .map_err(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whole days count from now; an RFC 3339 date-time is read in UTC, with its offset.
    #[test]
    fn expiry_is_days_from_now_or_a_date_time() {
        let now = 1_800_000_000;
        assert_eq!(expiry("30d", now), Ok(now + 30 * DAY));
        assert_eq!(expiry("0d", now), Ok(now));
        for (given, seconds) in [
            ("1970-01-01T00:00:00Z", 0),
            ("2020-01-01T00:00:00Z", 1_577_836_800),
            ("2027-01-31T00:00:00Z", 1_801_353_600),
            ("2027-01-31t07:00:00.25+07:00", 1_801_353_600),
            ("2027-01-30T19:30:00-04:30", 1_801_353_600),
            ("2028-02-29T00:00:00Z", 1_835_395_200),
            ("9999-01-01T00:00:00Z", 253_370_764_800),
        ] {
            assert_eq!(expiry(given, now), Ok(seconds), "{given}");
        }
        for given in [
            "",
            "d",
            "-1d",
            "1.5d",
            "99999999999999999999d",
            "184467440737095516d",
            "2027-01-31",
            "2027-01-31T00:00:00",
            "2027-02-29T00:00:00Z",
            "2027-13-01T00:00:00Z",
            "2027-01-31T24:00:00Z",
            "2027-01-31T00:00:00.Z",
            "2027-01-31T00:00:00Z ",
            "1969-12-31T23:59:59Z",
        ] {
            let taken = expiry(given, now);
            assert!(taken.is_err(), "{given}: {taken:?}");
        }
    }

    /// Channels must be the space's, an expiry must not have passed, and the permissions default
    /// to send and receive; repeats count once.
    #[test]
    fn grants_take_the_space_s_channels_and_a_later_expiry() {
        let roster = crate::spaces::tests::first(skald_proto::AccountId::generate());
        let mut body = roster.body().clone();
        let (default, other) = (ChannelId::generate(), ChannelId::generate());
        body.channels = vec![crate::spaces::tests::channel(default, &[])];
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let now = 1_800_000_000;
        let granted = |channels: &[ChannelId], expires: Option<&str>, permissions| Granted {
            channels: channels.iter().map(ToString::to_string).collect(),
            expires: expires.map(str::to_owned),
            permissions,
        };

        let checked = granted(&[default, default], None, vec![])
            .checked(&roster, now)
            .unwrap();
        assert_eq!(checked.channels.clone().into_vec(), [default]);
        assert_eq!(checked.expires, now + 90 * DAY);
        assert_eq!(checked.permissions, [Permission::Send, Permission::Receive]);
        let queues = vec![Permission::Queues, Permission::Queues];
        let checked = granted(&[default], Some("1d"), queues)
            .checked(&roster, now)
            .unwrap();
        assert_eq!(
            (checked.expires, checked.permissions),
            (now + DAY, vec![Permission::Queues])
        );

        for (refused, why) in [
            (granted(&[other], None, vec![]), "no channel"),
            (granted(&[], None, vec![]), "give a channel"),
            (granted(&[default], Some("0d"), vec![]), "expir"),
            (
                granted(&[default], Some("2020-01-01T00:00:00Z"), vec![]),
                "expir",
            ),
            (granted(&[default], Some("soon"), vec![]), "expir"),
        ] {
            let err = refused.checked(&roster, now).err().unwrap();
            assert!(err.contains(why), "{why}: {err}");
        }
        let unknown = Granted {
            channels: vec!["general".to_owned()],
            expires: None,
            permissions: vec![],
        };
        let err = unknown.checked(&roster, now).err().unwrap();
        assert!(err.contains("no channel ID"), "{err}");
    }

    /// An address is two names around a slash; the certificate's `name` is the address, cut to 64
    /// bytes.
    #[test]
    fn addresses_are_owner_and_agent() {
        let (owner, agent) = address("ci/deploy-bot").unwrap();
        assert_eq!((owner.as_str(), agent.as_str()), ("ci", "deploy-bot"));
        assert_eq!(cert_name(&owner, &agent), "ci/deploy-bot");
        for given in [
            "ci",
            "ci/",
            "/bot",
            "ci/deploy/bot",
            "CI/bot",
            "ci/deploy bot",
        ] {
            assert!(address(given).is_err(), "{given}");
        }
        let long = "a".repeat(63);
        let (owner, agent) = address(&format!("{long}/{long}")).unwrap();
        assert_eq!(cert_name(&owner, &agent).len(), CERT_NAME_MAX);
    }

    /// An agent's device is that of the entry with both its owner and its name, among agents that
    /// share an owner or a name.
    #[test]
    fn agent_is_found_by_owner_and_name() {
        let entry = crate::spaces::tests::agent;
        let agents = [
            entry("ci", "deploy-bot", 1),
            entry("ci", "release-bot", 2),
            entry("ops", "deploy-bot", 3),
        ];
        let mut body = crate::spaces::tests::first(skald_proto::AccountId::generate())
            .body()
            .clone();
        body.service_agents = agents.to_vec();
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let name = |text: &str| text.parse::<Name>().unwrap();
        for agent in &agents {
            assert_eq!(
                entry_of(&roster, &agent.owner, &agent.agent),
                Some(agent.device),
                "{}/{}",
                agent.owner,
                agent.agent
            );
        }
        assert_eq!(entry_of(&roster, &name("ops"), &name("release-bot")), None);
    }

    /// A token's line has its ID, address, permissions, channel IDs, creation, expiry and last use.
    #[test]
    fn token_lines_show_each_field() {
        let record = Record {
            id: "0193a1b2-0000-7000-8000-000000000001".to_owned(),
            owner: "ci".to_owned(),
            agent: "deploy-bot".to_owned(),
            permissions: vec!["send".to_owned(), "receive".to_owned()],
            channels: vec!["one".to_owned(), "two".to_owned()],
            created_at: "2026-11-02T00:00:00.000000Z".to_owned(),
            expires_at: "2027-01-31T00:00:00.000000Z".to_owned(),
            last_used_at: None,
            device: "device".to_owned(),
            token: None,
        };
        assert_eq!(
            line(&record),
            "0193a1b2-0000-7000-8000-000000000001  ci/deploy-bot  send,receive  channels one,two  \
             created 2026-11-02T00:00:00.000000Z  expires 2027-01-31T00:00:00.000000Z  last used \
             never"
        );
        let used = Record {
            last_used_at: Some("2026-11-03T00:00:00.000000Z".to_owned()),
            ..record
        };
        assert!(
            line(&used).ends_with("  last used 2026-11-03T00:00:00.000000Z"),
            "{}",
            line(&used)
        );
    }
}
