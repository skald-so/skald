//! The server's web address, which invite links start with, apart from its public address, as on
//! a cloud with an app and an API address: a device learns it from the answers the server gives
//! every sign-in and every run that follows spaces, and takes links that start with it.

use serde_json::Value;

use crate::kit::received;
use crate::{
    Made, Scene, YES, accept_at_terminal, assert_refused_saying, link_of, refused_at_terminal,
};

/// The web address the relay names in the server's answers.
const APP: &str = "https://app.skald.test";

/// The request for the session's account, and for its devices, as their lines start.
const SESSION: &str = "GET /session ";
const DEVICES: &str = "GET /devices ";

/// `answer`, a JSON object, with its `web` set to `web`, or taken out if `None`.
fn with_web(answer: &str, web: Option<&str>) -> String {
    let mut given: Value = serde_json::from_str(answer).expect("a JSON answer");
    let object = given.as_object_mut().expect("a JSON object");
    match web {
        Some(web) => object.insert("web".to_owned(), web.into()),
        None => object.remove("web"),
    };
    given.to_string()
}

/// `skald accept-invite` at a terminal on Bob's machine, given the link of `made`'s invite that
/// starts with [`APP`], confirmed, succeeds; the server holds his acceptance.
#[track_caller]
fn takes_the_app_link(scene: &Scene, made: &Made) {
    let link = link_of(APP, made.id(), &made.secret, made.revision.hash());

    let accepted = accept_at_terminal(&scene.bob.machine, &link, &[], (YES, &[]));

    assert!(accepted.shown.success, "{}", accepted.text());
    scene.assert_bob_accepted(made, scene.bob.handle.as_str());
}

/// Bob's machine signed up while the server named its public address as its web address. The link
/// of an invite that starts with [`APP`] is refused as another server's. Once the relay names
/// [`APP`] in the answers to `GET /devices`, Bob's `skald recv` learns it, and `skald
/// accept-invite`, given the same link, accepts the invite.
#[test]
fn web_address_a_run_that_connects_learns_starts_the_links_it_takes() {
    let scene = Scene::new(
        (7_305_409_001, "ada-on-an-app-address"),
        (7_305_409_002, "bob-learns-the-app-address"),
    );
    let made = scene.invite();
    let link = link_of(APP, made.id(), &made.secret, made.revision.hash());

    let refused = refused_at_terminal(&scene.bob.machine, &link);

    assert_refused_saying(&refused, &["server"]);
    scene
        .cloud()
        .relay
        .rewrite(DEVICES, |answer| with_web(answer, Some(APP)));
    received(&scene.bob.machine);

    takes_the_app_link(&scene, &made);
}

/// The relay names [`APP`] as the web address in the answers to `GET /session` and none in those
/// to `GET /devices` before Bob signs up. Right after his sign-up, `skald accept-invite`, given
/// the link of an invite that starts with [`APP`], accepts the invite.
#[test]
fn web_address_a_sign_in_learns_starts_the_links_it_takes() {
    let scene = Scene::with(
        (7_305_409_101, "ada-whose-app-bob-signs-in-at"),
        (7_305_409_102, "bob-signs-in-at-the-app"),
        |cloud| {
            (cloud.relay).rewrite(SESSION, |answer| with_web(answer, Some(APP)));
            (cloud.relay).rewrite(DEVICES, |answer| with_web(answer, None));
        },
    );
    let made = scene.invite();

    takes_the_app_link(&scene, &made);
}
