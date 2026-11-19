//! The recovery key on the page of `skald sign-in` (`membership.md`, "Account"): the account's
//! first device, answered so on its page, shows its recovery key there and not in its output; as an
//! agent, the line of a waiting recovery key names the page, and the page shows the key.

use skald_crypto::{RecoveryKey, open_master_backup};
use skald_proto::{Cbor as _, MasterBackup};

use crate::at_terminal::{AGENT, as_agent, keys_in};
use crate::browser::APPROVE;
use crate::cloud::{Cloud, shown_grant, unb64};
use crate::linking::says;
use crate::machine::{DEVICES, FIRST_DEVICE, Machine, Run, SERVER, SIGN_IN, YES};
use crate::pages::{
    FIRST_DEVICE_QUESTION, Opener, PAGE, Paged, WRITTEN_DOWN_BUTTON, YES_BUTTON, approve_next_as,
    assert_holds_none, start_with,
};
use crate::world::{World, run};

/// The lines of `text` that say a recovery key waits: those that hold `recovery key` and the
/// command that shows it, `skald sign-in --server <cloud>`.
fn waiting_lines<'t>(cloud: &Cloud, text: &'t str) -> Vec<&'t str> {
    let command = format!("skald sign-in --server {}", cloud.url);
    (text.lines())
        .filter(|line| line.contains(&command) && line.to_lowercase().contains("recovery key"))
        .collect()
}

/// Fails unless `key`, a recovery key in its shown form, opens the master backup the server holds
/// for the account of the session `token` to its master key.
#[track_caller]
fn assert_opens_the_backup(cloud: &Cloud, token: &str, key: &str) {
    let (status, held) = cloud.get_as("/account/master", token);
    assert_eq!(status, 200, "GET /account/master: {held}");
    let field = |name: &str| {
        unb64(
            held[name]
                .as_str()
                .unwrap_or_else(|| panic!("{name}: {held}")),
        )
    };
    let master: [u8; 32] = field("master").try_into().expect("a 32-byte master key");
    let backup = MasterBackup::from_cbor(&field("backup")).expect("a master backup");
    let key = RecoveryKey::parse(key).expect("a recovery key");
    open_master_backup(&backup, &key, cloud.account_of(token), &master)
        .expect("the backup opens with the key the page showed");
}

/// The person reads the recovery key `paged` shows while `command`, on `machine`, goes on, and
/// presses the button under it. Fails unless `command` then exits with success, its output holding
/// no key, the key opens the server's master backup, and a later command says no key waits.
#[track_caller]
fn written_down(cloud: &Cloud, machine: &Machine, mut command: Run, paged: &Paged) {
    let text = paged.wait_for(&mut command, "a recovery key", |text| {
        !keys_in(text).is_empty()
    });
    let keys = keys_in(&text);
    assert_eq!(keys.len(), 1, "keys on the page: {text}");
    paged.press(WRITTEN_DOWN_BUTTON);
    let finished = command.finish();
    assert!(finished.success, "{finished:?}");
    let secret = paged.opening.query.as_str();
    assert_holds_none("skald sign-in", &finished.shown(), &[], &[secret]);

    assert_opens_the_backup(cloud, &cloud.last_token(), &keys[0]);
    let later = run(machine, &[DEVICES]);
    assert!(later.success, "{later:?}");
    let shown = later.shown();
    let lines = waiting_lines(cloud, &shown);
    assert_eq!(lines, Vec::<&str>::new(), "{later:?}");
}

/// Ada's first machine signs in as an agent with `--browser`; on its page she answers that it is
/// the account's first device. The page shows the recovery key, and once she presses the button
/// under it, the command exits with success without the key in its output. The key opens the
/// server's master backup, and no later command says a key waits.
#[test]
fn a_first_device_answered_on_the_page_shows_its_recovery_key_there() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_179_301_001, "ada-first-on-a-page");
    let machine = Machine::new().with_host_name("ada-workstation");
    let opener = Opener::on(&machine);
    let before = cloud.grants().len();
    let args = as_agent(&[SIGN_IN, SERVER, &cloud.url, PAGE]);
    let mut signing_in = start_with(&machine, opener.program(), &args);
    approve_next_as(&world, &ada, before, &mut signing_in);
    let paged = Paged::opened(&world.browser, &opener, 0, &mut signing_in);
    paged.wait_for(&mut signing_in, "whether it is the first device", |text| {
        says(text, &[FIRST_DEVICE_QUESTION])
    });
    paged.press(YES_BUTTON);
    written_down(cloud, &machine, signing_in, &paged);
}

/// Ada's first machine signs in as an agent with `--first-device yes`: the recovery key waits, and
/// the run, then an agent's `skald devices`, each print one line on it that also names the page,
/// `--browser`. As an agent, `skald sign-in --server <cloud> --browser` opens the page, which shows
/// the key; once she presses the button under it, the command exits with success without the key
/// in its output, the key opens the server's master backup, and no later command says a key waits.
#[test]
fn as_an_agent_the_waiting_keys_line_names_the_page_and_the_page_shows_the_key() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_179_301_011, "ada-waits-for-a-page");
    let machine = Machine::new().with_host_name("ada-workstation");
    let before = cloud.grants().len();
    let mut first = machine.start(&as_agent(&[SIGN_IN, SERVER, &cloud.url, FIRST_DEVICE, YES]));
    let grant = shown_grant(cloud, &mut first, before);
    cloud.decide(&world.browser.page(), &ada, &grant, APPROVE);
    let first = first.finish();
    assert!(first.success, "{first:?}");
    let devices = run(&machine, &["--agent", AGENT, DEVICES]);
    for (what, shown) in [
        ("skald sign-in", first.shown()),
        ("skald devices", devices.shown()),
    ] {
        let lines = waiting_lines(cloud, &shown);
        assert_eq!(
            lines.len(),
            1,
            "{what}: one line on the waiting key: {shown}"
        );
        assert!(
            lines[0].contains(PAGE),
            "{what}: the line names no page: {}",
            lines[0]
        );
        assert_eq!(
            keys_in(&shown),
            Vec::<String>::new(),
            "{what} shows a key: {shown}"
        );
    }

    let opener = Opener::on(&machine);
    let args = as_agent(&[SIGN_IN, SERVER, &cloud.url, PAGE]);
    let mut showing = start_with(&machine, opener.program(), &args);
    let paged = Paged::opened(&world.browser, &opener, 0, &mut showing);
    written_down(cloud, &machine, showing, &paged);
}
