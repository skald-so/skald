//! The recovery key of 12 characters at the person's terminal (`docs/protocol/membership.md`,
//! "Account"): the account's first device shows it as 3 groups of 4 characters of Crockford's
//! base32, and a new machine links with it and reads the history through it, the key typed or
//! pasted into the hidden input: in any case, without `-`, as a bracketed paste, after a blank line
//! or with Windows line ends. Nothing echoes it, and a wrong key is refused. Against a
//! `skald-server` with GitHub sign-in through the stand-in, behind the recording relay of the
//! sign-in tests; the person's terminal is a pseudo-terminal, or a pseudo-console on Windows, and a
//! test plays the person in headless Chrome.
//!
//! As the tests of linking with the recovery key (`recovery_linking/main.rs`) and of the history
//! through it (`recovery_history/main.rs`), with their CLI: Ada's first machine signs up at a
//! terminal and shows her key; her new machine signs in with `--first-device no
//! --with-recovery-key`, naming the first device lost, and she types the key once it asks for it.
//! What a run shows is read here apart from the helpers those tests share: a key in its shown form
//! is `XXXX-XXXX-XXXX` in the upper-case alphabet [`ALPHABET`], its groups joined by `-`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod history;
mod pasted;
mod shown;

// The modules the tests of the channel history share (`history_common/mod.rs`), the played
// devices' lines of the tests of the history on new devices, the helpers of the tests of reading
// the history, and the terminal, signing in there and linking with the recovery key of the tests
// of the recovery key and of linking with it.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../recovery_linking/recovering.rs"]
mod recovering;
#[path = "../recovery_key/terminal.rs"]
mod terminal;

use skald_crypto::check_valid_device;
use skald_proto::DeviceId;

use at_terminal::first_at_terminal;
use cloud::Cloud;
use machine::Machine;
use recovering::{LOST, NEW_HOST, played};
use revocations::{account_certs, account_revocations};
use terminal::Shown;
use world::{Person, World};

/// Crockford's base32 alphabet, in the order of its values.
pub const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A terminal's bracketed-paste markers, around what is pasted.
pub const PASTE_START: &str = "\x1b[200~";
pub const PASTE_END: &str = "\x1b[201~";

/// An account made by its first machine at a terminal: its person, and the recovery key it showed,
/// in its shown form.
pub struct SignedUp {
    pub person: Person,
    pub key: String,
}

/// Whether `text` is a key in its shown form: 3 groups of 4 characters of [`ALPHABET`] joined by
/// `-`.
pub fn in_shown_form(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.len() == 14
        && chars.iter().enumerate().all(|(at, c)| match at {
            4 | 9 => *c == '-',
            _ => ALPHABET.contains(*c),
        })
}

/// Whether `c` may be part of a key's text: an ASCII letter or digit, or `-`.
fn keyish(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// The runs of letters, digits and `-` in `text`, each whole.
fn runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !keyish(c))
        .filter(|run| !run.is_empty())
}

/// Every key in its shown form in `text`, each once, in order: a run of letters, digits and `-`
/// that is one.
pub fn shown_keys(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for run in runs(text).filter(|run| in_shown_form(run)) {
        if !found.iter().any(|key| key == run) {
            found.push(run.to_owned());
        }
    }
    found
}

/// Whether `text` shows a longer key: a run of 4 or more groups of 4 letters and digits joined by
/// `-`, as the 13 groups of a 32-byte key in base32.
pub fn shows_a_longer_key(text: &str) -> bool {
    runs(text).any(|run| {
        let groups: Vec<&str> = run.split('-').collect();
        groups.len() >= 4 && groups.iter().all(|group| group.len() == 4)
    })
}

/// The letters and digits of `text` in upper case, with O read as 0 and I and L as 1, as the input
/// of a key reads them.
fn squeezed(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            upper => upper,
        })
        .collect()
}

/// Whether `text` shows the key `key`, in its shown form, in any case, with or without spaces and
/// `-`, with O, I or L for 0 and 1.
pub fn shows_key(text: &str, key: &str) -> bool {
    squeezed(text).contains(&squeezed(key))
}

