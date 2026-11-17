//! No wrap to a browser that is revoked, has no valid certificate or is of an account outside the
//! channel, while a valid browser of a member account gets each epoch's.

use std::collections::BTreeSet;

use skald_proto::DeviceId;

use crate::chain::{carol, commits_empty, joins_space};
use crate::fixtures::{Issuer, Keyed};
use crate::kit::received;
use crate::revocations::revoked_by;
use crate::talk::Talk;
use crate::wraps::{
    assert_no_wrap_to, assert_wrapped, held_epochs, linked_browser, sent_to, valid_browser,
};

/// The talk, with a valid browser of Ada's account set up; Ada's next `skald recv` wraps epoch 1
/// to it, and fails unless it does. Gives the browser.
fn with_ada_s_browser(ids: &[u64], logins: &[&str]) -> (Talk, Keyed) {
    let mut talk = Talk::new(ids, logins, false);
    let browser = valid_browser(&talk.world.cloud, &talk.ada, "ada-valid-browser");
    received(&talk.ada.machine);
    assert_wrapped(&mut talk, &browser, 1);
    (talk, browser)
}

/// Bob's device commits an empty Commit, and Ada's next `skald recv` runs: fails unless the server
/// holds the wrap of the new epoch to `control`. Gives the new epoch.
#[track_caller]
fn next_epoch_wrapped_to(talk: &mut Talk, control: &Keyed) -> u64 {
    let at = talk.at();
    let epoch = commits_empty(&talk.world.cloud, &mut talk.played[0], at);
    received(&talk.ada.machine);
    assert_wrapped(talk, control, epoch);
    epoch
}

/// Fails unless the server holds no wrap to `barred`, and Ada's machine sent none after the
/// front's first `from` uploads, once Ada's runs wrapped two epochs to her valid browser.
#[track_caller]
fn assert_never_wrapped(talk: &mut Talk, (control, from): (&Keyed, usize), barred: DeviceId) {
    next_epoch_wrapped_to(talk, control);
    assert_no_wrap_to(talk, from, barred);
}

/// A browser of Ada's account whose certificate a key of no device signed, in place of the
/// account's master key, as only a server could: Ada's runs wrap no epoch to it.
#[test]
fn browser_without_a_valid_certificate_gets_no_wrap() {
    let (mut talk, control) = with_ada_s_browser(
        &[7_300_003_001, 7_300_003_002],
        &["ada-skips-forged-browser", "bob-commits-past-forgery"],
    );
    let from = talk.front.uploads().len();
    let barred = linked_browser(
        &talk.world.cloud,
        &talk.ada,
        "server-made-browser",
        &Issuer::Server,
    );
    received(&talk.ada.machine);

    assert_never_wrapped(&mut talk, (&control, from), barred.id);
}

/// Carol is a member of the space and its default channel but not of the talk's channel; a browser
/// of her account, certified by her device, gets no wrap of the talk's channel from Ada's runs.
#[test]
fn browser_of_an_account_outside_the_channel_gets_no_wrap() {
    let (mut talk, control) = with_ada_s_browser(
        &[7_300_004_001, 7_300_004_002],
        &["ada-wraps-members-only", "bob-commits-for-members"],
    );
    let carol = carol(&talk, 7_300_004_003, "carol-outside-channel");
    let (cloud, at) = (&talk.world.cloud, talk.at());
    joins_space(cloud, &talk.ada, at.0, &carol.person);
    let from = talk.front.uploads().len();
    let barred = valid_browser(cloud, &carol.person, "carol-web-browser");
    received(&talk.ada.machine);

    assert_never_wrapped(&mut talk, (&control, from), barred.id);
}

/// A browser of Ada's account, revoked by Ada's device before Ada's machine has run since it was
/// set up: Ada's runs wrap no epoch to it.
#[test]
fn browser_revoked_before_its_first_wrap_gets_none() {
    let (mut talk, control) = with_ada_s_browser(
        &[7_300_005_001, 7_300_005_002],
        &["ada-revokes-new-browser", "bob-commits-after-revocation"],
    );
    let from = talk.front.uploads().len();
    let barred = valid_browser(&talk.world.cloud, &talk.ada, "ada-lost-browser");
    revoked_by(&talk.world.cloud, &talk.ada, barred.id);
    received(&talk.ada.machine);

    assert_never_wrapped(&mut talk, (&control, from), barred.id);
}

/// Two browsers of Ada's account get the wrap of epoch 1. Ada's device revokes one of them; after
/// Bob's empty Commit, Ada's next run wraps the new epoch to the other alone: the server holds the
/// revoked browser's wrap of epoch 1 alone, and Ada's machine sent it no wrap since.
#[test]
fn revoked_browser_gets_no_wrap_of_a_later_epoch() {
    let mut talk = Talk::new(
        &[7_300_006_001, 7_300_006_002],
        &["ada-revokes-old-browser", "bob-commits-after-loss"],
        false,
    );
    let cloud = &talk.world.cloud;
    let control = valid_browser(cloud, &talk.ada, "ada-kept-browser");
    let barred = valid_browser(cloud, &talk.ada, "ada-stolen-browser");
    received(&talk.ada.machine);
    for browser in [&control, &barred] {
        assert_wrapped(&mut talk, browser, 1);
    }
    revoked_by(&talk.world.cloud, &talk.ada, barred.id);
    received(&talk.ada.machine);
    let from = talk.front.uploads().len();

    next_epoch_wrapped_to(&mut talk, &control);

    assert_eq!(
        held_epochs(&talk, barred.id),
        BTreeSet::from([1]),
        "the epochs wrapped to the revoked browser"
    );
    let sent = sent_to(&talk.front, talk.channel, from, barred.id);
    assert!(sent.is_empty(), "wraps to the revoked browser: {sent:?}");
}
