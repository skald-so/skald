//! Account key changes: changed master keys, conflicting master keys and reassigned handles.

use skald_crypto::account_fingerprint;
use skald_proto::Role;

use crate::agents::{ACME_DIR, CLAUDE};
use crate::kit::assert_names_none;
use crate::terminal::Terminal;
use crate::world::{
    ACME, ADD_RULE, AGENT, GLOBEX, MEMBERS, World, create, dir_of, members, next_by, path, run,
    store, words,
};
use crate::{
    BOB_HOST, CAROL_HOST, Pair, assert_says_none, assert_says_waiting, assert_waits_and_shows_once,
    member_with_master, other_master, seen_at_terminal,
};

/// The command that shows a fingerprint and asks whether it matches, and the person's answer.
const FINGERPRINT: &str = "fingerprint";
const YES: &str = "yes";

/// What the key change of a verified account says.
const VERIFIED: &str = "verified";

/// What the key change of the own account's master key says.
const MASTER: &str = "master";

/// A fixture revision 4, signed by Bob's device, gives Ada another `master`; Bob's `skald members
/// acme` accepts it.
fn adas_master_changes(pair: &Pair) {
    let fourth = next_by(&pair.third, &pair.bob, |list| {
        let ada = (list.iter_mut())
            .find(|member| member.account == pair.ada.account)
            .expect("Ada");
        ada.master = other_master();
    });
    store(&pair.world.cloud, &pair.bob.token, &fourth);
    members(&pair.bob.machine, ACME);
}

/// A revision that changes Ada's `master` leaves a key change on Bob's machine that waits and
/// shows once, naming her.
#[test]
fn a_members_changed_master_key_waits_until_seen() {
    let pair = Pair::new(
        [4_188_101_001, 4_188_101_002],
        ["ada-whose-master-changes", "bob-sees-adas-master-change"],
    );

    adas_master_changes(&pair);

    let machine = &pair.bob.machine;
    assert_waits_and_shows_once(machine, &[MEMBERS, ACME], &pair.adas_names());
}

/// Bob verified Ada at a terminal; a revision that changes her `master` leaves a key change that
/// waits and shows once, naming her and saying `verified`.
#[test]
fn a_verified_members_changed_master_key_waits_until_seen() {
    let pair = Pair::new(
        [4_188_102_001, 4_188_102_002],
        ["ada-trusted-then-changed", "bob-checked-ada-before"],
    );
    let machine = &pair.bob.machine;
    let mut asked = Terminal::start(machine, &[FINGERPRINT, ACME, pair.ada.handle.as_str()]);
    asked.wait_to_show("?");
    asked.type_line(YES);
    let verified = asked.finish();
    assert!(verified.success, "{}", verified.text);
    assert_says_none(&verified.text);

    adas_master_changes(&pair);

    let shown = assert_waits_and_shows_once(machine, &[MEMBERS, ACME], &pair.adas_names());
    assert!(
        shown.text.to_lowercase().contains(VERIFIED),
        "the key change does not say Ada was verified: {}",
        shown.text
    );
}

/// Bob verifies Ada. As the space's admin, he assigns her handle to Carol in the next revision.
/// His machine shows Carol's fingerprint without Ada's verification and keeps a key change.
#[test]
fn a_verified_members_handle_assigned_to_another_account_leaves_a_key_change() {
    let pair = Pair::new(
        [4_188_105_001, 4_188_105_002],
        ["ada-before-reassignment", "bob-checks-reassignment"],
    );
    let machine = &pair.bob.machine;
    let handle = pair.ada.handle.as_str();
    let original = account_fingerprint(pair.ada.account, &pair.ada.master);
    let mut terminal = Terminal::start(machine, &[FINGERPRINT, ACME, handle]);
    terminal.wait_to_show("?");
    terminal.type_line(YES);
    let verified = terminal.finish();
    assert!(verified.success, "{}", verified.text);
    assert!(verified.text.contains(&original), "{}", verified.text);
    let before = members(machine, ACME);
    assert!(
        before
            .stdout
            .lines()
            .any(|line| { line.contains(&original) && words(line).contains(&VERIFIED) }),
        "Ada's fingerprint is not verified: {before:?}"
    );

    let carol = pair
        .world
        .sign_up(4_188_105_003, "carol-after-reassignment", CAROL_HOST);
    let fourth = next_by(&pair.third, &pair.bob, |list| {
        let member = list
            .iter_mut()
            .find(|member| member.account == pair.ada.account)
            .expect("Ada");
        let mut replacement = carol.member(Role::Member);
        replacement.handle = member.handle.clone();
        *member = replacement;
    });
    store(&pair.world.cloud, &pair.bob.token, &fourth);

    let fingerprint = account_fingerprint(carol.account, &carol.master);
    assert_ne!(fingerprint, original);
    let after = members(machine, ACME);
    assert!(
        after
            .stdout
            .lines()
            .any(|line| { line.contains(&fingerprint) && !words(line).contains(&VERIFIED) }),
        "Carol's fingerprint is not shown without verification: {after:?}"
    );
    assert!(
        !after.stdout.contains(&original),
        "Ada's fingerprint is still shown: {after:?}"
    );
    let names = [
        handle.to_owned(),
        pair.ada.account.to_string(),
        carol.account.to_string(),
    ];
    assert_waits_and_shows_once(machine, &[MEMBERS, ACME], &names);
}

