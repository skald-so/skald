//! History retention (`docs/protocol/membership.md`, "Uploads"; `docs/dev/architecture.md`, "Cloud
//! limits", "History kept"): with the setting, the server deletes by itself each entry stored longer
//! ago than the retention and, once it has deleted an entry of an epoch, the links and wraps of every
//! earlier epoch; those of that epoch and later ones stay, the current epoch's included, whatever
//! their own age. It keeps that cutoff: a link or wrap of an earlier epoch uploaded later is refused
//! and not stored. A channel with no entries keeps its links and wraps. Without the setting, the
//! history is kept for good, as a self-hosted server applies no plan limits.
//!
//! The interfaces, as these tests use them, besides those of `main.rs`:
//! - The setting `--history-retention <SECONDS>`, also `SKALD_HISTORY_RETENTION`; unset, there is
//!   no retention.
//! - The server keeps the time it stored an entry in a `timestamptz` column of the row that holds
//!   the entry's bytes, and reads it there: the tests move it back to let time pass. Links and wraps
//!   may have times too; the tests move those back as well.
//! - The server deletes what the retention deletes within 30 seconds, with no connection of any
//!   device.
//!
//! bob, a member of every epoch, uploads every item unless a test says otherwise.

use std::thread;
use std::time::{Duration, Instant};

use crate::api::Session;
use crate::common::hex;
use crate::common::times::{DAY, YEAR};
use crate::fixture::Fixture;
use crate::kit::{
    ENTRIES, Item, LINKS, WRAPS, assert_history, bob_rotates, entry, formed, formed_with, given,
    link, refuses, stores, wrap,
};
use crate::scene::{Scene, empty};
use crate::wire::assert_nowhere;

/// The server setting of the history retention, in seconds; unset, there is none.
const RETENTION: &str = "--history-retention";
/// The environment variable of the same setting.
const RETENTION_ENV: &str = "SKALD_HISTORY_RETENTION";

/// The retention the tests set: the cloud's (`docs/dev/architecture.md`, "Cloud limits").
const KEPT: u64 = 90 * DAY;

/// How soon the server deletes by itself what the retention deletes.
const DELETION: Duration = Duration::from_secs(30);
/// How often a test looks whether the server has deleted it.
const LOOK: Duration = Duration::from_millis(250);

/// The scene of `main.rs` with the channel's group formed at epoch 1, on a server with the
/// retention [`KEPT`]; GitHub user IDs from `github` on.
fn retained(github: u64) -> Scene {
    let kept = KEPT.to_string();
    formed_with(github, &[RETENTION, &kept]).0
}

/// One statement that moves back by `seconds` each `timestamptz` column of every row holding
/// `bytes`, as if that much time had passed since the server stored them; if `required`, it fails
/// when no such row has one.
fn moved_back_holding(bytes: &[u8], seconds: u64, required: bool) -> String {
    let hex = hex(bytes);
    format!(
        "DO $$
         DECLARE
           timed record;
           n bigint;
           moved bigint := 0;
         BEGIN
           FOR timed IN
             SELECT c.table_name, c.column_name FROM information_schema.columns c
             JOIN information_schema.tables t USING (table_schema, table_name)
             WHERE c.table_schema = current_schema() AND t.table_type = 'BASE TABLE'
               AND c.data_type = 'timestamp with time zone'
           LOOP
             EXECUTE format(
               'UPDATE %I t SET %I = t.%I - make_interval(secs => {seconds})
                WHERE strpos(t::text, %L) > 0',
               timed.table_name, timed.column_name, timed.column_name, '{hex}');
             GET DIAGNOSTICS n = ROW_COUNT;
             moved := moved + n;
           END LOOP;
           IF {required} AND moved = 0 THEN
             RAISE EXCEPTION 'no stored time of the item %', '{hex}';
           END IF;
         END $$"
    )
}

/// Moves the time the server stored the entry `item` back by `seconds`.
fn age(f: &Fixture, item: &Item, seconds: u64) {
    f.db()
        .execute(&moved_back_holding(item.bytes(), seconds, true));
}

/// Moves the times the server keeps of each of `items`, links and wraps, back by `seconds`, if it
/// keeps any.
fn age_any(f: &Fixture, items: &[&Item], seconds: u64) {
    for item in items {
        f.db()
            .execute(&moved_back_holding(item.bytes(), seconds, false));
    }
}

/// `items`' bytes, sorted, as [`given`] gives them.
fn sorted(items: &[&Item]) -> Vec<Vec<u8>> {
    let mut bytes: Vec<Vec<u8>> = items.iter().map(|item| item.bytes().to_vec()).collect();
    bytes.sort();
    bytes
}

/// Waits up to [`DELETION`] until `session` gets exactly `links`, `wraps` and `entries` of the
/// channel's history, then checks it as [`assert_history`] does.
#[track_caller]
fn await_history(session: &Session, f: &Fixture, expected: [&[&Item]; 3], what: &str) {
    let deadline = Instant::now() + DELETION;
    while Instant::now() < deadline {
        let all = ([LINKS, WRAPS, ENTRIES].iter().zip(expected))
            .all(|(kind, items)| given(session, f, kind) == sorted(items));
        if all {
            break;
        }
        thread::sleep(LOOK);
    }
    assert_history(session, f, expected, what);
}

