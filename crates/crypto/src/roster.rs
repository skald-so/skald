//! Signing roster revisions and accepting them: `docs/protocol/membership.md`, "Roster" and
//! "Policy", checks 1 to 4.

use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;

use skald_proto::{
    AccountId, Cbor, Cert, ChannelId, DecodeError, DeviceId, DeviceKind, Name, ROSTER_LABEL, Role,
    Roster, RosterBody,
};

use crate::cert::check_device;
use crate::sign::{SigningKey, verify};

/// The revision of `body`, signed by `key`, the signing key of the device in `body.signer`.
///
/// # Errors
/// Why `body` breaks the spec's CDDL, such as a `revision` of 0 or a `history` under a day.
pub fn sign_roster(body: &RosterBody, key: &SigningKey) -> Result<Roster, DecodeError> {
    let bytes = body.to_cbor();
    let signature = key.sign(ROSTER_LABEL, &[&bytes]);
    Roster::from_parts(bytes, signature)
}

/// Why a client does not accept a roster revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterError {
    /// Check 1: `signer` is not a valid `client` or `browser` device of an admin of revision n.
    Signer(DeviceId),
    /// Check 1: the signature does not verify with the key of `signer`'s certificate.
    Signature,
    /// Check 2: `space` is not that of revision n.
    Space,
    /// Check 2: `name` is not that of revision n.
    Name,
    /// Check 2: `revision` is not n+1; holds n+1.
    Revision { expected: u64, actual: u64 },
    /// Check 2: `prev` is not the hash of revision n.
    Prev,
    /// Check 3: no member is an admin.
    NoAdmin,
    /// Check 3: a list repeats an item; holds the list.
    Repeated(&'static str),
    /// Check 3: the owner of a service agent is a member's handle.
    OwnerIsHandle(Name),
    /// Check 4: a service agent's device has no valid `service` certificate on a member's account.
    ServiceAgent(DeviceId),
    /// Check 4: an account in a channel is not a member.
    ChannelMember {
        channel: ChannelId,
        account: AccountId,
    },
}

impl fmt::Display for RosterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signer(device) => write!(
                f,
                "the signer {device} is no valid client or browser device of an admin"
            ),
            Self::Signature => f.write_str("the signature does not verify with the signer's key"),
            Self::Space => f.write_str("another space than the previous revision's"),
            Self::Name => f.write_str("another name than the previous revision's"),
            Self::Revision { expected, actual } => {
                write!(f, "revision {actual}, not {expected}")
            }
            Self::Prev => f.write_str("prev is not the hash of the previous revision"),
            Self::NoAdmin => f.write_str("no member is an admin"),
            Self::Repeated(list) => write!(f, "{list} repeat an item"),
            Self::OwnerIsHandle(owner) => {
                write!(f, "the service agent owner {owner} is a member's handle")
            }
            Self::ServiceAgent(device) => write!(
                f,
                "the service agent {device} has no valid service certificate on a member's account"
            ),
            Self::ChannelMember { channel, account } => {
                write!(f, "{account} in channel {channel} is not a member")
            }
        }
    }
}

impl std::error::Error for RosterError {}

/// Checks 3 and 4 of a revision on its own, with the certificates the server returns: each
/// service agent's certificate is checked against the `master` the revision gives its account.
///
/// # Errors
/// The first check that fails.
pub fn check_roster(roster: &Roster, certs: &[Cert]) -> Result<(), RosterError> {
    let body = roster.body();
    check_content(body, certs, |account| {
        body.members
            .iter()
            .find(|member| member.account == account)
            .map(|member| member.master)
    })
}

