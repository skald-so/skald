//! What the tests share: the revocation command with `--keep`, revocations the tests sign with
//! `keep`, the devices as a member of a space checks them, a channel whose group holds only
//! revoked devices of Ada's while a valid one is outside it, and the `mls` envelopes the front saw
//! sent in a channel.

use skald_crypto::{DeviceCheckError, MlsFraming, check_valid_device};
use skald_proto::{ChannelId, DeviceId, NonEmpty, Revocation, SpaceId};

use crate::cloud::Cloud;
use crate::devices::second_machine;
use crate::fixtures::fronted;
use crate::front::Front;
use crate::kit::{Space, assert_members, create_space, group, received};
use crate::machine::{DEVICES, Finished, Machine};
use crate::revocations::{REVOKE_DEVICE, body, signed, space_revocations, store_revocation};
use crate::talk::{ADA_HOST, is_commit, is_message, sent_in};
use crate::world::{ACME, Person, World, lines_naming, run, space_certs};

/// The option of the revocation command that names a certified device to keep.
pub const KEEP: &str = "--keep";

/// The host names of the other machines of Ada's account, their certificates' `name`.
pub const SECOND_HOST: &str = "ada-laptop";
pub const THIRD_HOST: &str = "ada-tablet";
pub const FOURTH_HOST: &str = "ada-new-desk";
pub const FIFTH_HOST: &str = "ada-phone";

/// Runs `skald revoke-device <device>` on `machine`, with `--keep <kept>` for each of `keep`.
pub fn revoke_keeping(machine: &Machine, device: DeviceId, keep: &[DeviceId]) -> Finished {
    let device = device.to_string();
    let kept: Vec<String> = keep.iter().map(ToString::to_string).collect();
    let mut args = vec![REVOKE_DEVICE, device.as_str()];
    for one in &kept {
        args.extend([KEEP, one.as_str()]);
    }
    run(machine, &args)
}

/// As [`revoke_keeping`]; fails unless the run succeeds and its output shows each device of
/// `certified` by its certificate's `name`, on a line that is no warning.
#[track_caller]
pub fn revoked_keeping(
    machine: &Machine,
    device: DeviceId,
    keep: &[DeviceId],
    certified: &[&str],
) -> Finished {
    let revoked = revoke_keeping(machine, device, keep);
    assert!(revoked.success, "{revoked:?}");
    for name in certified {
        assert!(
            !lines_naming(&revoked, name).is_empty(),
            "the certified device {name} is not shown: {revoked:?}"
        );
    }
    revoked
}

/// `person`'s device revokes `device`, keeping `keep`, as the CLI would: the revocation, signed by
/// the device and stored from its session.
#[track_caller]
pub fn signed_keeping(
    cloud: &Cloud,
    person: &Person,
    device: DeviceId,
    keep: &[DeviceId],
) -> Revocation {
    let mut revocation = body(person.account, device, Some(person.device));
    revocation.keep = NonEmpty::from_vec(keep.to_vec());
    let revocation = signed(&revocation, &person.key);
    store_revocation(cloud, &person.token, &revocation);
    revocation
}

/// Runs `skald devices` on `machine`; fails unless it succeeds.
#[track_caller]
pub fn devices_of(machine: &Machine) -> Finished {
    let shown = run(machine, &[DEVICES]);
    assert!(shown.success, "{shown:?}");
    shown
}

/// How a member of `space` whose session is `token` checks `device`, of the account whose master
/// key is `master`: with the certificates and revocations the server gives it for the space.
pub fn checked_by_member(
    cloud: &Cloud,
    (token, space): (&str, SpaceId),
    master: &[u8; 32],
    device: DeviceId,
) -> Result<(), DeviceCheckError> {
    let certs = space_certs(cloud, token, space);
    let revocations = space_revocations(cloud, token, space);
    check_valid_device(master, &certs, &revocations, device).map(|_| ())
}