/// An agent discovers a handle reassignment while listing members. The person sees both accounts
/// in the terminal notice. Once acknowledged, the count is gone from the agent's next listing.
#[test]
fn a_handle_reassignment_shows_only_the_count_to_the_agent() {
    let pair = Pair::new(
        [4_188_106_001, 4_188_106_002],
        ["ada-before-agent-notice", "bob-reads-agent-notice"],
    );
    let (world, bob, ada) = (&pair.world, &pair.bob, &pair.ada);
    let machine = &bob.machine;
    let folder = dir_of(machine, ACME_DIR);
    let bound = run(machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "{bound:?}");

    let carol = world.sign_up(4_188_106_003, "carol-after-agent-notice", CAROL_HOST);
    let mut replacement = carol.member(Role::Member);
    replacement.handle = ada.handle.clone();
    let next = next_by(&pair.third, bob, |list| {
        let former = list.iter_mut().find(|member| member.account == ada.account);
        *former.expect("Ada") = replacement;
    });
    store(&world.cloud, &bob.token, &next);

    let handle = ada.handle.as_str();
    let accounts = [ada.account.to_string(), carol.account.to_string()];
    let list = || {
        let listed = machine
            .start_in(&folder, &[AGENT, CLAUDE, MEMBERS, ACME])
            .finish();
        assert!(listed.success, "{listed:?}");
        listed
    };
    let listed = list();
    let summary = assert_says_waiting(&listed.shown(), Some(1));
    assert!(
        !summary.contains(ACME) && !summary.contains(handle),
        "{summary}"
    );
    assert_names_none(&listed, &accounts);

    let shown = seen_at_terminal(machine);
    assert!(
        shown.text.lines().any(|line| {
            line.contains(handle) && accounts.iter().all(|account| line.contains(account))
        }),
        "the notice does not name the handle and both accounts: {}",
        shown.text
    );
    assert_says_none(&list().shown());
}

/// Bob's machine creates `acme` and `globex`; revision 3 of `acme` adds Ada with her master key and
/// revision 3 of `globex` with another. His machine accepts both: a key change waits and shows
/// once, naming Ada.
#[test]
fn one_account_with_two_master_keys_waits_until_seen() {
    let pair = Pair::new(
        [4_188_103_001, 4_188_103_002],
        ["ada-with-two-masters", "bob-sees-two-masters-of-ada"],
    );
    let (cloud, bob) = (&pair.world.cloud, &pair.bob);
    let second = create(cloud, bob, GLOBEX);
    let third = next_by(&second, bob, |list| {
        list.push(member_with_master(&pair.ada, Role::Member, other_master()));
    });
    store(cloud, &bob.token, &third);

    members(&bob.machine, GLOBEX);

    assert_waits_and_shows_once(&bob.machine, &[MEMBERS, ACME], &pair.adas_names());
}

/// Revision 3 of Bob's `acme`, signed by his device, lists his own account with another master key
/// than the one his machine pinned: a key change waits and shows once, saying `master`.
#[test]
fn the_own_account_with_another_master_key_waits_until_seen() {
    let world = World::start();
    let bob = world.sign_up(4_188_104_001, "bob-own-master-swapped", BOB_HOST);
    let second = create(&world.cloud, &bob, ACME);
    assert_says_none(&members(&bob.machine, ACME).shown());
    let third = next_by(&second, &bob, |list| {
        let own = (list.iter_mut())
            .find(|member| member.account == bob.account)
            .expect("Bob");
        own.master = other_master();
    });
    store(&world.cloud, &bob.token, &third);

    members(&bob.machine, ACME);

    assert_waits_and_shows_once(&bob.machine, &[MEMBERS, ACME], &[MASTER.to_owned()]);
}
