//! A device linked with the recovery key issues its own certificate, and signs its key too.

use crate::linking::{Ada, LOST, new_device, recovered};
use crate::world::World;
use crate::{
    ADA_GITHUB, ADA_HOST, ADA_LOGIN, Api, Kind, NEW_HOST, assert_says_signed, assert_signed_by,
    machine, new_key, sign_up_at_terminal, ssh_dir, with_ssh_keygen,
};

/// Ada's first machine, without `ssh-keygen`, signs up at her terminal and shows the recovery key.
/// A new machine with a listed key in her SSH directory links with that key, naming the first
/// device lost: its certificate, issued by the master key, carries a signature by the listed key,
/// and the run says so.
#[test]
fn a_device_linked_with_the_recovery_key_signs_its_key_too() {
    let world = World::start();
    let api = Api::start();
    let first = machine(ADA_HOST, &api.url());
    let identity = (ADA_GITHUB, ADA_LOGIN);
    let (person, key, _) = sign_up_at_terminal(&world, identity, (first, ADA_HOST), (false, &[]));
    let ada = Ada {
        person,
        key: key.expect("the recovery key"),
    };
    let first = ada.person.device;
    let new = with_ssh_keygen(machine(NEW_HOST, &api.url()));
    let ssh = new_key(&ssh_dir(&new).join("id_ed25519"), Kind::Ed25519, "");
    api.list((ADA_GITHUB, ADA_LOGIN), &[&ssh], &[]);

    let shown = recovered(&world, &ada, &new, &[(LOST, first)]);

    let device = new_device(&world.cloud, &ada.person, &[first]);
    let cert = (world.cloud.cert(&ada.person.token, device)).expect("the new certificate");
    assert_eq!(cert.body().issuer, None, "issued by the master key");
    assert_signed_by(&cert, ADA_GITHUB, &ssh, &[]);
    assert_says_signed(&shown.text, &ssh, ADA_LOGIN);
}
