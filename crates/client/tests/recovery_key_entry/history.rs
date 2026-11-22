//! A new machine linked with the key reads the history through it.

use skald_proto::Role;

use crate::devices::{BOB, said_by};
use crate::fixtures::fronted;
use crate::kit::received;
use crate::reading::{assert_history, sends, shown_history};
use crate::talk::{ADA_HOST, Talk};
use crate::world::ACME;
use crate::{SignedUp, assert_linked, links_with, signs_up};

/// Ada signs up at a terminal, and in a channel of `acme` with Bob, a member, Bob and Ada each say a
/// line. Her new machine links with her key typed in lower case without `-`, naming her first
/// device lost: the run shows no key, its device's certificate is valid under the master key, and
/// `skald history` there shows both lines, in order.
#[test]
fn a_new_machine_links_with_the_key_typed_in_lower_case_without_dashes_and_reads_the_history() {
    let (world, front) = fronted();
    let SignedUp { person, key } = signs_up(&world, (9_120_002_001, "ada-types-short"), ADA_HOST);
    let mut talk = Talk::of(
        (world, front, person),
        ACME,
        (&[9_120_002_002], &["bob-orders-parts"]),
        (false, Role::Member),
    );
    let at = talk.at();
    let space = talk.space.name.clone();
    let said = vec![
        said_by(&mut talk, BOB, (at, 1), "bob lists the parts to order").0,
        sends(&talk.ada, (&space, at.1), "ada orders the parts"),
    ];
    received(&talk.ada.machine);
    let typed = key.to_lowercase().replace('-', "");

    let (machine, shown) = links_with(&talk.world, &talk.ada, &typed);

    assert_linked(&talk.world.cloud, &talk.ada, &key, &shown);
    let history = shown_history(&machine, at.1);
    assert_history(&history, at.1, &said, &[]);
}
