//! Who gets an account's devices: only the account's own sessions.

use std::collections::BTreeSet;

use skald_proto::DeviceId;

use crate::api::Person;
use crate::common::devices::assert_refused;
use crate::sessions::{Api, COOKIE};
use crate::shown::{flow, list_response, person, shown};

/// The IDs of `person`'s two devices.
fn ids_of(person: &Person) -> BTreeSet<DeviceId> {
    BTreeSet::from([person.laptop.id, person.uncertified.id])
}

/// Ada's and Bob's web sessions each get their own account's devices and none of the other's. A
/// request without a session, or with a cookie of no session, gets `401` and no device.
#[test]
fn only_the_accounts_sessions_get_its_devices() {
    let flow = flow();
    let api = Api::of(&flow);
    let (ada, ada_cookie) = person(&flow, 3_224_100_101, "ada");
    let (bob, bob_cookie) = person(&flow, 3_224_100_102, "bob");

    for (cookie, owner, who) in [(&ada_cookie, &ada, "Ada"), (&bob_cookie, &bob, "Bob")] {
        let got: BTreeSet<DeviceId> = shown(&api, cookie)
            .values()
            .map(|device| device.id)
            .collect();
        assert_eq!(got, ids_of(owner), "{who}'s web session");
    }

    let unknown = format!("{COOKIE}=bm8tc3VjaC1zZXNzaW9uLWF0LWFsbC0wMDAwMDAwMDAwMDA");
    for (headers, what) in [
        (vec![], "without a session"),
        (
            vec![("cookie", unknown.as_str())],
            "with an unknown session",
        ),
    ] {
        let response = list_response(&api, &headers);
        assert_refused(&response, 401);
        let body = String::from_utf8_lossy(&response.body);
        for id in ids_of(&ada).iter().chain(&ids_of(&bob)) {
            assert!(!body.contains(&id.to_string()), "{what}: {body}");
        }
    }
}
