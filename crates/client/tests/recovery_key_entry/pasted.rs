//! A key pasted into the hidden input, as a terminal sends it, and a wrong key.

use crate::machine::Machine;
use crate::recovering::{NEW_HOST, STORE_CERT};
use crate::talk::ADA_HOST;
use crate::world::World;
use crate::{
    PASTE_END, PASTE_START, SignedUp, assert_linked, certified_devices, links_on, links_with,
    shows_key, signs_up,
};

/// Ada, GitHub user `id`, signs up at a terminal; her new machine is given what `pasted` makes of
/// her key, in its shown form, once it asks for it, then Enter, naming her first device lost. Fails
/// unless it links, as [`assert_linked`] says.
#[track_caller]
fn links_pasted(id: u64, login: &str, pasted: impl Fn(&str) -> String) {
    let world = World::start();
    let ada = signs_up(&world, (id, login), ADA_HOST);

    let (_, shown) = links_with(&world, &ada.person, &pasted(&ada.key));

    assert_linked(&world.cloud, &ada.person, &ada.key, &shown);
}

/// `key` in lower case, with o for each 0 and i and l, in turn, for each 1.
fn with_letters(key: &str) -> String {
    let mut ones = 0;
    key.to_lowercase()
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => {
                ones += 1;
                if ones % 2 == 1 { 'i' } else { 'l' }
            }
            other => other,
        })
        .collect()
}

#[test]
fn a_key_sent_as_a_bracketed_paste_links() {
    links_pasted(9_120_003_001, "ada-pastes-bracketed", |key| {
        format!("{PASTE_START}{key}{PASTE_END}")
    });
}

#[test]
fn a_key_after_a_blank_line_links() {
    links_pasted(9_120_003_101, "ada-pastes-after-a-blank", |key| {
        format!("\r{key}")
    });
}

#[test]
fn a_key_with_windows_line_ends_links() {
    links_pasted(9_120_003_201, "ada-pastes-crlf", |key| format!("{key}\r\n"));
}

/// One paste holds it all: the markers, Windows line ends and a blank line around the key, in
/// lower case with letters for its 0s and 1s.
#[test]
fn a_key_in_lower_case_with_letters_for_digits_pasted_among_blank_lines_and_markers_links() {
    links_pasted(9_120_003_301, "ada-pastes-it-all", |key| {
        format!("{PASTE_START}\r\n{}\r\n{PASTE_END}", with_letters(key))
    });
}

/// Ada types a key of the right form that is not hers: the run fails without echoing it, and no
/// machine stored a certificate. A later run on that machine, given her key as a bracketed paste,
/// links it.
#[test]
fn a_wrong_key_is_refused_and_the_right_one_links_later() {
    let world = World::start();
    let cloud = &world.cloud;
    let SignedUp { person: ada, key } = signs_up(&world, (9_120_003_401, "ada-mistypes"), ADA_HOST);
    let first = key.chars().next().expect("a key");
    let wrong = format!("{}{}", if first == 'Z' { 'Y' } else { 'Z' }, &key[1..]);
    let machine = Machine::new().with_host_name(NEW_HOST);
    let since = cloud.relay.mark();

    let refused = links_on(&world, &ada, &machine, &wrong);

    assert!(!refused.success, "a wrong key links: {refused:?}");
    assert!(
        !shows_key(&refused.text, &wrong),
        "the key is echoed: {refused:?}"
    );
    assert_eq!(cloud.count_sent(since, STORE_CERT), 0, "{refused:?}");
    assert_eq!(
        certified_devices(cloud, &ada.token),
        [ada.device],
        "{refused:?}"
    );
    let pasted = format!("{PASTE_START}{key}{PASTE_END}");
    let later = links_on(&world, &ada, &machine, &pasted);
    assert_linked(cloud, &ada, &key, &later);
}
