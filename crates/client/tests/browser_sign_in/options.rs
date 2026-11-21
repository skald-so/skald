//! No option says whether a device is the account's first, and `--with-recovery-key` on an
//! account without a master key creates none.

use crate::asks_whether_first;
use crate::at_terminal::{shown_key, sign_in};
use crate::browser::APPROVE;
use crate::cloud::{GIVE_MASTER, certified, ids};
use crate::common::{Stub, response};
use crate::grants::shown_grant;
use crate::linking::says;
use crate::machine::{Machine, SERVER, SIGN_IN};
use crate::terminal::Terminal;
use crate::with_opener;
use crate::world::World;

/// The option that went, and the one that links with the recovery key.
const FIRST_DEVICE: &str = "--first-device";
const WITH_RECOVERY_KEY: &str = "--with-recovery-key";

/// A server that is down: a stub that answers every request with `503`.
fn down() -> Stub {
    Stub::start(|_| response("503 Service Unavailable", "application/json", "{}"))
}

/// `skald sign-in --first-device yes` and `--first-device no` are refused as an unknown option,
/// naming it, before anything is sent.
#[test]
fn first_device_is_refused_as_an_unknown_option() {
    let server = down();
    let machine = Machine::new();

    for answer in ["yes", "no"] {
        let finished = machine
            .start(&[SIGN_IN, SERVER, &server.url, FIRST_DEVICE, answer])
            .finish();

        assert!(!finished.success, "{finished:?}");
        let shown = finished.shown();
        let lower = shown.to_lowercase();
        assert!(
            shown.contains(&format!("'{FIRST_DEVICE}'"))
                && (lower.contains("unexpected argument") || lower.contains("unknown")),
            "not refused as an unknown option: {shown}"
        );
    }
    assert_eq!(server.requests(), Vec::<String>::new(), "requests sent");
}

/// `skald sign-in --help` names no option of whether a device is the account's first, and still
/// names its other options.
#[test]
fn help_names_no_first_device_option() {
    let finished = Machine::new().start(&[SIGN_IN, "--help"]).finish();

    assert!(finished.success, "{finished:?}");
    assert!(finished.stdout.contains(WITH_RECOVERY_KEY), "{finished:?}");
    assert!(
        !finished.stdout.contains(FIRST_DEVICE),
        "{}",
        finished.stdout
    );
}

/// Ada's machine on a new account signs in at a terminal with `--with-recovery-key`: once she
/// approves its code, it says the account has no master key and creates none, sending none and
/// holding no certificate, and asks nothing. Its next `skald sign-in` makes it the account's first
/// device, which shows the recovery key.
#[test]
fn with_recovery_key_on_an_account_without_a_master_key_says_so_and_creates_none() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_312_640_031, "ada-no-master-to-recover");
    let (machine, _opener) = with_opener("ada-workstation");
    let mark = cloud.relay.mark();

    let mut run = Terminal::start(&machine, &sign_in(cloud, &[WITH_RECOVERY_KEY]));
    let grant = shown_grant(cloud, &mut run, 0);
    cloud.decide(&world.browser.page(), &ada, &grant, APPROVE);
    let shown = run.finish();

    assert!(says(&shown.text, &["master key"]), "{}", shown.text);
    assert!(!asks_whether_first(&shown.text), "{}", shown.text);
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    let token = cloud.last_token();
    assert_eq!(cloud.master(&token), None, "a master key created");
    assert_eq!(certified(&cloud.devices(&token)), Vec::new());

    let mut again = Terminal::start(&machine, &sign_in(cloud, &[]));
    shown_key(&mut again);
    again.enter();
    let shown = again.finish();
    assert!(shown.success, "{}", shown.text);
    let devices = cloud.devices(&token);
    assert!(cloud.master(&token).is_some(), "no master key");
    assert_eq!(certified(&devices), ids(&devices), "{devices:?}");
}
