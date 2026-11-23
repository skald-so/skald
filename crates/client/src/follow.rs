//! Following a space's roster (`docs/protocol/membership.md`, "Roster"): before a command uses a
//! space, the device fetches the space's revisions and the devices of its accounts, and accepts the
//! revisions after the last one it accepted in order, with checks 1 to 4. A revision that fails,
//! and every revision built on it, has no effect: the device keeps the last revision it accepted,
//! warns its user and keeps working with it. Another valid revision with a number it accepted means
//! the server forked the roster: the device keeps its own and warns. It warns when a revision it
//! accepts changes a member's `master`, again for an account its person verified, which is then
//! no longer verified ("Account fingerprint"), and when a member's device is revoked, valid again
//! or new since it last checked it ("Device checks"), and when a member account's record changes
//! its `recovery` ([`crate::records`]). A revision's signer is checked with the revocations of its
//! account. The run that accepts a revision changing a member's role tells its user: the member's
//! handle, its new role and the handle of the signing device's account. The first run to learn that
//! the server reports a member deleted tells its user, naming its handle; its devices are then not
//! admitted ([`crate::send::admitted`]).
//!
//! A device follows only the spaces it has an anchor for: revision 1 of a space it created, its
//! personal space included, which counts as accepted before the server takes it, or the latest
//! revision its link gave it. Any other revision counts as accepted only once the server gives it,
//! those the device signed too. A device whose earliest accepted revision is after revision 1 also
//! takes, without a warning, each earlier one the server gives whose hash is the `prev` of one it
//! took, back to revision 1: the history checks senders against them ("History entry").

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{RosterError, accept_next, check_anchors, check_device, check_valid_device};
use skald_proto::{
    AccountId, AccountRecord, Anchors, Cbor as _, Cert, DeviceId, DeviceKind, Member, Revocation,
    Role, Roster, SpaceId,
};

use crate::connection::Connection;
use crate::devices::{self, text};
use crate::key_changes;
use crate::records;
use crate::revocations;
use crate::state::{self, State};

/// How many times a run accepts a space's revisions again when another run accepted some first, or
/// signs a revision again when another came first.
pub const ATTEMPTS: usize = 5;

/// `RosterRevisions` in the API spec.
#[derive(Deserialize)]
struct Revisions {
    revisions: Vec<String>,
}

/// Whom the device trusts a space's revisions by: its own account and the account's master key it
/// pinned.
pub struct Own {
    pub account: AccountId,
    pub master: [u8; 32],
}

/// The devices of a list, such as of a space's accounts, as the server gives them: their IDs, the
/// certificates that decode, the revocations that decode with those the device holds, and, for a
/// space, the account records of its accounts that decode, unchecked, and the accounts the server
/// reports deleted, its word alone.
#[derive(Clone, Default)]
pub struct SpaceDevices {
    pub ids: Vec<DeviceId>,
    pub certs: Vec<Cert>,
    pub revocations: Vec<Revocation>,
    pub records: Vec<AccountRecord>,
    pub deleted: BTreeSet<AccountId>,
}

/// The master key a device checks the certificates of `account` against in `roster`: the one `own`
/// pinned for its own account, otherwise the `master` the revision gives the account's member.
pub fn master_of(roster: &Roster, own: &Own, account: AccountId) -> Option<[u8; 32]> {
    if account == own.account {
        Some(own.master)
    } else {
        roster.body().member(account).map(|member| member.master)
    }
}

impl SpaceDevices {
    /// The valid certificates of `member`'s devices, `member` of `roster`, that its `excluded` does
    /// not list, checked with the revocations against the master key [`master_of`] gives.
    pub fn of(&self, roster: &Roster, own: &Own, member: &Member) -> Vec<&Cert> {
        let Some(master) = master_of(roster, own, member.account) else {
            return Vec::new();
        };
        let excluded = |device: &DeviceId| {
            (member.excluded.as_ref()).is_some_and(|excluded| excluded.contains(device))
        };
        let checked = devices::check(&master, self);
        (checked.linked.into_iter())
            .filter(|cert| cert.body().account == member.account && !excluded(&cert.body().device))
            .collect()
    }
}

/// What a `service` device starts its space's roster from: the hash of its key secret's anchor,
/// and the anchors objects of its account's devices ([`roster_start`]).
pub struct Start {
    pub anchor: [u8; 32],
    pub objects: Vec<Anchors>,
}

/// Fetches the revisions of `space`, the space the state holds, and the devices of its accounts, as
/// the session `token`, accepts the new revisions, records them in `state` and prints the warnings,
/// those of master keys ([`master_warnings`]) of `every_space` the device follows or only of this
/// one. Gives the space's last accepted revision, its devices and the revisions the server gives.
/// If `hold`, role changes and reported deletions wait for the person's run. A `service` device
/// accepts revisions up to its roster's `start` ([`roster_start`]) without checking their signer
/// for revocation.
pub fn sync(
    connection: &mut Connection,
    (state, token): (&State, &str),
    space: &state::Space,
    (own, every_space, hold): (&Own, bool, bool),
    start: Option<&Start>,
) -> Result<(Roster, SpaceDevices, Vec<Roster>), String> {
    let (id, name) = (space.id, &space.name);
    let mut warnings = Vec::new();
    let mut given = Vec::new();
    for (place, entry) in listed(connection, token, id)?.iter().enumerate() {
        match decoded(entry) {
            Some(roster) => given.push(roster),
            None => warnings.push(format!(
                "revision {} the server gives of the space {name} does not decode, so it has no \
                 effect",
                place + 1
            )),
        }
    }
    let devices = space_devices(connection, token, state, id)?;

    let trusted = match start {
        Some(start) => {
            let (trusted, warning) = roster_start(start, &given, &devices, own);
            warnings.extend(warning);
            trusted
        }
        None => BTreeSet::new(),
    };
    let held = read(state, id, name)?;
    let (current, followed, notices) =
        settle(state, (id, name), held, (&given, &trusted), (&devices, own))?;
    revocations::keep(state, &devices, |account| master_of(&current, own, account))?;
    let mut changed = device_changes(state, &current, &devices, own)?;
    changed.extend(master_warnings(state, own, (!every_space).then_some(id))?);
    // A stateless service agent's run wraps no history key, so it takes no account record.
    let records = if crate::service::stateless() {
        Vec::new()
    } else {
        records::settle(state, (&current, &devices), own)?
    };
    for warning in (warnings.iter())
        .chain(&followed)
        .chain(&changed)
        .chain(&records)
    {
        eprintln!("skald: warning: {warning}");
    }
    // A stateless run accepts the same revisions each time.
    if !crate::service::stateless() {
        let lines: Vec<String> = notices
            .iter()
            .chain(&deletion_notices(state, &current, &devices, own)?)
            .map(|notice| format!("skald: {notice}"))
            .collect();
        if hold {
            if !lines.is_empty() {
                crate::recv::keep_held(state.keep_warnings(&lines), &lines);
            }
        } else {
            for line in lines {
                eprintln!("{line}");
            }
        }
    }
    Ok((current, devices, given))
}

/// The notice of each member of `roster`, a space's last accepted revision, but `own`'s account,
/// that `devices` reports deleted and the device has not told its user of since the server last
/// reported it ([`State::report_deleted`]).
fn deletion_notices(
    state: &State,
    roster: &Roster,
    devices: &SpaceDevices,
    own: &Own,
) -> Result<Vec<String>, String> {
    let body = roster.body();
    let reported: BTreeSet<AccountId> = (body.members.iter())
        .map(|member| member.account)
        .filter(|account| *account != own.account && devices.deleted.contains(account))
        .collect();
    let new = state.report_deleted(body.space, &reported).map_err(text)?;
    Ok((new.iter())
        .filter_map(|account| body.member(*account))
        .map(|member| {
            format!(
                "the server reports the member {} of the space {} deleted; its devices leave the \
                 space's channels",
                member.handle.as_str(),
                body.name.as_str()
            )
        })
        .collect())
}

