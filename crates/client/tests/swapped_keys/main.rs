//! Certificates and master keys a compromised server swaps (`docs/protocol/membership.md`, "Device
//! certificate" and "Device checks").
//!
//! - A client keeps the certificate of each device it trusted. When the server later gives that
//!   device with another certificate, the client warns once of a key change and does not trust the
//!   device until the server gives its certificate back; then it trusts it again with no warning. A
//!   certificate that fails the proof check or check 1 still counts nowhere. The same certificates
//!   given again, or one withheld, warn of nothing.
//! - A second body of a device that passes the proof check, such as a stolen device makes with its
//!   own keys, makes neither body valid, as two bodies given at once do, also once the server gives
//!   only one of them.
//! - A client warns once when the revisions it accepted in its spaces give one account different
//!   master keys, and when they give its own account one other than the one it pinned. Each space's
//!   devices are still checked against that space's revisions.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, which rewrites the device lists the server gives, as a compromised server
//! would; a test plays the person on the verification page in headless Chrome. The CLI runs on
//! Ada's machine and Bob's. Ada's laptop is a device the tests write to the server's database on
//! her account, certified by her machine's device with the keys its secrets file holds, as the
//! tests of sending set devices up (`sending/fixtures.rs`); the tests hold its keys, as a thief of
//! the laptop would. A swapped certificate is one the tests write to the server's database in place
//! of the laptop's. Bob's machine creates `acme`, and a revision his device signs adds Ada as a
//! member, as the tests of roster revisions do (`roster_revisions/main.rs`). A second machine of
//! Bob's is linked by hand, as in the tests of the history on new devices
//! (`history_devices/main.rs`).
//!
//! The CLI, as these tests use it, beside `skald devices` (`device_list/main.rs`) and `skald members
//! <space>` (`roster_revisions/main.rs`):
//! - a device it trusts shows on a line that is no warning, holding the device's ID and its
//!   certificate's `name`; in `skald devices`, above the line that says `not linked`;
//! - the warning of a key change, and of a device's two bodies: a line saying `warning` that holds
//!   the device's ID, on either output;
//! - the warning of an account with different master keys: a line saying `warning` and `master`
//!   that holds the account's handle or its ID; for the device's own account, a line saying
//!   `warning` and `master`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod bodies;
mod masters;
mod swaps;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices, and the helpers of the tests of reading the
// history they use.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;

use serde_json::Value;
use skald_proto::{Cbor as _, Cert, DeviceId, Role, SpaceId};

use fixtures::{Issuer, Keyed, add_device, hex};
use machine::{DEVICES, Finished, NOT_LINKED};
use world::{
    ACME, Person, World, create, devices_request, members, next_by, plain_lines, run, store,
    warnings, words,
};

/// The host names of Ada's and Bob's machines, their certificates' `name`.
pub const ADA_HOST: &str = "ada-workstation";
pub const BOB_HOST: &str = "bob-workstation";

/// The `name` of the certificate of Ada's laptop.
pub const LAPTOP: &str = "ada-laptop";

/// The request for the account's devices, as its line starts.
const OWN_DEVICES: &str = "GET /devices ";

/// Ada's account, with her machine and her laptop, and Bob's machine, which created `acme` and
/// added Ada to it; both machines trust the laptop.
pub struct Scene {
    pub world: World,
    pub ada: Person,
    pub bob: Person,
    pub space: SpaceId,
    pub laptop: Keyed,
    /// The laptop's certificate, as the server's database holds it.
    pub cert: Cert,
}

