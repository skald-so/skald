//! `skald create-channel` and `skald channels`: an admin's device adds a channel to a space by the
//! next roster revision, which it signs with a channel ID the server issued and its own account as
//! the channel's member with `history` true, after the default channel
//! (`docs/protocol/membership.md`, "Creating a channel"), and creates the channel's group at once
//! ([`crate::groups`]); and a space's channels, as its last accepted revision gives them, with the
//! devices in each one's group as this device holds it. The revision is written and sent as
//! [`crate::spaces`] says.

use std::collections::BTreeMap;

use skald_crypto::MemberDevice;
use skald_proto::{Cert, Channel, ChannelId, ChannelMember, Roster, Unknown};

use crate::groups;
use crate::spaces::{Synced, heading, issue_channel, needs_default};

/// Adds a channel to `synced`, a space the device follows: by the next revision, signed by the
/// device, of an admin.
pub fn create(synced: Synced) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster: latest,
        devices,
    } = synced;
    let name = space.name.as_str();
    follower.may_change(&space, &latest)?;
    if needs_default(&latest) {
        return Err(format!(
            "the space {name} has no default channel with every member yet, which this device \
             adds first; run the command again"
        ));
    }
    let id = issue_channel(&mut follower.connection, &follower.token, space.id)?;
    let channel = Channel {
        id,
        members: vec![ChannelMember {
            account: follower.own.account,
            history: true,
        }],
        services: Vec::new(),
        unknown: Unknown::default(),
    };
    let next = follower.next_with((&latest, &devices), None, &[channel])?;
    let stored = follower.submit(&space, &latest, &next)?;
    println!(
        "Created the channel {id} in the space {name}: this device signed its revision {}.",
        stored.body().revision
    );
    follower.sync(&space)?;
    groups::align_accepted(&mut follower);
    Ok(())
}

/// Shows the channels of `synced`, a space the device follows.
pub fn list(synced: Synced) -> Result<(), String> {
    let Synced {
        follower,
        roster,
        devices,
        ..
    } = synced;
    let keys = follower.keys()?;
    let held = groups::held(&follower, &keys, &roster.body().channels)?;
    let admitted: Vec<ChannelId> = roster
        .body()
        .channels
        .iter()
        .filter(|channel| {
            groups::admitted(&follower, &roster, &devices, channel.id).contains_key(&follower.this)
        })
        .map(|channel| channel.id)
        .collect();
    print!("{}", listing(&roster, &held, &admitted, &devices.certs));
    Ok(())
}

/// What `skald channels` shows of `roster`, a space's last accepted revision: a line per channel
/// with its ID and its members' handles; then the devices in its group, of `held`, the groups this
/// device holds, by the `name` of their certificate among `certs`, or by ID; or, for a channel of
/// `admitted`, those this device is admitted to, that it is not in.
fn listing(
    roster: &Roster,
    held: &BTreeMap<ChannelId, Vec<MemberDevice>>,
    admitted: &[ChannelId],
    certs: &[Cert],
) -> String {
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
        out.push_str(&format!("  {}  {}", channel.id, handles.join(", ")));
        match held.get(&channel.id) {
            Some(members) => {
                let names: Vec<String> = members
                    .iter()
                    .map(|member| {
                        certs
                            .iter()
                            .map(Cert::body)
                            .find(|cert| {
                                cert.device == member.device && cert.signing == member.signing
                            })
                            .map_or_else(|| member.device.to_string(), |cert| cert.name.clone())
                    })
                    .collect();
                out.push_str(&format!("  devices: {}", names.join(", ")));
            }
            None if admitted.contains(&channel.id) => {
                out.push_str("  this device is not in its group")
            }
            None => {}
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::tests::Account;
    use crate::spaces::tests::{first, member};
    use skald_crypto::SigningKey;
    use skald_proto::{AccountId, DeviceId, RosterBody};

    #[test]
    fn listing_shows_each_channel_with_its_members_handles_and_its_groups_devices() {
        let (ada, bob, gone) = (
            AccountId::generate(),
            AccountId::generate(),
            AccountId::generate(),
        );
        let key = SigningKey::generate();
        let mut body = first(ada).body().clone();
        let space = body.space;
        let sign = |body: &RosterBody| skald_crypto::sign_roster(body, &key).unwrap();
        let none = BTreeMap::new();
        assert_eq!(
            listing(&sign(&body), &none, &[], &[]),
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
        let (laptop, ..) = Account::new().client();
        let member = |cert: &Cert| MemberDevice {
            device: cert.body().device,
            signing: cert.body().signing,
        };
        // A device without a certificate, and one whose leaf has another key than its certificate.
        let unknown = MemberDevice {
            device: DeviceId::from_signing_key(&[7; 32]),
            signing: [7; 32],
        };
        let rekeyed = MemberDevice {
            signing: [8; 32],
            ..member(&laptop)
        };
        let held = BTreeMap::from([(ids[0], vec![member(&laptop), unknown, rekeyed])]);

        assert_eq!(
            listing(
                &sign(&body),
                &held,
                &[ids[0], ids[1]],
                std::slice::from_ref(&laptop)
            ),
            format!(
                "The space acme ({space}), revision 3:\n  {}  bob, ada  devices: laptop, {}, {}\n  \
                 {}  ada  this device is not in its group\n  {}  {gone}\n  {}  \n",
                ids[0], unknown.device, rekeyed.device, ids[1], ids[2], ids[3]
            )
        );
    }
}
