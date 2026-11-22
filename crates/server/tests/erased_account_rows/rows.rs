//! alice fills the tables the tests of erasing an account's data leave empty, then deletes her
//! account; 30 days later the server erases them, but for her Commit bob has not fetched and her
//! wrap to bob's recovery key, and clears her sign-up time and the creator of `acme`.

use std::thread;
use std::time::Instant;

use skald_crypto::{SigningKey, sign_queue_key};
use skald_proto::{AccountId, Cbor as _, ChannelId, QueueKeyBody, SpaceId, Unknown};

use crate::common::TestDb;
use crate::common::devices::{Device, random};
use crate::deletion::deletes;
use crate::erasure::{ERASED_AFTER, ERASURE, Kept, Traces, erased_within, move_deletion_back};
use crate::history::{Item, WRAPS, formed_with, given, stores, wrap};
use crate::scene::{Peer, Scene, empty, payloads, removing};
use crate::sessions::Api;
use crate::wire::{mailbox, query};

impl Kept {
    /// This, with `wraps`, wraps the account's devices signed to other accounts.
    fn with_wraps(self, wraps: &[&Item]) -> Self {
        // A kept item is any bytes a row holds.
        self.with_entries(wraps)
    }
}

/// The queue key object of `channel`'s epoch `epoch`, signed by the device `signer`.
fn queue_key(channel: ChannelId, epoch: u64, signer: &Device) -> Item {
    let body = QueueKeyBody {
        channel,
        epoch,
        key: random(),
        signer: signer.id,
        unknown: Unknown::default(),
    };
    let signing = SigningKey::from_bytes(&signer.signing.secret);
    Item::QueueKey((sign_queue_key(&body, &signing).expect("a queue key object")).to_cbor())
}

/// `peer` acks every entry its mailbox holds in `db`.
fn acks_all(peer: &mut Peer, db: &TestDb) {
    if let Some((last, _)) = mailbox(db, peer.id).last() {
        peer.socket.ack(u64::try_from(*last).expect("a seq from 1"));
    }
}

/// Writes rows of alice's, of `s`, to the tables no request of these tests fills: a challenge of
/// her laptop; a link run of her uncertified device, which her laptop waits for; a wrong user code
/// and a wrong pairing of her account; her acceptance of an invite to `acme`; a queue of `general`
/// she created, with a task her laptop uploaded, a task of bob's laptop addressed to hers, and one
/// her laptop claimed and released, which it waits to claim again.
fn fill(s: &Scene) {
    let t = &s.fixture.team;
    let (alice, laptop, uncertified) = (t.alice.account, t.alice.laptop.id, t.alice.uncertified.id);
    let (space, general, bobs) = (t.space, s.fixture.general, t.bob.laptop.id);
    s.fixture.db().execute(&format!(
        "DO $$
         DECLARE
           queue uuid := gen_random_uuid();
           uploaded uuid := gen_random_uuid();
           addressed uuid := gen_random_uuid();
           released uuid := gen_random_uuid();
           claim uuid := gen_random_uuid();
           invite uuid := gen_random_uuid();
           run bigint;
         BEGIN
           INSERT INTO device_challenges (challenge_hash, device_id, expires_at)
           VALUES (decode(md5(random()::text), 'hex'), '{laptop}', now() + interval '60 days');
           INSERT INTO link_runs (device_id, session, hello, expires_at, pairing, token_hash)
           VALUES ('{uncertified}', decode(md5(random()::text), 'hex'), '\\x01',
                   now() + interval '60 days', md5(random()::text),
                   decode(md5(random()::text), 'hex'))
           RETURNING id INTO run;
           INSERT INTO long_poll_link_runs (device_id, run_id) VALUES ('{laptop}', run);
           INSERT INTO wrong_user_codes (account_id) VALUES ('{alice}');
           INSERT INTO wrong_pairings (account_id) VALUES ('{alice}');
           INSERT INTO invites (id, space_id, expires_at)
           VALUES (invite, '{space}', now() + interval '1 day');
           INSERT INTO acceptances (invite_id, account_id, acceptance)
           VALUES (invite, '{alice}', '\\x01');
           INSERT INTO queues (id, channel_id, name, mode, lease, max_attempts, creator_id)
           VALUES (queue, '{general}', 'alices', 'at-least-once', 300, 5, '{alice}');
           INSERT INTO tasks (id, queue_id, uploader_id, queued_task)
           VALUES (uploaded, queue, '{laptop}', '\\x01');
           INSERT INTO tasks (id, queue_id, uploader_id, queued_task, addressee)
           VALUES (addressed, queue, '{bobs}', '\\x02', '{laptop}');
           INSERT INTO tasks (id, queue_id, uploader_id, queued_task, generation, attempts,
                              claim_id, claim_device)
           VALUES (released, queue, '{bobs}', '\\x03', 1, 1, claim, '{laptop}');
           INSERT INTO claims (id, task_id, generation, device_id, lease, ended)
           VALUES (claim, released, 1, '{laptop}', 300, 'release');
           INSERT INTO task_uploads (device_id, task_id) VALUES ('{laptop}', uploaded);
           INSERT INTO task_waits (task_id, device_id, until)
           VALUES (released, '{laptop}', now() + interval '60 days');
         END $$"
    ));
}

