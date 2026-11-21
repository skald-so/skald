//! A waiting `skald recv --wait` follows a space another device of its account created, on a
//! `space-notice` of that space, with no other command.

use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::WAIT;
use crate::fixtures::await_running;
use crate::joined::{Account, Withheld, anchored};
use crate::kit::{RECV, create_space, latest, received};
use crate::world::{ACME, next_by, store};

/// The flag of `skald recv` that waits for what is to come.
const WAIT_FLAG: &str = "--wait";
/// The request that opens a WebSocket connection to the server, as its line starts.
const WEBSOCKET: &str = "GET /ws ";

/// The relay withholds the account's anchors objects. Ada's first machine creates `acme`, and her
/// second machine runs `skald recv`, so it does not follow `acme`. The second machine's `skald recv
/// --wait` connects and gets the anchors objects withheld; then the relay gives them again, and a
/// revision 3 of `acme`, signed as her first device, is stored, which sends the waiting run a
/// notice of `acme`. With no other command, the second device follows `acme`: the server soon holds
/// its anchors object naming `acme` at revision 3.
#[test]
fn waiting_recv_follows_a_space_on_its_revision_s_notice() {
    let account = Account::new(4_811_406_101, "ada-waits-on-her-laptop", &[]);
    let (cloud, ada, laptop) = (account.cloud(), &account.ada, &account.laptop);
    let withheld = Withheld::start(cloud);
    let acme = create_space(cloud, ada, ACME);
    received(&laptop.machine);
    let (since, before) = (cloud.relay.mark(), withheld.emptied());
    let mut waiting = laptop.machine.start(&[RECV, WAIT_FLAG]);
    await_running(
        &mut waiting,
        "connection that got the anchors objects withheld",
        || cloud.count_sent(since, WEBSOCKET) > 0 && withheld.emptied() > before,
    );
    withheld.stop();
    let third = next_by(&latest(cloud, &ada.token, acme.id), ada, |_| {});

    store(cloud, &ada.token, &third);

    let deadline = Instant::now() + WAIT;
    while anchored(cloud, laptop, acme.id) != Some(third.hash()) {
        let gone = waiting.exited();
        if gone || Instant::now() >= deadline {
            let followed = anchored(cloud, laptop, acme.id) == Some(third.hash());
            assert!(
                followed,
                "the waiting run did not follow acme (exited: {gone}): {}",
                waiting.shown()
            );
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        account.front.notices_to(laptop.device).contains(&acme.id),
        "no notice of acme reached the second device"
    );
    waiting.kill();
}
