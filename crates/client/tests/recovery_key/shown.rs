//! Where the recovery key is shown: at the person's terminal, once; run as an agent or with its
//! output a pipe, `skald sign-in` shows none and the key waits, kept in the key storage, until the
//! person's next `skald sign-in` at a terminal shows it.

use std::fs;
use std::path::PathBuf;

use skald_github_stand_in::User;

use crate::browser::Browser;
use crate::cloud::{Cloud, GIVE_MASTER, shown_grant};
use crate::machine::{DEVICES, FIRST_DEVICE, KEY_STORAGE, Machine, YES};
use crate::system;
use crate::terminal::Terminal;
use crate::{
    APPROVE, as_agent, assert_quiet, assert_waits, bytes_hold_key, first_at_terminal, held,
    holds_key, key_at_sign_in, keys, keys_in, opens_the_backup, quiet_sign_in, setup, sign_in,
    written_down,
};

/// Every file of `machine` with its bytes.
fn snapshot(machine: &Machine) -> Vec<(PathBuf, Vec<u8>)> {
    machine
        .files()
        .into_iter()
        .filter_map(|file| fs::read(&file).ok().map(|bytes| (file, bytes)))
        .collect()
}

/// What `machine` prints for `skald devices` with its output a pipe and at a terminal, and for
/// `skald --agent <AGENT> recv`, each a run of its own.
fn other_commands(machine: &Machine) -> Vec<(&'static str, String)> {
    let home = machine.root().join("home");
    vec![
        ("skald devices", machine.start(&[DEVICES]).finish().shown()),
        (
            "skald devices at a terminal",
            Terminal::start(machine, &[DEVICES]).finish().text,
        ),
        (
            "skald --agent recv",
            machine
                .start_in(&home, &as_agent(&["recv"]))
                .finish()
                .shown(),
        ),
    ]
}

/// Fails unless, once `key` was shown, `skald sign-in` at a terminal and the other commands on
/// `machine` neither show it nor say a key waits.
#[track_caller]
fn assert_gone(cloud: &Cloud, machine: &Machine, key: &[u8; 32]) {
    let again = quiet_sign_in(cloud, machine, &[]);
    assert!(!holds_key(&again, key), "{again}");
    for (what, text) in other_commands(machine) {
        assert_quiet(cloud, what, &text);
        assert!(!holds_key(&text, key), "{what}: {text}");
    }
}

/// The account's first machine, signed in at a terminal, shows a recovery key in the spec's form
/// once, saying what it is for; the server's backup opens with it to the master key the machine
/// pinned, and the server's record names its recovery HPKE key. Neither the server's database, nor
/// any request the machine sent, nor any file of the machine holds the key or the master private
/// key. Later runs show no key and say none waits.
#[test]
fn first_machine_at_a_terminal_shows_a_recovery_key_that_opens_the_backup() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_170_101_001, "ada-recovery-terminal-standin");
    let machine = Machine::new().with_host_name("ada-terminal");

    let run = first_at_terminal(&cloud, &browser.page(), &machine, &ada, &[]);
    let (key, shown) = written_down(run);

    let said = shown.text.to_lowercase();
    for part in ["recovery key", "only", "history"] {
        assert!(said.contains(part), "it does not say {part:?}: {shown:?}");
    }
    let token = cloud.last_token();
    let seed = opens_the_backup(&cloud, &machine, &token, &key);
    let rows = cloud.rows().join("\n").into_bytes();
    let sent = cloud.relay.sent_bytes();
    for (place, bytes) in [("the database", &rows), ("the requests", &sent)] {
        assert!(
            !bytes_hold_key(bytes, &key),
            "{place} hold the recovery key"
        );
        assert!(
            !keys::holds(bytes, &seed),
            "{place} hold the master private key"
        );
    }
    for (file, bytes) in snapshot(&machine) {
        assert!(
            !bytes_hold_key(&bytes, &key),
            "{file:?} holds the recovery key"
        );
        assert!(
            !keys::holds(&bytes, &seed),
            "{file:?} holds the master private key"
        );
    }

    let mark = cloud.relay.mark();
    assert_gone(&cloud, &machine, &key);
    assert_eq!(
        cloud.count_sent(mark, GIVE_MASTER),
        0,
        "the master key sent again"
    );
}

/// Signs `machine` in as `user`'s first device with its output a pipe, so the recovery key waits:
/// the run shows none and prints the line on it; the server took the master key with its record
/// and backup. Gives the session token.
fn signed_in_through_a_pipe(
    cloud: &Cloud,
    browser: &Browser,
    machine: &Machine,
    user: &User,
) -> String {
    let mut run = machine.start(&sign_in(cloud, &[FIRST_DEVICE, YES]));
    let grant = shown_grant(cloud, &mut run, cloud.grants().len());
    cloud.decide(&browser.page(), user, &grant, APPROVE);
    let finished = run.finish();
    assert!(finished.success, "{finished:?}");
    assert_waits(cloud, "skald sign-in into a pipe", &finished.shown());
    let token = cloud.last_token();
    let held = held(cloud, &token);
    assert!(
        held.master.is_some() && held.record.is_some() && held.backup.is_some(),
        "the server took no master key, record and backup"
    );
    token
}

