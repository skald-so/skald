//! What the tests share: the admin commands, the change each makes to a revision, the revisions the
//! server holds after one, and what the front saw a machine send in a channel.

use skald_crypto::accept_next;
use skald_proto::{
    ChannelId, ChannelMember, DeviceId, NonEmpty, RawEnvelope, Roster, RosterBody, SpaceId,
};

use crate::cloud::Cloud;
use crate::fixtures::{sent_after, strs};
use crate::front::Front;
use crate::kit::{assert_members, group, received, rosters};
use crate::machine::{Finished, Machine, NO, YES};
use crate::played::Played;
use crate::talk::Talk;
use crate::world::{ADMIN, Person, run, space_certs};

/// The admin commands.
pub const REMOVE_MEMBER: &str = "remove-member";
pub const EXCLUDE_DEVICE: &str = "exclude-device";
pub const ADD_TO_CHANNEL: &str = "add-to-channel";
pub const REMOVE_FROM_CHANNEL: &str = "remove-from-channel";

/// The option of `skald add-to-channel` that says whether the member reads the history from before
/// it joined.
pub const HISTORY: &str = "--history";

/// What `skald remove-from-channel` says when it refuses the default channel.
pub const DEFAULT: &str = "default";

/// A change an admin's device makes to a space by a command.
#[derive(Clone, Copy)]
pub enum Edit<'a> {
    /// Removes the person from the space and from every channel.
    RemoveMember(&'a Person),
    /// Bars the device of the person's machine by its member's `excluded`.
    Exclude(&'a Person),
    /// Adds the person to the channel, with `history`.
    AddToChannel(ChannelId, &'a Person, bool),
    /// Removes the person from the channel.
    RemoveFromChannel(ChannelId, &'a Person),
}

impl Edit<'_> {
    /// The command's arguments, on the space named `space`.
    pub fn args(&self, space: &str) -> Vec<String> {
        let owned = |parts: &[&str]| parts.iter().map(|part| (*part).to_owned()).collect();
        match *self {
            Edit::RemoveMember(person) => owned(&[REMOVE_MEMBER, space, person.handle.as_str()]),
            Edit::Exclude(person) => owned(&[EXCLUDE_DEVICE, space, &person.device.to_string()]),
            Edit::AddToChannel(channel, person, history) => owned(&[
                ADD_TO_CHANNEL,
                space,
                &channel.to_string(),
                person.handle.as_str(),
                HISTORY,
                if history { YES } else { NO },
            ]),
            Edit::RemoveFromChannel(channel, person) => owned(&[
                REMOVE_FROM_CHANNEL,
                space,
                &channel.to_string(),
                person.handle.as_str(),
            ]),
        }
    }

    /// The person the change is about.
    pub fn person(&self) -> &Person {
        match *self {
            Edit::RemoveMember(person)
            | Edit::Exclude(person)
            | Edit::AddToChannel(_, person, _)
            | Edit::RemoveFromChannel(_, person) => person,
        }
    }

    /// Makes the change in `body`, a revision's body.
    pub fn apply(&self, body: &mut RosterBody) {
        let account = self.person().account;
        let channel_at = |body: &RosterBody, id: ChannelId| {
            body.channels
                .iter()
                .position(|channel| channel.id == id)
                .expect("a channel of the space")
        };
        match *self {
            Edit::RemoveMember(_) => {
                let mut members = body.members.clone().into_vec();
                members.retain(|member| member.account != account);
                body.members = NonEmpty::from_vec(members).expect("a member left");
                for channel in &mut body.channels {
                    channel.members.retain(|entry| entry.account != account);
                }
            }
            Edit::Exclude(person) => {
                let mut members = body.members.clone().into_vec();
                let member = members
                    .iter_mut()
                    .find(|member| member.account == account)
                    .expect("a member");
                let mut excluded = member
                    .excluded
                    .clone()
                    .map_or_else(Vec::new, |e| e.into_vec());
                excluded.push(person.device);
                member.excluded = NonEmpty::from_vec(excluded);
                body.members = NonEmpty::from_vec(members).expect("a member");
            }
            Edit::AddToChannel(id, _, history) => {
                let at = channel_at(body, id);
                body.channels[at]
                    .members
                    .push(ChannelMember { account, history });
            }
            Edit::RemoveFromChannel(id, _) => {
                let at = channel_at(body, id);
                body.channels[at]
                    .members
                    .retain(|entry| entry.account != account);
            }
        }
    }
}

/// A change that takes a member's device out of channels.
#[derive(Clone, Copy)]
pub enum Removal {
    /// The device is excluded.
    Exclude,
    /// The member is removed from the talk's channel.
    RemoveFromChannel,
    /// The member is removed from the space.
    RemoveMember,
}

impl Removal {
    /// The change about `person`, in the space whose non-default channel is `channel`.
    pub fn of(self, person: &Person, channel: ChannelId) -> Edit<'_> {
        match self {
            Removal::Exclude => Edit::Exclude(person),
            Removal::RemoveFromChannel => Edit::RemoveFromChannel(channel, person),
            Removal::RemoveMember => Edit::RemoveMember(person),
        }
    }

    /// The channels it takes the device out of, of `default` and `channel`.
    pub fn out_of(self, default: ChannelId, channel: ChannelId) -> Vec<ChannelId> {
        match self {
            Removal::RemoveFromChannel => vec![channel],
            Removal::Exclude | Removal::RemoveMember => vec![default, channel],
        }
    }
}

/// Runs the command of `edit` on the space named `space` on `machine`.
pub fn edit(machine: &Machine, space: &str, edit: Edit) -> Finished {
    run(machine, &strs(&edit.args(space)))
}

/// As [`edit`]; fails unless it succeeds.
#[track_caller]
pub fn edited(machine: &Machine, space: &str, change: Edit) -> Finished {
    let ran = edit(machine, space, change);
    assert!(ran.success, "{ran:?}");
    ran
}

/// `body` with its members in account order, each `excluded` in device order, and each channel's
/// members in account order; the channels stay in their order.
fn in_order(body: &RosterBody) -> RosterBody {
    let mut body = body.clone();
    let mut members = body.members.clone().into_vec();
    members.sort_by_key(|member| member.account);
    for member in &mut members {
        if let Some(excluded) = member.excluded.take() {
            let mut devices = excluded.into_vec();
            devices.sort();
            member.excluded = NonEmpty::from_vec(devices);
        }
    }
    body.members = NonEmpty::from_vec(members).expect("a member");
    for channel in &mut body.channels {
        channel.members.sort_by_key(|entry| entry.account);
    }
    body
}

/// Fails unless `next`, a revision the server holds, is the one after `previous` that `signer`'s
/// device signed with `change` and nothing else, and the signer accepts it.
#[track_caller]
pub fn assert_change(
    cloud: &Cloud,
    (previous, next): (&Roster, &Roster),
    signer: &Person,
    change: Edit,
) {
    let mut expected = previous.body().clone();
    expected.revision += 1;
    expected.prev = Some(previous.hash());
    expected.signer = signer.device;
    expected.timestamp = next.body().timestamp;
    change.apply(&mut expected);
    assert_eq!(
        in_order(next.body()),
        in_order(&expected),
        "not the next revision, signed by the signer's device, with the change alone"
    );
    let certs = space_certs(cloud, &signer.token, expected.space);
    let accepted = accept_next(previous, next, &certs, signer.account, &signer.master);
    assert_eq!(accepted, Ok(()), "the signer does not accept it");
}

/// Runs the command of `change` on Ada's machine on the talk's space; fails unless it succeeds and
/// the server then holds one more revision, the one after the last with `change` alone, signed by
/// Ada's device. Then runs `skald recv` on Ada's machine. Gives how many sends the front saw before
/// the command.
#[track_caller]
pub fn ada_makes(talk: &Talk, change: Edit) -> usize {
    let (cloud, token, space) = (&talk.world.cloud, &talk.ada.token, talk.space.id);
    let held = rosters(cloud, token, space).len();
    let from = talk.front.sent().len();
    edited(&talk.ada.machine, &talk.space.name, change);
    let (previous, next) = last_two(cloud, token, space, held);
    assert_change(cloud, (&previous, &next), &talk.ada, change);
    received(&talk.ada.machine);
    from
}

/// The devices of Ada, Bob and Carol in `talk`, which plays Bob's and Carol's.
pub fn devices(talk: &Talk) -> [DeviceId; 3] {
    [
        talk.ada.device,
        talk.played[0].device(),
        talk.played[1].device(),
    ]
}

/// Fails unless `played`'s device, and the server's GroupInfo for the session `token`, hold each
/// of `devices` once in the group of `at`'s channel, and no other.
#[track_caller]
pub fn assert_group_is(
    cloud: &Cloud,
    (played, token): (&Played, &str),
    at: (SpaceId, ChannelId),
    devices: &[DeviceId],
) {
    assert_members(&played.members(at.1), devices);
    assert_members(&group(cloud, token, at.0, at.1).members, devices);
}

/// The revision before the last of `space` and the last, for the session `token`; fails unless the
/// server holds one more than `held` revisions.
#[track_caller]
pub fn last_two(cloud: &Cloud, token: &str, space: SpaceId, held: usize) -> (Roster, Roster) {
    let mut all = rosters(cloud, token, space);
    assert_eq!(all.len(), held + 1, "not one more revision: {all:?}");
    let last = all.pop().expect("a revision");
    (all.pop().expect("a revision before"), last)
}

/// Fails unless `finished` failed saying `why`.
#[track_caller]
pub fn assert_refused(finished: &Finished, why: &str) {
    assert!(!finished.success, "{finished:?}");
    assert!(
        finished.shown().to_lowercase().contains(why),
        "it does not say {why}: {finished:?}"
    );
}

/// Fails unless `finished` failed saying the account is no admin.
#[track_caller]
pub fn assert_refused_as_no_admin(finished: &Finished) {
    assert_refused(finished, ADMIN);
}

/// The distinct envelopes of `channel` the front saw sent after its first `from` sends, in the
/// order each was first sent: a retry with the same bytes is no other one.
pub fn envelopes_sent(front: &Front, channel: ChannelId, from: usize) -> Vec<RawEnvelope> {
    let mut distinct: Vec<RawEnvelope> = Vec::new();
    for sent in sent_after(front, from) {
        let envelope = sent.envelope;
        if envelope.envelope().header.channel == channel
            && distinct.iter().all(|seen| seen.bytes() != envelope.bytes())
        {
            distinct.push(envelope);
        }
    }
    distinct
}

/// The request that stores a revision of `space`, as its line starts.
pub fn store_request(space: SpaceId) -> String {
    format!("POST /spaces/{space}/revisions")
}

/// The requests that change a space: storing a revision, issuing a channel ID, as their lines
/// start.
pub const SPACE_CHANGE: &str = "POST /spaces/";
