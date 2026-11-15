//! What counting revocations revoke, in checks 3 and 4 of `membership.md`, "Device checks": a
//! revocation revokes `device`, each device in `also` and every device one of them certified,
//! except those in `keep`; of several counting revocations of one device, a device it certified
//! stays valid only if every one lists it in `keep`.

use skald_crypto::{DeviceCheckError, check_device, check_valid_device};

use crate::fixtures::*;

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn without_revocations_every_certified_device_is_valid() {
    assert_revoked(&[], &[]);
    assert_statuses(
        &[],
        &[
            (BY_SERVER, Status::NoValidCertificate),
            (UNCERTIFIED, Status::NoValidCertificate),
        ],
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_revoked_by_another_client_device_of_its_account_is_revoked_with_every_device_under_it()
{
    assert_revoked(&[Rev::by(LAPTOP, DESKTOP)], &DESKTOP_AND_UNDER);
    assert_revoked(&[Rev::by(RECOVERED, DESKTOP)], &DESKTOP_AND_UNDER);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_revoked_by_a_browser_of_its_account_is_revoked_with_every_device_under_it() {
    assert_revoked(&[Rev::by(BROWSER, DESKTOP)], &DESKTOP_AND_UNDER);
    assert_revoked(&[Rev::by(PHONE_BROWSER, DESKTOP)], &DESKTOP_AND_UNDER);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_revoked_by_itself_is_revoked_with_every_device_under_it() {
    assert_revoked(&[Rev::by(DESKTOP, DESKTOP)], &DESKTOP_AND_UNDER);
    assert_revoked(&[Rev::by(BROWSER, BROWSER)], &[BROWSER]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_revoked_by_the_master_key_is_revoked_with_every_device_under_it() {
    assert_revoked(&[Rev::by_master(DESKTOP)], &DESKTOP_AND_UNDER);
    assert_revoked(&[Rev::by_master(PHONE_BROWSER)], &[PHONE_BROWSER]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn revoking_the_first_device_revokes_every_device_its_chain_reaches() {
    assert_revoked(&[Rev::by(RECOVERED, LAPTOP)], &LAPTOP_AND_UNDER);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn devices_in_keep_stay_valid_with_the_devices_under_them() {
    assert_revoked(
        &[Rev::by(LAPTOP, DESKTOP).keep(&[PHONE])],
        &[DESKTOP, TABLET],
    );
    assert_revoked(
        &[Rev::by_master(DESKTOP).keep(&[PHONE, TABLET])],
        &[DESKTOP],
    );
    assert_revoked(
        &[Rev::by(RECOVERED, LAPTOP).keep(&[DESKTOP, SERVICE])],
        &[LAPTOP, BROWSER],
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn keep_of_a_device_further_down_the_chain_does_not_keep_it() {
    // Check 4: the laptop's revocation keeps the phone but not the desktop, the next device down
    // the phone's chain.
    assert_revoked(
        &[Rev::by(RECOVERED, LAPTOP).keep(&[PHONE])],
        &LAPTOP_AND_UNDER,
    );
    assert_revoked(
        &[Rev::by(LAPTOP, DESKTOP).keep(&[PHONE_BROWSER])],
        &DESKTOP_AND_UNDER,
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn check_4_holds_for_each_revoked_issuer_in_the_chain() {
    // The laptop keeps the desktop, and the desktop keeps the phone.
    assert_revoked(
        &[
            Rev::by(RECOVERED, LAPTOP).keep(&[DESKTOP]),
            Rev::by(RECOVERED, DESKTOP).keep(&[PHONE]),
        ],
        &[LAPTOP, BROWSER, SERVICE, DESKTOP, TABLET],
    );
    // The desktop keeps no device: the phone's chain fails at it, though the laptop keeps the
    // desktop.
    assert_revoked(
        &[
            Rev::by(RECOVERED, LAPTOP).keep(&[DESKTOP]),
            Rev::by(RECOVERED, DESKTOP),
        ],
        &LAPTOP_AND_UNDER,
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_two_revocations_revoke_stays_valid_only_if_both_list_it_in_keep() {
    let laptop = Rev::by(LAPTOP, DESKTOP).keep(&[PHONE, TABLET]);
    assert_revoked(
        &[laptop.clone(), Rev::by(BROWSER, DESKTOP).keep(&[PHONE])],
        &[DESKTOP, TABLET],
    );
    assert_revoked(
        &[
            laptop.clone(),
            Rev::by(BROWSER, DESKTOP).keep(&[TABLET, PHONE]),
        ],
        &[DESKTOP],
    );
    assert_revoked(
        &[laptop.clone(), Rev::by(DESKTOP, DESKTOP)],
        &DESKTOP_AND_UNDER,
    );
    assert_revoked(
        &[laptop, Rev::by_master(DESKTOP).keep(&[TABLET])],
        &[DESKTOP, PHONE, PHONE_BROWSER],
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn devices_in_also_of_a_master_key_revocation_are_revoked_with_the_devices_under_them() {
    assert_revoked(
        &[Rev::by_master(BROWSER).also(&[DESKTOP])],
        &[BROWSER, DESKTOP, PHONE, TABLET, PHONE_BROWSER],
    );
    assert_revoked(
        &[Rev::by_master(BROWSER)
            .also(&[DESKTOP, SERVICE])
            .keep(&[TABLET])],
        &[BROWSER, SERVICE, DESKTOP, PHONE, PHONE_BROWSER],
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_by_a_revoked_device_counts() {
    assert_revoked(
        &[Rev::by(LAPTOP, BROWSER), Rev::by(BROWSER, TABLET)],
        &[BROWSER, TABLET],
    );
    // A device revoked by itself, and one revoked by the device it revoked.
    assert_revoked(
        &[Rev::by(PHONE, PHONE), Rev::by(PHONE, SERVICE)],
        &[PHONE, PHONE_BROWSER, SERVICE],
    );
    assert_revoked(
        &[Rev::by(BROWSER, TABLET), Rev::by(TABLET, BROWSER)],
        &[BROWSER, TABLET],
    );
    // The desktop, revoked with the laptop it revokes, which certified it.
    assert_revoked(&[Rev::by(DESKTOP, LAPTOP)], &LAPTOP_AND_UNDER);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_of_a_device_of_another_account_does_not_revoke_it() {
    // The other account's device is checked against its own master key.
    assert_revoked(&[Rev::by(LAPTOP, OTHER_LAPTOP)], &[]);
    assert_revoked(&[Rev::by_master(OTHER_LAPTOP)], &[]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revoked_device_is_told_apart_from_one_without_a_valid_certificate() {
    let master = public(MASTER);
    let certs = certs();
    let revocations = [
        Rev::by(LAPTOP, DESKTOP).revocation(),
        Rev::by(LAPTOP, BY_SERVER).revocation(),
        Rev::by(LAPTOP, UNCERTIFIED).revocation(),
    ];
    let check = |seed| check_valid_device(&master, &certs, &revocations, device(seed));

    // The revoked device named is the one check 3 or 4 fails at: the device, or the revoked
    // issuer whose revocations do not keep the next device down the chain.
    let revoked = check(DESKTOP);
    assert_eq!(
        revoked,
        Err(DeviceCheckError::Revoked {
            device: device(DESKTOP)
        })
    );
    assert_eq!(
        check(PHONE_BROWSER),
        Err(DeviceCheckError::Revoked {
            device: device(DESKTOP)
        })
    );
    let revoked = revoked.unwrap_err();
    assert_ne!(
        revoked.to_string(),
        DeviceCheckError::NoCertificate.to_string()
    );

    // Without a valid certificate the reason is that of checks 1 and 2, revoked or not.
    assert_eq!(check(UNCERTIFIED), Err(DeviceCheckError::NoCertificate));
    assert_eq!(
        check(BY_SERVER).map(|cert| cert.body().device),
        check_device(&master, &certs, device(BY_SERVER)).map(|cert| cert.body().device)
    );

    // A revocation does not stop a revoked device's certificate from passing checks 1 and 2.
    assert!(check_device(&master, &certs, device(DESKTOP)).is_ok());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_valid_device_is_given_the_certificate_checks_1_and_2_give() {
    let master = public(MASTER);
    let certs = certs();
    let revocations = [Rev::by(LAPTOP, TABLET).revocation()];
    for seed in [LAPTOP, DESKTOP, PHONE, PHONE_BROWSER] {
        assert_eq!(
            check_valid_device(&master, &certs, &revocations, device(seed)),
            check_device(&master, &certs, device(seed)),
            "{}",
            name(seed)
        );
    }
}
