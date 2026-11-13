//! `skald create-channel` and `skald channels`: an admin's device adds a channel to a space by the
//! next roster revision, which it signs with a channel ID the server issued and its own account as
//! the channel's member with `history` true (`docs/protocol/membership.md`, "Creating a channel");
//! and a space's channels, as its last accepted revision gives them. The revision is written and
//! sent as [`crate::spaces`] says.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use skald_proto::{
    Cbor as _, Channel, ChannelId, ChannelMember, Roster, SpaceId, Unknown, VersionRange,
};

use crate::connection::{Connection, Method};
use crate::devices::text;
use crate::secrets;
use crate::spaces::{CHANGED_MEANWHILE, Follower, admin_device, no_admin};

/// `NewChannel` in the API spec.
#[derive(Deserialize)]
struct NewChannel {
    channel: String,
}

/// Adds a channel to the space `name` that the device of the machine whose state is in `dir`, its
/// secrets in `storage`, follows, once it accepted the revisions the server has of it: by the next
/// revision, signed by the device, of an admin.
pub fn create(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    name: &str,
) -> Result<(), String> {
    let (mut follower, space) = Follower::of_space(dir, storage, speaks, timeout, name)?;
    let (latest, _) = follower.sync(&space)?;
    let name = space.name.as_str();
    if follower.space(name)?.pending.is_some() {
        return Err(format!(
            "the space {name} waits for the server to take a revision this device signed; this \
             device adds no channel until then"
        ));
    }
    if !admin_device(&latest, follower.own.account, follower.this) {
        return Err(no_admin(&latest));
    }
    let id = issue(&mut follower.connection, &follower.token, space.id)?;
    let channel = Channel {
        id,
        members: vec![ChannelMember {
            account: follower.own.account,
            history: true,
        }],
        services: Vec::new(),
        unknown: Unknown::default(),
    };
    let next = follower.next_with(&latest, &[channel])?.to_cbor();
    if !follower
        .state
        .set_pending(space.id, &latest.to_cbor(), None, &next)
        .map_err(text)?
    {
        return Err(CHANGED_MEANWHILE.to_owned());
    }
    let stored = follower.store(&space, next).map_err(|unsent| unsent.why)?;
    println!(
        "Created the channel {id} in the space {name}: this device signed its revision {}.",
        stored.body().revision
    );
    Ok(())
}

/// `POST /spaces/<id>/channels`: a channel ID the server issues for the space `id` to the session
/// `token`.
fn issue(connection: &mut Connection, token: &str, id: SpaceId) -> Result<ChannelId, String> {
    let path = format!("/spaces/{id}/channels");
    let body = connection
        .send_json_as(Method::Post, &path, token, b"{}")
        .map_err(text)?;
    let unexpected = || "unexpected answer from the server: a channel without its ID".to_owned();
    let issued: NewChannel = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    issued.channel.parse().map_err(|_| unexpected())
}

/// Shows the channels of the space `name` that the device of the machine whose state is in `dir`,
/// its secrets in `storage`, follows, after accepting the revisions the server has of it.
pub fn list(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    name: &str,
) -> Result<(), String> {
    let (mut follower, space) = Follower::of_space(dir, storage, speaks, timeout, name)?;
    let (roster, _) = follower.sync(&space)?;
    print!("{}", listing(&roster));
    Ok(())
}

/// What `skald channels` shows of `roster`, a space's last accepted revision: a line per channel
/// with its ID and its members' handles.
fn listing(roster: &Roster) -> String {
    let body = roster.body();
    let mut out = format!(
        "The space {} ({}), revision {}:\n",
        body.name.as_str(),
        body.space,
        body.revision
    );
    if body.channels.is_empty() {
        out.push_str("  no channel yet\n");
    }
    for channel in &body.channels {
        let handles: Vec<String> = channel
            .members
            .iter()
            .map(|entry| {
                body.members
                    .iter()
                    .find(|member| member.account == entry.account)
                    .map_or_else(
                        || entry.account.to_string(),
                        |member| member.handle.as_str().to_owned(),
                    )
            })
            .collect();
        out.push_str(&format!("  {}  {}\n", channel.id, handles.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use skald_crypto::SigningKey;
    use skald_proto::{AccountId, DeviceId, Member, Role, RosterBody};

    #[test]
    fn listing_shows_each_channel_with_its_members_handles() {
        let (ada, bob, gone) = (
            AccountId::generate(),
            AccountId::generate(),
            AccountId::generate(),
        );
        let key = SigningKey::generate();
        let mut body = RosterBody::first(
            SpaceId::generate(),
            "acme".parse().unwrap(),
            ada,
            "ada".parse().unwrap(),
            [3; 32],
            DeviceId::from_signing_key(&key.public_key()),
            1_800_000_000,
        );
        let space = body.space;
        let sign = |body: &RosterBody| skald_crypto::sign_roster(body, &key).unwrap();
        assert_eq!(
            listing(&sign(&body)),
            format!("The space acme ({space}), revision 1:\n  no channel yet\n")
        );

        body.revision = 3;
        body.members.push(Member {
            account: bob,
            handle: "bob".parse().unwrap(),
            role: Role::Member,
            master: [4; 32],
            invite: None,
            excluded: None,
            unknown: Unknown::default(),
        });
        let channel = |members: &[(AccountId, bool)]| Channel {
            id: ChannelId::generate(),
            members: members
                .iter()
                .map(|&(account, history)| ChannelMember { account, history })
                .collect(),
            services: Vec::new(),
            unknown: Unknown::default(),
        };
        body.channels = vec![
            channel(&[(bob, false), (ada, true)]),
            channel(&[(ada, true)]),
            // A revision that fails check 4: an account in a channel that is no member.
            channel(&[(gone, true)]),
            channel(&[]),
        ];
        let ids: Vec<ChannelId> = body.channels.iter().map(|channel| channel.id).collect();

        assert_eq!(
            listing(&sign(&body)),
            format!(
                "The space acme ({space}), revision 3:\n  {}  bob, ada\n  {}  ada\n  {}  {gone}\n  \
                 {}  \n",
                ids[0], ids[1], ids[2], ids[3]
            )
        );
    }
}
