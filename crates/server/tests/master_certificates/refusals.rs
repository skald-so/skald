//! A certificate without `issuer` that the account's master key did not sign is refused, with
//! nothing stored.

use crate::api::Person;
use crate::common::device_flow::DeviceFlow;
use crate::common::devices::Key;
use crate::{INVALID_CERTIFICATE, assert_problem, held, issued_by_master, put_cert};

/// Alice's desktop's certificate without `issuer`, signed by a key that is not her account's
/// master key: by a key of no account, as a server would sign it, and by Bob's account's master
/// key. Each is refused with `400` and the problem of an invalid certificate, and the server
/// holds none.
#[test]
fn a_certificate_another_key_signed_in_place_of_the_master_key_is_refused() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_176_300_001, "alice");
    let bob = Person::new(&flow, 3_176_300_002, "bob");
    let desktop = &alice.uncertified;
    let stranger = Key::generate();

    for (key, what) in [
        (&stranger, "another key"),
        (&bob.master, "bob's master key"),
    ] {
        let refused = put_cert(&alice, desktop, &issued_by_master(&alice, desktop, key));

        assert_problem(&refused, 400, INVALID_CERTIFICATE, what);
        assert_eq!(held(&alice, desktop), None, "{what}: stored");
    }
}
