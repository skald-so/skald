//! `skald remove-member`, `skald exclude-device`, `skald add-to-channel` and `skald
//! remove-from-channel`: an admin's device changes a space's members by the next roster revision,
//! which it signs, checks as its members accept it, and writes and sends as [`crate::spaces`]
//! says (`docs/protocol/membership.md`, "Roster"). It then brings the groups of the space's
//! channels in line with it ([`crate::groups`]): it removes the devices no longer admitted, and
//! adds a new channel member's devices where it is a member device.

use skald_proto::{
    AccountId, Cert, ChannelId, ChannelMember, DeviceId, Member, NonEmpty, RosterBody,
};

use crate::connection::printable;
use crate::spaces::{Synced, finish};

/// A change to a space's members, as its command names it.
pub enum Edit {
    /// Removes the member of this handle from the space and from every channel.
    RemoveMember(String),
    /// Bars this device, by its ID, by its member's `excluded`.
    Exclude(String),
    /// Adds the member of the handle to the channel, with whether it reads the history from before
    /// it joined.
    AddToChannel {
        channel: String,
        handle: String,
        history: bool,
    },
    /// Removes the member of the handle from the channel, which is not the default one.
    RemoveFromChannel { channel: String, handle: String },
}

/// Makes `edit` in `synced`, a space the device follows: by the next revision, signed by the
/// device, of an admin.
pub fn run(synced: Synced, edit: &Edit) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster: latest,
        devices,
    } = synced;
    follower.may_change(&space, &latest)?;
    let mut body = latest.body().clone();
    let own = (follower.this, follower.own.account);
    let done = make(&mut body, edit, own, &devices.certs)?;
    let next = follower.next_by(&latest, |next| {
        next.members = body.members;
        next.channels = body.channels;
    })?;
    follower
        .accepts(&latest, &next, &devices.certs)
        .map_err(|err| format!("{err}; nothing is sent"))?;
    let stored = follower.submit(&space, &latest, &next)?;
    println!(
        "{done} in the space {}: this device signed its revision {}.",
        space.name,
        stored.body().revision
    );
    finish(&mut follower, &space.name);
    Ok(())
}

/// Makes `edit` in `body`, a space's last accepted revision, on the device `this` of `account`,
/// the devices of the space's accounts having `certs`; gives what it did, or why it does nothing.
fn make(
    body: &mut RosterBody,
    edit: &Edit,
    (this, own): (DeviceId, AccountId),
    certs: &[Cert],
) -> Result<String, String> {
    let space = body.name.as_str().to_owned();
    match edit {
        Edit::RemoveMember(handle) => {
            let account = member(body, handle)?.account;
            if account == own {
                return Err(format!(
                    "{handle} is this device's own account; this device does not remove it from \
                     the space {space}"
                ));
            }
            let members: Vec<Member> = (body.members.iter())
                .filter(|member| member.account != account)
                .cloned()
                .collect();
            body.members = NonEmpty::from_vec(members).ok_or("the space keeps a member")?;
            for channel in &mut body.channels {
                channel.members.retain(|entry| entry.account != account);
            }
            Ok(format!("Removed {handle} from the space"))
        }
        Edit::Exclude(device) => {
            let id: DeviceId = device
                .parse()
                .map_err(|_| format!("\"{}\" is no device ID", printable(device)))?;
            if id == this {
                return Err("this device does not bar itself".to_owned());
            }
            let account = certs
                .iter()
                .map(|cert| cert.body())
                .find(|cert| cert.device == id && body.member(cert.account).is_some())
                .map(|cert| cert.account)
                .ok_or_else(|| format!("no member of the space {space} has the device {id}"))?;
            let mut members = body.members.clone().into_vec();
            let member = (members.iter_mut())
                .find(|member| member.account == account)
                .ok_or("the device's member is in the space")?;
            let mut barred = member
                .excluded
                .clone()
                .map_or_else(Vec::new, |e| e.into_vec());
            if barred.contains(&id) {
                return Err(format!(
                    "the device {id} is barred from the space {space} already"
                ));
            }
            barred.push(id);
            member.excluded = NonEmpty::from_vec(barred);
            let handle = member.handle.as_str().to_owned();
            body.members = NonEmpty::from_vec(members).ok_or("the space keeps a member")?;
            Ok(format!("Barred the device {id} of {handle}"))
        }
        Edit::AddToChannel {
            channel,
            handle,
            history,
        } => {
            let account = member(body, handle)?.account;
            let at = channel_at(body, channel)?;
            let channel = &mut body.channels[at];
            if channel.members.iter().any(|entry| entry.account == account) {
                return Err(format!("{handle} is in the channel {} already", channel.id));
            }
            channel.members.push(ChannelMember {
                account,
                history: *history,
            });
            Ok(format!("Added {handle} to the channel {}", channel.id))
        }
        Edit::RemoveFromChannel { channel, handle } => {
            let account = member(body, handle)?.account;
            let at = channel_at(body, channel)?;
            let channel = &mut body.channels[at];
            if at == 0 {
                return Err(format!(
                    "the channel {} is the default channel of the space {space}, which keeps \
                     every member; nothing is sent",
                    channel.id
                ));
            }
            let before = channel.members.len();
            channel.members.retain(|entry| entry.account != account);
            if channel.members.len() == before {
                return Err(format!("{handle} is not in the channel {}", channel.id));
            }
            Ok(format!("Removed {handle} from the channel {}", channel.id))
        }
    }
}