/// What an `mls` envelope a machine sent in a channel holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An external Commit: the sending device joins by itself.
    Joining,
    /// Another Commit.
    Commit,
    /// An application message.
    Message,
    /// Anything else, such as a Welcome.
    Other,
}

/// The distinct envelopes of `channel` the front saw sent after its first `from` sends, in the
/// order each was first sent, each as what it holds and its MLS epoch, 0 if it has none.
pub fn sent_kinds(front: &Front, channel: ChannelId, from: usize) -> Vec<(Kind, u64)> {
    sent_in(front, channel, from)
        .iter()
        .map(|sent| {
            let envelope = sent.envelope.envelope();
            let epoch = match MlsFraming::read(&envelope.payload) {
                Some(MlsFraming::Message { epoch, .. }) => epoch,
                _ => 0,
            };
            let kind = if MlsFraming::is_external_commit(&envelope.payload) {
                Kind::Joining
            } else if is_commit(envelope) {
                Kind::Commit
            } else if is_message(envelope) {
                Kind::Message
            } else {
                Kind::Other
            };
            (kind, epoch)
        })
        .collect()
}

/// Ada's account, whose first machine created `acme`, and the default channel of `acme`, whose
/// group holds her first machine's device and her second machine's, both revoked, and, if asked
/// for, her third machine's, valid; and her fourth machine, linked by her first, valid and outside
/// the group, which has not run yet.
pub struct Revoked {
    pub world: World,
    pub front: Front,
    /// Ada's first machine, which linked the others; revoked by its own device, which keeps the
    /// fourth machine's device and the third's.
    pub ada: Person,
    /// Her second machine; revoked by her first machine's device.
    pub second: Person,
    /// Her third machine, if any; valid.
    pub third: Option<Person>,
    /// Her fourth machine.
    pub fourth: Person,
    pub space: Space,
    /// The epoch of the default channel's group once the revocations are stored.
    pub epoch: u64,
}

impl Revoked {
    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// The default channel's space and ID.
    pub fn at(&self) -> (SpaceId, ChannelId) {
        (self.space.id, self.space.default)
    }

    /// The default channel's group as the server's GroupInfo shows it.
    #[track_caller]
    pub fn group(&self) -> crate::kit::Group {
        group(
            self.cloud(),
            &self.ada.token,
            self.space.id,
            self.space.default,
        )
    }
}

/// Builds [`Revoked`]: Ada, GitHub user `id` with `login`, signs up and creates `acme`; her second
/// machine and, if `with_third`, her third, both linked by her first, join the default channel by
/// themselves; her fourth is linked by her first. Then her first machine's device revokes her
/// second's, and itself keeping her fourth's and third's: revocations the tests sign and store, as
/// no machine of hers runs between them and the fourth machine's run.
#[track_caller]
pub fn revoked_group((id, login): (u64, &str), with_third: bool) -> Revoked {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(id, login, ADA_HOST);
    let space = create_space(cloud, &ada, ACME);
    received(&ada.machine);
    let at = (space.id, space.default);
    let mut devices = vec![ada.device];
    let mut linked = |host: &'static str| {
        let machine = second_machine(&world, &ada, host);
        received(&machine.machine);
        devices.push(machine.device);
        assert_members(&group(cloud, &ada.token, at.0, at.1).members, &devices);
        machine
    };
    let second = linked(SECOND_HOST);
    let third = with_third.then(|| linked(THIRD_HOST));
    let fourth = second_machine(&world, &ada, FOURTH_HOST);
    signed_keeping(cloud, &ada, second.device, &[]);
    let mut kept = vec![fourth.device];
    kept.extend(third.as_ref().map(|third| third.device));
    signed_keeping(cloud, &ada, ada.device, &kept);
    let epoch = group(cloud, &ada.token, at.0, at.1).epoch;
    Revoked {
        world,
        front,
        ada,
        second,
        third,
        fourth,
        space,
        epoch,
    }
}