/// Records in `state` whether each device of a member of `roster`, a space's last accepted
/// revision, is revoked, its certificate checked as [`devices::check`] does among `devices`
/// against the master key [`master_of`] gives; gives the warning of each revoked, valid again or
/// new since the device last checked it.
pub fn device_changes(
    state: &State,
    roster: &Roster,
    devices: &SpaceDevices,
    own: &Own,
) -> Result<Vec<String>, String> {
    let body = roster.body();
    let mut warnings = Vec::new();
    for member in body.members.iter() {
        let Some(master) = master_of(roster, own, member.account) else {
            continue;
        };
        let checked = devices::check(&master, devices);
        let whose = format!("{} in {}", member.handle.as_str(), body.name.as_str());
        warnings.extend(devices::statuses(
            state,
            &checked,
            Some(member.account),
            &whose,
        )?);
    }
    Ok(warnings)
}

/// The revisions of the space `id` the server gives the session `token` that decode.
pub fn given(connection: &mut Connection, token: &str, id: SpaceId) -> Result<Vec<Roster>, String> {
    Ok(listed(connection, token, id)?
        .iter()
        .filter_map(|entry| decoded(entry))
        .collect())
}

/// The revision `entry`, in base64url as the server gives it, if it decodes.
fn decoded(entry: &str) -> Option<Roster> {
    URL_SAFE_NO_PAD
        .decode(entry)
        .ok()
        .and_then(|bytes| Roster::from_cbor(&bytes).ok())
}

/// The revisions of the space `id` the server gives the session `token`, each in base64url as it
/// gives it.
pub fn listed(
    connection: &mut Connection,
    token: &str,
    id: SpaceId,
) -> Result<Vec<String>, String> {
    let body = connection
        .get_as(&format!("/spaces/{id}/revisions"), token)
        .map_err(text)?;
    let list: Revisions = serde_json::from_slice(&body)
        .map_err(|_| "unexpected answer from the server: a revision list without its revisions")?;
    Ok(list.revisions)
}

/// The devices of the accounts of the space `id`, as the server gives them to the session `token`,
/// with the revocations `state` holds.
pub fn space_devices(
    connection: &mut Connection,
    token: &str,
    state: &State,
    id: SpaceId,
) -> Result<SpaceDevices, String> {
    devices::fetch(connection, &format!("/spaces/{id}/devices"), token, state)
}

/// What `state` holds of the space `id` named `name`.
pub fn read(state: &State, id: SpaceId, name: &str) -> Result<state::Accepted, String> {
    state
        .accepted(id)
        .map_err(text)?
        .ok_or_else(|| format!("this device dropped the space {name} meanwhile"))
}

/// Accepts the revisions of `given` that follow `held`, what `state` held of the space `id` named
/// `name` when the run read it, with `devices`, its accounts', and `own`, and records them. While
/// another run accepted revisions of the space first, it accepts again from what `state` then
/// holds. Gives the space's last accepted revision, the warnings and the role changes of the
/// revisions it recorded ([`changed_roles`]); keeps the master keys they change and the accounts
/// no longer verified as key changes ([`key_changes::keep`]).
fn settle(
    state: &State,
    (id, name): (SpaceId, &str),
    mut held: state::Accepted,
    (given, trusted): (&[Roster], &BTreeSet<[u8; 32]>),
    (devices, own): (&SpaceDevices, &Own),
) -> Result<(Roster, Vec<String>, Vec<String>), String> {
    for _ in 0..ATTEMPTS {
        let last = decode(&held.last, name)?;
        let Followed {
            accepted,
            earlier,
            warnings,
        } = follow_trusting(&last, &held.hashes, (given, trusted), devices, own);
        let changes = master_changes(&last, &accepted);
        let changed: Vec<String> = (std::iter::once(&last).chain(&accepted))
            .zip(&accepted)
            .flat_map(|(previous, next)| changed_masters(previous, next))
            .collect();
        let mut recorded = earlier;
        let notices: Vec<String> = (std::iter::once(&last).chain(&accepted))
            .zip(&accepted)
            .flat_map(|(previous, next)| changed_roles(previous, next, devices, own))
            .collect();
        // A run that takes nothing writes back the same revision, so it gives what the state
        // holds if another run accepted revisions first.
        let newest = match accepted.split_last() {
            None => last,
            Some((newest, before)) => {
                recorded.push((last.body().revision, last.hash()));
                recorded.extend(
                    before
                        .iter()
                        .map(|roster| (roster.body().revision, roster.hash())),
                );
                newest.clone()
            }
        };
        if state
            .accept(id, &held.last, &recorded, &newest.to_cbor())
            .map_err(text)?
        {
            let unverified = unverify(state, name, &changes)?;
            for line in changed.iter().chain(&unverified) {
                key_changes::keep(state, line)?;
            }
            let mut warnings = warnings;
            warnings.extend(unverified);
            return Ok((newest, warnings, notices));
        }
        held = read(state, id, name)?;
    }
    Err(format!(
        "other skald runs kept accepting revisions of the space {name}; run the command again"
    ))
}

/// Each member whose `master` the revisions of `accepted`, in order after `last`, change: its
/// account, its handle and its master key before the change.
fn master_changes(last: &Roster, accepted: &[Roster]) -> Vec<(AccountId, String, [u8; 32])> {
    let previous = std::iter::once(last).chain(accepted);
    (previous.zip(accepted))
        .flat_map(|(previous, next)| {
            (next.body().members.iter()).filter_map(|member| {
                let before = previous.body().member(member.account)?;
                (before.master != member.master).then(|| {
                    let handle = member.handle.as_str().to_owned();
                    (member.account, handle, before.master)
                })
            })
        })
        .collect()
}

/// Records in `state` that each account of `changes` ([`master_changes`]) its person verified for
/// its master key before the change is no longer verified ("Account fingerprint"), and gives the
/// warning of each; `name` names the space.
fn unverify(
    state: &State,
    name: &str,
    changes: &[(AccountId, String, [u8; 32])],
) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    for (account, handle, before) in changes {
        if state.verified(*account, before).map_err(text)? {
            state.set_verified(*account, before, false).map_err(text)?;
            warnings.push(format!(
                "{handle} in the space {name} is no longer verified: its master key changed; \
                 compare fingerprints again: skald fingerprint {name} {handle}"
            ));
        }
    }
    Ok(warnings)
}

/// The revision of the space `name` in `bytes`, as the local state holds it.
pub fn decode(bytes: &[u8], name: &str) -> Result<Roster, String> {
    Roster::from_cbor(bytes)
        .map_err(|err| state::Error::Malformed(format!("the revision of the space {name}: {err}")))
        .map_err(text)
}

/// What following a space's revisions gives.
#[derive(Debug, Default, PartialEq)]
pub struct Followed {
    /// The revisions newly accepted, in order.
    pub accepted: Vec<Roster>,
    /// The number and hash of each revision newly taken before the earliest accepted one, as
    /// [`earlier`] gives them.
    pub earlier: Vec<(u64, [u8; 32])>,
    /// What the device warns its user of.
    pub warnings: Vec<String>,
}

/// Accepts the revisions of `given`, all those the server gives of a space, that follow `last`,
/// the space's last accepted revision, the hashes of the earlier ones it accepted in `hashes`, with
/// `devices`, its accounts', and the master key `own` pinned; takes those [`earlier`] gives.
pub fn follow(
    last: &Roster,
    hashes: &BTreeMap<u64, [u8; 32]>,
    given: &[Roster],
    devices: &SpaceDevices,
    own: &Own,
) -> Followed {
    follow_trusting(last, hashes, (given, &BTreeSet::new()), devices, own)
}