/// As the person, with its output a pipe, `skald sign-in` shows no key and prints the line; while
/// the key waits, `skald devices`, also at a terminal, and an agent's `skald recv` print the line
/// and not the key, and the secrets file holds the key, the local state not. The next `skald
/// sign-in` at a terminal shows it, and it opens the backup; from then on no command prints it or
/// the line, and no file holds it.
#[test]
fn with_its_output_a_pipe_sign_in_shows_none_and_the_key_waits_in_the_secrets_file() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_170_101_002, "ada-recovery-pipe-standin");
    let machine = Machine::new().with_host_name("ada-pipe");
    let token = signed_in_through_a_pipe(&cloud, &browser, &machine, &ada);
    let waiting = other_commands(&machine);
    for (what, text) in &waiting {
        assert_waits(&cloud, what, text);
    }
    let files = snapshot(&machine);
    let state = machine.state_bytes();

    let key = key_at_sign_in(&cloud, &machine);

    opens_the_backup(&cloud, &machine, &token, &key);
    for (what, text) in &waiting {
        assert!(!holds_key(text, &key), "{what}: {text}");
    }
    let holders: Vec<&PathBuf> = files
        .iter()
        .filter(|(_, bytes)| bytes_hold_key(bytes, &key))
        .map(|(file, _)| file)
        .collect();
    let secrets: Vec<PathBuf> = machine.files_holding(&token);
    assert_eq!(
        holders,
        secrets.iter().collect::<Vec<_>>(),
        "files that held the waiting key"
    );
    assert!(
        !bytes_hold_key(&state, &key),
        "the local state held the recovery key"
    );
    for (file, bytes) in snapshot(&machine) {
        assert!(
            !bytes_hold_key(&bytes, &key),
            "{file:?} holds the shown key"
        );
    }
    assert_gone(&cloud, &machine, &key);
}

/// As an agent, `skald sign-in` shows no key, also at a terminal, and its later runs show none
/// either; the person's next `skald sign-in` at a terminal shows it, and it opens the backup.
#[test]
fn as_an_agent_sign_in_shows_none_and_the_persons_next_sign_in_does() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_170_101_003, "ada-recovery-agent-standin");
    let machine = Machine::new().with_host_name("ada-agent");

    let run = first_at_terminal(&cloud, &browser.page(), &machine, &ada, &as_agent(&[]));
    let agent = run.finish();

    assert!(agent.success, "{agent:?}");
    assert_eq!(keys_in(&agent.text), Vec::<String>::new(), "{agent:?}");
    let token = cloud.last_token();
    assert!(
        held(&cloud, &token).backup.is_some(),
        "the server took no backup"
    );
    let again =
        Terminal::start(&machine, &as_agent(&sign_in(&cloud, &[FIRST_DEVICE, YES]))).finish();
    assert_eq!(keys_in(&again.text), Vec::<String>::new(), "{again:?}");
    let recv = machine
        .start_in(&machine.root().join("home"), &as_agent(&["recv"]))
        .finish();
    assert_waits(&cloud, "skald --agent recv", &recv.shown());

    let key = key_at_sign_in(&cloud, &machine);

    opens_the_backup(&cloud, &machine, &token, &key);
    for text in [&agent.text, &again.text] {
        assert!(!holds_key(text, &key), "{text}");
    }
    quiet_sign_in(&cloud, &machine, &as_agent(&[]));
}

/// With the system's key storage, only with `SKALD_TEST_REAL_KEY_STORAGE=1`: while the recovery
/// key waits, the key storage's entry of the machine holds it and no file does; once it is shown,
/// the entry no longer holds it. The test deletes the entry.
#[test]
fn with_the_systems_key_storage_its_entry_holds_the_key_only_until_it_is_shown() {
    if std::env::var("SKALD_TEST_REAL_KEY_STORAGE").as_deref() != Ok("1") {
        return;
    }
    let (cloud, browser) = setup();
    let ada = cloud.user(4_170_101_004, "ada-recovery-keyring-standin");
    let machine = Machine::new()
        .with_host_name("ada-keyring")
        .with_env(KEY_STORAGE, "auto");
    let entry = system::Entry::of(&machine);
    signed_in_through_a_pipe(&cloud, &browser, &machine, &ada);
    let waiting = entry.value().expect("the entry holds the secrets");
    let files = snapshot(&machine);

    let key = key_at_sign_in(&cloud, &machine);

    assert!(
        bytes_hold_key(waiting.as_bytes(), &key),
        "the entry did not hold the waiting key"
    );
    for (file, bytes) in &files {
        assert!(
            !bytes_hold_key(bytes, &key),
            "{file:?} held the recovery key"
        );
    }
    let after = entry.value().expect("the entry holds the secrets");
    assert!(
        !bytes_hold_key(after.as_bytes(), &key),
        "the entry holds the shown key"
    );
}
