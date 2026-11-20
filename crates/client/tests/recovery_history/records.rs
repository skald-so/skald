//! Member devices take the valid account record with the highest `revision` of each member account,
//! checked with the `master` the roster gives for it: a later one with a new `recovery` warns and
//! takes the later wraps, and one that does not verify with that key changes nothing.

use skald_crypto::{HpkeKey, RecoveryKey, SigningKey, sign_account};
use skald_proto::{AccountBody, AccountRecord, Cbor as _, Unknown};

use crate::chain::{adds_to_channel, commits_empty};
use crate::kit::received;
use crate::machine::Finished;
use crate::revocations::Fixture;
use crate::talk::Talk;
use crate::world::warnings;
use crate::{add_records, assert_wrapped_to_recovery, with_record};

/// What a warning of a changed recovery key says.
const RECOVERY: &str = "recovery";

/// The talk of Ada and Bob, and Carol's fixture account, a member of the talk's channel, whose
/// master key the test holds and whose record, of revision 1 with the recovery key `first`, the
/// server's database holds.
struct Records {
    talk: Talk,
    carol: Fixture,
    /// The HPKE key pair of Carol's first recovery key.
    first: HpkeKey,
}

impl Records {
    /// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next; Carol, the third,
    /// is a fixture account with the record of revision 1. A revision Ada's device signs adds Carol
    /// to the channel, and Ada's next `skald recv` runs. Bob's device commits an empty Commit, and
    /// Ada's next run wraps the new epoch to Carol's first recovery key; fails unless it does.
    fn new(ids: [u64; 3], logins: [&str; 3]) -> Self {
        let mut talk = Talk::new(&ids[..2], &logins[..2], false);
        let cloud = &talk.world.cloud;
        let carol = Fixture::new(
            cloud,
            &talk.world.browser,
            (ids[2], logins[2]),
            ["carol-records-desk", "carol-records-laptop"],
        );
        let first = new_recovery();
        let record = record_of(&carol, (&first, 1), &carol.master);
        with_record(cloud, carol.person.account, &record.to_cbor());
        adds_to_channel(cloud, &talk.ada, talk.at(), &carol.person);
        received(&talk.ada.machine);
        next_epoch_wrapped_to(&mut talk, &carol, &first);
        Records { talk, carol, first }
    }
}

/// The HPKE key pair of a new recovery key.
fn new_recovery() -> HpkeKey {
    HpkeKey::from_recovery_key(&RecoveryKey::generate().to_bytes())
}

/// Bob's device commits an empty Commit in the talk's channel, and Ada's next `skald recv` runs;
/// fails unless the server then holds the wrap of the new epoch to Carol's recovery key, opening
/// with `recovery`. Gives what the run showed.
#[track_caller]
fn next_epoch_wrapped_to(talk: &mut Talk, carol: &Fixture, recovery: &HpkeKey) -> Finished {
    let at = talk.at();
    let epoch = commits_empty(&talk.world.cloud, &mut talk.played[0], at);
    let ran = received(&talk.ada.machine);
    assert_wrapped_to_recovery(talk, (carol.person.account, recovery), epoch);
    ran
}

/// Carol's account record of `revision`, naming the recovery HPKE key of `recovery` and Carol's
/// master key, signed by `signer`.
fn record_of(
    carol: &Fixture,
    (recovery, revision): (&HpkeKey, u64),
    signer: &SigningKey,
) -> AccountRecord {
    let body = AccountBody {
        account: carol.person.account,
        master: carol.master.public_key(),
        recovery: recovery.public_key(),
        revision,
        unknown: Unknown::default(),
    };
    sign_account(&body, signer).expect("an account record")
}

/// `record` with its signature broken.
fn broken(record: &AccountRecord) -> AccountRecord {
    let mut signature = *record.signature();
    signature[0] ^= 1;
    AccountRecord::from_parts(record.body_bytes().to_vec(), signature).expect("an account record")
}

/// Whether `ran` warns of a changed recovery key of Carol's account: on a warning line that says
/// `recovery` and holds her handle or her account's ID.
fn warns_of_carol_s_recovery(ran: &Finished, carol: &Fixture) -> bool {
    let handle = carol.person.handle.to_string().to_lowercase();
    let account = carol.person.account.to_string();
    warnings(ran)
        .iter()
        .any(|line| line.contains(RECOVERY) && (line.contains(&handle) || line.contains(&account)))
}

/// The relay adds to the devices of the talk's space Carol's record of revision 2 with a new
/// recovery key, signed by her master key. Ada's next `skald recv` warns of Carol's changed
/// recovery key; the wrap of the epoch after Bob's next empty Commit opens with the new key, and
/// that run warns no more.
#[test]
fn a_later_record_with_a_new_recovery_key_warns_and_takes_the_later_wraps() {
    let mut records = Records::new(
        [4_177_703_001, 4_177_703_002, 4_177_703_003],
        [
            "ada-checks-records",
            "bob-commits-around",
            "carol-new-recovery",
        ],
    );
    let second = new_recovery();
    let later = record_of(&records.carol, (&second, 2), &records.carol.master);
    let space = records.talk.space.id;
    add_records(records.talk.cloud(), space, vec![later.to_cbor()]);

    let ran = received(&records.talk.ada.machine);

    assert!(
        warns_of_carol_s_recovery(&ran, &records.carol),
        "no warning of Carol's new recovery key: {ran:?}"
    );
    let then = next_epoch_wrapped_to(&mut records.talk, &records.carol, &second);
    assert!(
        !warns_of_carol_s_recovery(&then, &records.carol),
        "the same record warned again: {then:?}"
    );
}

/// The relay adds to the devices of the talk's space Carol's record of revision 2 with a new
/// recovery key and its signature broken; later it gives in its place one of revision 3 with
/// another new key, signed by a key other than her master key. After each, Ada's runs do not warn
/// of Carol's recovery key, and the wrap of the epoch after Bob's next empty Commit opens with her
/// first key.
#[test]
fn a_record_that_does_not_verify_with_the_member_s_master_key_changes_nothing() {
    let mut records = Records::new(
        [4_177_704_001, 4_177_704_002, 4_177_704_003],
        [
            "ada-drops-bad-records",
            "bob-commits-past-them",
            "carol-keeps-her-key",
        ],
    );
    let carol = &records.carol;
    let forged = [
        broken(&record_of(carol, (&new_recovery(), 2), &carol.master)),
        record_of(carol, (&new_recovery(), 3), &SigningKey::generate()),
    ];
    let space = records.talk.space.id;

    for record in forged {
        add_records(records.talk.cloud(), space, vec![record.to_cbor()]);
        let ran = received(&records.talk.ada.machine);
        let then = next_epoch_wrapped_to(&mut records.talk, &records.carol, &records.first);

        for shown in [&ran, &then] {
            assert!(
                !warns_of_carol_s_recovery(shown, &records.carol),
                "a warning of Carol's recovery key: {shown:?}"
            );
        }
    }
}