/// The scene: in `general`, formed at epoch 1 on a server that counts messages, bob and carol ack
/// their mailboxes. alice's laptop stores a wrap of epoch 1 to bob's recovery key and the queue key
/// object of epoch 1, sends an application message and commits an empty Commit, which bob and
/// carol process; bob's laptop then commits the Remove of hers, which carol processes, and carol
/// acks her mailbox, while bob's still holds alice's message and Commit. alice fills the other
/// tables as [`fill`] does.
struct Filled {
    s: Scene,
    /// alice's data, as the server gave it before her deletion.
    alice: Traces,
    /// What `acme` keeps of her: its revisions, her devices' certificates and her wrap.
    kept: Kept,
    /// alice's wrap to bob's recovery key.
    wrap: Item,
    /// alice's application message and Commit, as MLS messages.
    message: Vec<u8>,
    commit: Vec<u8>,
}

impl Filled {
    /// The scene, with GitHub user IDs from `github` on; alice has not deleted her account yet.
    #[track_caller]
    fn new(github: u64) -> Self {
        let (mut s, _) = formed_with(github, &["--monthly-messages-limit", "100"]);
        let general = s.fixture.general;
        for peer in [&mut s.bob, &mut s.carol] {
            acks_all(peer, s.fixture.db());
        }
        let t = &s.fixture.team;
        let to_bob = wrap(general, 1, &t.bob, &t.alice);
        let object = queue_key(general, 1, &t.alice.laptop);
        stores(&mut s.alice, &s.fixture, &to_bob, "alice's wrap to bob");
        stores(
            &mut s.alice,
            &s.fixture,
            &object,
            "alice's queue key object",
        );
        let message = (s.alice.mls.encrypt(general, b"before the deletion")).expect("a message");
        s.alice.stores(&s.fixture, &message, &[]);
        let commit = s.alice.commits(&s.fixture, empty);
        for peer in [&mut s.bob, &mut s.carol] {
            peer.process(&s.fixture, &commit);
        }
        let removal = s.bob.commits(&s.fixture, removing(&[s.alice.id]));
        s.carol.process(&s.fixture, &removal);
        acks_all(&mut s.carol, s.fixture.db());
        fill(&s);
        let t = &s.fixture.team;
        let devices = [t.alice.laptop.id, t.alice.uncertified.id];
        let alice = Traces::of(&t.alice, github, &devices);
        let kept = Kept::of_space(&t.bob.session, t.space, &devices).with_wraps(&[&to_bob]);
        Filled {
            s,
            alice,
            kept,
            wrap: to_bob,
            message,
            commit,
        }
    }

    fn db(&self) -> &TestDb {
        self.s.fixture.db()
    }

    /// alice deletes her account, its deletion is moved back 30 days, and the server erases it.
    #[track_caller]
    fn erased(&self) {
        let t = &self.s.fixture.team;
        deletes(&Api::of(&t.flow), &t.alice);
        move_deletion_back(self.db(), t.alice.account, ERASED_AFTER);
        until_erased(self.db(), t.alice.account, t.space);
    }
}

/// Waits for up to [`ERASURE`] until the server has erased `account`; then checks that its row
/// keeps no sign-up time and that `space`, which it created, keeps no creator.
#[track_caller]
fn until_erased(db: &TestDb, account: AccountId, space: SpaceId) {
    let deadline = Instant::now() + ERASURE;
    let (account, space) = (account.to_string(), space.to_string());
    loop {
        let (erased, created, creator): (bool, Option<String>, Option<String>) =
            query(db, async |conn| {
                sqlx::query_as(
                    "SELECT a.erased, a.created_at::text, s.creator_id::text
                     FROM accounts a, spaces s WHERE a.id = $1::uuid AND s.id = $2::uuid",
                )
                .bind(&account)
                .bind(&space)
                .fetch_one(conn)
                .await
            });
        if erased {
            assert_eq!(created, None, "the erased account's sign-up time");
            assert_eq!(creator, None, "the creator of acme");
            return;
        }
        assert!(Instant::now() < deadline, "not erased after {ERASURE:?}");
        thread::sleep(ERASURE / 120);
    }
}

/// Once alice's account is erased, bob's mailbox holds her Commit, which he has not fetched, but
/// not her application message; once bob acks it, no row holds her data beyond what `acme` keeps:
/// not her queue key object nor any row [`fill`] wrote.
#[test]
fn every_table_is_erased_but_for_handshake_messages_until_fetched() {
    let mut filled = Filled::new(3_304_050_001);

    filled.erased();

    let bob = filled.s.bob.id;
    let waiting = payloads(filled.db(), bob);
    assert_eq!(waiting, vec![filled.commit.clone()], "bob's mailbox");
    acks_all(&mut filled.s.bob, filled.s.fixture.db());
    erased_within(filled.db(), &filled.alice, &filled.kept);
}

/// Once alice's account is erased, bob's session still gets her wrap to his recovery key.
#[test]
fn members_still_get_the_wraps_its_devices_signed() {
    let filled = Filled::new(3_304_050_011);

    filled.erased();

    let f = &filled.s.fixture;
    let wraps = given(&f.team.bob.session, f, WRAPS);
    assert!(
        wraps.contains(&filled.wrap.bytes().to_vec()),
        "alice's wrap to bob"
    );
}