impl Scene {
    /// GitHub users `ids[0]` with `logins[0]`, Ada, and the next, Bob, sign up. The laptop is
    /// written to the database, certified by Ada's machine's device; Bob's machine creates `acme`,
    /// and revision 3, signed by his device, adds Ada as a member. Each machine looks once, then
    /// fails unless both trust the laptop and Ada's machine.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let world = World::start();
        let [ada, bob] = [(0, ADA_HOST), (1, BOB_HOST)]
            .map(|(at, host)| world.sign_up(ids[at], logins[at], host));
        let cloud = &world.cloud;
        let laptop = add_device(cloud, ada.account, LAPTOP, &Issuer::Device(&ada));
        let cert = (cloud.cert(&ada.token, laptop.id)).expect("the laptop's certificate");
        let second = create(cloud, &bob, ACME);
        let third = next_by(&second, &bob, |members| {
            members.push(ada.member(Role::Member));
        });
        store(cloud, &bob.token, &third);
        let scene = Scene {
            space: third.body().space,
            world,
            ada,
            bob,
            laptop,
            cert,
        };
        let first = scene.look();
        scene.assert_trusted(&first);
        assert!(
            first.trusts(scene.ada.device, ADA_HOST),
            "Ada's machine is not trusted: {first:?}"
        );
        scene
    }

    /// Runs `skald devices` on Ada's machine and `skald members acme` on Bob's; fails unless both
    /// succeed.
    pub fn look(&self) -> Looked {
        let own = run(&self.ada.machine, &[DEVICES]);
        assert!(own.success, "{own:?}");
        Looked {
            own,
            member: members(&self.bob.machine, ACME),
        }
    }

    /// Fails unless both machines trust the laptop in `looked`.
    #[track_caller]
    pub fn assert_trusted(&self, looked: &Looked) {
        assert!(
            looked.trusts(self.laptop.id, LAPTOP),
            "the laptop is not trusted: {looked:?}"
        );
    }

    /// Fails unless neither machine trusts the laptop in `looked`, and no line of either but its
    /// warnings holds one of `names`.
    #[track_caller]
    pub fn assert_untrusted(&self, looked: &Looked, names: &[&str]) {
        let id = self.laptop.id;
        assert!(
            !linked_in_devices(&looked.own, id, LAPTOP),
            "Ada's machine trusts the laptop: {looked:?}"
        );
        assert!(
            !shown_in_members(&looked.member, id, LAPTOP),
            "Bob's machine trusts the laptop: {looked:?}"
        );
        for finished in [&looked.own, &looked.member] {
            for name in names {
                assert!(
                    !(plain_lines(finished).iter()).any(|line| words(line).contains(name)),
                    "{name} is shown: {looked:?}"
                );
            }
        }
    }

    /// Writes `cert` to the server's database as the laptop's certificate.
    pub fn give(&self, cert: &Cert) {
        self.world.cloud.execute(&format!(
            "UPDATE devices SET cert = '\\x{}'::bytea WHERE id = '{}'::uuid",
            hex(&cert.to_cbor()),
            self.laptop.id
        ));
    }

    /// From now on, the relay gives both machines the device lists `change` makes of the server's:
    /// Ada's account's and those of `acme`'s accounts.
    pub fn change_lists(&self, change: impl Fn(&mut Vec<Value>) + Send + Sync + Clone + 'static) {
        let space = devices_request(self.space);
        for request in [OWN_DEVICES, space.as_str()] {
            let change = change.clone();
            self.world.cloud.relay.rewrite(request, move |body| {
                let Ok(mut answer) = serde_json::from_str::<Value>(body) else {
                    return body.to_owned();
                };
                if let Some(listed) = answer.get_mut("devices").and_then(Value::as_array_mut) {
                    change(listed);
                }
                answer.to_string()
            });
        }
    }

    /// From now on, the relay gives both machines the device lists as the server gives them.
    pub fn stop_changing(&self) {
        self.change_lists(|_| {});
    }
}

/// What Ada's machine and Bob's showed in one look.
#[derive(Debug)]
pub struct Looked {
    /// `skald devices` on Ada's machine.
    pub own: Finished,
    /// `skald members acme` on Bob's machine.
    pub member: Finished,
}

impl Looked {
    /// Whether both machines trust `device`, whose certificate's name is `name`.
    pub fn trusts(&self, device: DeviceId, name: &str) -> bool {
        linked_in_devices(&self.own, device, name) && shown_in_members(&self.member, device, name)
    }
}

/// Whether `shown`, the output of `skald devices`, lists `device` as linked: on a line that is no
/// warning, above the line that says `not linked`, with its certificate's name `name`.
pub fn linked_in_devices(shown: &Finished, device: DeviceId, name: &str) -> bool {
    let id = device.to_string();
    let mut below = false;
    for line in shown.stdout.lines() {
        let lower = line.to_lowercase();
        if lower.contains(world::WARNING) {
            continue;
        }
        if line.contains(&id) {
            return !below && !lower.contains(NOT_LINKED) && words(line).contains(&name);
        }
        below |= lower.contains(NOT_LINKED);
    }
    false
}

/// Whether `shown`, the output of `skald members`, shows `device` as a device with a valid
/// certificate: on a line that is no warning, holding its ID and its certificate's name `name`.
pub fn shown_in_members(shown: &Finished, device: DeviceId, name: &str) -> bool {
    let id = device.to_string();
    plain_lines(shown)
        .iter()
        .any(|line| line.contains(&id) && words(line).contains(&name))
}

/// The warning lines of `shown` that hold `device`'s ID.
pub fn warnings_naming(shown: &Finished, device: DeviceId) -> Vec<String> {
    let id = device.to_string().to_lowercase();
    warnings(shown)
        .into_iter()
        .filter(|line| line.contains(&id))
        .collect()
}
