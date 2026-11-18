//! The anchors a device linked with the recovery key starts from: those of the account's devices no
//! master-key revocation marks as stolen. An anchor off the chain of the lowest one keeps it out of
//! the space; anchors that name different personal spaces make it pin none.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_crypto::{SigningKey, sign_anchors};
use skald_proto::{Anchors, AnchorsBody, Cbor as _, DeviceId, Roster, SpaceId};

use crate::anchors::{held, personal_space};
use crate::devices::second_machine;
use crate::linked::{SECOND_HOST, THIRD_HOST};
use crate::machine::Machine;
use crate::terminal::Shown;
use crate::world::{
    ACME, GLOBEX, Person, World, create, holds_word, members, next_by, next_with, plain_lines,
    spaces, store, warnings,
};
use crate::{
    ADA_HOST, Ada, KEEP, NEW_HOST, STOLEN, new_device, recovered, sign_up, warnings_in, with_bob,
};

/// The request whose answer gives the account's anchors objects, as its line starts.
const GIVE_ANCHORS: &str = "GET /account/anchors ";

/// Ada's account with Bob an admin of `acme`, and her second and third machines, which her first
/// certified at revision 3, each anchoring `acme` at a later revision Bob signed: the second at
/// revision 4, the third at 5. Bob then signs revision 6, which her first machine accepts, and the
/// first creates `globex`, which only it anchors.
struct Anchored {
    world: World,
    ada: Ada,
    second: Person,
    third: Person,
    acme: SpaceId,
    /// Revision 4 and revision 5 of `acme`.
    revisions: [Roster; 2],
    bob: Person,
}

impl Anchored {
    fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let world = World::start();
        let cloud = &world.cloud;
        let ada = sign_up(&world, (ids[0], logins[0]), ADA_HOST);
        let (bob, acme, third_revision) = with_bob(&world, &ada.person, (ids[1], logins[1]));
        let second = second_machine(&world, &ada.person, SECOND_HOST);
        let third = second_machine(&world, &ada.person, THIRD_HOST);
        let fourth = next_by(&third_revision, &bob, |_| {});
        store(cloud, &bob.token, &fourth);
        members(&second.machine, ACME);
        let fifth = next_by(&fourth, &bob, |_| {});
        store(cloud, &bob.token, &fifth);
        members(&third.machine, ACME);
        store(cloud, &bob.token, &next_by(&fifth, &bob, |_| {}));
        members(&ada.person.machine, ACME);
        create(cloud, &ada.person, GLOBEX);
        Anchored {
            world,
            ada,
            second,
            third,
            acme,
            revisions: [fourth, fifth],
            bob,
        }
    }

    /// Ada links a new machine with the recovery key, naming her first device stolen and her second
    /// and third kept; gives the machine and what the run showed.
    fn recover(&self) -> (Machine, Shown) {
        let machine = Machine::new().with_host_name(NEW_HOST);
        let named = [
            (STOLEN, self.ada.person.device),
            (KEEP, self.second.device),
            (KEEP, self.third.device),
        ];
        let shown = recovered(&self.world, &self.ada, &machine, &named);
        (machine, shown)
    }

    /// The device of the new machine.
    fn new_device(&self) -> DeviceId {
        let known = [
            self.ada.person.device,
            self.second.device,
            self.third.device,
        ];
        new_device(&self.world.cloud, &self.ada.person, &known)
    }

    /// From now on the relay gives, in place of the third device's anchors object, the one `change`
    /// makes of its body, signed with the third device's key, as a compromised server would that
    /// holds that key.
    fn forge_third(&self, change: impl Fn(&mut AnchorsBody) + Send + Sync + 'static) {
        let device = self.third.device;
        let key = SigningKey::from_bytes(&self.third.key.to_bytes());
        self.world.cloud.relay.rewrite(GIVE_ANCHORS, move |answer| {
            let mut answer: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
            for object in answer["anchors"].as_array_mut().into_iter().flatten() {
                let bytes = URL_SAFE_NO_PAD
                    .decode(object.as_str().unwrap_or_default())
                    .unwrap_or_default();
                let Ok(anchors) = Anchors::from_cbor(&bytes) else {
                    continue;
                };
                if anchors.body().device == device {
                    let mut body = anchors.body().clone();
                    change(&mut body);
                    let forged = sign_anchors(&body, &key).expect("an anchors object");
                    *object = Value::String(URL_SAFE_NO_PAD.encode(forged.to_cbor()));
                }
            }
            answer.to_string()
        });
    }
}

