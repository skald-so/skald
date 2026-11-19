//! The account's other machines: of two first machines at once, the one whose master key the
//! server refuses drops its recovery key with it; a machine that is not the first gets none; on an
//! account made before recovery keys, `skald sign-in` says it has none.

use std::thread;
use std::time::Duration;

use crate::cloud::GIVE_MASTER;
use crate::machine::{DEVICES, FIRST_DEVICE, Machine, NO, RUN_LIMIT, YES};
use crate::terminal::{Shown, Terminal};
use crate::{
    LINK_CODE, approve, assert_quiet, backup, bytes_hold_key, first_at_terminal, held, holds_key,
    keys, keys_in, opens_the_backup, setup, sign_in, until_link_code, written_down,
};

/// `bytes` in lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lets `runs` finish, the person pressing Enter on each once it shows a recovery key and ending
/// each that waits for linking once it shows its link code; gives what each printed.
fn settled(runs: Vec<Terminal>) -> Vec<Shown> {
    let mut runs: Vec<Option<Terminal>> = runs.into_iter().map(Some).collect();
    let mut entered = vec![false; runs.len()];
    let mut shown: Vec<Option<Shown>> = runs.iter().map(|_| None).collect();
    let started = std::time::Instant::now();
    while shown.iter().any(Option::is_none) {
        for at in 0..runs.len() {
            let Some(run) = runs[at].as_mut() else {
                continue;
            };
            if run.exited() {
                shown[at] = runs[at].take().map(Terminal::finish);
            } else if run.shown().contains(LINK_CODE) {
                shown[at] = runs[at].take().map(until_link_code);
            } else if !entered[at] && !keys_in(&run.shown()).is_empty() {
                run.enter();
                entered[at] = true;
            }
        }
        assert!(started.elapsed() < RUN_LIMIT, "the runs did not finish");
        thread::sleep(Duration::from_millis(50));
    }
    shown.into_iter().map(Option::unwrap).collect()
}

/// Two first machines of one account send their master keys at once: the relay holds the first
/// until the second comes. The one the server takes shows the key that opens the backup; the
/// other shows none, keeps none and says none waits, then or later, and sends no master key again.
#[test]
fn of_two_first_machines_at_once_the_refused_one_keeps_and_shows_no_recovery_key() {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(4_170_103_001, "ada-recovery-race-standin");
    let machines = [
        Machine::new().with_host_name("ada-one"),
        Machine::new().with_host_name("ada-two"),
    ];
    cloud.relay.hold(GIVE_MASTER, 2);

    let runs: Vec<Terminal> = machines
        .iter()
        .map(|machine| {
            let grants = cloud.grants().len();
            let args = sign_in(&cloud, &[FIRST_DEVICE, YES]);
            let mut run = Terminal::start(machine, &args);
            approve(&cloud, &page, &ada, &mut run, grants);
            run
        })
        .collect();
    let shown = settled(runs);

    assert_eq!(
        cloud.relay.held(),
        2,
        "both did not send a master key: {shown:?}"
    );
    let showing: Vec<usize> = (0..2)
        .filter(|at| !keys_in(&shown[*at].text).is_empty())
        .collect();
    assert_eq!(showing.len(), 1, "{shown:?}");
    let (winner, loser) = (showing[0], 1 - showing[0]);
    let keys_shown = keys_in(&shown[winner].text);
    assert_eq!(keys_shown.len(), 1, "{shown:?}");
    let key = backup::key_bytes(&keys_shown[0]);
    let token = cloud.last_token();
    opens_the_backup(&cloud, &machines[winner], &token, &key);
    assert_quiet(&cloud, "the refused machine", &shown[loser].text);
    let loser_machine = &machines[loser];
    for file in loser_machine.files() {
        let bytes = std::fs::read(&file).expect("read a machine's file");
        assert!(
            !bytes_hold_key(&bytes, &key),
            "{file:?} holds the account's key"
        );
        assert!(
            keys_in(&String::from_utf8_lossy(&bytes)).is_empty(),
            "{file:?} holds a key"
        );
    }

    let mark = cloud.relay.mark();
    let args = sign_in(&cloud, &[FIRST_DEVICE, YES]);
    let again = until_link_code(Terminal::start(loser_machine, &args));
    assert_quiet(&cloud, "the refused machine's next sign-in", &again.text);
    let devices = loser_machine.start(&[DEVICES]).finish();
    assert_quiet(
        &cloud,
        "the refused machine's skald devices",
        &devices.shown(),
    );
    assert_eq!(
        cloud.count_sent(mark, GIVE_MASTER),
        0,
        "the refused machine sent again"
    );
}