/// The 60 bits of the key `key`, in its shown form.
fn key_value(key: &str) -> u64 {
    key.chars()
        .filter(|c| *c != '-')
        .map(|c| ALPHABET.find(c).expect("a character of the alphabet") as u64)
        .fold(0, |value, digit| (value << 5) | digit)
}

/// Whether `bytes`, what a machine or the server keeps or sends, hold the key `key`: as text, as
/// [`shows_key`] finds it, or its 60 bits in 8 bytes, in any of the encodings of the device keys'
/// tests.
pub fn holds_key(bytes: &[u8], key: &str) -> bool {
    let value = key_value(key);
    shows_key(&String::from_utf8_lossy(bytes), key)
        || [
            value.to_be_bytes(),
            value.to_le_bytes(),
            (value << 4).to_be_bytes(),
        ]
        .iter()
        .any(|form| keys::holds(bytes, form))
}

/// GitHub user `id` with `login` signs up on a new machine with the host name `host` at a
/// terminal: approves its code, and presses Enter once it shows a recovery key. Fails unless the
/// run succeeds and shows one key, of 3 groups of 4, and no longer one.
#[track_caller]
pub fn signs_up(world: &World, (id, login): (u64, &str), host: &'static str) -> SignedUp {
    let (cloud, machine) = (&world.cloud, Machine::new().with_host_name(host));
    let user = cloud.user(id, login);
    let mut run = first_at_terminal(cloud, &world.browser.page(), &machine, &user, &[]);
    run.wait_for("a recovery key", |text| {
        !shown_keys(text).is_empty() || shows_a_longer_key(text)
    });
    run.enter();
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    assert!(
        !shows_a_longer_key(&finished.text),
        "a key longer than 3 groups of 4 is shown: {finished:?}"
    );
    let keys = shown_keys(&finished.text);
    assert_eq!(keys.len(), 1, "keys of 3 groups of 4 shown: {finished:?}");
    SignedUp {
        person: world.person(user, machine, host),
        key: keys[0].clone(),
    }
}

/// The person of `ada`'s first machine starts linking `machine` with the recovery key at a
/// terminal, naming that device lost, and gives `input`, typed or pasted, once it asks for the
/// key; gives what the run showed.
#[track_caller]
pub fn links_on(world: &World, ada: &Person, machine: &Machine, input: &str) -> Shown {
    let named = [(LOST, ada.device)];
    played(world, (&ada.user, Some(input)), machine, &named, || false).finish()
}

/// As [`links_on`], on a new machine; gives it and what the run showed.
#[track_caller]
pub fn links_with(world: &World, ada: &Person, input: &str) -> (Machine, Shown) {
    let machine = Machine::new().with_host_name(NEW_HOST);
    let shown = links_on(world, ada, &machine, input);
    (machine, shown)
}

/// The devices with a certificate of the account of the session `token`.
pub fn certified_devices(cloud: &Cloud, token: &str) -> Vec<DeviceId> {
    cloud::certified(&cloud.devices(token))
}

/// Fails unless `shown`, the run that linked a new machine of `ada`'s account with the recovery key
/// `key`, succeeded without showing the key, and the server holds the new device's certificate,
/// issued by the master key, valid under the master key `ada`'s first machine pinned. Gives the new
/// device.
#[track_caller]
pub fn assert_linked(cloud: &Cloud, ada: &Person, key: &str, shown: &Shown) -> DeviceId {
    assert!(
        shown.success,
        "linking with the recovery key failed: {shown:?}"
    );
    assert!(!shows_key(&shown.text, key), "the key is echoed: {shown:?}");
    let new = cloud.new_device(&ada.token, &[ada.device]);
    let cert = cloud
        .cert(&ada.token, new)
        .expect("the new device's certificate");
    assert_eq!(cert.body().issuer, None, "its issuer: {:?}", cert.body());
    let certs = account_certs(cloud, &ada.token);
    let revocations = account_revocations(cloud, &ada.token);
    let checked = check_valid_device(&ada.master, &certs, &revocations, new);
    assert!(checked.is_ok(), "the new device is not valid: {checked:?}");
    new
}
