//! What an offline guess at a recovery key costs someone who holds the server's database: Argon2id
//! with the spec's parameters, set for about a second on a laptop (`docs/protocol/membership.md`,
//! "Account"), in every build but a test build. A test build, of the tests of the crates whose
//! dev-dependencies turn on skald-crypto's feature `test-cost`, derives at the least cost until
//! `derive_at_spec_cost`; cargo resolves no other build of the workspace with it, and a release
//! build with it does not compile. Natively only: WASM has no clock these tests can read, so this
//! target is the client's.
//!
//! The API, as these tests use it: `skald_crypto::new_recovery` and `open_recovery`, as the tests
//! of master backups use them (`crates/crypto/tests/recovery_backups.rs`), and
//! `derive_at_spec_cost()`.

use std::collections::BTreeSet;
use std::process::Command;
use std::time::{Duration, Instant};

use skald_crypto::{
    RecoveryKey, SigningKey, derive_at_spec_cost, new_recovery, open_recovery, sign_account,
};
use skald_proto::AccountId;

/// The least an offline guess may take: a fifth of the spec's second on a laptop, so a fast
/// machine passes too.
const LEAST: Duration = Duration::from_millis(200);

/// With an account's record and master backup, as the server's database holds them, each wrong
/// guess at its recovery key takes at least [`LEAST`] before it is refused.
#[test]
fn each_offline_guess_at_a_recovery_key_takes_at_least_a_fifth_of_a_second() {
    derive_at_spec_cost();
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

/// The features cargo resolves skald-crypto with, on every target, for `edges` of `cargo tree` from
/// every package of the workspace.
fn crypto_features(edges: &str) -> BTreeSet<String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "tree",
            "--workspace",
            "--locked",
            "--target",
            "all",
            "--edges",
            edges,
        ])
        .args(["--invert", "skald-crypto", "--prefix", "none"])
        .output()
        .expect("cargo tree runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "cargo tree failed: {}{stdout}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("skald-crypto feature \""))
        .filter_map(|rest| rest.split_once('"'))
        .map(|(feature, _)| feature.to_owned())
        .collect()
}

/// No build of a package of the workspace without its tests derives at a test build's cost: cargo
/// resolves skald-crypto without `test-cost` for them, the release binaries, the server's image
/// and `cargo install` among them; only the dev-dependencies of tests turn it on.
#[test]
fn only_a_test_build_derives_at_less_than_the_spec_cost() {
    let shipped = crypto_features("features,normal,build");
    assert!(shipped.contains("default"), "no features read: {shipped:?}");
    assert!(
        !shipped.contains("test-cost"),
        "a build without tests turns on test-cost: {shipped:?}"
    );
    let tests = crypto_features("features,normal,build,dev");
    assert!(
        tests.contains("test-cost"),
        "the tests do not turn on test-cost: {tests:?}"
    );
}
