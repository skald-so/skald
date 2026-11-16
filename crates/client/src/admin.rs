//! `skald remove-member`, `skald exclude-device`, `skald add-to-channel`, `skald
//! remove-from-channel` and `skald set-history-period`: an admin's device changes a space's members
//! or its policy's `history` by the next roster revision, which it signs, checks as its members
//! accept it, and writes and sends as [`crate::spaces`] says (`docs/protocol/membership.md`,
//! "Roster" and "Policy"). It then brings the groups of the space's channels in line with it
//! ([`crate::groups`]): it removes the devices no longer admitted, adds a new channel member's
//! devices where it is a member device, and rotates the history key of an epoch older than the
//! period.

use skald_crypto::check_device;
use skald_proto::{Cert, ChannelId, ChannelMember, DeviceId, Member, NonEmpty, RosterBody};

use crate::connection::printable;
use crate::follow::Own;
use crate::spaces::{Synced, exclude, finish, remove_members};

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
    /// Sets the policy's `history` to these seconds, at least a day.
    HistoryPeriod(u64),
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
    let done = make(
        &mut body,
        edit,
        (follower.this, &follower.own),
        &devices.certs,
    )?;
    let next = follower.next_by(&latest, |next| {
        next.members = body.members;
        next.channels = body.channels;
        next.policy = body.policy;
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

/// Makes `edit` in `body`, a space's last accepted revision, on the device `this` of `own`'s
/// account, with `certs`, the certificates the server gave of the space's devices; gives what it
/// did, or why it does nothing. A device is barred under the member whose master key its
/// certificate chains to ([`check_device`]), the master key `own` pins for its own account.
fn make(
    body: &mut RosterBody,
    edit: &Edit,
    (this, own): (DeviceId, &Own),
    certs: &[Cert],
) -> Result<String, String> {
    let space = body.name.as_str().to_owned();
    match edit {
        Edit::RemoveMember(handle) => {
            let account = member(body, handle)?.account;
            if account == own.account {
                return Err(format!(
                    "{handle} is this device's own account; this device does not remove it from \
                     the space {space}"
                ));
            }
            remove_members(body, &[account])?;
            Ok(format!("Removed {handle} from the space"))
        }
        Edit::Exclude(device) => {
            let id: DeviceId = device
                .parse()
                .map_err(|_| format!("\"{}\" is no device ID", printable(device)))?;
            if id == this {
                return Err("this device does not bar itself".to_owned());
            }
            let mut members = body.members.clone().into_vec();
            let member = (members.iter_mut())
                .find(|member| {
                    let master = if member.account == own.account {
                        own.master
                    } else {
                        member.master
                    };
                    check_device(&master, certs, id)
                        .is_ok_and(|cert| cert.body().account == member.account)
                })
                .ok_or_else(|| format!("no member of the space {space} has the device {id}"))?;
            if !exclude(member, id) {
                return Err(format!(
                    "the device {id} is barred from the space {space} already"
                ));
            }
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
        Edit::HistoryPeriod(seconds) => {
            let period = period(*seconds);
            if body.policy.history == *seconds {
                return Err(format!(
                    "the history period of the space {space} is {period} already"
                ));
            }
            body.policy.history = *seconds;
            Ok(format!("Set the history period to {period}"))
        }
    }
}

/// `seconds` as whole days, or else as whole hours, or else as seconds.
fn period(seconds: u64) -> String {
    let (count, unit) = match seconds {
        _ if seconds.is_multiple_of(86_400) => (seconds / 86_400, "day"),
        _ if seconds.is_multiple_of(3_600) => (seconds / 3_600, "hour"),
        _ => (seconds, "second"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
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
    use skald_proto::{AccountId, CertBody, Channel, DeviceKind, Unknown};

    /// The certificate of the device of `key` on `account`, signed by `issuer` as the master key.
    fn certify(account: AccountId, issuer: &SigningKey, key: &SigningKey) -> Cert {
        let body = CertBody {
            account,
            device: DeviceId::from_signing_key(&key.public_key()),
            kind: DeviceKind::Client,
            signing: key.public_key(),
            hpke: [4; 32],
            issuer: None,
            issued: 1_800_000_000,
            ssh: None,
            name: "laptop".to_owned(),
            unknown: Unknown::default(),
        };
        skald_crypto::issue(&body, issuer, key).unwrap()
    }

    /// Ada's `acme` with Bob a member, its default channel with both and a channel with Ada; the
    /// certificate of a device of Bob's, chained to Bob's master key, and that device's key; Ada's
    /// account with the master key its device pinned; and the ID of Ada's device.
    fn acme() -> (RosterBody, (Cert, SigningKey), Own, DeviceId) {
        let (ada, bob) = (AccountId::generate(), AccountId::generate());
        let bob_master = SigningKey::generate();
        let mut body = first(ada).body().clone();
        body.members
            .push(member(bob, "bob", bob_master.public_key()));
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
        let cert = certify(bob, &bob_master, &key);
        let own = Own {
            account: ada,
            master: SigningKey::generate().public_key(),
        };
        (body, (cert, key), own, DeviceId::from_signing_key(&[1; 32]))
    }

    /// Each edit changes its member or device alone; adding to a channel keeps `history` as given.
    #[test]
    fn edits_change_their_member_alone() {
        let (body, (cert, _), ada, this) = acme();
        let bob = cert.body().account;
        let (default, team) = (body.channels[0].id, body.channels[1].id);
        let own = (this, &ada);
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

        let set = edited(Edit::HistoryPeriod(36 * 3_600));
        let mut expected = body.clone();
        expected.policy.history = 36 * 3_600;
        assert_eq!(set, expected);
    }

    /// A period reads as whole days, hours or seconds.
    #[test]
    fn periods_read_in_their_largest_whole_unit() {
        assert_eq!(period(86_400), "1 day");
        assert_eq!(period(3 * 86_400), "3 days");
        assert_eq!(period(36 * 3_600), "36 hours");
        assert_eq!(period(90_001), "90001 seconds");
    }

    /// An edit that would change nothing, or that the space keeps from, is refused and changes
    /// nothing: the default channel keeps every member, and the device keeps its own account and
    /// device.
    #[test]
    fn edits_that_do_not_apply_are_refused() {
        let (body, (cert, _), ada, this) = acme();
        let (default, team) = (
            body.channels[0].id.to_string(),
            body.channels[1].id.to_string(),
        );
        let mut barred = body.clone();
        let device = cert.body().device;
        make(
            &mut barred,
            &Edit::Exclude(device.to_string()),
            (this, &ada),
            std::slice::from_ref(&cert),
        )
        .unwrap();
        let in_channel = |channel: &str, handle: &str| Edit::RemoveFromChannel {
            channel: channel.to_owned(),
            handle: handle.to_owned(),
        };
        let same = Edit::HistoryPeriod(body.policy.history);
        let cases: [(&RosterBody, Edit, &str); 10] = [
            (&body, same, "already"),
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
            let refused = make(
                &mut edited,
                &edit,
                (this, &ada),
                std::slice::from_ref(&cert),
            );
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
            (this, &ada),
            &[],
        );
        assert!(
            unknown.as_ref().is_err_and(|err| err.contains("no member")),
            "{unknown:?}"
        );
        assert_eq!(edited, body);
    }

    /// A device is barred under the member its certificate chains to, Ada's own by the master key
    /// her device pinned, whatever other certificates of it the server gives first: one on Ada's
    /// account without the device's proof, as the server could make it, is passed over. With no
    /// valid certificate of the device under any member, such as two of it with different bodies,
    /// barring it is refused and changes nothing.
    #[test]
    fn device_is_barred_only_under_the_member_its_certificate_chains_to() {
        let (body, (cert, key), mut ada, this) = acme();
        let bob = cert.body().account;
        let device = cert.body().device;
        let bar = Edit::Exclude(device.to_string());
        let barred_of = |body: &RosterBody, account| {
            let member = body.member(account).unwrap();
            member.excluded.as_deref().unwrap_or_default().to_vec()
        };
        let made_up = SigningKey::generate();
        // The server holds the device's public key but not its private one.
        let mut without_proof = certify(ada.account, &made_up, &made_up).body().clone();
        (without_proof.device, without_proof.signing) = (device, key.public_key());
        let without_proof = skald_crypto::issue(&without_proof, &made_up, &made_up).unwrap();
        let with_proof = certify(ada.account, &made_up, &key);

        let mut edited = body.clone();
        let certs = [without_proof, cert.clone()];
        make(&mut edited, &bar, (this, &ada), &certs).unwrap();
        assert_eq!(barred_of(&edited, bob), [device]);
        assert!(barred_of(&edited, ada.account).is_empty());

        for certs in [vec![with_proof.clone()], vec![with_proof, cert]] {
            let mut edited = body.clone();
            let refused = make(&mut edited, &bar, (this, &ada), &certs);
            assert!(
                refused.as_ref().is_err_and(|err| err.contains("no member")),
                "{refused:?}"
            );
            assert_eq!(edited, body);
        }

        let master = SigningKey::generate();
        ada.master = master.public_key();
        let laptop = certify(ada.account, &master, &SigningKey::generate());
        let mut edited = body.clone();
        let bar = Edit::Exclude(laptop.body().device.to_string());
        make(
            &mut edited,
            &bar,
            (this, &ada),
            std::slice::from_ref(&laptop),
        )
        .unwrap();
        assert_eq!(barred_of(&edited, ada.account), [laptop.body().device]);
    }
}
