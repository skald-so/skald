//! Master-key revocations, `membership.md`, "Revocation": one marks as stolen `device` and every
//! device whose chain passes through it other than through a device in `keep`, and voids every
//! revocation they signed, which restores the devices those revoked but the ones in its `also`.
//! Nothing voids a master-key revocation.

use crate::fixtures::*;

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_master_key_revocation_voids_the_revocations_of_the_stolen_device() {
    let by_desktop = [Rev::by(DESKTOP, BROWSER), Rev::by(DESKTOP, LAPTOP)];
    assert_revoked(&by_desktop, &LAPTOP_AND_UNDER);

    let mut revs = by_desktop.to_vec();
    revs.push(Rev::by_master(DESKTOP));
    assert_revoked(&revs, &DESKTOP_AND_UNDER);

    // Revoked by itself first, then as stolen.
    assert_revoked(
        &[
            Rev::by(DESKTOP, DESKTOP),
            Rev::by(DESKTOP, SERVICE),
            Rev::by_master(DESKTOP),
        ],
        &DESKTOP_AND_UNDER,
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_master_key_revocation_voids_the_revocations_of_every_device_under_the_stolen_one() {
    let under = [
        Rev::by(PHONE, BROWSER),
        Rev::by(PHONE_BROWSER, SERVICE),
        Rev::by(TABLET, RECOVERED),
    ];
    assert_revoked(&under, &[BROWSER, SERVICE, RECOVERED]);

    let mut revs = under.to_vec();
    revs.push(Rev::by_master(DESKTOP));
    assert_revoked(&revs, &DESKTOP_AND_UNDER);

    let mut revs = under.to_vec();
    revs.push(Rev::by_master(LAPTOP));
    assert_revoked(&revs, &LAPTOP_AND_UNDER);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn devices_in_keep_of_a_master_key_revocation_are_not_stolen() {
    // The phone and its browser are kept: their revocations count; the tablet's are void.
    assert_revoked(
        &[
            Rev::by(PHONE, BROWSER),
            Rev::by(PHONE_BROWSER, SERVICE),
            Rev::by(TABLET, RECOVERED),
            Rev::by_master(DESKTOP).keep(&[PHONE]),
        ],
        &[DESKTOP, TABLET, BROWSER, SERVICE],
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn devices_in_also_stay_revoked_when_their_revocations_are_voided() {
    assert_revoked(
        &[
            Rev::by(DESKTOP, BROWSER),
            Rev::by(PHONE, SERVICE),
            Rev::by_master(DESKTOP).also(&[BROWSER]),
        ],
        &[DESKTOP, PHONE, TABLET, PHONE_BROWSER, BROWSER],
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_voided_revocation_has_no_say_in_keep() {
    let revs = [
        Rev::by(LAPTOP, DESKTOP).keep(&[PHONE]),
        Rev::by(TABLET, DESKTOP),
    ];
    assert_revoked(&revs, &DESKTOP_AND_UNDER);

    let mut revs = revs.to_vec();
    revs.push(Rev::by_master(TABLET));
    assert_revoked(&revs, &[DESKTOP, TABLET]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_master_key_revocation_stays_counting_whatever_else_names_its_device() {
    // Another master-key revocation keeps its device.
    assert_revoked(
        &[
            Rev::by_master(BROWSER),
            Rev::by_master(LAPTOP).keep(&[BROWSER, DESKTOP, SERVICE]),
        ],
        &[LAPTOP, BROWSER],
    );
    // Another master-key revocation, of the device that certified its device, keeps its device.
    assert_revoked(
        &[
            Rev::by_master(PHONE),
            Rev::by_master(DESKTOP).keep(&[PHONE]),
        ],
        &DESKTOP_AND_UNDER,
    );
    // Other revocations of its device keep the devices under it.
    assert_revoked(
        &[
            Rev::by_master(DESKTOP),
            Rev::by(LAPTOP, DESKTOP).keep(&[PHONE, TABLET]),
            Rev::by_master(DESKTOP).keep(&[PHONE, TABLET]),
        ],
        &DESKTOP_AND_UNDER,
    );
    // Its device revokes itself, and the master key revokes the device that certified it.
    assert_revoked(
        &[
            Rev::by_master(BROWSER),
            Rev::by(BROWSER, BROWSER),
            Rev::by_master(LAPTOP).keep(&[DESKTOP, BROWSER]),
        ],
        &[LAPTOP, BROWSER, SERVICE],
    );
}

/// The devices of the account, by seed in [`CERTIFIED`]'s order, that a master-key revocation of
/// `revs` marks as stolen.
fn stolen(revs: &[Rev]) -> Vec<u8> {
    let revocations: Vec<_> = revs.iter().map(Rev::revocation).collect();
    (CERTIFIED.iter().copied())
        .filter(|&seed| {
            skald_crypto::marked_stolen(&public(MASTER), &certs(), &revocations, device(seed))
        })
        .collect()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_master_key_revocation_marks_as_stolen_its_device_and_those_under_it_but_kept_ones() {
    assert_eq!(stolen(&[Rev::by_master(DESKTOP)]), DESKTOP_AND_UNDER);
    assert_eq!(
        stolen(&[Rev::by_master(DESKTOP).keep(&[PHONE])]),
        [DESKTOP, TABLET]
    );
    assert_eq!(
        stolen(&[Rev::by_master(PHONE), Rev::by_master(RECOVERED)]),
        [PHONE, PHONE_BROWSER, RECOVERED]
    );
    // A device's revocation, one another key signs or a broken one marks none.
    let other_key = Rev {
        key: OTHER_MASTER,
        ..Rev::by_master(DESKTOP)
    };
    let broken = Rev {
        broken: true,
        ..Rev::by_master(DESKTOP)
    };
    for revs in [[Rev::by(LAPTOP, DESKTOP)], [other_key], [broken]] {
        assert_eq!(stolen(&revs), Vec::<u8>::new(), "{revs:#?}");
    }
}
