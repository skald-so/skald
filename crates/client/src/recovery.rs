//! The account's recovery key on its first device (`docs/protocol/membership.md`, "Account"): kept
//! in the secrets from its creation until its person has seen it, shown only by `skald sign-in` at
//! the person's terminal or on its local page, never in an agent's output; meanwhile every other
//! command says that it waits.

use std::io::{self, BufRead};
use std::path::Path;

use skald_crypto::RecoveryKey;
use zeroize::Zeroizing;

use crate::output::{self, Stream};
use crate::person::Person;
use crate::secrets::{self, Secrets};
use crate::sign_in;
use crate::state::{self, Link, State};

/// The secret that holds the recovery key until it is shown.
const RECOVERY_KEY: &str = "recovery_key";

/// Keeps `key` in `stored`, the secrets, and saves them in `secrets`.
pub fn keep(
    secrets: &Secrets,
    stored: &mut secrets::Map,
    key: &RecoveryKey,
) -> Result<(), secrets::Error> {
    stored.insert(RECOVERY_KEY.to_owned(), key.shown().as_str().to_owned());
    secrets.save(stored)
}

/// Deletes the recovery key from `stored`, the secrets, and saves them in `secrets`, if they hold
/// one.
pub fn forget(secrets: &Secrets, stored: &mut secrets::Map) -> Result<(), secrets::Error> {
    match stored.remove(RECOVERY_KEY) {
        Some(_) => secrets.save(stored),
        None => Ok(()),
    }
}

/// The recovery key `stored`, the secrets, hold, if they hold a readable one.
fn kept(stored: &secrets::Map) -> Option<RecoveryKey> {
    RecoveryKey::parse(stored.get(RECOVERY_KEY)?).ok()
}

/// The line that says a recovery key waits, with the command that shows it, on `server`. A
/// notice, not an error: no `skald:` before it.
fn waits(server: &str) -> String {
    format!(
        "A recovery key waits to be shown: run skald sign-in --server {server} at a terminal, or \
         add --browser to see it on a local page."
    )
}

/// What the person reads under the recovery key.
const KEEP_IT: &str = "Write it down and keep it apart from your devices: it is the only way back \
                       to the account and its history when no linked device is left.";

/// At the end of `skald sign-in` on `server`, the device in `state` holding its certificate: a
/// recovery key that waits in `secrets` is shown to `person`, if the run has one, and deleted once
/// the person says it is written down; otherwise the run says that it waits.
pub fn show(
    state: &State,
    secrets: &Secrets,
    server: &str,
    person: Option<Person<'_>>,
) -> Result<(), sign_in::Error> {
    if state.recovery_waits()?.is_none() {
        return Ok(());
    }
    if !matches!(
        state.device()?.map(|device| device.link),
        Some(Link::Certified { .. })
    ) {
        eprintln!("{}", waits(server));
        return Ok(());
    }
    let mut stored = secrets.load()?;
    let Some(key) = kept(&stored) else {
        // A run deleted it and stopped before it recorded so.
        state.set_recovery_waits(None)?;
        return Ok(());
    };
    let page = match person {
        None => {
            eprintln!("{}", waits(server));
            return Ok(());
        }
        Some(Person::Page(page)) => page,
        Some(Person::Terminal) => return at_terminal(state, secrets, &mut stored, &key),
    };
    page.show(Some(key.shown().as_str()))
        .map_err(sign_in::Error::Io)?;
    let question = format!(
        "The account's recovery key, shown only this once, is above. {KEEP_IT} Once it is written \
         down, press the button: this device then deletes it."
    );
    let answered = page.ask(&question, &["Written down"], None, true);
    let answered = answered.map_err(sign_in::Error::Io)?.recv().is_ok();
    page.show(None).map_err(sign_in::Error::Io)?;
    if answered {
        written_down(state, secrets, &mut stored)?;
    }
    Ok(())
}

/// Shows `key`, of `stored`, the secrets, at the person's terminal, and has it written down once
/// the person presses Enter.
fn at_terminal(
    state: &State,
    secrets: &Secrets,
    stored: &mut secrets::Map,
    key: &RecoveryKey,
) -> Result<(), sign_in::Error> {
    let shown = key.shown();
    let mut text = Zeroizing::new(String::from(
        "\nThe account's recovery key, shown only this once:\n\n    ",
    ));
    text.push_str(&shown);
    text.push_str("\n\n");
    text.push_str(KEEP_IT);
    text.push_str(" Press Enter once it is written down: ");
    output::write(Stream::Out, text.as_bytes())
        .map_err(|err| sign_in::Error::Io(format!("cannot show the recovery key: {err}")))?;
    let mut line = String::new();
    let read = io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|err| sign_in::Error::Io(format!("cannot read the terminal: {err}")))?;
    if read == 0 {
        println!();
        return Ok(());
    }
    written_down(state, secrets, stored)
}

/// Deletes the recovery key, which its person wrote down, from `stored`, the secrets.
fn written_down(
    state: &State,
    secrets: &Secrets,
    stored: &mut secrets::Map,
) -> Result<(), sign_in::Error> {
    forget(secrets, stored)?;
    state.set_recovery_waits(None)?;
    println!("The recovery key is deleted from this device.");
    Ok(())
}

/// What every command but `skald sign-in` says while a recovery key waits on the machine whose
/// state directory is `dir`: one line, never the key.
pub fn say_if_waiting(dir: &Path) {
    if !dir.join(state::FILE).is_file() {
        return;
    }
    let Ok(state) = State::open(dir) else {
        return;
    };
    if let Ok(Some(server)) = state.recovery_waits() {
        eprintln!("{}", waits(&server));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn recovery_key_is_kept_read_back_and_forgotten() {
        let dir = TestDir::new("recovery-kept");
        let secrets = Secrets::new(&dir, secrets::Storage::File);
        let mut stored = secrets::Map::from([("other".to_owned(), "kept".to_owned())]);
        let key = RecoveryKey::generate();
        keep(&secrets, &mut stored, &key).unwrap();
        let loaded = secrets.load().unwrap();
        assert_eq!(
            *kept(&loaded).expect("a key").shown(),
            *key.shown(),
            "the same key"
        );
        forget(&secrets, &mut stored).unwrap();
        let loaded = secrets.load().unwrap();
        assert!(kept(&loaded).is_none());
        assert_eq!(loaded.get("other").map(String::as_str), Some("kept"));
        let broken = secrets::Map::from([(RECOVERY_KEY.to_owned(), "AAAA".to_owned())]);
        assert!(kept(&broken).is_none(), "no key of 4 characters");
    }

    #[test]
    fn waiting_line_names_the_command_and_no_key() {
        let line = waits("https://skald.example");
        assert!(line.contains("recovery key"), "{line}");
        assert!(
            line.contains("skald sign-in --server https://skald.example"),
            "{line}"
        );
        assert!(!line.contains('\n'), "one line: {line}");
        assert!(!line.starts_with("skald:"), "no error: {line}");
    }
}
