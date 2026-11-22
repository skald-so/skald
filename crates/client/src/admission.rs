//! Admitting newcomers on an admin's device (`docs/protocol/membership.md`, "Invite"). A run of
//! `skald recv`, one-shot, waiting or through the helper, takes the waiting acceptances of each
//! space it syncs, on connecting and on a `space-notice`, in which this device is an admin's at its
//! last accepted revision: as an agent, of every space, its folder's or not. It admits a newcomer
//! only after checks 1 to 4 of "Invite" ([`judge`]): it then signs the next revision with the
//! member, its account, handle, the invite's role, the acceptance's `master` and the invite, and
//! the newcomer in the default channel, the first one, with `history` true and in the invite's
//! other channels that the revision has, with their `history`, and sends it as any revision it
//! signs ([`Follower::checked_submit`]). An acceptance a revision already admitted is done.
//!
//! It tells its user which handle it admitted to which space, and warns of an acceptance it does
//! not admit, with the handle and the check, and of each device of an admitted newcomer whose
//! certificate is not chained to the acceptance's `master`, which then gets into no channel. A run
//! that shows nothing of a space, an agent's of another one, keeps what it tells for the person's
//! run. The newcomer's devices get into the channels as a new channel member's do
//! ([`crate::groups`]).

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{check_acceptance, check_device, check_invite, check_valid_device};
use skald_proto::{
    Acceptance, Cbor as _, ChannelId, ChannelMember, DeviceKind, Invite, InviteBody, Member, Name,
    NonEmpty, Role, Roster, SpaceId, Unknown,
};

use crate::devices;
use crate::follow::{self, SpaceDevices};
use crate::groups::Rosters;
use crate::recv;
use crate::spaces::{Follower, admin_device, now};
use crate::state;

/// `WaitingAcceptances` in the API spec.
#[derive(Deserialize)]
struct WaitingAcceptances {
    acceptances: Vec<Waiting>,
}

/// `WaitingAcceptance` in the API spec.
#[derive(Deserialize)]
struct Waiting {
    invite: String,
    acceptance: String,
    #[serde(flatten)]
    devices: devices::Devices,
}

/// An acceptance the server gives as waiting, decoded, with its account's devices.
struct Given {
    invite: Invite,
    acceptance: Acceptance,
    devices: SpaceDevices,
}

/// What the checks make of an acceptance.
enum Verdict {
    /// A revision admitted it already.
    Done,
    /// It passes: the member to add.
    Admit(Member),
    /// It fails: why.
    Refused(String),
}

