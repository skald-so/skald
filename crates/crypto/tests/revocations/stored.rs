//! What a server checks of revocations: whether a new one counts beside those it stored, and which
//! devices of the account checks 3 and 4 revoke, without checks 1 and 2, which it made when it took
//! each certificate.

use skald_crypto::{counts, revoked_devices};
use skald_proto::Revocation;

use crate::fixtures::*;

fn revocations(revs: &[Rev]) -> Vec<Revocation> {
    revs.iter().map(Rev::revocation).collect()
}

/// Whether `new` counts against the account's master key and every certificate, beside `stored`.
fn counts_beside(stored: &[Rev], new: &Rev) -> bool {
    counts(
        &public(MASTER),
        &certs(),
        &revocations(stored),
        &new.revocation(),
    )
}

/// The devices of the account that `revs` revoke, by seed, sorted as the IDs are.
fn revoked(revs: &[Rev]) -> Vec<u8> {
    let found = revoked_devices(&public(MASTER), &certs(), &revocations(revs), ACCOUNT);
    let mut seeds: Vec<u8> = (0..=u8::MAX)
        .filter(|&seed| found.contains(&device(seed)))
        .collect();
    seeds.sort_by_key(|&seed| device(seed));
    assert_eq!(seeds.len(), found.len(), "{found:?}");
    seeds
}

fn sorted(seeds: &[u8]) -> Vec<u8> {
    let mut seeds = seeds.to_vec();
    seeds.sort_by_key(|&seed| device(seed));
    seeds
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_that_counts_alone_counts_beside_none() {
    for rev in [
        Rev::by(LAPTOP, DESKTOP),
        Rev::by(DESKTOP, DESKTOP),
        Rev::by(BROWSER, PHONE),
        Rev::by_master(LAPTOP),
        Rev::by_master(DESKTOP).also(&[RECOVERED]),
    ] {
        assert!(counts_beside(&[], &rev), "{rev:#?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_ignored_revocation_does_not_count() {
    let stored = [Rev::by(LAPTOP, DESKTOP)];
    for rev in [
        Rev {
            broken: true,
            ..Rev::by(LAPTOP, TABLET)
        },
        Rev {
            label: "skald cert",
            ..Rev::by(LAPTOP, TABLET)
        },
        Rev::by(LAPTOP, TABLET).also(&[PHONE]),
        Rev::by(SERVICE, TABLET),
        Rev::by(BY_SERVER, TABLET),
        Rev::by(UNCERTIFIED, TABLET),
        Rev {
            key: OTHER_MASTER,
            ..Rev::by_master(TABLET)
        },
    ] {
        assert!(!counts_beside(&[], &rev), "alone: {rev:#?}");
        assert!(!counts_beside(&stored, &rev), "beside another: {rev:#?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_copy_of_a_stored_revocation_with_a_broken_signature_does_not_count() {
    let stored = Rev::by(LAPTOP, DESKTOP);
    let broken = Rev {
        broken: true,
        ..stored.clone()
    };
    assert!(!counts_beside(std::slice::from_ref(&stored), &broken));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_by_a_device_a_stored_master_key_revocation_marks_as_stolen_does_not_count() {
    let stolen = [Rev::by_master(DESKTOP)];
    assert!(!counts_beside(&stolen, &Rev::by(DESKTOP, LAPTOP)));
    assert!(!counts_beside(&stolen, &Rev::by(PHONE_BROWSER, LAPTOP)));
    assert!(counts_beside(&stolen, &Rev::by(LAPTOP, RECOVERED)));
    let kept = [Rev::by_master(DESKTOP).keep(&[PHONE])];
    assert!(counts_beside(&kept, &Rev::by(PHONE_BROWSER, LAPTOP)));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_revoked_devices_are_those_checks_3_and_4_revoke() {
    for revs in [
        vec![],
        vec![Rev::by(LAPTOP, DESKTOP)],
        vec![Rev::by(DESKTOP, DESKTOP).keep(&[PHONE])],
        vec![Rev::by_master(LAPTOP).keep(&[DESKTOP])],
        vec![
            Rev::by(LAPTOP, DESKTOP).keep(&[PHONE]),
            Rev::by(RECOVERED, DESKTOP).keep(&[PHONE, TABLET]),
        ],
        vec![
            Rev::by(DESKTOP, PHONE),
            Rev::by_master(DESKTOP).keep(&[PHONE]),
        ],
        vec![
            Rev::by(DESKTOP, LAPTOP),
            Rev::by_master(DESKTOP).also(&[TABLET]).keep(&[PHONE]),
        ],
    ] {
        let by_checks: Vec<u8> = CERTIFIED
            .into_iter()
            .filter(|&seed| status(&revs, seed) == Status::Revoked)
            .collect();
        assert_eq!(revoked(&revs), sorted(&by_checks), "{revs:#?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_without_a_valid_certificate_is_revoked_without_checks_1_and_2() {
    assert_eq!(revoked(&[]), Vec::<u8>::new());
    assert_eq!(revoked(&[Rev::by_master(BY_SERVER)]), [BY_SERVER]);
    assert_eq!(revoked(&[Rev::by(LAPTOP, BY_SERVER)]), [BY_SERVER]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn only_the_accounts_devices_are_revoked() {
    let other = |rev: Rev| Rev {
        account: OTHER_ACCOUNT,
        ..rev
    };
    assert_eq!(revoked(&[other(Rev::by_master(FOREIGN))]), Vec::<u8>::new());
    assert_eq!(
        revoked(&[Rev::by_master(FOREIGN), Rev::by_master(OTHER_LAPTOP)]),
        Vec::<u8>::new()
    );
}
