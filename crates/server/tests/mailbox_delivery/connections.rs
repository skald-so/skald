//! Several connections of one device ("Mailbox"): the server sends each of them the entries in
//! `seq` order.

use std::thread;

use crate::link::{bytes, messages, store_to};
use crate::scene::Scene;
use crate::wire::envelopes_of;

#[test]
fn two_connections_of_one_device_each_get_every_entry_in_order() {
    let mut s = Scene::new(3_121_240_301);
    let mut carol = s.link(&s.team().carol);
    let sent = messages(1, &bytes(&s.send_bob(2, true)));
    let mut links = [s.bob_from(0, &sent), s.bob_from(0, &sent)];

    // Two devices sending at once.
    thread::scope(|scope| {
        let (fixture, bob) = (&s.fixture, s.bob);
        for sender in [&mut s.alice, &mut carol] {
            scope.spawn(move || store_to(sender, fixture, bob, 10, true));
        }
    });

    // `envelopes_of` checks that the mailbox's seqs are 1 to 22.
    let mailbox = envelopes_of(s.db(), s.bob);
    assert_eq!(mailbox.len(), 22, "each envelope once");
    for link in &mut links {
        assert_eq!(link.entries(20), messages(3, &mailbox[2..]));
        link.assert_quiet();
    }
}
