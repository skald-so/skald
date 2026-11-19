//! A device linked with a link code gets into the account's channels: the linked device adds it
//! once it has KeyPackages, at the end of `skald link` or at a later run, or it joins by itself.
//! The device that certified it adds it also where another device signs the space's revisions.
//! Each of the account's channels then has it as one leaf, and it shows the history from before
//! its link once a member device has run since.

use skald_proto::Role;

use crate::devices::{BOB, said_by};
use crate::kit::{Change, assert_members, group, latest, received, revise};
use crate::linking::{NewDevice, link};
use crate::reading::{assert_history, sends, shown_history};
use crate::talk::Talk;
use crate::world::ACME;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// In the talk's channel Bob says a line and Ada's machine says one. Ada's new machine is linked
/// with its code on her first one; then the new machine runs, and her first one. Both of the
/// space's channels have Ada's two devices and Bob's, each once, and the new machine's `skald
/// history` shows both lines, oldest first.
#[test]
fn the_linked_device_gets_into_the_accounts_channels_and_reads_their_history() {
    let mut talk = Talk::new(
        &[4_175_201_001, 4_175_201_002],
        &["ada-links-into-channels", "bob-in-the-channel"],
        false,
    );
    let (at, space) = (talk.at(), talk.space.name.clone());
    let said = vec![
        said_by(&mut talk, BOB, (at, 1), "bob opens the launch checklist").0,
        sends(&talk.ada, (&space, at.1), "ada adds the release notes"),
    ];

    let mut new = NewDevice::start(&talk.world, &talk.ada, LAPTOP);
    link(talk.cloud(), &talk.ada, &mut new);
    let laptop = new.into_person(talk.cloud(), &talk.ada);
    received(&laptop.machine);
    received(&talk.ada.machine);
    received(&laptop.machine);

    let devices = [talk.ada.device, talk.played[BOB].device(), laptop.device];
    for channel in [talk.space.default, at.1] {
        let held = group(talk.cloud(), &talk.ada.token, at.0, channel);
        assert_members(&held.members, &devices);
    }
    let shown = shown_history(&laptop.machine, at.1);
    assert_history(&shown, at.1, &said, &[]);
}

/// Bob is an admin of the talk's space and his device signs its latest revision, so Ada's first
/// device adds only the devices it certified. Ada's new machine is linked with its code on her first
/// one: by the end of `skald link` her first device has added it to both of the space's channels,
/// before her new machine runs again.
#[test]
fn the_certifying_device_adds_the_new_one_by_the_end_of_skald_link() {
    let talk = Talk::with(
        ACME,
        &[4_175_201_011, 4_175_201_012],
        &["ada-certifies-the-laptop", "bob-signs-acme"],
        false,
        (Role::Admin, |_| {}),
    );
    let (cloud, ada, bob) = (talk.cloud(), &talk.ada, &talk.played[BOB].person);
    let at = talk.at();
    revise(
        cloud,
        &latest(cloud, &bob.token, at.0),
        bob,
        &Change::default(),
    );
    received(&ada.machine);

    let mut new = NewDevice::start(&talk.world, ada, LAPTOP);
    link(cloud, ada, &mut new);
    for channel in [talk.space.default, at.1] {
        let held = group(cloud, &ada.token, at.0, channel);
        assert_members(&held.members, &[ada.device, bob.device, new.device]);
    }
}
