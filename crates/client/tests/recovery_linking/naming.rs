//! The devices the user of a device linked with the recovery key names: stolen ones it revokes with
//! the master key, keeping the devices named kept, which voids the revocations the stolen devices
//! signed, unless named kept revoked; lost or wiped ones it revokes with its own key.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_crypto::{DeviceCheckError, SigningKey, sign_revocation};
use skald_proto::{Cbor as _, DeviceId, RevocationBody, Unknown};

use crate::devices::second_machine;
use crate::linked::{SECOND_HOST, THIRD_HOST, checked_by_member, revoke_keeping};
use crate::machine::Machine;
use crate::world::{Person, World};
use crate::{
    ADA_HOST, Ada, KEEP, KEEP_REVOKED, LOST, NEW_HOST, STOLEN, first_line_naming, new_device,
    recovered, revoked_once_by, sign_up, with_bob,
};

/// Whether the first line of `text` that shows `host` says it is revoked.
#[track_caller]
fn shown_revoked(text: &str, host: &str) -> bool {
    first_line_naming(text, host)
        .unwrap_or_else(|| panic!("{host} is not shown: {text}"))
        .to_lowercase()
        .contains("revoked")
}

/// Fails unless `checked`, a member's check of a device, says it is revoked.
#[track_caller]
fn assert_revoked(checked: Result<(), DeviceCheckError>, what: &str) {
    assert!(
        matches!(checked, Err(DeviceCheckError::Revoked { .. })),
        "{what}: {checked:?}"
    );
}

/// Ada's first machine, her account's, with Bob an admin of her space `acme`; her second machine,
/// which the first certified; her third, which the second certified. Gives Ada, Bob, `acme`, the
/// second and the third.
fn chain(
    world: &World,
    ids: [u64; 2],
    logins: [&str; 2],
) -> (Ada, Person, crate::SpaceId, [Person; 2]) {
    let ada = sign_up(world, (ids[0], logins[0]), ADA_HOST);
    let (bob, acme, _) = with_bob(world, &ada.person, (ids[1], logins[1]));
    let second = second_machine(world, &ada.person, SECOND_HOST);
    let third = second_machine(world, &second, THIRD_HOST);
    (ada, bob, acme, [second, third])
}

/// The first device revoked the second, keeping the third, then was stolen. On a new machine Ada
/// links with the recovery key naming the first stolen, the second kept and the third lost. The run
/// shows the second revoked and the others not; the server then holds one master-key revocation of
/// the first, keeping the second, which voids the first's revocation of the second, so Bob takes
/// the second as valid; and a revocation of the third signed by the new device.
#[test]
fn stolen_devices_are_revoked_with_the_master_key_and_lost_ones_with_the_new_device() {
    let world = World::start();
    let cloud = &world.cloud;
    let (ada, bob, acme, [second, third]) = chain(
        &world,
        [4_176_200_001, 4_176_200_002],
        ["ada-recovery-naming", "bob-recovery-naming"],
    );
    let first = ada.person.device;
    let revoked = revoke_keeping(&ada.person.machine, second.device, &[third.device]);
    assert!(revoked.success, "{revoked:?}");
    let machine = Machine::new().with_host_name(NEW_HOST);

    let shown = recovered(
        &world,
        &ada,
        &machine,
        &[(STOLEN, first), (KEEP, second.device), (LOST, third.device)],
    );

    let text = &shown.text;
    assert!(!shown_revoked(text, ADA_HOST), "the first device: {text}");
    assert!(
        shown_revoked(text, SECOND_HOST),
        "the second device: {text}"
    );
    assert!(!shown_revoked(text, THIRD_HOST), "the third device: {text}");
    let new = new_device(cloud, &ada.person, &[first, second.device, third.device]);
    let by_master = revoked_once_by(cloud, &ada.person, first, None);
    let kept: Vec<DeviceId> = by_master
        .body()
        .keep
        .as_deref()
        .unwrap_or_default()
        .to_vec();
    assert!(kept.contains(&second.device), "its keep: {by_master:?}");
    let member = (bob.token.as_str(), acme);
    let master = &ada.person.master;
    let checked = checked_by_member(cloud, member, master, second.device);
    assert!(checked.is_ok(), "the kept second device: {checked:?}");
    revoked_once_by(cloud, &ada.person, third.device, Some(new));
}

