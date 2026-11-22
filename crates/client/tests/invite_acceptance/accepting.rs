//! Bob accepts an invite to `acme` at a terminal: the server holds his acceptance, his device
//! follows `acme` from the link's revision and anchors it, and once a revision admits him, his
//! device joins the default channel by itself and exchanges messages with Ada's there.

use skald_proto::{DeviceKind, Role};

use crate::fixtures::{Issuer, add_device_of_kind, send};
use crate::kit::{assert_members, group, received};
use crate::talk::{ADA_HOST, received_until};
use crate::world::{ACME, ADMIN, CREATE_CHANNEL, MEMBER, run, words};
use crate::{HANDLE, NO, Scene, YES, accept_at_terminal, give_invite, resigned, says_after};

/// What Bob's and Ada's machines say to each other in `acme`'s default channel.
const BOB_SAYS: &str = "invite acceptance test: bob says hello to acme";
const ADA_SAYS: &str = "invite acceptance test: ada welcomes bob";

/// Ada's machine creates two channels in `acme`, revisions 3 and 4; the invite, to a member, names
/// the default channel and both, and its link revision 4; then Ada's machine creates a third
/// channel, revision 5. `skald accept-invite` at a terminal on Bob's machine, given the link: it
/// shows `acme`, the role `member`, Ada's handle and her device's name and the invite's 3 channels,
/// and asks to confirm; Bob says yes. It succeeds, saying that it waits for an admin's device and
/// how to bind a folder. The server holds one waiting acceptance, Bob's, naming his account, his
/// pinned master key and his username, signed with the invite key; the anchors object of Bob's
/// device names `acme` at revision 5. A `skald recv` on his machine while he waits succeeds.
///
/// A revision 6, signed as Ada's device, admits Bob with the invite and adds him to the default
/// channel. Bob's next `skald recv` joins the default channel by itself: its group holds Ada's
/// device and Bob's, and its GroupInfo is signed by Bob's device. Once Ada's machine takes in its
/// mailbox, Bob's message in the default channel shows on Ada's machine, and Ada's on Bob's.
#[test]
fn person_of_another_account_accepts_at_a_terminal_and_joins_the_default_channel() {
    let scene = Scene::new(
        (7_305_401_001, "ada-invites-bob"),
        (7_305_401_002, "bob-accepts-an-invite"),
    );
    let (cloud, ada, bob) = (scene.cloud(), &scene.ada, &scene.bob);
    for _ in 0..2 {
        let created = run(&ada.machine, &[CREATE_CHANNEL, ACME]);
        assert!(created.success, "a channel: {created:?}");
    }
    let channels: Vec<_> = scene
        .latest()
        .body()
        .channels
        .iter()
        .map(|c| c.id)
        .collect();
    let made = scene.invite_with(|body| {
        body.channels = vec![
            (channels[0], true),
            (channels[1], true),
            (channels[2], false),
        ];
    });
    let created = run(&ada.machine, &[CREATE_CHANNEL, ACME]);
    assert!(created.success, "a channel: {created:?}");
    let fifth = scene.latest();
    assert_eq!(fifth.body().revision, 5, "{fifth:?}");

    let accepted = accept_at_terminal(&bob.machine, &made.link, &[], (YES, &[]));

    assert!(accepted.shown.success, "{}", accepted.text());
    let summary = accepted.summary.to_lowercase();
    for part in [ACME, ada.handle.as_str(), ADA_HOST] {
        assert!(
            summary.contains(part),
            "it does not show {part:?}: {}",
            accepted.summary
        );
    }
    for word in [MEMBER, "3"] {
        assert!(
            summary.lines().any(|line| words(line).contains(&word)),
            "it does not show {word:?}: {}",
            accepted.summary
        );
    }
    assert!(
        says_after(&accepted.summary, accepted.text(), "wait"),
        "it does not say that it waits for an admin's device: {}",
        accepted.text()
    );
    assert!(
        accepted.text().contains("add-rule") || accepted.text().contains(".skald.toml"),
        "it does not say how to bind a folder: {}",
        accepted.text()
    );
    scene.assert_bob_accepted(&made, bob.handle.as_str());
    assert_eq!(
        scene.bob_anchors(),
        Some(fifth.hash()),
        "Bob's anchors object does not name acme at its revision 5"
    );
    received(&bob.machine);

    scene.admit_bob(&made, bob.handle.as_str());
    received(&bob.machine);

    let joined = group(cloud, &ada.token, scene.space.id, scene.space.default);
    assert_members(&joined.members, &[ada.device, bob.device]);
    assert_eq!(
        joined.signer, bob.device,
        "Bob's device did not join by itself: {joined:?}"
    );
    received(&ada.machine);
    let said = send(&bob.machine, scene.space.default, BOB_SAYS);
    assert!(said.success, "{said:?}");
    received_until(&ada.machine, |shown| shown.contains(BOB_SAYS));
    let said = send(&ada.machine, scene.space.default, ADA_SAYS);
    assert!(said.success, "{said:?}");
    received_until(&bob.machine, |shown| shown.contains(ADA_SAYS));
}

