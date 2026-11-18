//! A device linked with a link code gets into the account's channels: the linked device adds it
//! once it has KeyPackages, at the end of `skald link` or at a later run, or it joins by itself.
//! Each of the account's channels then has it as one leaf, and it shows the history from before
//! its link once a member device has run since.

use crate::devices::{BOB, said_by};
use crate::kit::{assert_members, group, received};
use crate::linking::{NewDevice, link};
use crate::reading::{assert_history, sends, shown_history};
use crate::talk::Talk;

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