/// As [`follow`], accepting the revisions whose hash `trusted` holds without the check that their
/// signer is not revoked ([`roster_start`]).
fn follow_trusting(
    last: &Roster,
    hashes: &BTreeMap<u64, [u8; 32]>,
    (given, trusted): (&[Roster], &BTreeSet<[u8; 32]>),
    devices: &SpaceDevices,
    own: &Own,
) -> Followed {
    let name = last.body().name.as_str();
    let accepts = |previous: &Roster, next: &Roster| {
        accept_next(previous, next, &devices.certs, own.account, &own.master)?;
        if trusted.contains(&next.hash()) {
            return Ok(());
        }
        signer_unrevoked(previous, next.body().signer, devices, own)
    };
    let mut known = hashes.clone();
    known.insert(last.body().revision, last.hash());
    let mut followed = Followed {
        earlier: earlier(last, &known, given),
        ..Followed::default()
    };
    known.extend(followed.earlier.iter().copied());
    let mut sorted: Vec<&Roster> = given.iter().collect();
    sorted.sort_by_key(|roster| roster.body().revision);
    let mut current = last;
    for &roster in &sorted {
        let number = roster.body().revision;
        let hash = roster.hash();
        let next = current.body().revision.checked_add(1);
        match known.get(&number) {
            Some(accepted) if *accepted == hash => {}
            Some(_) if number == 1 => followed.warnings.push(format!(
                "the server gives another revision 1 of the space {name} than the one this device \
                 holds as its anchor; it has no effect"
            )),
            Some(_) => {
                // The revision this device accepted before it, if it holds it or the server gives
                // it.
                let previous = known.get(&(number - 1)).and_then(|accepted| {
                    std::iter::once(last)
                        .chain(&followed.accepted)
                        .chain(sorted.iter().copied())
                        .find(|roster| roster.hash() == *accepted)
                });
                followed.warnings.push(match previous.map(|previous| accepts(previous, roster)) {
                    Some(Ok(())) => format!(
                        "the server forked the roster of the space {name}: it gives another valid \
                         revision {number} than the one this device accepted, which it keeps"
                    ),
                    Some(Err(err)) => format!(
                        "revision {number} of the space {name} is another than the one this \
                         device accepted and fails: {err}; it has no effect"
                    ),
                    None => format!(
                        "revision {number} of the space {name} is another than the one this \
                         device accepted and builds on none it accepted; it has no effect"
                    ),
                });
            }
            None if Some(number) == next => match accepts(current, roster) {
                Ok(()) => {
                    followed.warnings.extend(changed_masters(current, roster));
                    known.insert(number, hash);
                    followed.accepted.push(roster.clone());
                    current = roster;
                }
                Err(err) => followed.warnings.push(format!(
                    "revision {number} of the space {name} fails: {err}; it has no effect, and \
                     this device keeps revision {}",
                    current.body().revision
                )),
            },
            // After the next revision, or before the earliest accepted one and off the chain of
            // `prev` back from it.
            None => followed.warnings.push(format!(
                "revision {number} of the space {name} builds on no revision this device \
                 accepted; it has no effect, and this device keeps revision {}",
                current.body().revision
            )),
        }
    }
    followed
}

/// The revisions a `service` device accepts without the check that their signer is not revoked,
/// by hash, of `given`, all those the server gives of its space, with `devices`, its accounts', and
/// the master key `own` pinned (`docs/protocol/membership.md`, "Anchors"): those from its key
/// secret's anchor, `start.anchor`, up to its roster's start. The start is the latest revision that
/// the anchors object of a valid `client` device of its account names, of `start.objects`, when the
/// revisions from the anchor up to it pass through every other one so named, numbered above the
/// anchor; otherwise the anchor, with the warning it gives. An object counts only with a signature
/// that verifies with its device's certificate; named revisions at or below the anchor do not
/// count, and one above it whose `prev` chain does not reach the anchor is off that chain.
pub fn roster_start(
    start: &Start,
    given: &[Roster],
    devices: &SpaceDevices,
    own: &Own,
) -> (BTreeSet<[u8; 32]>, Option<String>) {
    let by_hash: BTreeMap<[u8; 32], &Roster> =
        given.iter().map(|roster| (roster.hash(), roster)).collect();
    let none = (BTreeSet::new(), None);
    let Some(anchor) = by_hash.get(&start.anchor) else {
        return none;
    };
    let (space, from) = (anchor.body().space, anchor.body().revision);
    // The hashes from `roster` back to the anchor, by `prev`; none if they do not reach it.
    let chain = |roster: &Roster| {
        let mut hashes = BTreeSet::new();
        let mut at = roster;
        while at.body().revision > from {
            hashes.insert(at.hash());
            match at.body().prev.and_then(|prev| by_hash.get(&prev)) {
                Some(previous) if previous.body().revision + 1 == at.body().revision => {
                    at = previous;
                }
                _ => return None,
            }
        }
        (at.hash() == start.anchor).then_some(hashes)
    };
    // Each named revision above the anchor, with the hashes of its chain to the anchor, if any.
    let mut named: Vec<(&Roster, Option<BTreeSet<[u8; 32]>>)> = Vec::new();
    for object in &start.objects {
        let body = object.body();
        let valid = body.account == own.account
            && check_valid_device(
                &own.master,
                &devices.certs,
                &devices.revocations,
                body.device,
            )
            .is_ok_and(|cert| {
                cert.body().kind == DeviceKind::Client
                    && check_anchors(object, &cert.body().signing).is_ok()
            });
        if !valid {
            continue;
        }
        for (_, hash) in body.anchors.iter().filter(|(of, _)| *of == space) {
            if let Some(roster) = by_hash.get(hash)
                && roster.body().revision > from
            {
                named.push((roster, chain(roster)));
            }
        }
    }
    let Some((latest, hashes)) = named
        .iter()
        .max_by_key(|(roster, _)| roster.body().revision)
    else {
        return none;
    };
    if let Some(hashes) = hashes
        && named
            .iter()
            .all(|(roster, _)| hashes.contains(&roster.hash()))
    {
        return (hashes.clone(), None);
    }
    let name = anchor.body().name.as_str();
    let warning = format!(
        "the anchors objects of this account's devices name revisions of the space {name} that \
         are not on one chain, up to revision {}: this service device starts from its key \
         secret's anchor, revision {from}",
        latest.body().revision
    );
    (BTreeSet::new(), Some(warning))
}

/// The number and hash of each revision of `given` before the earliest of `known`, the revisions
/// accepted by number, that the device takes: going back from that earliest one, as `last` or
/// `given` holds it, each whose hash is the `prev` of the one after it, down to revision 1 or the
/// first that `given` lacks.
fn earlier(
    last: &Roster,
    known: &BTreeMap<u64, [u8; 32]>,
    given: &[Roster],
) -> Vec<(u64, [u8; 32])> {
    let Some((&first, &hash)) = known.first_key_value() else {
        return Vec::new();
    };
    let with = |number: u64, hash: [u8; 32]| {
        std::iter::once(last)
            .chain(given)
            .find(|roster| roster.body().revision == number && roster.hash() == hash)
    };
    let mut taken = Vec::new();
    let mut after = with(first, hash);
    while let Some(roster) = after {
        let (Some(prev), Some(number)) =
            (roster.body().prev, roster.body().revision.checked_sub(1))
        else {
            break;
        };
        after = with(number, prev);
        if after.is_some() {
            taken.push((number, prev));
        }
    }
    taken
}

