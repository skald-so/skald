//! KeyPackages as `docs/protocol/membership.md`, "KeyPackages", says, and the checks before an
//! Add. The time is the caller's, in seconds since the Unix epoch: each step that depends on it
//! takes it. A KeyPackage's lifetime runs 60 days from an hour before the time it is made at; a
//! device adds a KeyPackage only if its MLS signature verifies, its lifetime holds at the caller's
//! time and spans at most 60 days, its ciphersuite is `0x0001`, and its `identity` and
//! `signature_key` match the certificate of a device the caller gives as admitted ("Admitted
//! devices"). A KeyPackage's private keys go once it expires at the caller's time; each KeyPackage
//! but the last-resort one adds its device once.

use std::time::Duration;

use mls_rs::{
    CipherSuite, ExtensionList, MlsMessage, ProtocolVersion,
    crypto::HpkePublicKey,
    group::{LeafNode, LeafNodeSource},
    mls_rs_codec::{MlsDecode, MlsEncode},
    time::MlsTime,
};
use skald_crypto::{KeyPackage, MlsDevice, MlsError, SealedState, issue};
use skald_proto::{AccountId, Cert, CertBody, ChannelId, DeviceKind, Unknown};

use crate::fixtures::{device_id, load, new_state, signing_key};
use crate::foreign;
use crate::steps::{bytes, step};

const HOUR: u64 = 3_600;
const DAY: u64 = 24 * HOUR;
/// The longest lifetime of a KeyPackage, and that of each KeyPackage a device makes.
const LIFETIME: u64 = 60 * DAY;
/// The caller's time the tests start from, far from the machine's clock, so a step that reads
/// that clock instead fails.
const T: u64 = 1_900_000_000;
/// The seed of the key that issues the tests' certificates.
const MASTER: u8 = 90;

/// A certificate of account `[seed; 16]` for device ID `device` with the `signing` key, as the
/// caller gives an admitted device's valid certificate.
fn cert_with(seed: u8, device: u8, signing: [u8; 32]) -> Cert {
    let body = CertBody {
        account: AccountId::from_bytes([seed; 16]),
        device: device_id(device),
        kind: DeviceKind::Client,
        signing,
        hpke: [0; 32],
        issuer: None,
        issued: 0,
        ssh: None,
        name: "Test".to_owned(),
        unknown: Unknown::default(),
    };
    issue(&body, &signing_key(MASTER), &signing_key(seed)).expect("a certificate")
}

/// The certificate of device `seed` with its own key.
fn cert(seed: u8) -> Cert {
    cert_with(seed, seed, signing_key(seed).public_key())
}

/// A KeyPackage device `seed` makes at `now`.
fn key_package(seed: u8, state: &mut SealedState, now: u64) -> KeyPackage {
    step(seed, state, |device| {
        device.key_package(now).expect("a KeyPackage")
    })
}

/// The last-resort KeyPackage device `seed` makes at `now`.
fn last_resort(seed: u8, state: &mut SealedState, now: u64) -> KeyPackage {
    step(seed, state, |device| {
        device
            .last_resort_key_package(now)
            .expect("a last-resort KeyPackage")
    })
}

/// A new device `seed` that creates `channel`'s group at `now`.
fn creator(seed: u8, channel: ChannelId, now: u64) -> SealedState {
    let mut state = new_state(seed);
    step(seed, &mut state, |device| {
        device.create_group(channel, now).expect("a new group");
    });
    state
}

/// Device `seed` adds the devices of `key_packages` to `channel`'s group at `now`, `admitted` the
/// certificates the caller gives; gives the Commit, or why it is refused.
fn try_add(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    key_packages: &[&[u8]],
    admitted: &[Cert],
    now: u64,
) -> Result<Vec<u8>, MlsError> {
    step(seed, state, |device| {
        device
            .add_devices(channel, key_packages, admitted, now)
            .map(bytes)
    })
}

/// Device `seed` adds the devices of `key_packages` at `now`, and the server stores the Commit;
/// gives its Welcome.
fn add_and_store(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    key_packages: &[&[u8]],
    admitted: &[Cert],
    now: u64,
) -> Vec<u8> {
    try_add(seed, state, channel, key_packages, admitted, now).expect("a Commit of the Adds");
    step(seed, state, |device| {
        bytes(
            device
                .commit_stored(channel)
                .expect("the stored Commit merges")
                .expect("the Welcome of a Commit with Adds"),
        )
    })
}

/// Device `seed` joins from `welcome` at `now`, the caller's checks passing.
fn try_join(
    seed: u8,
    state: &mut SealedState,
    welcome: &[u8],
    now: u64,
) -> Result<ChannelId, MlsError> {
    step(seed, state, |device: &mut MlsDevice| {
        device.join(welcome, now, |_| true)
    })
}

