//! `skald create-channel` and `skald channels`: an admin's device adds a channel to a space by the
//! next roster revision, which it signs with a channel ID the server issued and its own account as
//! the channel's member with `history` true (`docs/protocol/membership.md`, "Creating a channel");
//! and a space's channels, as its last accepted revision gives them. The revision is written and
//! sent as [`crate::spaces`] says.

use serde::Deserialize;
use skald_proto::{Cbor as _, Channel, ChannelId, ChannelMember, Roster, SpaceId, Unknown};

use crate::connection::Connection;
use crate::devices::text;
use crate::spaces::{CHANGED_MEANWHILE, Synced, admin_device, heading, no_admin};

/// `NewChannel` in the API spec.
#[derive(Deserialize)]
struct NewChannel {
    channel: String,
}

/// Adds a channel to `synced`, a space the device follows: by the next revision, signed by the
/// device, of an admin.
pub fn create(synced: Synced) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster: latest,
        ..
    } = synced;
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
    let body = connection.post_as(&path, token).map_err(text)?;
    let unexpected = || "unexpected answer from the server: a channel without its ID".to_owned();
    let issued: NewChannel = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    issued.channel.parse().map_err(|_| unexpected())
}

/// Shows the channels of `synced`, a space the device follows.
pub fn list(synced: Synced) -> Result<(), String> {
    print!("{}", listing(&synced.roster));
    Ok(())
}

/// What `skald channels` shows of `roster`, a space's last accepted revision: a line per channel
/// with its ID and its members' handles.
fn listing(roster: &Roster) -> String {
    let body = roster.body();
    let mut out = heading(roster);
    if body.channels.is_empty() {
        out.push_str("  no channel yet\n");
    }
    for channel in &body.channels {
        let handles: Vec<String> = channel
            .members
            .iter()
            .map(|entry| {
                body.member(entry.account).map_or_else(
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
    use crate::spaces::tests::{first, member};
    use skald_crypto::SigningKey;
    use skald_proto::{AccountId, RosterBody};

    #[test]
    fn listing_shows_each_channel_with_its_members_handles() {
        let (ada, bob, gone) = (
            AccountId::generate(),
            AccountId::generate(),
            AccountId::generate(),
        );
        let key = SigningKey::generate();
        let mut body = first(ada).body().clone();
        let space = body.space;
        let sign = |body: &RosterBody| skald_crypto::sign_roster(body, &key).unwrap();
        assert_eq!(
            listing(&sign(&body)),
            format!("The space acme ({space}), revision 1:\n  no channel yet\n")
        );

        body.revision = 3;
        body.members.push(member(bob, "bob", [4; 32]));
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