/// Check 1 of "Roster" with the revocations of `devices`: the `signer` of the revision after
/// `previous`, whose certificate passes checks 1 and 2 against the master key of an admin's account
/// in `previous`, is not revoked.
fn signer_unrevoked(
    previous: &Roster,
    signer: DeviceId,
    devices: &SpaceDevices,
    own: &Own,
) -> Result<(), RosterError> {
    let revoked = chained_to(previous, signer, devices, own).any(|(_, master)| {
        check_valid_device(&master, &devices.certs, &devices.revocations, signer).is_err()
    });
    if revoked {
        return Err(RosterError::Signer(signer));
    }
    Ok(())
}

/// Each member of `roster`, with its master key as [`master_of`] gives it, whose account the
/// certificate of `device` among `devices` chains to by checks 1 and 2 of "Device checks".
fn chained_to<'r>(
    roster: &'r Roster,
    device: DeviceId,
    devices: &SpaceDevices,
    own: &Own,
) -> impl Iterator<Item = (&'r Member, [u8; 32])> {
    roster.body().members.iter().filter_map(move |member| {
        let master = master_of(roster, own, member.account)?;
        check_device(&master, &devices.certs, device)
            .is_ok_and(|cert| cert.body().account == member.account)
            .then_some((member, master))
    })
}

/// What the device tells its user of `next`, accepted after `previous`, for each member whose role
/// it changes: the member's handle, its new role and the handle of the account of `next`'s signer,
/// an admin's device in `previous` whose certificate among `devices` chains to the account's
/// master key.
fn changed_roles(
    previous: &Roster,
    next: &Roster,
    devices: &SpaceDevices,
    own: &Own,
) -> Vec<String> {
    let (before, body) = (previous.body(), next.body());
    let changed: Vec<&Member> = (body.members.iter())
        .filter(|member| (before.member(member.account)).is_some_and(|was| was.role != member.role))
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }
    let signer = body.signer;
    let by = chained_to(previous, signer, devices, own)
        .next()
        .map_or_else(
            || format!("the device {signer}"),
            |(admin, _)| admin.handle.as_str().to_owned(),
        );
    (changed.into_iter())
        .map(|member| {
            let role = match member.role {
                Role::Admin => "an admin",
                Role::Member => "a member",
            };
            format!(
                "{} is now {role} of the space {}, by {by}'s revision {}.",
                member.handle.as_str(),
                body.name.as_str(),
                body.revision
            )
        })
        .collect()
}

/// The warnings of `next`, accepted after `previous`, for each member whose `master` it changes.
fn changed_masters(previous: &Roster, next: &Roster) -> Vec<String> {
    let body = next.body();
    body.members
        .iter()
        .filter(|member| {
            previous
                .body()
                .member(member.account)
                .is_some_and(|before| before.master != member.master)
        })
        .map(|member| {
            format!(
                "revision {} of the space {} changes the master key of the member {}: only its \
                 devices with certificates from the new key count",
                body.revision,
                body.name.as_str(),
                member.handle.as_str()
            )
        })
        .collect()
}

/// The master keys the last accepted revisions of the device's spaces give one account, each with
/// the spaces that give it.
struct Masters {
    account: AccountId,
    handle: String,
    given: Vec<([u8; 32], Vec<String>)>,
}

