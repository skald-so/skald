//! Link codes of a new machine without a session (`membership.md`, "Link code"): it shows a code of
//! 12 characters, the run's pairing first, at once. A failed run spends one of the code's 3 runs,
//! and the next run's pairing changes the code shown, which then links. After the third failed run
//! the code ends; it ends at its expiry too, and a fresh code links. The new device checks the
//! issuer and the link object of its grant once its session by its key lists the account's devices:
//! a grant that fails is dropped with the session, and the master key is never pinned, also by
//! the next commands.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::DeviceKind;

use crate::cloud::Cloud;
use crate::fixtures::{Issuer, add_device_of_kind, fronted_with};
use crate::linking::{
    EXPIRED, LIFETIME_SETTING, LINK, NEW_CODE, OWN_DEVICE, STORE_CERT, THREE_TIMES, YES, codes_in,
    failures, link_failing, link_with, says, typed,
};
use crate::machine::{DEVICES, Machine, SERVER, SIGN_IN};
use crate::played_issuer::{Granted, answer_paired};
use crate::relay::strings_named;
use crate::terminal::{Shown, Terminal};
use crate::world::{Person, run};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// The base32 alphabet of RFC 4648, §6.
const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A run without a session opening, its request.
const OPEN: &str = "POST /links ";

/// The account's device list, its request.
const LIST: &str = "GET /devices ";

/// How long a test watches for what must not happen.
const QUIET: Duration = Duration::from_secs(3);

/// A new machine with no device and no session running `skald sign-in` at a terminal; nobody
/// approves its browser sign-in.
struct Fresh {
    /// Dropped first, so its run ends before the machine's directories go.
    run: Option<Terminal>,
    machine: Machine,
    /// When it started, from which the relay's requests are counted.
    started: Instant,
}

impl Fresh {
    /// Starts it on `machine`; gives it once it shows a code of 12 characters.
    #[track_caller]
    fn start(cloud: &Cloud, machine: Machine) -> Self {
        let started = cloud.relay.mark();
        let mut run = Terminal::start(&machine, &[SIGN_IN, SERVER, &cloud.url]);
        run.wait_for("a code of 12 characters", |shown| {
            codes_in(shown).iter().any(|code| code.len() == 14)
        });
        Fresh {
            run: Some(run),
            machine,
            started,
        }
    }

    fn run(&mut self) -> &mut Terminal {
        self.run.as_mut().expect("the new machine's run")
    }

    /// The code it shows last.
    #[track_caller]
    fn code(&mut self) -> String {
        codes_in(&self.run().shown()).pop().expect("a code shown")
    }

    /// Waits until it shows a code other than `old`; gives it.
    #[track_caller]
    fn next_code(&mut self, old: &str) -> String {
        self.run().wait_for("another code", |shown| {
            codes_in(shown).last().is_some_and(|code| code != old)
        });
        self.code()
    }

    /// Waits until it has said of `count` runs that they failed.
    #[track_caller]
    fn wait_for_failures(&mut self, count: usize) {
        self.run()
            .wait_for(&format!("{count} failed runs"), |shown| {
                failures(shown) >= count
            });
    }

    /// Waits for its run to exit; gives what it showed.
    fn finish(&mut self) -> Shown {
        self.run.take().expect("the new machine's run").finish()
    }
}

/// `code`, a shown code, with its last character changed: the same pairing, another code.
fn mistyped(code: &str) -> String {
    let (head, last) = code.split_at(code.len() - 1);
    let at = ALPHABET.find(last).expect("a code of the alphabet");
    let changed = ALPHABET.as_bytes()[(at + 1) % ALPHABET.len()] as char;
    format!("{head}{changed}")
}

/// The characters of `code` without its `-`.
fn plain(code: &str) -> String {
    code.chars().filter(|c| *c != '-').collect()
}

/// `skald link` on `first`'s machine after 3 failed runs in a row: confirms the new device shows a
/// new code, types `entry` and confirms the new device named `name`. Gives what it showed.
fn link_confirmed(first: &Person, entry: &str, name: &str) -> Shown {
    let mut run = Terminal::start(&first.machine, &[LINK]);
    run.wait_to_show(NEW_CODE);
    run.type_line(YES);
    run.wait_to_show(OWN_DEVICE);
    run.type_line(entry);
    run.wait_for("the new device's name", |shown| shown.contains(name));
    run.type_line(YES);
    run.finish()
}

/// Ada's linked device runs a mistyped code of her new machine: the run fails at both. The new
/// machine shows its code again with a new pairing and the same own 6 characters; her linked
/// device's `skald link` takes the code shown and links it.
#[test]
fn after_a_mistyped_code_the_code_shown_links() {
    let (world, _front) = fronted_with(|_| {});
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_951_001, "ada-mistypes-a-long-code", "ada-workstation");
    let mut new = Fresh::start(cloud, Machine::new().with_host_name(LAPTOP));
    let code = new.code();

    link_failing(&ada, &typed(&mistyped(&code)));
    new.wait_for_failures(1);
    let shown = new.next_code(&code);
    assert_eq!(plain(&shown)[6..], plain(&code)[6..], "the code's own part");

    let linked = link_with(&ada, &typed(&shown), LAPTOP, YES);
    assert!(linked.success, "skald link failed: {}", linked.text);
    let done = new.finish();
    assert!(done.success, "the new machine failed: {}", done.text);
}