/// A refused Add left `channel`'s group of device `seed` as it was: epoch 0, the device its one
/// member, no history key of epoch 1 and no Commit pending.
fn assert_unchanged(seed: u8, state: &mut SealedState, channel: ChannelId, what: &str) {
    let device = load(seed, state);
    assert_eq!(device.epoch(channel).expect("the group"), 0, "{what}");
    assert_eq!(
        device.members(channel).expect("the group").len(),
        1,
        "{what}"
    );
    assert!(device.history_key(channel, 1).is_none(), "{what}");
    assert!(
        step(seed, state, |device| device
            .commit_stored(channel)
            .map(|_| ()))
        .is_err(),
        "{what}: no Commit is pending"
    );
}

/// The `not_before` and `not_after` of the lifetime in `message`, a KeyPackage in an `MLSMessage`,
/// as any MLS implementation reads it.
fn lifetime_of(message: &[u8]) -> (u64, u64) {
    let package = MlsMessage::from_bytes(message)
        .expect("an MLSMessage")
        .into_key_package()
        .expect("a KeyPackage");
    // KeyPackage: version, cipher_suite, init_key, leaf_node, …
    let encoded = package.mls_encode_to_vec().expect("the KeyPackage's bytes");
    let reader = &mut encoded.as_slice();
    ProtocolVersion::mls_decode(reader).expect("the version");
    CipherSuite::mls_decode(reader).expect("the ciphersuite");
    HpkePublicKey::mls_decode(reader).expect("the init key");
    let leaf = LeafNode::mls_decode(reader).expect("the leaf node");
    let LeafNodeSource::KeyPackage(lifetime) = leaf.leaf_node_source else {
        panic!("a KeyPackage's leaf has a lifetime");
    };
    let not_after = lifetime.not_after.seconds_since_epoch();
    assert_eq!(
        package
            .expiration()
            .expect("the expiry")
            .seconds_since_epoch(),
        not_after
    );
    (lifetime.not_before.seconds_since_epoch(), not_after)
}

