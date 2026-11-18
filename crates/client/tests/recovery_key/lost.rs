//! A run that stops on its way: with the server's answer to the master key lost, or killed after
//! sending it or after showing the recovery key and before deleting it, the next `skald sign-in`
//! at a terminal finishes the job, and shows the key once.

use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::{GIVE_MASTER, WAIT};
use crate::machine::Machine;
use crate::relay::Lose;
use crate::{
    first_at_terminal, held, holds_key, key_at_sign_in, keys_in, opens_the_backup, quiet_sign_in,
    setup, shown_key,
};

/// The relay loses the server's answer to the master key as `how` says: it closes the connection,
/// or it withholds the answer and the test kills the CLI. That run shows no key; the next `skald
/// sign-in` at a terminal sends them again, leaves the server the record and backup it took and
/// shows the key once, which opens that backup; the one after shows none.
fn recovers_from_a_lost_answer(how: Lose, (id, login): (u64, &str)) {
    let (cloud, browser) = setup();
    cloud.relay.lose_answer(GIVE_MASTER, how);
    let machine = Machine::new().with_host_name("ada-lost-answer");

    let run = first_at_terminal(
        &cloud,
        &browser.page(),
        &machine,
        &cloud.user(id, login),
        &[],
    );
    let stopped = match how {
        Lose::Close => run.finish(),
        Lose::Withhold => {
            let deadline = Instant::now() + WAIT;
            while cloud.relay.lost() == 0 {
                assert!(Instant::now() < deadline, "no master key: {}", run.shown());
                thread::sleep(Duration::from_millis(50));
            }
            run.kill()
        }
    };
    assert_eq!(
        cloud.relay.lost(),
        1,
        "the server never answered the master key"
    );
    assert_eq!(keys_in(&stopped.text), Vec::<String>::new(), "{stopped:?}");
    let token = cloud.last_token();
    let taken = held(&cloud, &token);
    let (record, backup) = (
        taken.record.expect("the server took no record"),
        taken.backup.expect("the server took no backup"),
    );

    let mark = cloud.relay.mark();
    let key = key_at_sign_in(&cloud, &machine);

    assert!(
        cloud.count_sent(mark, GIVE_MASTER) >= 1,
        "the master key was not sent again"
    );
    let now = held(&cloud, &token);
    assert_eq!(now.master, taken.master, "another master key");
    assert_eq!(now.record.as_ref(), Some(&record), "another record");
    assert_eq!(now.backup.as_ref(), Some(&backup), "another backup");
    opens_the_backup(&cloud, &machine, &token, &key);
    assert!(!holds_key(&stopped.text, &key), "{stopped:?}");
    quiet_sign_in(&cloud, &machine, &[]);
}

#[test]
fn master_key_answer_dropped_by_the_proxy_leaves_the_key_to_the_next_sign_in() {
    recovers_from_a_lost_answer(Lose::Close, (4_170_102_001, "ada-recovery-lost-standin"));
}

#[test]
fn master_key_answer_lost_with_the_cli_killed_leaves_the_key_to_the_next_sign_in() {
    recovers_from_a_lost_answer(
        Lose::Withhold,
        (4_170_102_002, "ada-recovery-killed-standin"),
    );
}

/// Killed after showing the recovery key and before deleting it, before the person pressed Enter:
/// the next `skald sign-in` at a terminal shows the same key again, and later ones do not.
#[test]
fn killed_after_showing_the_key_the_next_sign_in_shows_it_again_and_later_ones_do_not() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_170_102_003, "ada-recovery-unread-standin");
    let machine = Machine::new().with_host_name("ada-unread");

    let mut run = first_at_terminal(&cloud, &browser.page(), &machine, &ada, &[]);
    let first = shown_key(&mut run);
    run.kill();
    let mark = cloud.relay.mark();
    let key = key_at_sign_in(&cloud, &machine);

    assert_eq!(crate::backup::shown_form(&key), first, "another key");
    assert_eq!(
        cloud.count_sent(mark, GIVE_MASTER),
        0,
        "the master key sent again"
    );
    opens_the_backup(&cloud, &machine, &cloud.last_token(), &key);
    for _ in 0..2 {
        quiet_sign_in(&cloud, &machine, &[]);
    }
}