/// Checks that the database holds none of `items`: the server deleted them.
#[track_caller]
fn assert_deleted(f: &Fixture, items: &[&Item]) {
    for item in items {
        assert_nowhere(f.db(), item.bytes());
    }
}

/// Lets the server delete whatever it would: a test that checks that something stays waits this
/// long first.
fn a_deletion_passes() {
    thread::sleep(DELETION);
}

/// bob commits an empty Commit, which the server stores; alice processes it, and carol's laptop,
/// offline, does not. Gives the new epoch.
#[track_caller]
fn bob_rotates_while_carol_is_offline(s: &mut Scene) -> u64 {
    let f = &s.fixture;
    let commit = s.bob.commits(f, empty);
    s.alice.process(f, &commit);
    (s.bob.mls.epoch(f.general)).expect("bob's epoch")
}

/// bob's wraps of the channel's epochs `epochs` to alice's recovery key.
fn bobs_wraps<const N: usize>(s: &Scene, epochs: [u64; N]) -> [Item; N] {
    let t = &s.fixture.team;
    epochs.map(|epoch| wrap(s.fixture.general, epoch, &t.alice, &t.bob))
}

#[test]
fn an_entry_past_the_retention_is_deleted_and_no_longer_given_and_a_newer_one_stays() {
    let mut s = retained(3_160_810_001);
    let f = &s.fixture;
    let g = f.general;
    let old = entry(g, 1);
    let newer = entry(g, 1);
    stores(&mut s.bob, f, &old, "bob's entry");
    stores(&mut s.carol, f, &newer, "carol's entry");
    age(f, &newer, KEPT - DAY);
    age(f, &old, KEPT + DAY);

    await_history(
        &s.alice.session,
        f,
        [&[], &[], &[&newer]],
        "the newer entry",
    );
    assert_deleted(f, &[&old]);
    assert_history(
        &s.dave.session,
        f,
        [&[], &[], &[&newer]],
        "to another member",
    );
}

#[test]
fn once_an_entry_of_an_epoch_is_deleted_the_links_and_wraps_of_earlier_epochs_are_deleted() {
    let mut s = retained(3_160_810_011);
    for expected in [2, 3, 4] {
        assert_eq!(bob_rotates(&mut s), expected, "bob's empty Commit");
    }
    let g = s.fixture.general;
    // Epoch 4 is the current one.
    let links = [link(g, 2, 1), link(g, 3, 2), link(g, 3, 1), link(g, 4, 3)];
    let wraps = bobs_wraps(&s, [1, 2, 3, 4]);
    let [old, of_3, of_4] = [entry(g, 3), entry(g, 3), entry(g, 4)];
    let f = &s.fixture;
    for item in links.iter().chain(&wraps).chain([&old, &of_3, &of_4]) {
        stores(&mut s.bob, f, item, "bob's upload");
    }
    // Their own age deletes no link or wrap.
    let keys: Vec<&Item> = links.iter().chain(&wraps).collect();
    age_any(f, &keys, YEAR);
    age(f, &old, KEPT + DAY);

    let [l2, l3, l3_from_1, l4] = &links;
    let [w1, w2, w3, w4] = &wraps;
    let kept = [&[l3, l3_from_1, l4][..], &[w3, w4], &[&of_3, &of_4]];
    await_history(&s.alice.session, f, kept, "of epoch 3 and later");
    assert_deleted(f, &[&old, l2, w1, w2]);
}

#[test]
fn a_channel_with_no_entries_keeps_its_links_and_wraps() {
    let mut s = retained(3_160_810_021);
    bob_rotates(&mut s);
    bob_rotates(&mut s);
    let g = s.fixture.general;
    let [l2, l3] = [link(g, 2, 1), link(g, 3, 2)];
    let [w1, w2, w3] = bobs_wraps(&s, [1, 2, 3]);
    let f = &s.fixture;
    let keys = [&l2, &l3, &w1, &w2, &w3];
    for item in keys {
        stores(&mut s.bob, f, item, "bob's upload");
    }
    age_any(f, &keys, 10 * YEAR);

    a_deletion_passes();
    let all = [&[&l2, &l3][..], &[&w1, &w2, &w3], &[]];
    assert_history(
        &s.alice.session,
        f,
        all,
        "with no entries, every link and wrap",
    );

    // The server does look: an entry of the current epoch past the retention deletes those of
    // earlier epochs.
    let past = entry(g, 3);
    stores(&mut s.bob, f, &past, "bob's entry");
    age(f, &past, KEPT + DAY);
    await_history(&s.alice.session, f, [&[&l3], &[&w3], &[]], "of epoch 3");
    assert_deleted(f, &[&past, &l2, &w1, &w2]);
}