/// The member of `body` whose handle is `handle`.
fn member<'b>(body: &'b RosterBody, handle: &str) -> Result<&'b Member, String> {
    (body.members.iter())
        .find(|member| member.handle.as_str() == handle)
        .ok_or_else(|| {
            format!(
                "the space {} has no member with the handle \"{}\"",
                body.name.as_str(),
                printable(handle)
            )
        })
}

/// Where `body` lists the channel whose ID is `channel`.
fn channel_at(body: &RosterBody, channel: &str) -> Result<usize, String> {
    let id: ChannelId = channel
        .parse()
        .map_err(|_| format!("\"{}\" is no channel ID", printable(channel)))?;
    (body.channels.iter())
        .position(|listed| listed.id == id)
        .ok_or_else(|| format!("the space {} has no channel {id}", body.name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spaces::tests::{first, member};
    use skald_crypto::SigningKey;
    use skald_proto::{CertBody, Channel, DeviceKind, Unknown};

    /// Ada's `acme` with Bob a member, its default channel with both and a channel with Ada; the
    /// certificate of a device of Bob's; and the ID of Ada's device.
    fn acme() -> (RosterBody, Cert, DeviceId) {
        let (ada, bob) = (AccountId::generate(), AccountId::generate());
        let mut body = first(ada).body().clone();
        body.members.push(member(bob, "bob", [4; 32]));
        let channel = |accounts: &[AccountId]| Channel {
            id: ChannelId::generate(),
            members: (accounts.iter())
                .map(|&account| ChannelMember {
                    account,
                    history: true,
                })
                .collect(),
            services: Vec::new(),
            unknown: Unknown::default(),
        };
        body.channels = vec![channel(&[ada, bob]), channel(&[ada])];
        let key = SigningKey::generate();
        let cert = CertBody {
            account: bob,
            device: DeviceId::from_signing_key(&key.public_key()),
            kind: DeviceKind::Client,
            signing: key.public_key(),
            hpke: [4; 32],
            issuer: None,
            issued: 1_800_000_000,
            ssh: None,
            name: "bob-laptop".to_owned(),
            unknown: Unknown::default(),
        };
        let cert = skald_crypto::issue(&cert, &SigningKey::generate(), &key).unwrap();
        (body, cert, DeviceId::from_signing_key(&[1; 32]))
    }

    /// Each edit changes its member or device alone; adding to a channel keeps `history` as given.
    #[test]
    fn edits_change_their_member_alone() {
        let (body, cert, this) = acme();
        let (ada, bob) = (body.members[0].account, cert.body().account);
        let (default, team) = (body.channels[0].id, body.channels[1].id);
        let own = (this, ada);
        let certs = std::slice::from_ref(&cert);
        let edited = |edit: Edit| {
            let mut edited = body.clone();
            make(&mut edited, &edit, own, certs).unwrap();
            edited
        };

        let removed = edited(Edit::RemoveMember("bob".to_owned()));
        assert_eq!(removed.members.len(), 1);
        assert!(removed.channels.iter().all(|it| it.members.len() == 1));

        let barred = edited(Edit::Exclude(cert.body().device.to_string()));
        assert_eq!(
            barred.member(bob).unwrap().excluded.as_deref(),
            Some(&[cert.body().device][..])
        );
        assert_eq!(barred.channels, body.channels);

        for history in [true, false] {
            let added = edited(Edit::AddToChannel {
                channel: team.to_string(),
                handle: "bob".to_owned(),
                history,
            });
            let bob_in = ChannelMember {
                account: bob,
                history,
            };
            assert_eq!(added.channels[1].members[1], bob_in);
            assert_eq!(added.members, body.members);
        }

        let mut added = body.clone();
        added.channels[1].members.push(ChannelMember {
            account: bob,
            history: false,
        });
        let edit = Edit::RemoveFromChannel {
            channel: team.to_string(),
            handle: "bob".to_owned(),
        };
        make(&mut added, &edit, own, certs).unwrap();
        assert_eq!(added, body);
        assert_ne!(default, team);
    }

    /// An edit that would change nothing, or that the space keeps from, is refused and changes
    /// nothing: the default channel keeps every member, and the device keeps its own account and
    /// device.
    #[test]
    fn edits_that_do_not_apply_are_refused() {
        let (body, cert, this) = acme();
        let ada = body.members[0].account;
        let (default, team) = (
            body.channels[0].id.to_string(),
            body.channels[1].id.to_string(),
        );
        let mut barred = body.clone();
        let device = cert.body().device;
        make(
            &mut barred,
            &Edit::Exclude(device.to_string()),
            (this, ada),
            std::slice::from_ref(&cert),
        )
        .unwrap();
        let in_channel = |channel: &str, handle: &str| Edit::RemoveFromChannel {
            channel: channel.to_owned(),
            handle: handle.to_owned(),
        };
        let cases: [(&RosterBody, Edit, &str); 9] = [
            (&body, Edit::RemoveMember("carol".to_owned()), "no member"),
            (&body, Edit::RemoveMember("ada".to_owned()), "own account"),
            (&body, Edit::Exclude("laptop".to_owned()), "no device id"),
            (&body, Edit::Exclude(this.to_string()), "itself"),
            (&barred, Edit::Exclude(device.to_string()), "already"),
            (
                &body,
                Edit::AddToChannel {
                    channel: default.clone(),
                    handle: "bob".to_owned(),
                    history: true,
                },
                "already",
            ),
            (&body, in_channel(&default, "bob"), "default"),
            (&body, in_channel(&team, "bob"), "not in the channel"),
            (
                &body,
                in_channel(&ChannelId::generate().to_string(), "ada"),
                "no channel",
            ),
        ];
        for (body, edit, why) in cases {
            let mut edited = body.clone();
            let refused = make(&mut edited, &edit, (this, ada), std::slice::from_ref(&cert));
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|err| err.to_lowercase().contains(why)),
                "{why}: {refused:?}"
            );
            assert_eq!(&edited, body, "{why}");
        }
        let mut edited = body.clone();
        let unknown = make(
            &mut edited,
            &Edit::Exclude(device.to_string()),
            (this, ada),
            &[],
        );
        assert!(
            unknown.as_ref().is_err_and(|err| err.contains("no member")),
            "{unknown:?}"
        );
        assert_eq!(edited, body);
    }
}
