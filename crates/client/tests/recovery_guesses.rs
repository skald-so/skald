//! What an offline guess at a recovery key costs someone who holds the server's database: Argon2id
//! with the spec's parameters, set for about a second on a laptop (`docs/protocol/membership.md`,
//! "Account"), in every build but a test build. A test build, of the tests of the crates whose
//! dev-dependencies turn on skald-crypto's feature `test-cost`, derives at the least cost but on a
//! thread while a `SpecCost` of it lives; cargo resolves no other build of the workspace with it, and a release
//! build with it does not compile. Natively only: WASM has no clock these tests can read, so this
//! target is the client's.
//!
//! The API, as these tests use it: `skald_crypto::new_recovery` and `open_recovery`, as the tests
//! of master backups use them (`crates/crypto/tests/recovery_backups.rs`), and
//! `SpecCost::enter()` and `derives_at_spec_cost()`.

use std::collections::BTreeSet;
use std::process::Command;
use std::sync::Barrier;
use std::time::{Duration, Instant};

use skald_crypto::{
    RecoveryKey, SigningKey, SpecCost, derives_at_spec_cost, new_recovery, open_recovery,
    sign_account,
};
use skald_proto::AccountId;

/// The least an offline guess may take: a fifth of the spec's second on a laptop, so a fast
/// machine passes too.
const LEAST: Duration = Duration::from_millis(200);

/// With an account's record and master backup, as the server's database holds them, each wrong
/// guess at its recovery key takes at least [`LEAST`] before it is refused.
#[test]
fn each_offline_guess_at_a_recovery_key_takes_at_least_a_fifth_of_a_second() {
    let _spec = SpecCost::enter();
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

/// A test build derives at the spec's cost only on the thread of a `SpecCost`, while it lives: a
/// backup a test made at a test build's cost opens at that cost while a test beside it in the same
/// process, as `cargo test` runs them, derives at the spec's, and after it.
#[test]
fn the_spec_cost_is_of_one_thread_while_its_scope_lives() {
    let master = SigningKey::generate();
    let recovery = RecoveryKey::generate();
    let account: AccountId = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e9"
        .parse()
        .expect("an account ID");
    assert!(
        !derives_at_spec_cost(),
        "a test build derives at the spec's cost"
    );
    let (body, backup) = new_recovery(&master, &recovery, account);
    let record = sign_account(&body, &master).expect("the record's body is valid");
    let (entered, opened) = (Barrier::new(2), Barrier::new(2));
    let (spec_body, spec_backup) = std::thread::scope(|threads| {
        // Nothing between the barriers panics, so a failed check cannot leave the other thread
        // waiting: each side's checks come after them.
        let beside = threads.spawn(|| {
            let _spec = SpecCost::enter();
            let at_spec = derives_at_spec_cost();
            entered.wait();
            let made = new_recovery(&master, &recovery, account);
            opened.wait();
            (at_spec, made)
        });
        entered.wait();
        let here_at_spec = derives_at_spec_cost();
        let here_opened = open_recovery(&record, &backup, &recovery);
        opened.wait();
        let (at_spec, made) = beside.join().expect("the thread at the spec's cost");
        assert!(
            at_spec,
            "a SpecCost did not set the spec's cost on its thread"
        );
        assert!(!here_at_spec, "another thread's scope reached this one");
        here_opened.expect("it opens at its own cost");
        made
    });
    assert_ne!(
        spec_body.recovery, body.recovery,
        "the spec's cost derived the same key as a test build's"
    );
    let spec_record = sign_account(&spec_body, &master).expect("the record's body is valid");
    assert!(
        open_recovery(&spec_record, &spec_backup, &recovery).is_err(),
        "a backup made at the spec's cost opened at a test build's"
    );
    {
        let _spec = SpecCost::enter();
        {
            let _again = SpecCost::enter();
        }
        assert!(derives_at_spec_cost(), "an inner scope ended the outer one");
        open_recovery(&spec_record, &spec_backup, &recovery).expect("it opens at the spec's cost");
    }
    assert!(!derives_at_spec_cost(), "the scope outlived its value");
    open_recovery(&record, &backup, &recovery).expect("it opens at its own cost after");
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
