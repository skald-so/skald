//! The device drops, warning its person, each value that fails a reader check: of a revoked device
//! or one not admitted to the channel, one a proxy replays with a lower revision than one the device
//! accepted, one a proxy moves to another channel, and one listing an agent of another account.

use skald_proto::{AgentState, AgentStatus};

use crate::edits::EXCLUDE_DEVICE;
use crate::kit::received;
use crate::revocations::revoked_by;
use crate::talk::BOB_HOST;
use crate::values::{
    Directory, VALID, assert_not_listed, assert_warned_of_drop, listed_json, listed_note,
    object_of, objects_of, person_lists, sealed, serve_values, status,
};
use crate::world::{ACME, now, run};

/// The certificate `name` of Bob's second machine.
const BOB_LAPTOP: &str = "bob-laptop";

/// The agent `address` available since a minute ago, with `note`.
fn available(address: &str, note: &str) -> AgentStatus {
    status(
        address,
        AgentState::Available,
        now() - 60,
        (Some(note), None),
    )
}

/// How Bob's second machine loses its standing before its value comes.
enum Barred {
    /// Bob's first device revokes it.
    Revoked,
    /// Ada's `skald exclude-device` bars it from `acme`.
    Excluded,
}

/// Bob's second machine, played, is added to the channel and uploads its value listing Bob's
/// `claude-b4r7ve`, which `skald agents --json` shows. Then it is barred as `barred` says, and Ada's
/// machine runs. A proxy gives Ada's machine, beside the server's values, one the test seals as
/// Bob's second machine with its key, under the channel's current epoch, listing Bob's
/// `claude-a9f2tw`. The person's `skald agents` lists no `claude-a9f2tw` and warns of the value of
/// `bob-laptop`.
fn drops_the_value_of_a_barred_device(ids: [u64; 2], logins: [&str; 2], barred: Barred) {
    let mut directory = Directory::new(ids, logins);
    let mut second = directory.second_machine(true, BOB_LAPTOP);
    let at = directory.at();
    let before = directory.bob_agent("claude-b4r7ve");
    let listed = available(&before, "barred test: the laptop runs the suite");
    directory.uploads(&mut second, (at, 1), vec![listed]);
    let objects = listed_json(directory.machine(), directory.folder());
    object_of(&objects, &before, second.device());

    match barred {
        Barred::Revoked => {
            let bob = &directory.agents.talk.played[0].person;
            revoked_by(directory.cloud(), bob, second.device());
        }
        Barred::Excluded => {
            let device = second.device().to_string();
            let excluded = run(directory.machine(), &[EXCLUDE_DEVICE, ACME, &device]);
            assert!(excluded.success, "not excluded: {excluded:?}");
        }
    }
    received(directory.machine());
    let after = directory.bob_agent("claude-a9f2tw");
    let talk = &mut directory.agents.talk;
    let (cloud, bob) = (&talk.world.cloud, &mut talk.played[0]);
    bob.take_in(cloud);
    let listed = available(&after, "barred test: the laptop still runs the suite");
    let value = sealed(
        &bob.mls,
        at,
        &second.person,
        (2, now() + VALID),
        vec![listed],
    );
    serve_values(cloud, at, (&[], &[value]));

    let shown = person_lists(directory.machine(), directory.folder());

    assert_not_listed(&shown, "claude-a9f2tw");
    assert_warned_of_drop(&shown, (second.device(), BOB_LAPTOP));
}

#[test]
fn drops_the_value_of_a_revoked_device() {
    drops_the_value_of_a_barred_device(
        [4_214_004_001, 4_214_004_002],
        ["ada-drops-a-revoked-value", "bob-revokes-his-laptop"],
        Barred::Revoked,
    );
}

#[test]
fn drops_the_value_of_a_device_not_admitted_to_the_channel() {
    drops_the_value_of_a_barred_device(
        [4_214_005_001, 4_214_005_002],
        ["ada-drops-an-excluded-value", "bob-has-his-laptop-excluded"],
        Barred::Excluded,
    );
}

