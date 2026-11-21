//! A waiting `skald recv --wait`, one-shot or through the helper, follows a space another device of
//! its account created, on a `space-notice` of that space, with no other command; so does a `skald
//! recv` that gets such a notice while it syncs.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::SpaceId;

use crate::cloud::WAIT;
use crate::fixtures::await_running;
use crate::helped::Helped;
use crate::joined::{Account, Withheld, anchored};
use crate::kit::{RECV, create_space, latest, received};
use crate::machine::Run;
use crate::world::{ACME, GLOBEX, Person, next_by, revisions_request, space_id, store};

/// The flag of `skald recv` that waits for what is to come.
const WAIT_FLAG: &str = "--wait";
/// The request that opens a WebSocket connection to the server, as its line starts.
const WEBSOCKET: &str = "GET /ws ";
/// How long a frame the front sent takes to reach a run, at most.
const PUSHED: Duration = Duration::from_millis(500);

/// The relay withholds the account's anchors objects. Ada's first machine creates `acme`, and her
/// second machine runs `skald recv`, so it does not follow `acme`. The second machine's `skald recv
/// --wait`, which `wait` starts, connects and gets the anchors objects withheld; then the relay
/// gives them again, and a revision 3 of `acme`, signed as her first device, is stored, which sends
/// the waiting run a notice of `acme`. With no other command, the second device follows `acme`: the
/// server soon holds its anchors object naming `acme` at revision 3.
#[track_caller]
fn assert_follows_on_notice(account: &Account, wait: impl FnOnce() -> Run) {
    let (cloud, ada, laptop) = (account.cloud(), &account.ada, &account.laptop);
    let withheld = Withheld::start(cloud);
    let acme = create_space(cloud, ada, ACME);
    received(&laptop.machine);
    let (since, before) = (cloud.relay.mark(), withheld.emptied());
    let mut waiting = wait();
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

/// [`assert_follows_on_notice`], one-shot.
#[test]
fn waiting_recv_follows_a_space_on_its_revision_s_notice() {
    let account = Account::new(4_811_406_101, "ada-waits-on-her-laptop", &[]);
    let laptop = &account.laptop;
    assert_follows_on_notice(&account, || laptop.machine.start(&[RECV, WAIT_FLAG]));
}

/// [`assert_follows_on_notice`], the wait going through a helper, which takes the notice in.
#[test]
fn waiting_recv_through_the_helper_follows_a_space_on_its_revision_s_notice() {
    let account = Account::new(4_811_406_111, "ada-waits-helped-on-her-laptop", &[]);
    // Dropped first: it kills the helper before the machine's directory goes.
    let helped = Helped::new(&account.laptop.machine);
    assert_follows_on_notice(&account, || helped.start(&[], &[RECV, WAIT_FLAG]));
}

/// Ada's first machine creates `globex` before her second machine is linked, so both follow it.
/// The relay withholds the account's anchors objects, and her first machine creates `acme`. The
/// second machine's `skald recv` gets the anchors objects withheld; the relay holds its fetch of
/// `globex`'s revisions, gives the anchors objects again, and a revision 3 of `acme`, signed as her
/// first device, is stored, whose notice reaches the run before the fetch goes on. Once the run
/// ends, the server holds the second device's anchors object naming `acme` at revision 3.
#[test]
fn recv_follows_a_space_whose_notice_comes_while_it_syncs() {
    let account = Account::new(4_811_406_121, "ada-hears-while-syncing", &[GLOBEX]);
    let (cloud, ada, laptop) = (account.cloud(), &account.ada, &account.laptop);
    let globex = space_id(cloud, &ada.token, GLOBEX);
    let withheld = Withheld::start(cloud);
    let acme = create_space(cloud, ada, ACME);
    let fetch = revisions_request(globex);
    cloud.relay.hold(&fetch, 2);
    let before = withheld.emptied();
    let mut run = laptop.machine.start(&[RECV]);
    await_running(&mut run, "held fetch after the withheld anchors", || {
        cloud.relay.held() > 0 && withheld.emptied() > before
    });
    withheld.stop();
    let third = next_by(&latest(cloud, &ada.token, acme.id), ada, |_| {});

    let earlier = notices(&account, laptop, acme.id);
    store(cloud, &ada.token, &third);
    await_running(&mut run, "notice of acme's revision 3", || {
        notices(&account, laptop, acme.id) > earlier
    });
    // A moment for the run to read it.
    thread::sleep(PUSHED);
    cloud.relay.send_through(&fetch);

    let ran = run.finish();
    assert!(ran.success, "{ran:?}");
    assert_eq!(
        anchored(cloud, laptop, acme.id),
        Some(third.hash()),
        "the run did not follow acme: {ran:?}"
    );
}

/// How many notices of `space` have reached `person`'s device.
fn notices(account: &Account, person: &Person, space: SpaceId) -> usize {
    let sent = account.front.notices_to(person.device);
    sent.iter().filter(|noticed| **noticed == space).count()
}