/// Ada's first machine certified her second and third; the second revoked the third, then was
/// stolen. Ada links a new machine with the recovery key naming the second stolen: its master-key
/// revocation voids the second's revocation of the third, which Bob then takes as valid. Named also
/// kept revoked, on another new machine of another account, the third stays revoked: the
/// master-key revocation lists it in `also`.
#[test]
fn a_device_named_kept_revoked_stays_revoked_after_its_revoker_is_revoked_as_stolen() {
    for (named_too, user) in [
        (false, (4_176_200_101, "ada-recovery-voids")),
        (true, (4_176_200_201, "ada-recovery-also")),
    ] {
        let world = World::start();
        let cloud = &world.cloud;
        let ada = sign_up(&world, user, ADA_HOST);
        let (bob, acme, _) = with_bob(&world, &ada.person, (user.0 + 1, "bob-recovery-also"));
        let second = second_machine(&world, &ada.person, SECOND_HOST);
        let third = second_machine(&world, &ada.person, THIRD_HOST);
        let revoked = revoke_keeping(&second.machine, third.device, &[]);
        assert!(revoked.success, "{revoked:?}");
        let machine = Machine::new().with_host_name(NEW_HOST);
        let mut named = vec![(STOLEN, second.device)];
        if named_too {
            named.push((KEEP_REVOKED, third.device));
        }

        let shown = recovered(&world, &ada, &machine, &named);

        assert!(shown_revoked(&shown.text, THIRD_HOST), "{shown:?}");
        let by_master = revoked_once_by(cloud, &ada.person, second.device, None);
        let also: Vec<DeviceId> = by_master
            .body()
            .also
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        let member = (bob.token.as_str(), acme);
        let checked = checked_by_member(cloud, member, &ada.person.master, third.device);
        if named_too {
            assert_eq!(also, vec![third.device], "its also: {by_master:?}");
            assert_revoked(checked, "the third device, kept revoked");
        } else {
            assert_eq!(also, Vec::<DeviceId>::new(), "its also: {by_master:?}");
            assert!(checked.is_ok(), "the third device: {checked:?}");
        }
        let first = checked_by_member(cloud, member, &ada.person.master, ada.person.device);
        assert!(first.is_ok(), "the first device: {first:?}");
    }
}

/// The request whose answer lists the account's devices and revocations, as its line starts.
const LIST_DEVICES: &str = "GET /devices ";

/// The proxy lists, beside the account's revocations, one of Ada's first device signed by another
/// key in place of the master key, when she names it stolen, or in place of her new device's, when
/// she names it lost, as a compromised server would. Her new machine revokes it all the same: the
/// server then holds one revocation of the first device, by the master key or by the new device.
#[test]
fn a_listed_revocation_that_does_not_verify_does_not_stand_for_the_new_devices_own() {
    for (option, user) in [
        (STOLEN, (4_176_200_301, "ada-recovery-forged-stolen")),
        (LOST, (4_176_200_401, "ada-recovery-forged-lost")),
    ] {
        let world = World::start();
        let cloud = &world.cloud;
        let ada = sign_up(&world, user, ADA_HOST);
        let (first, account) = (ada.person.device, ada.person.account);
        let by_master = option == STOLEN;
        let forger = SigningKey::generate().to_bytes();
        cloud.relay.rewrite(LIST_DEVICES, move |answer| {
            let mut list: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
            let new = (list["devices"].as_array().into_iter().flatten())
                .filter_map(|entry| entry["device"].as_str()?.parse::<DeviceId>().ok())
                .find(|device| *device != first);
            let body = RevocationBody {
                account,
                device: first,
                keep: None,
                signer: if by_master { None } else { new },
                timestamp: 1_800_000_000,
                also: None,
                unknown: Unknown::default(),
            };
            let forged =
                sign_revocation(&body, &SigningKey::from_bytes(&forger)).expect("a revocation");
            if let Some(listed) = list["revocations"].as_array_mut() {
                listed.push(Value::String(URL_SAFE_NO_PAD.encode(forged.to_cbor())));
            }
            list.to_string()
        });
        let machine = Machine::new().with_host_name(NEW_HOST);

        recovered(&world, &ada, &machine, &[(option, first)]);

        let new = new_device(cloud, &ada.person, &[first]);
        let signer = if by_master { None } else { Some(new) };
        revoked_once_by(cloud, &ada.person, first, signer);
    }
}