/// Admits the newcomers of the waiting acceptances of each space of `due` that the device follows,
/// as an admin's device, its last accepted revision and devices taken from `synced` if it holds
/// them: with a space for which `apart` holds, outside what the run shows, they are followed apart
/// and what it tells is kept for the person's run. A space it admits a newcomer to and does not
/// keep apart is synced again into `synced`. Warns of what fails.
pub fn admit(
    follower: &mut Follower,
    due: &BTreeSet<SpaceId>,
    synced: &mut Rosters,
    apart: impl Fn(SpaceId) -> bool,
) {
    let spaces = match follower.state.spaces() {
        Ok(spaces) => spaces,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    for space in spaces.iter().filter(|space| due.contains(&space.id)) {
        let apart = apart(space.id);
        follower.hold_notices = apart;
        let admitted = admit_to(follower, space, synced, apart);
        follower.hold_notices = false;
        if let Err(err) = admitted {
            tell(follower, apart, &format!("skald: warning: {err}"));
        }
    }
}

/// [`admit`] in `space`.
fn admit_to(
    follower: &mut Follower,
    space: &state::Space,
    synced: &mut Rosters,
    apart: bool,
) -> Result<(), String> {
    let name = space.name.as_str();
    let last = follow::decode(&space.accepted, name)?;
    // A revision this device signed waits: a later run admits once the server took it.
    if space.pending.is_some() || !admin_device(&last, follower.own.account, follower.this) {
        return Ok(());
    }
    let waiting = waiting(follower, space)?;
    if waiting.is_empty() {
        return Ok(());
    }
    let (mut latest, mut devices) = match synced.get(&space.id) {
        Some(found) if !apart => found.clone(),
        // One that failed to sync was warned of.
        None if !apart => return Ok(()),
        _ => follower.sync_once(space)?,
    };
    for given in waiting {
        let handle = given.acceptance.body().handle.as_str().to_owned();
        let member = match judge(follower, space, (&latest, &devices), &given) {
            Verdict::Done => continue,
            Verdict::Refused(why) => {
                let line =
                    format!("skald: warning: {handle} is not admitted to the space {name}: {why}");
                tell(follower, apart, &line);
                continue;
            }
            Verdict::Admit(member) => member,
        };
        if !admin_device(&latest, follower.own.account, follower.this) {
            return Ok(());
        }
        let next = follower.next_by((&latest, &devices), |body| {
            let mut members = body.members.clone().into_vec();
            members.push(member.clone());
            body.members = NonEmpty::from_vec(members).expect("a member");
            let invite = given.invite.body();
            for (at, channel) in body.channels.iter_mut().enumerate() {
                if let Some(history) = invited(invite, at, channel.id) {
                    let account = member.account;
                    channel.members.push(ChannelMember { account, history });
                }
            }
        })?;
        let held = follower.space(name)?;
        let stored = follower
            .checked_submit(&held, (&latest, &next), &devices.certs)
            .map_err(|err| format!("{handle} is not admitted to the space {name} yet: {err}"))?;
        if stored.body().member(member.account).is_some() {
            tell(
                follower,
                apart,
                &format!("skald: admitted {handle} to the space {name}"),
            );
            for device in unchained(&given.devices, &member.master) {
                let line = format!(
                    "skald: warning: the device {device} of {handle}, admitted to the space \
                     {name}, has no certificate chained to the master key of the acceptance, so it \
                     gets into no channel"
                );
                tell(follower, apart, &line);
            }
        }
        let held = follower.space(name)?;
        (latest, devices) = if apart {
            follower.sync_once(&held)?
        } else {
            let again = follower.sync(&held)?;
            synced.insert(space.id, again.clone());
            again
        };
    }
    Ok(())
}

/// Whether the newcomer of `invite` reads the earlier history of `channel`, at `at` among the
/// channels of the revision admitting it, if it joins it: the default channel, the first one, with
/// `history` true, whatever the invite names first; another one the invite names, as it says.
fn invited(invite: &InviteBody, at: usize, channel: ChannelId) -> Option<bool> {
    if at == 0 {
        return Some(true);
    }
    (invite.channels.iter()).find_map(|&(named, history)| (named == channel).then_some(history))
}

/// Shows `line`, a line of the run's error output, or keeps it for the person's run if `apart`.
fn tell(follower: &Follower, apart: bool, line: &str) {
    if apart {
        let lines = [line.to_owned()];
        recv::keep_held(follower.state.keep_warnings(&lines), &lines);
    } else {
        eprintln!("{line}");
    }
}

/// The waiting acceptances the server gives of `space`, oldest first; one that does not decode is
/// warned of and left out.
fn waiting(follower: &mut Follower, space: &state::Space) -> Result<Vec<Given>, String> {
    let name = space.name.as_str();
    let path = format!("/spaces/{}/acceptances", space.id);
    let body = (follower.connection)
        .get_as(&path, &follower.token)
        .map_err(|err| format!("the waiting acceptances of the space {name}: {err}"))?;
    let list: WaitingAcceptances = serde_json::from_slice(&body)
        .map_err(|_| "unexpected answer from the server: waiting acceptances without their list")?;
    let mut given = Vec::new();
    for entry in list.acceptances {
        let decoded = |text: &str| URL_SAFE_NO_PAD.decode(text).ok();
        let invite = decoded(&entry.invite).and_then(|bytes| Invite::from_cbor(&bytes).ok());
        let acceptance =
            decoded(&entry.acceptance).and_then(|bytes| Acceptance::from_cbor(&bytes).ok());
        let (Some(invite), Some(acceptance)) = (invite, acceptance) else {
            eprintln!(
                "skald: warning: a waiting acceptance the server gives of the space {name} does \
                 not decode, so it admits no one"
            );
            continue;
        };
        let devices = devices::settle(entry.devices, &follower.state)?;
        given.push(Given {
            invite,
            acceptance,
            devices,
        });
    }
    Ok(given)
}

/// Checks 1 to 4 of "Invite" of `given` in `space`, at `latest`, its last accepted revision, with
/// `devices`, its accounts': the invite, of the space, signed by an admin's device in `latest`, not
/// expired and no member's invite in a revision the device accepted; the acceptance of that
/// invite, signed with its `key`; its account no member and its handle no member's or service
/// agent owner's. The newcomer's devices are checked against the acceptance's `master` as the
/// revision admitting it gives it: those not chained to it get into no channel ([`unchained`]).
fn judge(
    follower: &Follower,
    space: &state::Space,
    (latest, devices): (&Roster, &SpaceDevices),
    given: &Given,
) -> Verdict {
    let (invite, acceptance) = (given.invite.body(), given.acceptance.body());
    let body = latest.body();
    if body
        .members
        .iter()
        .any(|member| member.invite == Some(invite.invite))
    {
        return Verdict::Done;
    }
    let refused = |why: &str| Verdict::Refused(why.to_owned());
    if invite.space != space.id || acceptance.invite != invite.invite {
        return refused("the server gives it with an invite of another space or ID");
    }
    let earlier = (follower.given.get(&space.id).into_iter().flatten())
        .filter(|roster| roster.body().revision <= body.revision);
    if earlier
        .flat_map(|roster| roster.body().members.iter())
        .any(|member| member.invite == Some(invite.invite))
    {
        return refused("a member joined with its invite before");
    }
    let signer = invite.signer;
    let Some(signing) = admins_key(follower, (latest, devices), signer) else {
        return Verdict::Refused(format!(
            "the signer {signer} of its invite is no admin's device in the space"
        ));
    };
    if check_invite(&given.invite, &signing).is_err() {
        return refused("its invite's signature does not verify with the signer's key");
    }
    if invite.expires <= now() {
        return refused("its invite expired");
    }
    if check_acceptance(&given.acceptance, &invite.key).is_err() {
        return refused("its signature does not verify with the invite's key");
    }
    if body.member(acceptance.account).is_some() {
        return refused("its account is already a member");
    }
    if handle_taken(latest, &acceptance.handle) {
        return refused("its handle is taken by a member or a service agent owner");
    }
    Verdict::Admit(Member {
        account: acceptance.account,
        handle: acceptance.handle.clone(),
        role: invite.role,
        master: acceptance.master,
        invite: Some(invite.invite),
        excluded: None,
        unknown: Unknown::default(),
    })
}

/// The signing key of `device` if it is an admin's device in `latest`: a `client` or `browser`
/// device of an admin's account, which its `excluded` does not list, with a valid certificate
/// among `devices`, checked against the master key [`follow::master_of`] gives.
fn admins_key(
    follower: &Follower,
    (latest, devices): (&Roster, &SpaceDevices),
    device: skald_proto::DeviceId,
) -> Option<[u8; 32]> {
    let body = latest.body();
    (body.members.iter())
        .filter(|member| member.role == Role::Admin)
        .filter(|member| admin_device(latest, member.account, device))
        .find_map(|admin| {
            let master = follow::master_of(latest, &follower.own, admin.account)?;
            let cert =
                check_valid_device(&master, &devices.certs, &devices.revocations, device).ok()?;
            let cert = cert.body();
            (cert.account == admin.account
                && matches!(cert.kind, DeviceKind::Client | DeviceKind::Browser))
            .then_some(cert.signing)
        })
}

/// Whether `handle` is a member's handle or a service agent owner's in `roster`.
fn handle_taken(roster: &Roster, handle: &Name) -> bool {
    let body = roster.body();
    body.members.iter().any(|member| member.handle == *handle)
        || (body.service_agents.iter()).any(|agent| agent.owner == *handle)
}

/// The devices among `devices` with a certificate that is not chained to `master`.
fn unchained(devices: &SpaceDevices, master: &[u8; 32]) -> Vec<skald_proto::DeviceId> {
    let mut seen = BTreeSet::new();
    (devices.ids.iter().copied())
        .filter(|device| seen.insert(*device))
        .filter(|device| (devices.certs.iter()).any(|cert| cert.body().device == *device))
        .filter(|device| check_device(master, &devices.certs, *device).is_err())
        .collect()
}