/// Whether a client of `account`, with its `master` key pinned, accepts `next` after `previous`,
/// the last revision it accepted: checks 1 to 4. The signer's device and the service agents' are
/// checked against `master` for `account`, and for another account against the `master` that
/// revision n gives it for the signer, and that `next` gives it for a service agent.
///
/// # Errors
/// The first check that fails.
pub fn accept_next(
    previous: &Roster,
    next: &Roster,
    certs: &[Cert],
    account: AccountId,
    master: &[u8; 32],
) -> Result<(), RosterError> {
    let (prev_body, body) = (previous.body(), next.body());
    let pinned = |revision: &RosterBody, member: AccountId| {
        if member == account {
            Some(*master)
        } else {
            revision
                .members
                .iter()
                .find(|entry| entry.account == member)
                .map(|entry| entry.master)
        }
    };

    // Check 1.
    let signer = prev_body
        .members
        .iter()
        .filter(|member| member.role == Role::Admin)
        .find_map(|admin| {
            let cert = check_device(&pinned(prev_body, admin.account)?, certs, body.signer).ok()?;
            let cert = cert.body();
            (cert.account == admin.account
                && matches!(cert.kind, DeviceKind::Client | DeviceKind::Browser))
            .then_some(cert.signing)
        })
        .ok_or(RosterError::Signer(body.signer))?;
    verify(
        &signer,
        ROSTER_LABEL,
        &[next.body_bytes()],
        next.signature(),
    )
    .map_err(|_| RosterError::Signature)?;

    // Check 2.
    if body.space != prev_body.space {
        return Err(RosterError::Space);
    }
    if body.name != prev_body.name {
        return Err(RosterError::Name);
    }
    let expected = prev_body.revision.saturating_add(1);
    if body.revision != expected {
        return Err(RosterError::Revision {
            expected,
            actual: body.revision,
        });
    }
    if body.prev != Some(previous.hash()) {
        return Err(RosterError::Prev);
    }

    check_content(body, certs, |member| pinned(body, member))
}

/// Checks 3 and 4, the master key of a member's account given by `master`.
fn check_content(
    body: &RosterBody,
    certs: &[Cert],
    master: impl Fn(AccountId) -> Option<[u8; 32]>,
) -> Result<(), RosterError> {
    // Check 3. Handles, owners and agents are names by their type.
    if !body.members.iter().any(|member| member.role == Role::Admin) {
        return Err(RosterError::NoAdmin);
    }
    let members = &body.members;
    unique(
        "members' accounts",
        members.iter().map(|member| member.account),
    )?;
    unique(
        "members' handles",
        members.iter().map(|member| &member.handle),
    )?;
    unique(
        "members' invites",
        members.iter().filter_map(|member| member.invite),
    )?;
    for member in members.iter() {
        unique(
            "excluded devices",
            member.excluded.iter().flat_map(|ids| ids.iter()),
        )?;
    }
    let agents = &body.service_agents;
    unique(
        "service agents' devices",
        agents.iter().map(|agent| agent.device),
    )?;
    unique(
        "service agents' addresses",
        agents.iter().map(|agent| (&agent.owner, &agent.agent)),
    )?;
    if let Some(agent) = agents
        .iter()
        .find(|agent| members.iter().any(|member| member.handle == agent.owner))
    {
        return Err(RosterError::OwnerIsHandle(agent.owner.clone()));
    }
    unique("channels", body.channels.iter().map(|channel| channel.id))?;
    for channel in &body.channels {
        unique(
            "channel members",
            channel.members.iter().map(|member| member.account),
        )?;
        unique("channel services", channel.services.iter())?;
    }

    // Check 4. `policy` passes its checks by its type.
    for agent in agents {
        let valid = members.iter().any(|member| {
            master(member.account)
                .and_then(|key| check_device(&key, certs, agent.device).ok())
                .is_some_and(|cert| {
                    cert.body().account == member.account && cert.body().kind == DeviceKind::Service
                })
        });
        if !valid {
            return Err(RosterError::ServiceAgent(agent.device));
        }
    }
    for channel in &body.channels {
        for entry in &channel.members {
            if !members.iter().any(|member| member.account == entry.account) {
                return Err(RosterError::ChannelMember {
                    channel: channel.id,
                    account: entry.account,
                });
            }
        }
    }
    Ok(())
}

/// Fails with [`RosterError::Repeated`] of `list` if `items` repeats an item.
fn unique<T: Eq + Hash>(
    list: &'static str,
    items: impl IntoIterator<Item = T>,
) -> Result<(), RosterError> {
    let mut seen = HashSet::new();
    if items.into_iter().all(|item| seen.insert(item)) {
        Ok(())
    } else {
        Err(RosterError::Repeated(list))
    }
}