/// The notes of Bob's two values; neither is anywhere else.
const FIRST_NOTE: &str = "replay test: starting the review";
const SECOND_NOTE: &str = "replay test: the review is done";

/// Bob's device uploads its value of revision 1, then of revision 2, each with its own note for his
/// agent; `skald agents --json` shows each in turn. Then a proxy gives Ada's machine the value of
/// revision 1 in place of the server's. The person's `skald agents` shows the note of revision 1
/// nowhere and warns of the value of `bob-workstation`.
#[test]
fn drops_a_replayed_value_of_a_lower_revision_than_one_accepted() {
    let mut directory = Directory::new(
        [4_214_006_001, 4_214_006_002],
        ["ada-sees-a-replay", "bob-is-replayed"],
    );
    let at = directory.at();
    let agent = directory.bob_agent("claude-r5p8ka");
    let bob_device = directory.bob().device();
    let mut values = Vec::new();
    for (revision, said) in [(1, FIRST_NOTE), (2, SECOND_NOTE)] {
        values.push(directory.bob_uploads((at, revision), vec![available(&agent, said)]));
        let (machine, folder) = (directory.machine(), directory.folder());
        let note = listed_note(machine, folder, &agent, bob_device);
        assert_eq!(note.as_deref(), Some(said), "revision {revision}");
    }
    serve_values(directory.cloud(), at, (&[bob_device], &values[..1]));

    let shown = person_lists(directory.machine(), directory.folder());

    assert_not_listed(&shown, FIRST_NOTE);
    assert_warned_of_drop(&shown, (bob_device, BOB_HOST));
}

/// Bob's device uploads in the default channel its value listing his agent `claude-m3v6qe`. A proxy
/// leaves it out of the default channel's values and gives it among the channel's. The person's
/// `skald agents` lists no `claude-m3v6qe` and warns of the value of `bob-workstation`.
#[test]
fn drops_a_value_a_proxy_moves_to_another_channel() {
    let mut directory = Directory::new(
        [4_214_007_001, 4_214_007_002],
        ["ada-sees-a-moved-value", "bob-is-moved"],
    );
    let (default, at) = (directory.default_at(), directory.at());
    let agent = directory.bob_agent("claude-m3v6qe");
    let bob_device = directory.bob().device();
    let listed = available(&agent, "moved test: watching the default channel");
    let moved = directory.bob_uploads((default, 1), vec![listed]);
    serve_values(directory.cloud(), default, (&[bob_device], &[]));
    serve_values(directory.cloud(), at, (&[], &[moved]));

    let shown = person_lists(directory.machine(), directory.folder());

    assert_not_listed(&shown, "claude-m3v6qe");
    assert_warned_of_drop(&shown, (bob_device, BOB_HOST));
}

/// Bob's device uploads its value listing his agent `claude-h2n9wd` and an agent of Ada's,
/// `codex-i4m6pz`. The person's `skald agents` lists neither, `--json` has neither, and it warns of
/// the value of `bob-workstation`.
#[test]
fn drops_a_value_listing_an_agent_of_another_account() {
    let mut directory = Directory::new(
        [4_214_008_001, 4_214_008_002],
        ["ada-is-impersonated", "bob-lists-adas-agent"],
    );
    let at = directory.at();
    let honest = directory.bob_agent("claude-h2n9wd");
    let impostor = directory.ada_agent("codex-i4m6pz");
    let bob_device = directory.bob().device();
    directory.bob_uploads(
        (at, 1),
        vec![
            available(&honest, "impostor test: bob's own agent"),
            available(&impostor, "impostor test: not bob's agent"),
        ],
    );

    let objects = listed_json(directory.machine(), directory.folder());
    let shown = person_lists(directory.machine(), directory.folder());

    for agent in [&honest, &impostor] {
        assert!(
            objects_of(&objects, agent).is_empty(),
            "{agent}: {objects:#?}"
        );
    }
    assert_not_listed(&shown, "claude-h2n9wd");
    assert_not_listed(&shown, "codex-i4m6pz");
    assert_warned_of_drop(&shown, (bob_device, BOB_HOST));
}
