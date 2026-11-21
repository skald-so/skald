//! The key the account's first device shows.

use std::fs;

use crate::cloud::unb64;
use crate::keys;
use crate::talk::ADA_HOST;
use crate::world::World;
use crate::{holds_key, in_shown_form, signs_up};

/// Ada's first machine, signed in at a terminal, shows one recovery key, of 3 groups of 4
/// characters of Crockford's base32, and no longer one. The server then holds the master key the
/// machine pinned, with an account record and a master backup. No file of the machine, no row of
/// the server's database and no request the machine sent holds the key, as text or as its 60 bits.
#[test]
fn the_first_device_shows_a_key_of_three_groups_of_four_that_nothing_keeps() {
    let world = World::start();
    let cloud = &world.cloud;

    let ada = signs_up(&world, (9_120_001_001, "ada-short-key"), ADA_HOST);

    assert!(in_shown_form(&ada.key), "{:?}", ada.key);
    let person = &ada.person;
    let (status, held) = cloud.get_as("/account/master", &person.token);
    assert_eq!(status, 200, "GET /account/master: {held}");
    for field in ["record", "backup"] {
        let bytes = unb64(
            held[field]
                .as_str()
                .unwrap_or_else(|| panic!("no {field}: {held}")),
        );
        assert!(!bytes.is_empty(), "an empty {field}: {held}");
    }
    let master = unb64(held["master"].as_str().expect("the master key"));
    assert_eq!(master, person.master, "the master key");
    assert!(
        keys::holds(&person.machine.state_bytes(), &master),
        "the machine did not pin the master key"
    );
    let places = [
        ("the database", cloud.rows().join("\n").into_bytes()),
        ("the requests", cloud.relay.sent_bytes()),
    ];
    for (place, bytes) in &places {
        assert!(!holds_key(bytes, &ada.key), "{place} hold the recovery key");
    }
    for file in person.machine.files() {
        let bytes = fs::read(&file).unwrap_or_default();
        assert!(
            !holds_key(&bytes, &ada.key),
            "{file:?} holds the recovery key"
        );
    }
}
