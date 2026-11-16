//! Which revocations count: one a compromised server adds with a broken signature or by a device of
//! another account, or one a Commit carries that does not count, changes nothing; a fixture
//! account's device revoked by another of its devices, then restored when a master-key revocation
//! voids that revocation, is warned of each time.

use skald_proto::{ChannelId, DeviceId, Revocation, Role};

use crate::fixtures::fronted;
use crate::kit::{Change, create_space, latest, received, revise, warnings_naming};
use crate::machine::{Finished, Machine};
use crate::played::Played;
use crate::revocations::{
    Fixture, add_revocations, assert_listed, body, broken, carried, revoked_by, signed,
    store_revocation,
};
use crate::talk::{ADA_HOST, BOB_HOST, Talk, one_message_sent};
use crate::world::{ACME, members, now};

/// The certificate `name`s of the fixture account's devices.
const FRED_LAPTOP: &str = "fred-laptop";
const FRED_PHONE: &str = "fred-phone";

/// Revocations of Bob's device that do not count: his own with its signature broken, and one by
/// Carol's device, of another account.
fn not_counting(bob: &Played, carol: &Played) -> Vec<Revocation> {
    let (account, device) = (bob.person.account, bob.device());
    vec![
        broken(&signed(
            &body(account, device, Some(device)),
            &bob.person.key,
        )),
        signed(
            &body(account, device, Some(carol.device())),
            &carol.person.key,
        ),
    ]
}

/// Whether `played` entered an epoch of `channel`'s group without `device`.
fn saw_removed(played: &Played, channel: ChannelId, device: DeviceId) -> bool {
    played
        .changes_of(channel)
        .iter()
        .any(|change| change.removed.iter().any(|member| member.device == device))
}

/// Fails unless `ran`, a run of Ada's `machine`, warns of nothing naming Bob's device, and her
/// `skald members` lists it valid.
#[track_caller]
fn assert_held_valid(ran: &Finished, machine: &Machine, bob: DeviceId) {
    let warned = warnings_naming(ran, bob);
    assert!(warned.is_empty(), "warned of Bob's device: {warned:?}");
    assert_listed(&members(machine, ACME), BOB_HOST, false);
}

/// Bob's machine's device revokes itself, as the CLI would: Ada's next run warns naming it, and
/// `skald members` lists it revoked.
#[track_caller]
fn assert_revocation_then_counts(talk: &Talk) {
    let bob = &talk.played[0].person;
    revoked_by(&talk.world.cloud, bob, bob.device);
    let ran = received(&talk.ada.machine);
    assert!(
        !warnings_naming(&ran, bob.device).is_empty(),
        "no warning of Bob's device revoked: {ran:?}"
    );
    assert_listed(&members(&talk.ada.machine, ACME), BOB_HOST, true);
}

/// The relay adds to the certificates Ada's machine gets a revocation of Bob's device with a broken
/// signature and one by Carol's device. Ada's next run warns of nothing naming Bob's device and
/// removes it from no channel; `skald members` lists it valid, and Ada's next message reaches it.
/// Bob's own revocation then counts.
#[test]
fn revocations_a_proxy_adds_that_do_not_count_change_nothing() {
    let mut talk = Talk::new(
        &[4_139_094_001, 4_139_094_002, 4_139_094_003],
        &["ada-sees-forgeries", "bob-forged-against", "carol-forges"],
        false,
    );
    let cloud = &talk.world.cloud;
    let forged = not_counting(&talk.played[0], &talk.played[1]);
    add_revocations(cloud, talk.space.id, &forged);
    let bob = talk.played[0].device();

    let ran = received(&talk.ada.machine);

    assert_held_valid(&ran, &talk.ada.machine, bob);
    let carol = &mut talk.played[1];
    carol.take_in(cloud);
    for channel in [talk.space.default, talk.channel] {
        assert!(!saw_removed(carol, channel, bob), "Bob's device removed");
    }
    let from = talk.front.sent().len();
    let text = "bob, the forged revocations did not count";
    let said = talk.send(text);
    assert!(said.success, "{said:?}");
    let msg_id = one_message_sent(&talk.front, talk.channel, from);
    talk.bob_reads_from_ada(&[(msg_id, 1, text)]);
    assert_revocation_then_counts(&talk);
}

/// Bob's device is not revoked. Carol's device commits its Remove carrying revocations that do not
/// count. Ada's next run warns of nothing naming Bob's device, and `skald members` lists it valid.
/// Bob's own revocation then counts.
#[test]
fn commit_carrying_revocations_that_do_not_count_leaves_the_device_valid() {
    let mut talk = Talk::new(
        &[4_139_094_101, 4_139_094_102, 4_139_094_103],
        &["ada-holds-bob-valid", "bob-not-revoked", "carol-lies"],
        false,
    );
    let (at, cloud) = (talk.at(), &talk.world.cloud);
    let forged = not_counting(&talk.played[0], &talk.played[1]);
    let bob = talk.played[0].device();
    let carol = &mut talk.played[1];
    let commit = (carol.mls)
        .remove_devices(at.1, &[bob], &carried(&forged), now())
        .expect("a Remove");
    carol.commits(cloud, at, &commit);
    carol.uploads_group_info(cloud, at);

    let ran = received(&talk.ada.machine);

    assert_held_valid(&ran, &talk.ada.machine, bob);
    assert_revocation_then_counts(&talk);
}

/// Fred, a member of `acme`, has an account whose master key the test holds and two devices it
/// certified. His phone revokes his laptop: Ada's next run warns naming the laptop, which `skald
/// members` lists revoked. His master key then revokes the phone, which voids the phone's
/// revocation: Ada's next run warns naming the laptop, which `skald members` lists valid, and the
/// phone revoked.
#[test]
fn device_revoked_by_another_device_and_restored_by_a_master_key_revocation_is_warned_of() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_139_094_201, "ada-watches-fred", ADA_HOST);
    let fred = Fixture::new(
        cloud,
        &world.browser,
        (4_139_094_202, "fred-fixture"),
        [FRED_LAPTOP, FRED_PHONE],
    );
    let space = create_space(cloud, &ada, ACME);
    let change = Change {
        members: vec![(&fred.person, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &latest(cloud, &ada.token, space.id), &ada, &change);
    received(&ada.machine);
    let (account, laptop, phone) = (fred.person.account, fred.person.device, fred.second.id);
    let quiet = received(&ada.machine);
    assert!(
        warnings_naming(&quiet, laptop).is_empty(),
        "warned of Fred's laptop with no change: {quiet:?}"
    );
    let by_phone = signed(&body(account, laptop, Some(phone)), &fred.second.signing);
    store_revocation(cloud, &fred.person.token, &by_phone);

    let revoked = received(&ada.machine);

    assert!(
        !warnings_naming(&revoked, laptop).is_empty(),
        "no warning of Fred's laptop revoked: {revoked:?}"
    );
    let shown = members(&ada.machine, ACME);
    assert_listed(&shown, FRED_LAPTOP, true);
    assert_listed(&shown, FRED_PHONE, false);
    let by_master = signed(&body(account, phone, None), &fred.master);
    store_revocation(cloud, &fred.person.token, &by_master);

    let restored = received(&ada.machine);

    assert!(
        !warnings_naming(&restored, laptop).is_empty(),
        "no warning of Fred's laptop restored: {restored:?}"
    );
    let shown = members(&ada.machine, ACME);
    assert_listed(&shown, FRED_LAPTOP, false);
    assert_listed(&shown, FRED_PHONE, true);
}
