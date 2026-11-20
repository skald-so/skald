//! A device linked with the recovery key keeps the history keys the wraps give it only sealed.

use std::fs;
use std::path::PathBuf;

use crate::chain::key_of;
use crate::devices::BOB;
use crate::keys::holds;
use crate::reading::{assert_history, shown_history};
use crate::{recovers, talked};

/// In the talk of [`talked`], Ada's first machine is wiped, and her new machine, linked with the
/// recovery key naming that device lost, shows with `skald history` the three lines. No file of the
/// new machine holds the key of epoch 1 or 2 that Bob's device derives, in any encoding.
#[test]
fn no_file_of_the_recovered_machine_holds_a_history_key_in_the_clear() {
    let (talk, key, said) = talked(
        &[4_177_707_001, 4_177_707_002],
        &["ada-keeps-keys-sealed", "bob-books-the-audit"],
    );

    let new = recovers(&talk, &key, &[talk.ada.device]);
    let shown = shown_history(&new.machine, talk.channel);

    assert_history(&shown, talk.channel, &said, &[]);
    let files = new.machine.files();
    for epoch in [1, 2] {
        let key = key_of(&talk.played[BOB], talk.channel, epoch).to_bytes();
        let clear: Vec<&PathBuf> = (files.iter())
            .filter(|file| fs::read(file).is_ok_and(|bytes| holds(&bytes, &*key)))
            .collect();
        assert_eq!(
            clear,
            Vec::<&PathBuf>::new(),
            "files of the new machine hold the key of epoch {epoch}"
        );
    }
}
