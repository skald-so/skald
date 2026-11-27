//! Admitting newcomers on an admin's device (`docs/protocol/membership.md`, "Invite"). A run of
//! `skald recv`, one-shot, waiting or through the helper, takes the waiting acceptances of each
//! space it syncs, on connecting and on a `space-notice`, in which this device is an admin's at its
//! last accepted revision: as an agent, of every space, its folder's or not. A service agent's run
//! takes none. It admits a newcomer
//! only after checks 1 to 4 of "Invite" ([`judge`]): it then signs the next revision with the
//! member, its account, handle, the invite's role, the acceptance's `master` and the invite, and
//! the newcomer in the default channel, the first one, with `history` true and in the invite's
//! other channels that the revision has, with their `history`, and sends it as any revision it
//! signs ([`Follower::checked_submit`]). An acceptance a revision already admitted is done.
//!
//! It tells its user which handle it admitted to which space, and warns of an acceptance it does
//! not admit, with the handle and the check. A run that shows nothing of a space, an agent's of
//! another one, keeps what it tells for the person's run. The newcomer's devices get into the
//! channels as a new channel member's do ([`crate::groups`]): only those chained to the `master`
//! the revision gives it.

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{admins_device, check_acceptance, check_invite};
use skald_proto::{
    Acceptance, Cbor as _, ChannelId, ChannelMember, Invite, InviteBody, Member, NonEmpty, Roster,
    SpaceId, Unknown,
};

use crate::follow::{self, SpaceDevices};
use crate::groups::Rosters;
use crate::recv;
use crate::spaces::{Follower, admin_device, handle_taken, now};
use crate::state;

/// `WaitingAcceptances` in the API spec.
#[derive(Deserialize)]
struct WaitingAcceptances {
    acceptances: Vec<Waiting>,
}

/// `WaitingAcceptance` in the API spec, without the account's devices: those get into channels by
/// the `master` the revision admitting it gives ([`crate::groups`]).
#[derive(Deserialize)]
struct Waiting {
    invite: String,
    acceptance: String,
}

/// An acceptance the server gives as waiting, decoded.
struct Given {
    invite: Invite,
    acceptance: Acceptance,
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
/// keep apart is synced again into `synced`. Warns of what fails. A service agent's run admits
/// none: only a `client` or `browser` device admits, and its token is no session to ask for the
/// waiting acceptances with.
pub fn admit(
    follower: &mut Follower,
    due: &BTreeSet<SpaceId>,
    synced: &mut Rosters,
    apart: impl Fn(SpaceId) -> bool,
) {
    if follower.service.is_some() {
        return;
    }
    let spaces = match follower.state.spaces() {
        Ok(spaces) => spaces,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    for space in spaces.iter().filter(|space| due.contains(&space.id)) {
        let apart = apart(space.id);
        let held_notices = follower.hold_notices;
        follower.hold_notices = held_notices || apart;
        let admitted = admit_to(follower, space, synced, apart);
        follower.hold_notices = held_notices;
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
        given.push(Given { invite, acceptance });
    }
    Ok(given)
}

/// Checks 1 to 4 of "Invite" of `given` in `space`, at `latest`, its last accepted revision, with
/// `devices`, its accounts': the invite, of the space, signed by an admin's device in `latest`, not
/// expired and no member's invite in a revision the device accepted, each of which the server
/// gives ([`accepted_given`]); the acceptance of that invite, signed with its `key`; its account
/// no member and its handle no member's or service agent owner's. The newcomer's devices are
/// checked against the acceptance's `master` as the revision admitting it gives it.
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
    let Some(earlier) = accepted_given(follower, space.id) else {
        return refused(
            "the server does not give every revision of the space this device accepted, so it \
             cannot tell whether a member joined with its invite",
        );
    };
    if (earlier.iter())
        .flat_map(|roster| roster.body().members.iter())
        .any(|member| member.invite == Some(invite.invite))
    {
        return refused("a member joined with its invite before");
    }
    let signer = invite.signer;
    let listed = (&devices.certs[..], &devices.revocations[..]);
    let master = |account| follow::master_of(latest, &follower.own, account);
    let Some(signing) = admins_device(body, listed, signer, master) else {
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
    if handle_taken(&body.members, &body.service_agents, &acceptance.handle) {
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

/// The revisions of `space` the device accepted, each as the server gives it ([`Follower::given`]);
/// `None` unless it gives each one, of which the state keeps only hashes.
fn accepted_given(follower: &Follower, space: SpaceId) -> Option<Vec<&Roster>> {
    let accepted = follower.state.accepted(space).ok()??;
    let given = follower.given.get(&space)?;
    (accepted.hashes.values())
        .map(|hash| given.iter().find(|roster| roster.hash() == *hash))
        .collect()
}
