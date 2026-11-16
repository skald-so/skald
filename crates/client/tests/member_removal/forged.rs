//! A certificate the server makes up: `skald exclude-device` bars a device only under the member
//! its certificate chains to.

use skald_crypto::{SigningKey, issue};
use skald_proto::{CertBody, DeviceKind, Role, Unknown};

use crate::edits::{Edit, assert_change, edited, last_two};
use crate::fixtures::fronted;
use crate::kit::{create_space, received, rosters, with_channel};
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST};
use crate::world::{ACME, change_devices, device_entry, now};

/// Bob and Carol are members of Ada's `acme`. The proxy puts first among the space's devices a
/// certificate of Bob's device on Carol's account, as a compromised server would make it: signed
/// by a key of its own, with no proof by the device's key. Ada's `skald exclude-device` of Bob's
/// device signs the revision that bars it under Bob, and Carol's `excluded` stays empty.
#[test]
fn exclude_device_passes_over_a_certificate_of_the_device_made_up_on_another_member() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_140_106_001, "ada-bars-forged", ADA_HOST);
    let bob = world.sign_up(4_140_106_002, "bob-truly-barred", BOB_HOST);
    let carol = world.sign_up(4_140_106_003, "carol-not-barred", CAROL_HOST);
    let space = create_space(cloud, &ada, ACME).id;
    with_channel(
        cloud,
        space,
        &ada,
        &[(&bob, Role::Member), (&carol, Role::Member)],
    );
    received(&ada.machine);
    let held = rosters(cloud, &ada.token, space).len();
    let server_key = SigningKey::generate();
    let made_up = CertBody {
        account: carol.account,
        device: bob.device,
        kind: DeviceKind::Client,
        signing: bob.key.public_key(),
        hpke: SigningKey::generate().public_key(),
        issuer: None,
        issued: now(),
        ssh: None,
        name: BOB_HOST.to_owned(),
        unknown: Unknown::default(),
    };
    let made_up = issue(&made_up, &server_key, &server_key).expect("a certificate");
    let entry = device_entry(bob.device, &made_up);
    change_devices(cloud, space, move |devices| {
        devices.insert(0, entry.clone())
    });

    edited(&ada.machine, ACME, Edit::Exclude(&bob));

    let (previous, next) = last_two(cloud, &ada.token, space, held);
    assert_change(cloud, (&previous, &next), &ada, Edit::Exclude(&bob));
    let carols = next.body().member(carol.account).expect("Carol, a member");
    assert_eq!(carols.excluded, None, "Carol's excluded");
}
