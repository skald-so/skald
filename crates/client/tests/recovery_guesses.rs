//! What an offline guess at a recovery key costs someone who holds the server's database: Argon2id
//! with the spec's parameters, set for about a second on a laptop (`docs/protocol/membership.md`,
//! "Account"), in every build, test builds included. Natively only: WASM has no clock these tests
//! can read, so this target is the client's.
//!
//! The API, as this test uses it: `skald_crypto::new_recovery` and `open_recovery`, as the tests of
//! master backups use them (`crates/crypto/tests/recovery_backups.rs`).

use std::time::{Duration, Instant};

use skald_crypto::{RecoveryKey, SigningKey, new_recovery, open_recovery, sign_account};
use skald_proto::AccountId;

/// The least an offline guess may take: a fifth of the spec's second on a laptop, so a fast
/// machine passes too.
const LEAST: Duration = Duration::from_millis(200);

/// With an account's record and master backup, as the server's database holds them, each wrong
/// guess at its recovery key takes at least [`LEAST`] before it is refused.
#[test]
fn each_offline_guess_at_a_recovery_key_takes_at_least_a_fifth_of_a_second() {
    let master = SigningKey::generate();
    let account: AccountId = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e9"
        .parse()
        .expect("an account ID");
    let (body, backup) = new_recovery(&master, &RecoveryKey::generate(), account);
    let record = sign_account(&body, &master).expect("the record's body is valid");

    for _ in 0..2 {
        let guess = RecoveryKey::generate();
        let started = Instant::now();
        let opened = open_recovery(&record, &backup, &guess);
        let took = started.elapsed();

        assert!(opened.is_err(), "a guess opened the backup");
        assert!(took >= LEAST, "a guess took {took:?}, less than {LEAST:?}");
    }
}
