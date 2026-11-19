//! A device linked with the recovery key certifies itself with the master key, and a member of the
//! account's spaces takes it as valid.

use crate::linked::checked_by_member;
use crate::machine::Machine;
use crate::world::{ACME, holds_word, plain_lines, spaces};
use crate::{
    ADA_HOST, LOST, NEW_HOST, assert_certified_by_the_master_key, first_line_naming, new_device,
    played, revoked_once_by, shows_key, sign_up, typed, with_bob,
};

/// Ada's first machine, which made her account and showed its recovery key, is lost. On a new
/// machine she signs in with the recovery key, typed in lowercase with spaces, naming the first
/// device lost. The run succeeds without echoing the key and shows the first device, not revoked;
/// the server then holds the new device's certificate, without `issuer`, valid under the master
/// key, which Bob, an admin of her space `acme`, takes as valid, and a revocation of the first
/// device signed by the new one. The new device follows `acme`, which the first device anchored.
#[test]
fn a_device_linked_with_the_recovery_key_certifies_itself_with_the_master_key() {
    let world = crate::World::start();
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_100_001, "ada-recovery-link"), ADA_HOST);
    let (bob, acme, _) = with_bob(&world, &ada.person, (4_176_100_002, "bob-recovery-link"));
    let first = ada.person.device;
    let machine = Machine::new().with_host_name(NEW_HOST);

    let typed = typed(&ada.key);
    let run = played(
        &world,
        (ada.user(), Some(&typed)),
        &machine,
        &[(LOST, first)],
        || false,
    );
    let shown = run.finish();

    assert!(shown.success, "{shown:?}");
    assert!(
        !shows_key(&shown.text, &ada.key),
        "the key is echoed: {shown:?}"
    );
    let listed = first_line_naming(&shown.text, ADA_HOST).expect("the first device is shown");
    assert!(!listed.to_lowercase().contains("revoked"), "{shown:?}");
    let new = new_device(cloud, &ada.person, &[first]);
    assert_certified_by_the_master_key(cloud, &ada.person, new);
    let by_bob = checked_by_member(cloud, (&bob.token, acme), &ada.person.master, new);
    assert!(by_bob.is_ok(), "Bob's check of the new device: {by_bob:?}");
    revoked_once_by(cloud, &ada.person, first, Some(new));
    let followed = spaces(&machine);
    assert!(holds_word(&plain_lines(&followed), ACME), "{followed:?}");
}
