//! Account records: a member account's later record with a new `recovery`, and a record that fails
//! its check with the member's master key.

use serde_json::Value;
use skald_crypto::{HpkeKey, SigningKey, sign_account};
use skald_proto::{AccountBody, AccountRecord, Cbor as _, MasterBackup, Unknown};

use crate::chain::adds_to_channel;
use crate::fixtures::hex;
use crate::kit::{RECV, b64, received};
use crate::revocations::Fixture;
use crate::talk::Talk;
use crate::world::devices_request;
use crate::{assert_says_none, assert_waits_and_shows_once};

/// Ada's talk with Bob, and Carol, a fixture account whose master key the test holds, in the
/// talk's channel; the server's database holds her record of revision 1, which Ada's machine took.
struct Records {
    talk: Talk,
    carol: Fixture,
}

impl Records {
    /// Ada is GitHub user `ids[0]` with `logins[0]`, Bob the next and Carol the third.
    fn new(ids: [u64; 3], logins: [&str; 3]) -> Self {
        let talk = Talk::new(&ids[..2], &logins[..2], false);
        let cloud = talk.cloud();
        let carol = Fixture::new(
            cloud,
            &talk.world.browser,
            (ids[2], logins[2]),
            ["carol-desk", "carol-tablet"],
        );
        let first = record(&carol, 1, &carol.master);
        let backup = MasterBackup {
            nonce: [0x5a; 12],
            ct: [0xa5; 48],
        };
        cloud.execute(&format!(
            "UPDATE accounts SET record = '\\x{}'::bytea, backup = '\\x{}'::bytea WHERE id = '{}'::uuid",
            hex(&first.to_cbor()),
            hex(&backup.to_cbor()),
            carol.person.account,
        ));
        adds_to_channel(cloud, &talk.ada, talk.at(), &carol.person);
        assert_says_none(&received(&talk.ada.machine).shown());
        Records { talk, carol }
    }

    /// The relay gives the devices of the talk's space with `given` among the account records, then
    /// Ada's `skald recv` runs.
    fn given(&self, given: &AccountRecord) {
        let added = Value::String(b64(&given.to_cbor()));
        let request = devices_request(self.talk.space.id);
        (self.talk.cloud().relay).rewrite(&request, move |body| {
            let mut list: Value = serde_json::from_str(body).expect("the server's devices");
            let mut records = list["records"].as_array().cloned().unwrap_or_default();
            records.push(added.clone());
            list["records"] = Value::Array(records);
            list.to_string()
        });
        received(&self.talk.ada.machine);
    }

    /// From now on, the relay gives the devices of the talk's space as the server gives them.
    fn as_the_server_gives(&self) {
        let request = devices_request(self.talk.space.id);
        (self.talk.cloud().relay).rewrite(&request, str::to_owned);
    }

    /// Carol's handle and account ID, as a key change of her record may name her.
    fn names(&self) -> Vec<String> {
        let person = &self.carol.person;
        vec![person.handle.to_string(), person.account.to_string()]
    }
}

/// Carol's account record of `revision` with a new recovery key, signed by `signer`.
fn record(carol: &Fixture, revision: u64, signer: &SigningKey) -> AccountRecord {
    let body = AccountBody {
        account: carol.person.account,
        master: carol.master.public_key(),
        recovery: HpkeKey::generate().public_key(),
        revision,
        unknown: Unknown::default(),
    };
    sign_account(&body, signer).expect("an account record")
}

/// The relay gives Carol's record of revision 2 with a new recovery key, signed by her master key:
/// a key change waits on Ada's machine and shows once, naming Carol.
#[test]
fn a_changed_recovery_key_waits_until_seen() {
    let records = Records::new(
        [4_188_301_001, 4_188_301_002, 4_188_301_003],
        [
            "ada-sees-a-new-recovery",
            "bob-beside-records",
            "carol-new-recovery-key",
        ],
    );

    records.given(&record(&records.carol, 2, &records.carol.master));

    let machine = &records.talk.ada.machine;
    assert_waits_and_shows_once(machine, &[RECV], &records.names());
}

/// The relay gives Carol's record of revision 2 with a new recovery key, signed by a key other
/// than her master key: a key change waits on Ada's machine, also once the relay gives her record
/// as the server holds it, and shows once, naming Carol.
#[test]
fn a_record_that_fails_its_check_waits_until_seen() {
    let records = Records::new(
        [4_188_302_001, 4_188_302_002, 4_188_302_003],
        [
            "ada-sees-a-forged-record",
            "bob-beside-forgery",
            "carol-record-forged",
        ],
    );

    records.given(&record(&records.carol, 2, &SigningKey::generate()));
    records.as_the_server_gives();

    let machine = &records.talk.ada.machine;
    assert_waits_and_shows_once(machine, &[RECV], &records.names());
}