#[test]
fn after_the_deletion_a_link_or_wrap_of_an_earlier_epoch_is_refused() {
    let mut s = retained(3_160_810_031);
    assert_eq!(bob_rotates(&mut s), 2, "bob's empty Commit");
    // carol's laptop goes offline at epoch 2.
    for expected in [3, 4] {
        let epoch = bob_rotates_while_carol_is_offline(&mut s);
        assert_eq!(epoch, expected, "bob's empty Commit");
    }
    let g = s.fixture.general;
    let [w1, w2] = bobs_wraps(&s, [1, 2]);
    let [old, of_4] = [entry(g, 3), entry(g, 4)];
    let f = &s.fixture;
    for item in [&w1, &w2, &old, &of_4] {
        stores(&mut s.bob, f, item, "bob's upload");
    }
    age(f, &old, KEPT + DAY);
    await_history(
        &s.alice.session,
        f,
        [&[], &[], &[&of_4]],
        "epoch 3's cutoff",
    );
    assert_deleted(f, &[&old, &w1, &w2]);

    // carol's laptop comes back with what it made before epoch 3.
    let t = &f.team;
    let late = [
        link(g, 2, 1),
        wrap(g, 2, &t.alice, &t.carol),
        wrap(g, 1, &t.dave, &t.carol),
    ];
    s.carol.connect(&t.flow.stand.server, f, &t.carol);
    refuses(&mut s.carol, f, &late[0], "carol's link of epoch 2");

    // The server keeps the cutoff when it starts again.
    let kept = KEPT.to_string();
    let server = s.fixture.team.flow.stand.restart(&[RETENTION, &kept], &[]);
    let f = &s.fixture;
    let t = &f.team;
    s.carol.connect(&server, f, &t.carol);
    s.bob.connect(&server, f, &t.bob);
    for (item, what) in late
        .iter()
        .zip(["link of epoch 2", "wrap of epoch 2", "wrap of epoch 1"])
    {
        refuses(&mut s.carol, f, item, &format!("carol's {what}"));
    }

    // Those of the cutoff's epoch are still taken.
    let l3 = link(g, 3, 2);
    let w3 = wrap(g, 3, &t.alice, &t.bob);
    stores(&mut s.bob, f, &l3, "bob's link of epoch 3");
    stores(&mut s.bob, f, &w3, "bob's wrap of epoch 3");
    let given = [&[&l3][..], &[&w3], &[&of_4]];
    assert_history(&s.alice.session.on(&server), f, given, "epoch 3 on");
}

#[test]
fn the_cutoff_never_moves_back_when_an_entry_of_an_earlier_epoch_is_deleted_later() {
    let mut s = retained(3_160_810_051);
    for expected in [2, 3] {
        assert_eq!(bob_rotates(&mut s), expected, "bob's empty Commit");
    }
    let g = s.fixture.general;
    let [of_3, of_2] = [entry(g, 3), entry(g, 2)];
    let f = &s.fixture;
    stores(&mut s.bob, f, &of_3, "bob's entry of epoch 3");
    stores(&mut s.bob, f, &of_2, "bob's entry of epoch 2");
    age(f, &of_3, KEPT + DAY);
    await_history(
        &s.alice.session,
        f,
        [&[], &[], &[&of_2]],
        "epoch 3's cutoff",
    );

    // Deleted in a later look, the entry of epoch 2 leaves the cutoff at epoch 3.
    age(f, &of_2, KEPT + DAY);
    await_history(&s.alice.session, f, [&[], &[], &[]], "nothing");
    let t = &f.team;
    let late = [link(g, 2, 1), wrap(g, 2, &t.alice, &t.bob)];
    for (item, what) in late.iter().zip(["link of epoch 2", "wrap of epoch 2"]) {
        refuses(&mut s.bob, f, item, &format!("bob's {what}"));
    }
    assert_deleted(f, &[&of_3, &of_2, &late[0], &late[1]]);
}

#[test]
fn without_the_setting_an_entry_moved_back_years_stays() {
    let (mut s, _) = formed(3_160_810_041);
    bob_rotates(&mut s);
    let g = s.fixture.general;
    let l2 = link(g, 2, 1);
    let [w1, w2] = bobs_wraps(&s, [1, 2]);
    let [of_1, of_2] = [entry(g, 1), entry(g, 2)];
    let f = &s.fixture;
    for item in [&l2, &w1, &w2, &of_1, &of_2] {
        stores(&mut s.bob, f, item, "bob's upload");
    }
    age_any(f, &[&l2, &w1, &w2], 10 * YEAR);
    age(f, &of_1, 10 * YEAR);
    age(f, &of_2, 10 * YEAR);

    a_deletion_passes();
    let all = [&[&l2][..], &[&w1, &w2], &[&of_1, &of_2]];
    assert_history(&s.alice.session, f, all, "without the setting, everything");

    // Started again with the setting, from the environment, the server deletes both entries, and
    // the wraps of epochs before the later one's.
    let kept = KEPT.to_string();
    let server = s
        .fixture
        .team
        .flow
        .stand
        .restart(&[], &[(RETENTION_ENV, &kept)]);
    let f = &s.fixture;
    let session = s.alice.session.on(&server);
    await_history(&session, f, [&[&l2], &[&w2], &[]], "with the setting");
    assert_deleted(f, &[&of_1, &of_2, &w1]);
}