/// `skald accept-invite --handle bob-reviews` at a terminal on Bob's machine, given the link of an
/// invite, confirmed: the server holds his acceptance asking for the handle `bob-reviews`, and his
/// anchors object names `acme` at the link's revision.
#[test]
fn acceptance_asks_for_the_handle_the_person_names() {
    let scene = Scene::new(
        (7_305_401_101, "ada-invites-a-reviewer"),
        (7_305_401_102, "bob-names-himself"),
    );
    let made = scene.invite();

    let accepted = accept_at_terminal(
        &scene.bob.machine,
        &made.link,
        &[HANDLE, "bob-reviews"],
        (YES, &[]),
    );

    assert!(accepted.shown.success, "{}", accepted.text());
    scene.assert_bob_accepted(&made, "bob-reviews");
    assert_eq!(
        scene.bob_anchors(),
        Some(made.revision.hash()),
        "Bob's anchors object does not name acme at the link's revision"
    );
}

/// The relay gives the invite signed by a `browser` device of Ada's, an admin, that her device
/// certified, naming it as `signer`: an admin's device. `skald accept-invite` at a terminal on
/// Bob's machine, given the link, shows the browser's name and, confirmed, succeeds; the server
/// holds his acceptance.
#[test]
fn invite_signed_by_an_admin_s_browser_device_is_accepted() {
    let scene = Scene::new(
        (7_305_411_001, "ada-invites-from-her-browser"),
        (7_305_411_002, "bob-invited-from-a-browser"),
    );
    let made = scene.invite();
    let browser = add_device_of_kind(
        scene.cloud(),
        (scene.ada.account, DeviceKind::Browser),
        "ada-browser",
        &Issuer::Device(&scene.ada),
    );
    give_invite(
        scene.cloud(),
        &resigned(&made, &browser.signing, |body| body.signer = browser.id),
    );

    let accepted = accept_at_terminal(&scene.bob.machine, &made.link, &[], (YES, &[]));

    assert!(accepted.shown.success, "{}", accepted.text());
    assert!(
        accepted.summary.contains("ada-browser"),
        "it does not show the browser: {}",
        accepted.summary
    );
    scene.assert_bob_accepted(&made, scene.bob.handle.as_str());
}

/// An invite to `acme` for an admin: `skald accept-invite` at a terminal on Bob's machine, given
/// the link, shows the role `admin`, not `member`; Bob declines.
#[test]
fn invite_for_an_admin_shows_the_role_admin() {
    let scene = Scene::new(
        (7_305_411_101, "ada-hands-over-the-keys"),
        (7_305_411_102, "bob-asked-to-lead"),
    );
    let made = scene.invite_with(|body| body.role = Role::Admin);

    let declined = accept_at_terminal(&scene.bob.machine, &made.link, &[], (NO, &[]));

    let shown = |word: &str| (declined.summary.lines()).any(|line| words(line).contains(&word));
    assert!(shown(ADMIN), "it does not show admin: {}", declined.summary);
    assert!(!shown(MEMBER), "it shows member: {}", declined.summary);
}