/// Ada's first device, stolen, anchored `acme` at its latest revision and alone anchored `globex`.
/// Her new machine, linked with the recovery key naming the first stolen and the others kept,
/// starts from the anchors of the second and third devices: it follows `acme` and her personal
/// space, not `globex`, and its anchors object names `acme` at revision 6 and her personal space.
#[test]
fn the_new_device_starts_from_the_anchors_of_devices_not_named_stolen() {
    let anchored = Anchored::new(
        [4_176_500_001, 4_176_500_002],
        ["ada-recovery-anchors", "bob-recovery-anchors"],
    );
    let cloud = &anchored.world.cloud;
    let ada = &anchored.ada.person;
    let personal = personal_space(cloud, ada);

    let (machine, _) = anchored.recover();

    let followed = spaces(&machine);
    let lines = plain_lines(&followed);
    assert!(
        holds_word(&lines, ACME),
        "acme is not followed: {followed:?}"
    );
    assert!(
        !holds_word(&lines, GLOBEX),
        "globex is followed: {followed:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.to_lowercase().contains("personal")),
        "no personal space: {followed:?}"
    );
    let new = anchored.new_device();
    let objects = held(cloud, &ada.token);
    let own = objects
        .iter()
        .find(|object| object.device == new)
        .unwrap_or_else(|| panic!("no anchors object of the new device: {objects:?}"));
    let (_, sixth) = cloud
        .revisions(&anchored.bob.token, anchored.acme)
        .pop()
        .expect("revision 6");
    assert_eq!(
        own.anchors.get(&anchored.acme),
        Some(&sixth.hash()),
        "{own:?}"
    );
    assert_eq!(own.personal, Some(personal), "{own:?}");
}

/// The server gives, in place of the third device's anchors object, one signed with its key that
/// anchors `acme` at a fork of revision 5, off the chain of the second device's anchor. The new
/// machine warns of `acme` and does not follow it.
#[test]
fn an_anchor_off_the_chain_keeps_the_new_device_out_of_the_space() {
    let anchored = Anchored::new(
        [4_176_500_101, 4_176_500_102],
        ["ada-recovery-fork", "bob-recovery-fork"],
    );
    let (acme, bob) = (anchored.acme, &anchored.bob);
    let fork = next_with(&anchored.revisions[0], (bob.device, &bob.key), |body| {
        body.timestamp += 1_000;
    });
    assert_ne!(fork.hash(), anchored.revisions[1].hash(), "not a fork");
    let forked = fork.hash();
    anchored.forge_third(move |body| {
        body.anchors.retain(|(space, _)| *space != acme);
        body.anchors.push((acme, forked));
    });

    let (machine, shown) = anchored.recover();

    let followed = spaces(&machine);
    assert!(
        !holds_word(&plain_lines(&followed), ACME),
        "acme is followed: {followed:?}"
    );
    let said = [warnings_in(&shown.text), warnings(&followed)].concat();
    assert!(
        said.iter()
            .any(|line| line.contains(ACME) || line.contains(&acme.to_string())),
        "no warning of acme: {shown:?} {followed:?}"
    );
}

/// The server gives, in place of the third device's anchors object, one signed with its key that
/// names `acme` as the personal space. The new machine warns of the personal space and pins none:
/// its anchors object names no personal space.
#[test]
fn anchors_naming_different_personal_spaces_make_the_new_device_pin_none() {
    let anchored = Anchored::new(
        [4_176_500_201, 4_176_500_202],
        ["ada-recovery-personal", "bob-recovery-personal"],
    );
    let cloud = &anchored.world.cloud;
    let acme = anchored.acme;
    anchored.forge_third(move |body| body.personal = Some(acme));

    let (machine, shown) = anchored.recover();

    let followed = spaces(&machine);
    let said = [warnings_in(&shown.text), warnings(&followed)].concat();
    assert!(
        said.iter().any(|line| line.contains("personal")),
        "no warning of the personal space: {shown:?} {followed:?}"
    );
    let new = anchored.new_device();
    let objects = held(cloud, &anchored.ada.person.token);
    let own = objects
        .iter()
        .find(|object| object.device == new)
        .unwrap_or_else(|| panic!("no anchors object of the new device: {objects:?}"));
    assert_eq!(own.personal, None, "{own:?}");
}
