//! Which commands wait for the lock: `skald sign-in` on a signed-in machine and the runs that
//! create a missing state wait for it; the runs that only read the state take none.

use std::fs::File;
use std::thread;
use std::time::{Duration, Instant};

use crate::held::{
    LONG, SHORT, TIMEOUT_FLAG, WATCHED, assert_failed_on_the_lock, release, send_holding, start,
};
use crate::machine::{Machine, SERVER, SIGN_IN};
use crate::talk::Talk;
use crate::world::{ACME, ADD_RULE, APPROVE, FOLDER, RULES, dir_of, path};

/// The text the tests send; it is nowhere else.
const ADA_SAYS: &str = "lock test: the deploy window opens at nine";

/// The lock's file in the state directory.
const LOCK_FILE: &str = "state.lock";

/// A `skald send` on Ada's machine waits for the server with the lock held. `skald sign-in` on the
/// machine, already signed in, waits for the lock meanwhile and sends the server nothing as Ada;
/// once the send succeeds, it checks Ada's session and succeeds.
#[test]
fn sign_in_again_beside_a_send_waits_for_the_lock() {
    let talk = Talk::new(
        &[4_145_057_001, 4_145_057_002],
        &["ada-signs-in-meanwhile", "bob-idle-beside-sign-in"],
        false,
    );
    let cloud = talk.cloud();
    let bearer = format!("\nauthorization: bearer {}", talk.ada.token.to_lowercase());
    let holder = send_holding(&talk, ADA_SAYS);
    let mark = cloud.relay.mark();
    let mut signing_in = start(&talk, LONG, &[SIGN_IN, SERVER, &cloud.url]);

    let until = Instant::now() + WATCHED;
    while Instant::now() < until {
        signing_in.assert_running();
        let sent = cloud.relay.sent_since(mark);
        assert!(
            !sent.iter().any(|request| request.head.contains(&bearer)),
            "a sign-in waiting for the lock reached the server: {}",
            signing_in.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
    let finished = release(&talk, vec![holder, signing_in]);

    assert!(
        finished[1].shown().contains(&talk.ada.user.login),
        "{:?}",
        finished[1]
    );
}

/// While a `skald send` on Ada's machine holds the lock, `skald rules` and `skald folder` finish
/// before a run waiting for the lock would give up.
#[test]
fn reading_runs_beside_a_send_take_no_lock() {
    let talk = Talk::new(
        &[4_145_058_001, 4_145_058_002],
        &["ada-reads-meanwhile", "bob-idle-beside-reads"],
        false,
    );
    let folder = dir_of(&talk.ada.machine, "work/clientA");
    let holder = send_holding(&talk, ADA_SAYS);

    for args in [&[RULES][..], &[FOLDER, path(&folder)]] {
        let finished = start(&talk, SHORT, args).finish();
        assert!(finished.success, "{args:?}: {finished:?}");
    }
    release(&talk, vec![holder]);
}

/// On a machine with no state, a lock held on its state directory keeps `skald add-rule` and
/// `skald approve-binding` out: each waits for its timeout, then fails naming the lock.
#[test]
fn runs_that_create_the_state_wait_for_a_lock_held_there() {
    let machine = Machine::new();
    let folder = dir_of(&machine, "work/acme");
    let held = File::create(machine.state_dir().join(LOCK_FILE)).expect("create the lock's file");
    held.lock().expect("take the lock");
    let seconds = SHORT.to_string();

    for args in [
        &[ADD_RULE, path(&folder), ACME][..],
        &[APPROVE, path(&folder)],
    ] {
        let mut all = vec![TIMEOUT_FLAG, seconds.as_str()];
        all.extend_from_slice(args);
        let started = Instant::now();
        let finished = machine.start(&all).finish();

        assert_failed_on_the_lock(&finished);
        assert!(
            started.elapsed() >= Duration::from_secs(SHORT),
            "{args:?} did not wait for the lock: {finished:?}"
        );
    }
}
