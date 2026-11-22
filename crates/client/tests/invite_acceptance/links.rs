//! Links Bob's device refuses before it sends anything: one of another server than the device's,
//! and malformed ones.

use crate::{Scene, assert_refused_saying, link_of, refused_at_terminal};

/// The link of an invite the server holds, under another web address than the server's. `skald
/// accept-invite`, given it, fails saying `server`, sends nothing at all, and the server holds no
/// acceptance.
#[test]
fn link_of_another_server_is_refused_and_nothing_is_sent() {
    let scene = Scene::new(
        (7_305_403_001, "ada-on-her-server"),
        (7_305_403_002, "bob-gets-a-foreign-link"),
    );
    let made = scene.invite();
    let elsewhere = link_of(
        "http://elsewhere.test:4431",
        made.id(),
        &made.secret,
        made.revision.hash(),
    );
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &elsewhere);

    assert_refused_saying(&refused, &["server"]);
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}

/// Malformed forms of the link of an invite the server holds: with another path, with a query,
/// without a fragment, with a fragment a character short, with padding, and with a character
/// outside base64url. `skald accept-invite`, given each, fails saying `link` and sends nothing at
/// all; the server holds no acceptance.
#[test]
fn malformed_links_are_refused_and_nothing_is_sent() {
    let scene = Scene::new(
        (7_305_403_101, "ada-whose-link-is-mangled"),
        (7_305_403_102, "bob-gets-mangled-links"),
    );
    let made = scene.invite();
    let link = made.link.as_str();
    let (head, fragment) = link.split_once('#').expect("a fragment");
    let mut outside = fragment.to_owned();
    outside.replace_range(..1, "+");
    let malformed = [
        link.replacen("/invite/", "/invites/", 1),
        format!("{head}?from=ada#{fragment}"),
        head.to_owned(),
        format!("{head}#{}", &fragment[1..]),
        format!("{head}#{}=", &fragment[1..]),
        format!("{head}#{outside}"),
    ];
    let since = scene.cloud().relay.mark();

    for text in &malformed {
        let refused = refused_at_terminal(&scene.bob.machine, text);

        assert_refused_saying(&refused, &["link"]);
    }
    scene.assert_nothing_sent(since);
    scene.assert_nothing_accepted(&made, since);
}