/// Three runs of mistyped codes fail: the new machine then says the code failed 3 times and that
/// someone else may have tried, and opens no run and shows no code until its person asks. Asked,
/// it shows a fresh code, which links.
#[test]
fn three_failed_runs_end_the_code_and_a_fresh_one_links() {
    let (world, _front) = fronted_with(|_| {});
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_951_011, "ada-mistypes-thrice", "ada-workstation");
    let mut new = Fresh::start(cloud, Machine::new().with_host_name(LAPTOP));
    let mut code = new.code();

    for failed in 1..=3 {
        link_failing(&ada, &typed(&mistyped(&code)));
        new.wait_for_failures(failed);
        if failed < 3 {
            code = new.next_code(&code);
        }
    }
    new.run().wait_for("that the code failed 3 times", |shown| {
        says(shown, &THREE_TIMES)
    });
    thread::sleep(QUIET);
    assert_eq!(cloud.count_sent(new.started, OPEN), 3, "a run unasked");
    assert_eq!(new.code(), code, "a code unasked");

    new.run().enter();
    let fresh = new.next_code(&code);
    let linked = link_confirmed(&ada, &typed(&fresh), LAPTOP);
    assert!(linked.success, "skald link failed: {}", linked.text);
    let done = new.finish();
    assert!(done.success, "the new machine failed: {}", done.text);
}

/// The new machine's code lives 30 seconds here: it then says the code expired, and the linked
/// device's run with that code fails. Asked, it shows a fresh code, which links.
#[test]
fn a_code_expires_and_a_fresh_one_links() {
    let (world, _front) = fronted_with(|_| {});
    let cloud = &world.cloud;
    let ada = world.sign_up(
        4_175_951_021,
        "ada-lets-a-long-code-expire",
        "ada-workstation",
    );
    let machine = Machine::new()
        .with_host_name(LAPTOP)
        .with_env(LIFETIME_SETTING, "30");
    let mut new = Fresh::start(cloud, machine);
    let code = new.code();

    new.run()
        .wait_for("that the code expired", |shown| says(shown, &[EXPIRED]));
    link_failing(&ada, &typed(&code));

    new.run().enter();
    let fresh = new.next_code(&code);
    let linked = link_with(&ada, &typed(&fresh), LAPTOP, YES);
    assert!(linked.success, "skald link failed: {}", linked.text);
    let done = new.finish();
    assert!(done.success, "the new machine failed: {}", done.text);
}

/// Ada's account has a valid `browser` device. The played issuing device grants her new machine an
/// honest certificate with a link object the browser signed as its issuer. The server stores the
/// certificate and gives a session by the device's key, but the account's device list does not
/// come: the run fails, storing nothing, and `skald devices` next stores no certificate either.
/// With the list back, the next `skald sign-in` fails the check: it says so, and the session no
/// longer works. No run stored the certificate or said the device is linked.
#[test]
fn a_grant_that_fails_the_check_is_dropped_and_never_stored() {
    let (world, _front) = fronted_with(|_| {});
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_951_031, "ada-gets-a-browser-link", "ada-workstation");
    let browser = add_device_of_kind(
        cloud,
        (ada.account, DeviceKind::Browser),
        "ada-browser",
        &Issuer::Device(&ada),
    );
    let mut new = Fresh::start(cloud, Machine::new().with_host_name(LAPTOP));

    let paired = answer_paired(cloud, &ada, &new.code());
    let mut granted = Granted::honest(cloud, &ada, &paired);
    granted.link.issuer = browser.id;
    granted.link_signer = browser.signing;
    cloud.relay.refuse(LIST, "503 Service Unavailable");
    granted.send(cloud, &ada, &paired);
    let first = new.finish();
    assert!(!first.success, "the first run linked: {}", first.text);
    let token = strings_named(&cloud.relay.answered_since(new.started), "token")
        .pop()
        .expect("a session by the device's key");
    assert_eq!(cloud.session_of(&token).0, 200, "no session by its key");

    let next = run(&new.machine, &[DEVICES]);
    cloud.relay.stop_refusing();
    let again = Terminal::start(&new.machine, &[SIGN_IN, SERVER, &cloud.url]).finish();
    assert!(!again.success, "the next sign-in linked: {}", again.text);
    assert!(says(&again.text, &["checks"]), "{}", again.text);
    assert_eq!(cloud.session_of(&token).0, 401, "the session still works");
    assert_eq!(
        cloud.count_sent(new.started, STORE_CERT),
        0,
        "a certificate stored"
    );
    for (what, text) in [
        ("the first run", &first.text),
        ("skald devices", &next.shown()),
        ("the next sign-in", &again.text),
    ] {
        assert!(!says(text, &["linked by"]), "{what}: {text}");
    }
}