/// A KeyPackage, in an `MLSMessage`, of an MLS client outside the crate under test: device
/// `seed`'s key and device ID, ciphersuite `suite`, with mls-rs's lifetime from `now` to `now`
/// plus `ahead`.
fn foreign_key_package(seed: u8, suite: CipherSuite, ahead: u64, now: u64) -> Vec<u8> {
    foreign::client(
        seed,
        device_id(seed).as_bytes(),
        suite,
        Duration::from_secs(ahead),
    )
    .generate_key_package_message(
        ExtensionList::new(),
        ExtensionList::new(),
        Some(MlsTime::from(now)),
    )
    .and_then(|message| message.to_bytes())
    .expect("a KeyPackage of mls-rs")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_new_key_package_lives_sixty_days_from_an_hour_before_the_callers_time() {
    let mut state = new_state(40);
    for now in [T, T + 17 * DAY + 5] {
        for (package, what) in [
            (key_package(40, &mut state, now), "a KeyPackage"),
            (
                last_resort(40, &mut state, now),
                "the last-resort KeyPackage",
            ),
        ] {
            let (not_before, not_after) = lifetime_of(&package.message);
            assert_eq!(not_before, now - HOUR, "{what} made at {now}");
            assert_eq!(not_after, now - HOUR + LIFETIME, "{what} made at {now}");
            assert_eq!(
                package.expires, not_after,
                "{what}: the caller learns when it expires"
            );
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_valid_key_package_of_an_admitted_device_is_added() {
    let channel = ChannelId::generate();
    let mut joiner = new_state(42);
    let package = key_package(42, &mut joiner, T);
    let mut state = creator(41, channel, T + DAY);

    let welcome = add_and_store(
        41,
        &mut state,
        channel,
        &[&package.message],
        &[cert(43), cert(42)],
        T + DAY,
    );
    let members = load(41, &state).members(channel).expect("the group");
    let got: Vec<_> = members
        .iter()
        .map(|member| (member.device, member.signing))
        .collect();
    assert_eq!(
        got,
        [
            (device_id(41), signing_key(41).public_key()),
            (device_id(42), signing_key(42).public_key()),
        ]
    );
    assert_eq!(
        try_join(42, &mut joiner, &welcome, T + DAY).expect("the device joins"),
        channel
    );
    assert_eq!(load(42, &joiner).epoch(channel).expect("the group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_the_caller_does_not_give_as_admitted_is_not_added() {
    let channel = ChannelId::generate();
    let package = key_package(45, &mut new_state(45), T).message;
    let other = key_package(46, &mut new_state(46), T).message;
    let mut state = creator(44, channel, T);

    // The KeyPackages of one Add, the certificates the caller gives, and what the case is.
    let cases: [(&[&[u8]], Vec<Cert>, _); 5] = [
        (&[&package], vec![], "no device admitted"),
        (&[&package], vec![cert(46)], "another device admitted"),
        (
            &[&other, &package],
            vec![cert(46)],
            "one admitted device and one not, in one Add",
        ),
        (
            &[&package],
            vec![cert_with(45, 45, signing_key(47).public_key())],
            "the device's ID admitted with another signing key",
        ),
        (
            &[&package],
            vec![cert_with(45, 47, signing_key(45).public_key())],
            "the device's signing key admitted with another device ID",
        ),
    ];
    for (packages, admitted, what) in cases {
        assert!(
            try_add(44, &mut state, channel, packages, &admitted, T).is_err(),
            "refused: {what}"
        );
        assert_unchanged(44, &mut state, channel, what);
    }

    add_and_store(44, &mut state, channel, &[&package], &[cert(45)], T);
    assert_eq!(
        load(44, &state).members(channel).expect("the group")[1].device,
        device_id(45),
        "added once the caller gives it as admitted"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_package_whose_signature_does_not_verify_is_not_added() {
    let channel = ChannelId::generate();
    let package = key_package(49, &mut new_state(49), T).message;
    let mut state = creator(48, channel, T);

    // The last bytes of a KeyPackage are its signature.
    for at in [1, 2, 64] {
        let mut altered = package.clone();
        let index = altered.len() - at;
        altered[index] ^= 0x40;
        assert!(
            try_add(48, &mut state, channel, &[&altered], &[cert(49)], T).is_err(),
            "refused: the signature's byte {at} from the end altered"
        );
        assert_unchanged(48, &mut state, channel, "an altered signature");
    }

    add_and_store(48, &mut state, channel, &[&package], &[cert(49)], T);
    assert_eq!(load(48, &state).epoch(channel).expect("the group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_package_whose_lifetime_does_not_hold_at_the_callers_time_is_not_added() {
    let channel = ChannelId::generate();
    let mut state = creator(50, channel, T);

    // Made two hours after the caller's time: it holds from an hour after it.
    let early = key_package(51, &mut new_state(51), T + 2 * HOUR).message;
    // Made 61 days before: it expired a day and an hour before.
    let late = key_package(52, &mut new_state(52), T - 61 * DAY).message;
    for (package, what) in [(&early, "not yet valid"), (&late, "expired")] {
        assert!(
            try_add(
                50,
                &mut state,
                channel,
                &[package],
                &[cert(51), cert(52)],
                T
            )
            .is_err(),
            "refused: {what}"
        );
        assert_unchanged(50, &mut state, channel, what);
    }

    // Made 59 days before: it holds for another day less an hour.
    let held = key_package(53, &mut new_state(53), T - 59 * DAY).message;
    add_and_store(50, &mut state, channel, &[&held], &[cert(53)], T);
    assert_eq!(load(50, &state).epoch(channel).expect("the group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_package_whose_lifetime_spans_more_than_sixty_days_is_not_added() {
    let channel = ChannelId::generate();
    let mut state = creator(54, channel, T);
    let suite = CipherSuite::CURVE25519_AES128;

    for (ahead, what) in [
        (LIFETIME + 1, "60 days and a second"),
        (365 * DAY, "365 days"),
    ] {
        let package = foreign_key_package(55, suite, ahead, T);
        assert_eq!(lifetime_of(&package), (T, T + ahead));
        assert!(
            try_add(54, &mut state, channel, &[&package], &[cert(55)], T).is_err(),
            "refused: a lifetime of {what}"
        );
        assert_unchanged(54, &mut state, channel, what);
    }

    let package = foreign_key_package(55, suite, LIFETIME, T);
    assert_eq!(lifetime_of(&package), (T, T + LIFETIME));
    add_and_store(54, &mut state, channel, &[&package], &[cert(55)], T);
    assert_eq!(
        load(54, &state).members(channel).expect("the group")[1].device,
        device_id(55),
        "a lifetime of 60 days is added"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_package_of_another_ciphersuite_is_not_added() {
    let channel = ChannelId::generate();
    let mut state = creator(56, channel, T);

    // MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519, 0x0003: the same Ed25519 key.
    let package = foreign_key_package(57, CipherSuite::CURVE25519_CHACHA, LIFETIME, T);
    assert!(
        try_add(56, &mut state, channel, &[&package], &[cert(57)], T).is_err(),
        "refused: ciphersuite 0x0003"
    );
    assert_unchanged(56, &mut state, channel, "ciphersuite 0x0003");

    let package = foreign_key_package(57, CipherSuite::CURVE25519_AES128, LIFETIME, T);
    add_and_store(56, &mut state, channel, &[&package], &[cert(57)], T);
    assert_eq!(load(56, &state).epoch(channel).expect("the group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_last_resort_key_package_adds_its_device_to_two_groups() {
    let (first, second) = (ChannelId::generate(), ChannelId::generate());
    let mut joiner = new_state(60);
    let package = last_resort(60, &mut joiner, T).message;

    // The server hands the same last-resort KeyPackage to two devices.
    for (seed, channel, now) in [(58, first, T + DAY), (59, second, T + 2 * DAY)] {
        let mut state = creator(seed, channel, now);
        let welcome = add_and_store(seed, &mut state, channel, &[&package], &[cert(60)], now);
        assert_eq!(
            try_join(60, &mut joiner, &welcome, now + HOUR).expect("the device joins"),
            channel
        );
    }
    let device = load(60, &joiner);
    for channel in [first, second] {
        assert_eq!(device.epoch(channel).expect("both groups"), 1);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_package_other_than_the_last_resort_one_adds_its_device_once() {
    let (first, second) = (ChannelId::generate(), ChannelId::generate());
    let mut joiner = new_state(63);
    last_resort(63, &mut joiner, T);
    let package = key_package(63, &mut joiner, T).message;

    let welcomes: Vec<Vec<u8>> = [(61, first), (62, second)]
        .into_iter()
        .map(|(seed, channel)| {
            let mut state = creator(seed, channel, T);
            add_and_store(seed, &mut state, channel, &[&package], &[cert(63)], T)
        })
        .collect();
    assert_eq!(
        try_join(63, &mut joiner, &welcomes[0], T + HOUR).expect("the first use joins"),
        first
    );
    assert!(
        try_join(63, &mut joiner, &welcomes[1], T + HOUR).is_err(),
        "a second use of the KeyPackage fails"
    );
    let device = load(63, &joiner);
    assert!(device.epoch(second).is_err(), "no group of the second");
    assert_eq!(device.epoch(first).expect("the first group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_welcome_to_a_replaced_but_unexpired_key_package_joins() {
    let channel = ChannelId::generate();
    let mut joiner = new_state(65);
    let package = key_package(65, &mut joiner, T);
    // The device replaces it 7 days before it expires; the server can still hand it out.
    let replacement = key_package(65, &mut joiner, package.expires - 7 * DAY);
    assert!(replacement.expires > package.expires);

    let now = package.expires - DAY;
    let mut state = creator(64, channel, now);
    let welcome = add_and_store(
        64,
        &mut state,
        channel,
        &[&package.message],
        &[cert(65)],
        now,
    );
    // The Welcome waited in the mailbox.
    assert_eq!(
        try_join(65, &mut joiner, &welcome, package.expires - HOUR).expect("the device joins"),
        channel
    );
    assert_eq!(load(65, &joiner).epoch(channel).expect("the group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_welcome_to_an_expired_key_package_fails_and_leaves_no_group() {
    let channel = ChannelId::generate();
    let mut joiner = new_state(67);
    let package = key_package(67, &mut joiner, T);
    let now = package.expires - DAY;
    let mut state = creator(66, channel, now);
    let welcome = add_and_store(
        66,
        &mut state,
        channel,
        &[&package.message],
        &[cert(67)],
        now,
    );

    assert!(
        try_join(67, &mut joiner, &welcome, package.expires + DAY).is_err(),
        "the KeyPackage expired at the caller's time"
    );
    assert!(load(67, &joiner).epoch(channel).is_err(), "no group");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_expired_key_packages_private_keys_are_gone_once_a_step_runs_past_its_expiry() {
    let channel = ChannelId::generate();
    let mut joiner = new_state(69);
    let package = key_package(69, &mut joiner, T);
    let mut state = creator(68, channel, T + DAY);
    let welcome = add_and_store(
        68,
        &mut state,
        channel,
        &[&package.message],
        &[cert(69)],
        T + DAY,
    );
    let before = joiner.clone();

    // The device makes a KeyPackage at a time past the first one's expiry: that one's private
    // keys go, so the Welcome fails even at a time it would hold.
    key_package(69, &mut joiner, package.expires + DAY);
    assert!(
        try_join(69, &mut joiner, &welcome, T + 2 * DAY).is_err(),
        "the expired KeyPackage's private keys are deleted"
    );
    assert!(load(69, &joiner).epoch(channel).is_err(), "no group");

    let mut kept = before;
    assert_eq!(
        try_join(69, &mut kept, &welcome, T + 2 * DAY).expect("the keys before the expiry join"),
        channel
    );
}