/// Whether `text`, what a machine that waits for linking printed at a terminal, says on a line that
/// the recovery key links it too, with `skald sign-in --first-device no --with-recovery-key`.
fn offers_the_recovery_key(text: &str) -> bool {
    text.lines()
        .any(|line| line.contains("skald sign-in --first-device no --with-recovery-key"))
}

/// A second machine of the account gets no recovery key: it shows none and says none waits, at
/// its sign-in and later, and the account keeps the first machine's record and backup. Waiting for
/// linking, it says the account's recovery key links it too.
#[test]
fn a_second_machine_of_the_account_gets_no_recovery_key() {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(4_170_103_002, "ada-recovery-second-standin");
    let first = Machine::new().with_host_name("ada-first");
    let (key, _) = written_down(first_at_terminal(&cloud, &page, &first, &ada, &[]));
    let token = cloud.last_token();
    let before = held(&cloud, &token);
    let second = Machine::new().with_host_name("ada-second");

    let grants = cloud.grants().len();
    let mut run = Terminal::start(&second, &sign_in(&cloud, &[FIRST_DEVICE, NO]));
    approve(&cloud, &page, &ada, &mut run, grants);
    let signed_in = until_link_code(run);

    assert_quiet(&cloud, "the second machine's sign-in", &signed_in.text);
    assert!(!holds_key(&signed_in.text, &key), "{signed_in:?}");
    let again = until_link_code(Terminal::start(
        &second,
        &sign_in(&cloud, &[FIRST_DEVICE, NO]),
    ));
    assert_quiet(&cloud, "its next sign-in", &again.text);
    for (what, shown) in [("its sign-in", &signed_in), ("its next sign-in", &again)] {
        assert!(offers_the_recovery_key(&shown.text), "{what}: {shown:?}");
    }
    let text = second.start(&[DEVICES]).finish().shown();
    assert_quiet(&cloud, "its skald devices", &text);
    for file in second.files() {
        let bytes = std::fs::read(&file).expect("read a machine's file");
        assert!(
            !bytes_hold_key(&bytes, &key),
            "{file:?} holds the recovery key"
        );
    }
    let after = held(&cloud, &token);
    assert_eq!(after.record, before.record, "another record");
    assert_eq!(after.backup, before.backup, "another backup");
    opens_the_backup(&cloud, &first, &token, &key);
}

/// On an account whose master key the server's database holds without an account record, as an
/// account made before recovery keys does, `skald sign-in` at a terminal says the account has no
/// recovery key and shows none: on a device that waited for linking there, and on a new one.
#[test]
fn on_an_account_made_before_recovery_keys_sign_in_says_it_has_no_recovery_key() {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(4_170_103_003, "ada-recovery-old-standin");
    let old = Machine::new().with_host_name("ada-old");
    // Its result does not matter: the account has no master key yet.
    drop(cloud.signed_in(&page, &old, &ada, &[FIRST_DEVICE, NO]));
    let token = cloud.last_token();
    let account = cloud.account_of(&token);
    assert_eq!(held(&cloud, &token).master, None, "a master key");
    let master = keys::ed25519(&[0x6d; 32]);
    cloud.execute(&format!(
        "UPDATE accounts SET master = '\\x{}'::bytea WHERE id = '{account}'::uuid",
        hex(&master)
    ));
    assert_eq!(held(&cloud, &token).master, Some(master));

    let again = until_link_code(Terminal::start(&old, &sign_in(&cloud, &[FIRST_DEVICE, NO])));

    assert!(
        again.text.to_lowercase().contains("no recovery key"),
        "{again:?}"
    );
    assert_eq!(keys_in(&again.text), Vec::<String>::new(), "{again:?}");
    let new = Machine::new().with_host_name("ada-new");
    let grants = cloud.grants().len();
    let mut run = Terminal::start(&new, &sign_in(&cloud, &[FIRST_DEVICE, NO]));
    approve(&cloud, &page, &ada, &mut run, grants);
    let fresh = until_link_code(run);
    assert!(
        fresh.text.to_lowercase().contains("no recovery key"),
        "{fresh:?}"
    );
    assert_eq!(keys_in(&fresh.text), Vec::<String>::new(), "{fresh:?}");
}
