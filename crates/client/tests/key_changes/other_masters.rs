//! An account record the server hands out that names a master key other than the member's, signed
//! by that key, fails its check: a key change.

use serde_json::{Value, json};
use skald_crypto::{HpkeKey, SigningKey, sign_account};
use skald_proto::{AccountBody, Cbor as _, Unknown};

use crate::kit::{RECV, b64, received};
use crate::talk::Talk;
use crate::world::devices_request;
use crate::{assert_says_none, assert_waits_and_shows_once};

/// In Ada's talk with Bob, the relay once gives the devices of the talk's space with a record of
/// Bob's account that names another master key and is signed by it, and Ada's `skald recv` runs:
/// a key change waits on her machine, also once the relay gives the devices as the server does,
/// and shows once, naming Bob.
#[test]
fn a_record_naming_another_master_key_waits_until_seen() {
    let talk = Talk::new(
        &[4_188_971_001, 4_188_971_002],
        &["ada-sees-another-master", "bob-with-a-record-swapped"],
        false,
    );
    let machine = &talk.ada.machine;
    assert_says_none(&received(machine).shown());
    let bob = &talk.played[0].person;
    let key = SigningKey::generate();
    let body = AccountBody {
        account: bob.account,
        master: key.public_key(),
        recovery: HpkeKey::generate().public_key(),
        revision: 7,
        unknown: Unknown::default(),
    };
    let swapped = json!(b64(&sign_account(&body, &key).expect("a record").to_cbor()));
    let request = devices_request(talk.space.id);
    let relay = &talk.cloud().relay;
    relay.rewrite(&request, move |body| {
        let mut given: Value = serde_json::from_str(body).expect("the server's devices");
        let mut records = given["records"]
            .take()
            .as_array()
            .cloned()
            .unwrap_or_default();
        records.insert(0, swapped.clone());
        given["records"] = Value::from(records);
        given.to_string()
    });
    received(machine);
    relay.rewrite(&request, str::to_owned);

    let names = [bob.handle.to_string(), bob.account.to_string()];
    assert_waits_and_shows_once(machine, &[RECV], &names);
}