/// The warning of each account that the last accepted revisions of the device's spaces in `state`,
/// of each or only of `only`, give different master keys, the one `own` pinned for its own account
/// among them ("Device checks"); each set of keys of an account once, kept as a key change.
pub fn master_warnings(
    state: &State,
    own: &Own,
    only: Option<SpaceId>,
) -> Result<Vec<String>, String> {
    let mut accounts = vec![Masters {
        account: own.account,
        handle: String::new(),
        given: vec![(own.master, Vec::new())],
    }];
    let spaces = state.spaces().map_err(text)?;
    let followed = spaces
        .iter()
        .filter(|space| only.is_none_or(|id| id == space.id));
    for space in followed {
        let Ok(roster) = decode(&space.accepted, &space.name) else {
            continue;
        };
        for member in roster.body().members.iter() {
            let at = match accounts.iter().position(|it| it.account == member.account) {
                Some(at) => at,
                None => {
                    accounts.push(Masters {
                        account: member.account,
                        handle: member.handle.as_str().to_owned(),
                        given: Vec::new(),
                    });
                    accounts.len() - 1
                }
            };
            let given = &mut accounts[at].given;
            match given.iter_mut().find(|(key, _)| *key == member.master) {
                Some((_, names)) => names.push(space.name.clone()),
                None => given.push((member.master, vec![space.name.clone()])),
            }
        }
    }
    let mut warnings = Vec::new();
    for Masters {
        account,
        handle,
        given,
    } in accounts
    {
        if given.len() < 2 {
            continue;
        }
        let mut keys: Vec<String> = (given.iter())
            .map(|(master, _)| master.iter().map(|byte| format!("{byte:02x}")).collect())
            .collect();
        keys.sort();
        if !(state.master_warning(&format!("{account} {}", keys.join(" ")))).map_err(text)? {
            continue;
        }
        // The spaces that give a key, or a key other than the pinned one, as a subject.
        let spaces = |own_too: bool| -> String {
            let names: Vec<&str> = (given.iter())
                .filter(|(master, _)| own_too || *master != own.master)
                .flat_map(|(_, names)| names.iter().map(String::as_str))
                .collect();
            match names.as_slice() {
                [name] => format!("the space {name} gives"),
                names => format!("the spaces {} give", names.join(", ")),
            }
        };
        let warning = if account == own.account {
            format!(
                "{} this account a master key other than the one this device pinned: the server \
                 may have swapped it",
                spaces(false)
            )
        } else {
            format!(
                "{} the account {handle} different master keys: the server may have swapped one",
                spaces(true)
            )
        };
        key_changes::keep(state, &warning)?;
        warnings.push(warning);
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;
    use skald_crypto::{SigningKey, issue};
    use skald_proto::{CertBody, DeviceKind, Member, Name, Role, RosterBody, SpaceId, Unknown};

    /// An account with its master key and one device, certified by that key.
    struct Person {
        account: AccountId,
        handle: Name,
        master: SigningKey,
        key: SigningKey,
        cert: Cert,
    }

    impl Person {
        fn new(handle: &str) -> Self {
            let (account, master, key) = (
                AccountId::generate(),
                SigningKey::generate(),
                SigningKey::generate(),
            );
            let device = DeviceId::from_signing_key(&key.public_key());
            let body = CertBody {
                account,
                device,
                kind: DeviceKind::Client,
                signing: key.public_key(),
                hpke: [5; 32],
                issuer: None,
                issued: 1_800_000_000,
                ssh: None,
                github: None,
                name: format!("{handle}-laptop"),
                unknown: Unknown::default(),
            };
            let cert = issue(&body, &master, &key).expect("a certificate");
            Person {
                account,
                handle: handle.parse().expect("a handle"),
                master,
                key,
                cert,
            }
        }

        fn device(&self) -> DeviceId {
            self.cert.body().device
        }

        /// Its entry in a roster, as `role`.
        fn member(&self, role: Role) -> Member {
            Member {
                account: self.account,
                handle: self.handle.clone(),
                role,
                master: self.master.public_key(),
                invite: None,
                excluded: None,
                unknown: Unknown::default(),
            }
        }

        /// What it trusts revisions by.
        fn own(&self) -> Own {
            Own {
                account: self.account,
                master: self.master.public_key(),
            }
        }

        /// Revision 1 of the space `acme`, which it created.
        fn creates(&self) -> Roster {
            let body = RosterBody::first(
                SpaceId::generate(),
                "acme".parse().expect("a name"),
                self.account,
                self.handle.clone(),
                self.master.public_key(),
                self.device(),
                1_800_000_000,
            );
            skald_crypto::sign_roster(&body, &self.key).expect("a revision")
        }

        /// The revision after `previous` that its device signs, its members as `change` leaves
        /// them.
        fn signs(&self, previous: &Roster, change: impl FnOnce(&mut Vec<Member>)) -> Roster {
            let mut body = previous.body().clone();
            body.revision += 1;
            body.prev = Some(previous.hash());
            body.signer = self.device();
            let mut members = body.members.iter().cloned().collect();
            change(&mut members);
            body.members = skald_proto::NonEmpty::from_vec(members).expect("a member");
            skald_crypto::sign_roster(&body, &self.key).expect("a revision")
        }
    }

    /// Ada creates `acme`; its revision 2, by Ada, adds Bob as an admin and Carol as a member.
    struct Acme {
        ada: Person,
        bob: Person,
        carol: Person,
        first: Roster,
        second: Roster,
    }

    impl Acme {
        fn new() -> Self {
            let (ada, bob, carol) = (Person::new("ada"), Person::new("bob"), Person::new("carol"));
            let first = ada.creates();
            let second = ada.signs(&first, |members| {
                members.push(bob.member(Role::Admin));
                members.push(carol.member(Role::Member));
            });
            Acme {
                ada,
                bob,
                carol,
                first,
                second,
            }
        }

        fn certs(&self) -> Vec<Cert> {
            [&self.ada, &self.bob, &self.carol]
                .iter()
                .map(|person| person.cert.clone())
                .collect()
        }

        /// What Ada's device gets of `given` after `last`, with the hashes of the revisions before
        /// it.
        fn follow(&self, last: &Roster, given: &[&Roster]) -> Followed {
            let hashes = [&self.first, &self.second]
                .iter()
                .map(|roster| (roster.body().revision, roster.hash()))
                .filter(|(revision, _)| *revision < last.body().revision)
                .collect();
            let given: Vec<Roster> = given.iter().map(|roster| (*roster).clone()).collect();
            follow(last, &hashes, &given, &self.devices(), &self.ada.own())
        }

        /// Their devices, with `revocations`.
        fn devices_with(&self, revocations: Vec<Revocation>) -> SpaceDevices {
            let certs = self.certs();
            SpaceDevices {
                ids: certs.iter().map(|cert| cert.body().device).collect(),
                certs,
                revocations,
                records: Vec::new(),
                deleted: Default::default(),
            }
        }

        fn devices(&self) -> SpaceDevices {
            self.devices_with(Vec::new())
        }
    }

    /// A state that follows `acme` at its revision 2, what a run read of it then, and revisions 3
    /// by Bob and 4 by Ada after it.
    struct Race {
        state: State,
        id: SpaceId,
        stale: state::Accepted,
        third: Roster,
        fourth: Roster,
        _dir: TestDir,
    }

    impl Race {
        fn new(acme: &Acme, name: &str) -> Self {
            let dir = TestDir::new(name);
            let state = State::open(&dir).expect("a state");
            let id = acme.first.body().space;
            let first = acme.first.to_cbor();
            state.add_space(id, "acme", &first).unwrap();
            let hashes = [(1, acme.first.hash())];
            assert!(
                state
                    .accept(id, &first, &hashes, &acme.second.to_cbor())
                    .unwrap()
            );
            let stale = state.accepted(id).unwrap().unwrap();
            let third = acme.bob.signs(&acme.second, |_| {});
            let fourth = acme.ada.signs(&third, |_| {});
            Race {
                state,
                id,
                stale,
                third,
                fourth,
                _dir: dir,
            }
        }

        /// What the state holds now.
        fn held(&self) -> state::Accepted {
            self.state.accepted(self.id).unwrap().unwrap()
        }
    }

    /// The first sync to learn that the server reports a member deleted tells of it once, naming
    /// its handle; not of the device's own account, nor of a member it does not report.
    #[test]
    fn a_member_reported_deleted_is_told_once_but_not_the_devices_own_account() {
        let acme = Acme::new();
        let dir = TestDir::new("follow-deletion-notices");
        let state = State::open(&dir).expect("a state");
        let devices = SpaceDevices {
            deleted: BTreeSet::from([acme.ada.account, acme.bob.account]),
            ..acme.devices()
        };
        let own = acme.ada.own();

        let told = deletion_notices(&state, &acme.second, &devices, &own).unwrap();

        assert_eq!(told.len(), 1, "{told:?}");
        assert!(
            told[0].contains("the member bob of the space acme deleted"),
            "{told:?}"
        );
        let again = deletion_notices(&state, &acme.second, &devices, &own).unwrap();
        assert!(again.is_empty(), "{again:?}");
    }

    /// What a state holds once it accepted `before`, then `last`.
    fn accepted(last: &Roster, before: &[&Roster]) -> state::Accepted {
        state::Accepted {
            last: last.to_cbor(),
            hashes: before
                .iter()
                .map(|roster| (roster.body().revision, roster.hash()))
                .collect(),
        }
    }

    /// What `settle` gives of `acme` in `state` for a run given `given` that read `held`, with
    /// what it tells.
    fn settled_telling(
        acme: &Acme,
        state: &State,
        held: state::Accepted,
        given: &[&Roster],
    ) -> (Roster, Vec<String>, Vec<String>) {
        let given: Vec<Roster> = given.iter().map(|roster| (*roster).clone()).collect();
        let id = (acme.first.body().space, "acme");
        let trusted = BTreeSet::new();
        let own = acme.ada.own();
        settle(state, id, held, (&given, &trusted), (&acme.devices(), &own)).expect("settled")
    }

    /// What `settle` gives of `acme` in `state` for a run given `given` that read `held`.
    fn settled(
        acme: &Acme,
        state: &State,
        held: state::Accepted,
        given: &[&Roster],
    ) -> (Roster, Vec<String>) {
        let (roster, warnings, _) = settled_telling(acme, state, held, given);
        (roster, warnings)
    }

    /// The run that records a revision changing roles tells, for each member whose role it
    /// changes, its handle, its new role and the handle of the signer's account; no later run
    /// tells it again, and a revision that changes no role tells nothing.
    #[test]
    fn role_change_is_told_once_by_the_run_that_records_it() {
        let acme = Acme::new();
        let race = Race::new(&acme, "follow-told-roles");
        let third = acme.bob.signs(&acme.second, |members| {
            set_role(members, acme.carol.account, Role::Admin);
            set_role(members, acme.bob.account, Role::Member);
        });
        let fourth = acme.ada.signs(&third, |_| {});
        let given = [&acme.first, &acme.second, &third, &fourth];

        let (latest, _, told) = settled_telling(&acme, &race.state, race.held(), &given);

        assert_eq!(latest, fourth);
        assert_eq!(
            told,
            [
                "bob is now a member of the space acme, by bob's revision 3.",
                "carol is now an admin of the space acme, by bob's revision 3.",
            ]
        );
        let (_, _, again) = settled_telling(&acme, &race.state, race.held(), &given);
        assert!(again.is_empty(), "{again:?}");
        let (_, _, stale) = settled_telling(&acme, &race.state, race.stale.clone(), &given);
        assert!(stale.is_empty(), "{stale:?}");
    }

    /// Sets the role of `account` in `members`.
    fn set_role(members: &mut [Member], account: AccountId, role: Role) {
        if let Some(member) = members.iter_mut().find(|member| member.account == account) {
            member.role = role;
        }
    }

    #[test]
    fn revisions_after_the_last_are_accepted_in_order_without_a_warning() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |members| {
            set_role(members, acme.ada.account, Role::Member);
        });

        let followed = acme.follow(&acme.first, &[&third, &acme.first, &acme.second]);

        assert_eq!(
            followed,
            Followed {
                accepted: vec![acme.second.clone(), third.clone()],
                earlier: vec![],
                warnings: vec![],
            }
        );
        assert_eq!(
            acme.follow(&acme.second, &[&acme.first, &acme.second, &third])
                .accepted,
            vec![third]
        );
        assert_eq!(
            acme.follow(&acme.second, &[&acme.first, &acme.second]),
            Followed::default()
        );
    }

    #[test]
    fn revisions_before_the_earliest_accepted_on_its_prev_chain_are_taken_without_a_warning() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |_| {});
        let given = [acme.first.clone(), acme.second.clone(), third.clone()];

        // A device linked at revision 3 holds no earlier one.
        let followed = follow(
            &third,
            &BTreeMap::new(),
            &given,
            &acme.devices(),
            &acme.ada.own(),
        );

        assert_eq!(
            followed,
            Followed {
                accepted: vec![],
                earlier: vec![(2, acme.second.hash()), (1, acme.first.hash())],
                warnings: vec![],
            }
        );
    }

    #[test]
    fn revision_before_the_earliest_accepted_off_its_prev_chain_is_not_taken() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |_| {});
        let other_second = acme.ada.signs(&acme.first, |members| {
            set_role(members, acme.carol.account, Role::Admin);
        });

        let followed = follow(
            &third,
            &BTreeMap::new(),
            &[acme.first.clone(), other_second, third.clone()],
            &acme.devices(),
            &acme.ada.own(),
        );

        assert_eq!(followed.accepted, vec![]);
        assert_eq!(followed.earlier, vec![]);
        assert_eq!(
            followed.warnings,
            [1, 2]
                .map(|number| format!(
                    "revision {number} of the space acme builds on no revision this device \
                     accepted; it has no effect, and this device keeps revision 3"
                ))
                .to_vec()
        );
    }

    #[test]
    fn linked_device_records_the_revisions_it_takes_before_its_anchor_once() {
        let acme = Acme::new();
        let dir = TestDir::new("follow-linked");
        let state = State::open(&dir).expect("a state");
        let third = acme.bob.signs(&acme.second, |_| {});
        let id = acme.first.body().space;
        state.add_space(id, "acme", &third.to_cbor()).unwrap();
        let given = [&acme.first, &acme.second, &third];

        let held = state.accepted(id).unwrap().unwrap();
        assert_eq!(
            settled(&acme, &state, held, &given),
            (third.clone(), vec![])
        );
        let now = state.accepted(id).unwrap().unwrap();
        assert_eq!(now, accepted(&third, &[&acme.first, &acme.second]));

        assert_eq!(settled(&acme, &state, now.clone(), &given), (third, vec![]));
        assert_eq!(state.accepted(id).unwrap().unwrap(), now);
    }

    #[test]
    fn failing_revision_and_those_built_on_it_have_no_effect() {
        let acme = Acme::new();
        let by_carol = acme.carol.signs(&acme.second, |members| {
            set_role(members, acme.carol.account, Role::Admin);
        });
        let built = acme.bob.signs(&by_carol, |_| {});

        let followed = acme.follow(
            &acme.second,
            &[&acme.first, &acme.second, &by_carol, &built],
        );

        assert_eq!(followed.accepted, vec![]);
        let carol = acme.carol.device();
        assert_eq!(
            followed.warnings,
            vec![
                format!(
                    "revision 3 of the space acme fails: the signer {carol} is no valid client or \
                     browser device of an admin; it has no effect, and this device keeps revision 2"
                ),
                "revision 4 of the space acme builds on no revision this device accepted; it has \
                 no effect, and this device keeps revision 2"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn revision_by_a_revoked_device_of_an_admin_fails() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |_| {});
        let (account, device) = (acme.bob.account, acme.bob.device());
        let body = skald_proto::RevocationBody {
            account,
            device,
            keep: None,
            signer: Some(device),
            timestamp: 1_800_000_000,
            also: None,
            unknown: skald_proto::Unknown::default(),
        };
        let revocation = skald_crypto::sign_revocation(&body, &acme.bob.key).expect("a revocation");
        let mut forged = body.clone();
        forged.timestamp += 1;
        let forged = skald_crypto::sign_revocation(&forged, &acme.carol.key).expect("a revocation");
        let follow = |revocations: Vec<Revocation>| {
            let devices = acme.devices_with(revocations);
            follow(
                &acme.second,
                &BTreeMap::new(),
                std::slice::from_ref(&third),
                &devices,
                &acme.ada.own(),
            )
        };

        let revoked = follow(vec![revocation]);

        assert_eq!(revoked.accepted, vec![]);
        assert_eq!(
            revoked.warnings,
            vec![format!(
                "revision 3 of the space acme fails: the signer {device} is no valid client or \
                 browser device of an admin; it has no effect, and this device keeps revision 2"
            )]
        );
        assert_eq!(follow(vec![forged]).accepted, vec![third.clone()]);
    }

    #[test]
    fn valid_revision_with_the_number_of_a_failing_one_is_accepted() {
        let acme = Acme::new();
        let by_carol = acme.carol.signs(&acme.second, |_| {});
        let by_bob = acme.bob.signs(&acme.second, |_| {});

        let followed = acme.follow(&acme.second, &[&by_carol, &by_bob]);

        assert_eq!(followed.accepted, vec![by_bob]);
        assert_eq!(followed.warnings.len(), 1, "{followed:?}");
    }

    #[test]
    fn another_valid_revision_with_an_accepted_number_is_a_fork_and_the_own_one_is_kept() {
        let acme = Acme::new();
        let other = acme.ada.signs(&acme.first, |members| {
            members.push(acme.bob.member(Role::Member));
        });
        let built = acme.ada.signs(&other, |_| {});

        let followed = acme.follow(&acme.second, &[&acme.first, &other, &built]);

        assert_eq!(followed.accepted, vec![]);
        assert_eq!(
            followed.warnings,
            vec![
                "the server forked the roster of the space acme: it gives another valid revision 2 \
                 than the one this device accepted, which it keeps"
                    .to_owned(),
                "revision 3 of the space acme fails: prev is not the hash of the previous \
                 revision; it has no effect, and this device keeps revision 2"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn two_valid_revisions_with_one_number_in_one_list_are_a_fork_and_the_first_is_kept() {
        let acme = Acme::new();
        let by_bob = acme.bob.signs(&acme.second, |_| {});
        let by_ada = acme.ada.signs(&acme.second, |_| {});

        let followed = acme.follow(&acme.second, &[&by_bob, &by_ada]);

        assert_eq!(followed.accepted, vec![by_bob]);
        assert_eq!(
            followed.warnings,
            vec![
                "the server forked the roster of the space acme: it gives another valid revision 3 \
                 than the one this device accepted, which it keeps"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn another_revision_with_an_accepted_number_that_fails_or_follows_none_is_no_fork() {
        let acme = Acme::new();
        let by_bob = acme.bob.signs(&acme.first, |_| {});
        let third = acme.ada.signs(&acme.second, |_| {});
        let other_third = acme.ada.signs(&by_bob, |_| {});

        let followed = acme.follow(&third, &[&acme.first, &by_bob, &other_third]);

        assert_eq!(followed.accepted, vec![]);
        let bob = acme.bob.device();
        assert_eq!(
            followed.warnings,
            vec![
                format!(
                    "revision 2 of the space acme is another than the one this device accepted \
                     and fails: the signer {bob} is no valid client or browser device of an \
                     admin; it has no effect"
                ),
                "revision 3 of the space acme is another than the one this device accepted and \
                 builds on none it accepted; it has no effect"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn another_revision_1_than_the_anchor_has_no_effect() {
        let acme = Acme::new();
        let other = acme.ada.creates();

        let followed = acme.follow(&acme.first, &[&other]);

        assert_eq!(
            followed.warnings,
            vec![
                "the server gives another revision 1 of the space acme than the one this device \
                 holds as its anchor; it has no effect"
                    .to_owned()
            ]
        );
    }

    /// `roster` as a revision of another space named `name`, its members as `change` leaves them,
    /// signed by `key`.
    fn elsewhere(
        roster: &Roster,
        name: &str,
        change: impl Fn(&mut Member),
        key: &SigningKey,
    ) -> Roster {
        let mut body = roster.body().clone();
        body.space = SpaceId::generate();
        body.name = name.parse().expect("a name");
        let mut members: Vec<Member> = body.members.iter().cloned().collect();
        members.iter_mut().for_each(change);
        body.members = skald_proto::NonEmpty::from_vec(members).expect("a member");
        skald_crypto::sign_roster(&body, key).expect("a revision")
    }

    /// Two spaces that give Carol different master keys warn once, in a run that covers both; a
    /// space that gives the device's own account another key than the pinned one warns, naming
    /// it, also in a run of that space only; the same keys again warn of nothing.
    #[test]
    fn spaces_that_give_an_account_two_master_keys_warn_once() {
        let acme = Acme::new();
        let dir = TestDir::new("follow-master-warnings");
        let state = State::open(&dir).unwrap();
        let carol = acme.carol.account;
        let globex = elsewhere(
            &acme.second,
            "globex",
            |member| {
                if member.account == carol {
                    member.master = [9; 32];
                }
            },
            &acme.ada.key,
        );
        for roster in [&acme.second, &globex] {
            let body = roster.body();
            (state.add_space(body.space, body.name.as_str(), &roster.to_cbor())).unwrap();
        }
        let own = acme.ada.own();

        let only = Some(acme.second.body().space);
        assert!(master_warnings(&state, &own, only).unwrap().is_empty());
        assert_eq!(
            master_warnings(&state, &own, None).unwrap(),
            vec![
                "the spaces acme, globex give the account carol different master keys: the \
                 server may have swapped one"
                    .to_owned()
            ]
        );
        assert!(master_warnings(&state, &own, None).unwrap().is_empty());

        let ada = acme.ada.account;
        let initech = elsewhere(
            &acme.second,
            "initech",
            |member| {
                if member.account == ada {
                    member.master = [8; 32];
                }
            },
            &acme.ada.key,
        );
        let body = initech.body();
        (state.add_space(body.space, "initech", &initech.to_cbor())).unwrap();
        assert_eq!(
            master_warnings(&state, &own, Some(body.space)).unwrap(),
            vec![
                "the space initech gives this account a master key other than the one this \
                 device pinned: the server may have swapped it"
                    .to_owned()
            ]
        );
        assert!(master_warnings(&state, &own, None).unwrap().is_empty());
    }

    #[test]
    fn accepted_revision_that_changes_a_members_master_is_warned_of() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |members| {
            for member in members
                .iter_mut()
                .filter(|member| member.role == Role::Member)
            {
                member.master = [9; 32];
            }
        });

        let followed = acme.follow(&acme.second, &[&third]);

        assert_eq!(followed.accepted, vec![third]);
        assert_eq!(
            followed.warnings,
            vec![
                "revision 3 of the space acme changes the master key of the member carol: only \
                 its devices with certificates from the new key count"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn run_whose_space_another_run_moved_on_shows_what_the_state_holds_without_a_warning() {
        let acme = Acme::new();
        let race = Race::new(&acme, "follow-moved-on");
        let given = [&acme.first, &acme.second, &race.third, &race.fourth];

        // Another run moves the space from revision 2 to 4.
        assert_eq!(
            settled(&acme, &race.state, race.held(), &given),
            (race.fourth.clone(), vec![])
        );
        let now = race.held();
        assert_eq!(
            now,
            accepted(&race.fourth, &[&acme.first, &acme.second, &race.third])
        );

        assert_eq!(
            settled(&acme, &race.state, race.stale.clone(), &given),
            (race.fourth.clone(), vec![])
        );
        assert_eq!(race.held(), now);
    }

    #[test]
    fn run_that_takes_nothing_while_another_run_moved_on_gives_what_the_state_holds() {
        let acme = Acme::new();
        let race = Race::new(&acme, "follow-takes-nothing");
        let all = [&acme.first, &acme.second, &race.third, &race.fourth];
        assert_eq!(
            settled(&acme, &race.state, race.held(), &all),
            (race.fourth.clone(), vec![])
        );

        // A run that read revision 2 is given no later one.
        let stale = [&acme.first, &acme.second];

        assert_eq!(
            settled(&acme, &race.state, race.stale.clone(), &stale),
            (race.fourth.clone(), vec![])
        );
    }

    #[test]
    fn run_whose_space_another_run_moved_to_another_fork_keeps_that_one() {
        let acme = Acme::new();
        let race = Race::new(&acme, "follow-other-fork");
        let other = acme.ada.signs(&acme.second, |_| {});

        // Another run, given another revision 3, accepts it.
        let given = [&acme.first, &acme.second, &other];
        assert_eq!(
            settled(&acme, &race.state, race.held(), &given),
            (other.clone(), vec![])
        );

        let given = [&acme.first, &acme.second, &race.third, &race.fourth];
        assert_eq!(
            settled(&acme, &race.state, race.stale.clone(), &given),
            (
                other.clone(),
                vec![
                    "the server forked the roster of the space acme: it gives another valid \
                     revision 3 than the one this device accepted, which it keeps"
                        .to_owned(),
                    "revision 4 of the space acme fails: prev is not the hash of the previous \
                     revision; it has no effect, and this device keeps revision 3"
                        .to_owned(),
                ]
            )
        );
        assert_eq!(race.held(), accepted(&other, &[&acme.first, &acme.second]));
    }

    /// The revocation of Bob's device, which Bob's device signs.
    fn bob_revoked(acme: &Acme) -> Revocation {
        let device = acme.bob.device();
        let body = skald_proto::RevocationBody {
            account: acme.bob.account,
            device,
            keep: None,
            signer: Some(device),
            timestamp: 1_800_000_000,
            also: None,
            unknown: skald_proto::Unknown::default(),
        };
        skald_crypto::sign_revocation(&body, &acme.bob.key).expect("a revocation")
    }

    /// The anchors object of `person`'s device, signed by `key`, naming `named` as its revision of
    /// `acme`.
    fn anchors_of(person: &Person, key: &SigningKey, named: &Roster) -> Anchors {
        let body = skald_proto::AnchorsBody {
            account: person.account,
            device: person.device(),
            anchors: vec![(named.body().space, named.hash())],
            personal: None,
            unknown: Unknown::default(),
        };
        skald_crypto::sign_anchors(&body, key).expect("an anchors object")
    }

    /// Ada's other `client` device, its key, certificate and person.
    fn other_device(ada: &Person) -> Person {
        let key = SigningKey::generate();
        let mut body = ada.cert.body().clone();
        body.device = DeviceId::from_signing_key(&key.public_key());
        body.signing = key.public_key();
        body.name = "ada-desktop".to_owned();
        let cert = issue(&body, &ada.master, &key).expect("a certificate");
        Person {
            account: ada.account,
            handle: ada.handle.clone(),
            master: SigningKey::generate(),
            key,
            cert,
        }
    }

    /// The hashes of `rosters`.
    fn hashes(rosters: &[&Roster]) -> BTreeSet<[u8; 32]> {
        rosters.iter().map(|roster| roster.hash()).collect()
    }

    /// Ada's `acme` with revisions 3 by Bob, 4 by Ada, and 5 by Bob, whose device is revoked; Ada's
    /// other `client` device, her `service` device, and the service device's run, whose key
    /// secret's anchor is revision 2.
    struct Run {
        acme: Acme,
        desktop: Person,
        service: Person,
        third: Roster,
        fourth: Roster,
        given: Vec<Roster>,
        devices: SpaceDevices,
    }

    impl Run {
        fn new() -> Self {
            let acme = Acme::new();
            let desktop = other_device(&acme.ada);
            let mut service = other_device(&acme.ada);
            let mut body = service.cert.body().clone();
            body.kind = DeviceKind::Service;
            service.cert = issue(&body, &acme.ada.master, &service.key).expect("a certificate");
            let third = acme.bob.signs(&acme.second, |_| {});
            let fourth = acme.ada.signs(&third, |_| {});
            let fifth = acme.bob.signs(&fourth, |_| {});
            let given = [&acme.first, &acme.second, &third, &fourth, &fifth]
                .map(Clone::clone)
                .to_vec();
            let mut devices = acme.devices_with(vec![bob_revoked(&acme)]);
            for person in [&desktop, &service] {
                devices.ids.push(person.device());
                devices.certs.push(person.cert.clone());
            }
            Run {
                acme,
                desktop,
                service,
                third,
                fourth,
                given,
                devices,
            }
        }

        /// The revisions trusted with `objects`, and the warning.
        fn start(&self, objects: Vec<Anchors>) -> (BTreeSet<[u8; 32]>, Option<String>) {
            let start = Start {
                anchor: self.acme.second.hash(),
                objects,
            };
            roster_start(&start, &self.given, &self.devices, &self.acme.ada.own())
        }
    }

    #[test]
    fn service_device_trusts_the_revisions_up_to_the_latest_its_accounts_devices_anchored() {
        let run = Run::new();
        let (acme, desktop) = (&run.acme, &run.desktop);
        let (trusted, warning) = run.start(vec![
            anchors_of(desktop, &desktop.key, &run.third),
            anchors_of(&acme.ada, &acme.ada.key, &run.fourth),
            anchors_of(&acme.ada, &acme.ada.key, &acme.first),
        ]);
        assert_eq!(
            (trusted.clone(), warning),
            (hashes(&[&run.third, &run.fourth]), None)
        );

        // Bob's revoked device signed revisions 3 and 5: the third is trusted, the fifth fails.
        let own = acme.ada.own();
        let followed = |trusted: &BTreeSet<[u8; 32]>| {
            follow_trusting(
                &acme.second,
                &BTreeMap::new(),
                (&run.given, trusted),
                &run.devices,
                &own,
            )
        };
        let trusting = followed(&trusted);
        assert_eq!(
            trusting.accepted,
            vec![run.third.clone(), run.fourth.clone()]
        );
        assert_eq!(trusting.warnings.len(), 1, "{:?}", trusting.warnings);
        assert!(
            trusting.warnings[0].starts_with("revision 5 of the space acme fails"),
            "{:?}",
            trusting.warnings
        );
        assert_eq!(
            followed(&BTreeSet::new()).accepted,
            vec![],
            "without the start"
        );
    }

    #[test]
    fn service_device_without_anchored_revisions_above_its_anchor_trusts_none() {
        let run = Run::new();
        let (acme, service) = (&run.acme, &run.service);
        for (what, object) in [
            ("forged", anchors_of(&acme.ada, &acme.bob.key, &run.fourth)),
            (
                "of another account",
                anchors_of(&acme.bob, &acme.bob.key, &run.fourth),
            ),
            (
                "at or below the anchor",
                anchors_of(&acme.ada, &acme.ada.key, &acme.second),
            ),
            (
                "of a service device",
                anchors_of(service, &service.key, &run.fourth),
            ),
        ] {
            assert_eq!(run.start(vec![object]), (BTreeSet::new(), None), "{what}");
        }
    }

    #[test]
    fn service_device_whose_accounts_devices_anchored_a_fork_starts_from_its_anchor() {
        let mut run = Run::new();
        let other = run.acme.ada.signs(&run.acme.second, |_| {});
        run.given.push(other.clone());
        let (acme, desktop) = (&run.acme, &run.desktop);
        let (trusted, warning) = run.start(vec![
            anchors_of(&acme.ada, &acme.ada.key, &run.fourth),
            anchors_of(desktop, &desktop.key, &other),
        ]);
        assert_eq!(trusted, BTreeSet::new());
        assert_eq!(
            warning,
            Some(
                "the anchors objects of this account's devices name revisions of the space acme \
                 that are not on one chain, up to revision 4: this service device starts from \
                 its key secret's anchor, revision 2"
                    .to_owned()
            )
        );
    }

    #[test]
    fn service_device_counts_no_revoked_devices_anchors_and_warns_of_those_off_its_anchors_chain() {
        let mut run = Run::new();
        let acme = &run.acme;
        let off_second = acme.ada.signs(&acme.first, |_| {});
        let off_third = acme.ada.signs(&off_second, |_| {});
        run.given.extend([off_second, off_third.clone()]);
        let desktop = &run.desktop;
        // The warning of named revisions up to `latest` that are off one chain.
        let off = |latest: u64| {
            let warning = format!(
                "the anchors objects of this account's devices name revisions of the space acme \
                 that are not on one chain, up to revision {latest}: this service device starts \
                 from its key secret's anchor, revision 2"
            );
            (BTreeSet::new(), Some(warning))
        };
        let alone = run.start(vec![anchors_of(&acme.ada, &acme.ada.key, &off_third)]);
        assert_eq!(alone, off(3), "off the anchor's chain");
        let beside = run.start(vec![
            anchors_of(&acme.ada, &acme.ada.key, &off_third),
            anchors_of(desktop, &desktop.key, &run.fourth),
        ]);
        assert_eq!(beside, off(4), "beside an on-chain one");

        let body = skald_proto::RevocationBody {
            account: acme.ada.account,
            device: desktop.device(),
            keep: None,
            signer: Some(acme.ada.device()),
            timestamp: 1_800_000_000,
            also: None,
            unknown: skald_proto::Unknown::default(),
        };
        let revocation = skald_crypto::sign_revocation(&body, &acme.ada.key).expect("a revocation");
        let mut devices = run.devices.clone();
        devices.revocations.push(revocation);
        let start = Start {
            anchor: acme.second.hash(),
            objects: vec![anchors_of(desktop, &desktop.key, &run.fourth)],
        };
        assert_eq!(
            roster_start(&start, &run.given, &devices, &acme.ada.own()),
            (BTreeSet::new(), None),
            "of a revoked device"
        );
        assert_eq!(
            roster_start(&start, &run.given, &run.devices, &acme.ada.own()).0,
            hashes(&[&run.third, &run.fourth]),
            "of the device before its revocation"
        );
    }

    #[test]
    fn service_device_accepts_a_trusted_revision_only_if_an_admin_signed_it() {
        let acme = Acme::new();
        let by_carol = acme.carol.signs(&acme.second, |_| {});
        let given = [&acme.first, &acme.second, &by_carol].map(Clone::clone);
        let start = Start {
            anchor: acme.second.hash(),
            objects: vec![anchors_of(&acme.ada, &acme.ada.key, &by_carol)],
        };
        let own = acme.ada.own();
        let (trusted, _) = roster_start(&start, &given, &acme.devices(), &own);
        assert_eq!(trusted, hashes(&[&by_carol]));
        let followed = follow_trusting(
            &acme.second,
            &BTreeMap::new(),
            (&given, &trusted),
            &acme.devices(),
            &own,
        );
        assert_eq!(followed.accepted, vec![]);
        assert_eq!(followed.warnings.len(), 1, "{:?}", followed.warnings);
    }
}
