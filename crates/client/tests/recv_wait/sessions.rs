//! Sessions that wait through one helper, each for its own messages; the person's wait for anything
//! the person's `skald recv` shows; a wait with messages already there; waits that reach the server
//! for nothing while nothing comes; a session that a command beside it gives its messages; and
//! messages that wake no session they are not for.

use std::time::Instant;

use crate::agents::{CLAUDE, CODEX};
use crate::cloud::alone;
use crate::fixtures::{await_running, person_address};
use crate::helped::{Helped, VERSION_SETTING, WEBSOCKET, older, times, with_client_b};
use crate::kit::{RECV, bind_client_a, received};
use crate::machine::Finished;
use crate::talk::{
    BOB_HOST, Line, Talk, assert_shows_once, delivered, encrypted, one_message_sent, stored,
};
use crate::waits::{
    ARRIVAL, AT_ONCE, SHOWN_WITHIN, TRACE_SETTING, TRACED, WAIT_FLAG, Waits, assert_got,
    assert_idle, assert_keeps_waiting, assert_shown_within, await_connections,
};
use crate::world::{ACME, ADD_RULE, AGENT, CLIENT_A, CLIENT_B, dir_of, path, run};

/// What Bob's device says in each space; none is anywhere else.
const BOB_IN_A: &str = "wait test: clienta's invoices 7 and 9 are open";
const BOB_IN_B: &str = "wait test: clientb's audit starts monday";

/// What Bob's device says to claude, what Ada sends, and what waits for a run.
const TO_CLAUDE: &str = "wait test: claude, rerun the flaky suite";
const ADA_SAYS: &str = "wait test: the release is tagged";
const WAITING: [&str; 2] = [
    "wait test: the first report is in",
    "wait test: the second report is in",
];
const AFTER_QUIET: &str = "wait test: the backup finished overnight";
const KEPT_FOR_CLAUDE: &str = "wait test: claude, the acme invoices are due";
const FOR_ACME: &str = "wait test: acme's standup moved to ten";
const TO_CODEX: &str = "wait test: codex, review the migration";

/// Ada's machine follows `clienta` and `clientb`, each with a channel that holds Bob's device;
/// `~/work/clientA` is bound to `clienta` and `~/work/clientB` to `clientb`, and the device has
/// run as claude in each. Through one helper, claude waits in both folders, and both keep waiting
/// while nothing comes. Bob's device says something in clienta's channel: the session in
/// `~/work/clientA` shows it within [`ARRIVAL`] and exits, the one in `~/work/clientB` keeps
/// waiting. Bob's device then says something in clientb's channel: that session shows it within
/// [`ARRIVAL`] and exits. Each session showed only its own message, once; the same helper served
/// every run, and the server saw one connection of Ada's machine for them all. No other test's
/// cloud runs meanwhile, so the times are those of the client, not of other tests' processes.
#[test]
fn sessions_in_two_folders_wait_through_one_helper_and_each_gets_only_its_own_messages() {
    let _alone = alone();
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_148_011_001, 4_148_011_002],
        &["ada-waits-in-two-folders", "bob-writes-to-two-clients"],
        false,
    );
    let (other, elsewhere) = with_client_b(&mut talk);
    let in_a = bind_client_a(&talk.ada.machine);
    let in_b = dir_of(&talk.ada.machine, "work/clientB");
    let bound = run(&talk.ada.machine, &[ADD_RULE, path(&in_b), CLIENT_B]);
    assert!(bound.success, "not bound: {bound:?}");
    let helped = Helped::new(&talk.ada.machine);
    let since = talk.cloud().relay.mark();
    for folder in [&in_a, &in_b] {
        let ran = helped.run_in(folder, &[AGENT, CLAUDE, RECV]);
        assert!(ran.success, "{ran:?}");
    }
    let helper = helped.one_helper();
    let wait = [AGENT, CLAUDE, RECV, WAIT_FLAG];
    let traced = [(TRACE_SETTING, "1")];
    let mut session_a = helped.start_in(&in_a, &traced, &wait);
    let mut session_b = helped.start_in(&in_b, &traced, &wait);
    assert_keeps_waiting(&mut session_a);
    assert_keeps_waiting(&mut session_b);
    let bob = &mut talk.played[0];
    let from_bob = [
        person_address(CLIENT_A, &bob.person),
        person_address(CLIENT_B, &bob.person),
    ];

    let in_clienta = (talk.space.id, talk.channel);
    let said = encrypted(
        bob,
        in_clienta,
        talk.channel,
        &Line::new(&from_bob[0], 1, BOB_IN_A),
    );
    stored(&talk.world.cloud, bob, &said);

    assert_shown_within(&session_a, BOB_IN_A, (Instant::now(), ARRIVAL));
    session_b.assert_running();
    let in_clientb = (other.id, elsewhere);
    let said = encrypted(
        bob,
        in_clientb,
        elsewhere,
        &Line::new(&from_bob[1], 1, BOB_IN_B),
    );
    stored(&talk.world.cloud, bob, &said);
    assert_shown_within(&session_b, BOB_IN_B, (Instant::now(), ARRIVAL));

    let got_a = session_a.finish();
    let got_b = session_b.finish();
    assert_only(&got_a, BOB_IN_A, BOB_IN_B);
    assert_only(&got_b, BOB_IN_B, BOB_IN_A);
    assert_eq!(helped.helpers(), [helper], "not the same helper");
    // Bob's device, which the test plays, opens one through the same relay for each of its two
    // messages.
    assert_eq!(
        talk.cloud().count_sent(since, WEBSOCKET),
        1 + 2,
        "not one connection for every session"
    );
}

/// Fails unless `finished` succeeded, showing `own` once and nothing of `other`.
#[track_caller]
fn assert_only(finished: &Finished, own: &str, other: &str) {
    assert!(finished.success, "{finished:?}");
    let shown = finished.shown();
    assert_eq!(times(&shown, own), 1, "{own:?} not shown once: {shown}");
    assert!(!shown.contains(other), "it shows {other:?}: {shown}");
}

/// The device has run as claude in `~/work/acme`, through a helper, and the person's `skald recv`
/// has shown what came so far. The person's `skald recv --wait` keeps waiting while nothing comes;
/// Bob's device sends a message to claude, and the person's wait shows it once and exits. Ada's
/// `skald send` then sends a message, and her `skald recv` shows what came so far. Her next
/// `skald recv --wait` keeps waiting while nothing comes; once Bob's device reads her message, it
/// shows `delivered` for it from his device, once, and exits.
#[test]
fn person_wait_shows_the_first_message_whatever_agent_it_is_for_or_the_first_status() {
    let mut waits = Waits::new(
        [4_148_051_001, 4_148_051_002],
        ["ada-waits-as-herself", "bob-writes-to-claude"],
    );
    waits.helped_as(CLAUDE);
    waits.helped.received();
    let mut person = waits.helped.start(&[], &[RECV, WAIT_FLAG]);
    assert_keeps_waiting(&mut person);
    let bob = waits.agents.bob();
    let mut line = Line::new(&bob, 1, TO_CLAUDE);
    line.to = Some(waits.agents.address(CLAUDE));

    waits.agents.bob_sends(&line);

    let first = person.finish();
    assert!(first.success, "{first:?}");
    let channel = waits.agents.talk.channel;
    assert_shows_once(&first.shown(), TO_CLAUDE, (channel, &bob, BOB_HOST));

    let talk = &waits.agents.talk;
    let from = talk.front.sent().len();
    waits.helped.sent(talk, ADA_SAYS);
    let msg_id = one_message_sent(&talk.front, channel, from);
    waits.helped.received();
    let mut statuses = waits.helped.start(&[], &[RECV, WAIT_FLAG]);
    assert_keeps_waiting(&mut statuses);
    let bob_device = waits.agents.talk.played[0].device();

    waits.agents.talk.played[0].take_in(&waits.agents.talk.world.cloud);

    let status = statuses.finish();
    assert!(status.success, "{status:?}");
    assert_eq!(
        delivered(&status.shown(), msg_id, bob_device),
        1,
        "delivered is not shown once: {status:?}"
    );
}

/// The device has run as claude in `~/work/acme`, in one-shot mode, and Bob's device sends two
/// messages to the channel. The person's `skald recv --wait` in one-shot mode shows both, each
/// once, within [`SHOWN_WITHIN`] of its start, and exits within [`AT_ONCE`] of showing them; so
/// does claude's `skald recv --wait`, through the helper it starts, each by its reference.
#[test]
fn wait_with_messages_waiting_shows_them_and_exits_at_once() {
    let mut waits = Waits::new(
        [4_148_081_001, 4_148_081_002],
        ["ada-finds-mail-waiting", "bob-left-two-reports"],
    );
    waits.agents.run_as(CLAUDE);
    let bob = waits.agents.bob();
    let ids: Vec<_> = (1..)
        .zip(WAITING)
        .map(|(counter, text)| waits.agents.bob_sends(&Line::new(&bob, counter, text)))
        .collect();
    let channel = waits.agents.talk.channel;

    let person = waits.agents.talk.ada.machine.start(&[RECV, WAIT_FLAG]);
    let (person, [shown, exited]) = person.finish_after_showing(&WAITING);

    assert!(person.success, "{person:?}");
    for text in WAITING {
        assert_shows_once(&person.shown(), text, (channel, &bob, BOB_HOST));
    }
    assert!(
        shown < SHOWN_WITHIN && exited < AT_ONCE,
        "the person's wait showed them after {shown:?}, exited {exited:?} later: {person:?}"
    );

    let (agent, [shown, exited]) = waits.session(CLAUDE).finish_after_showing(&WAITING);

    for (text, id) in WAITING.iter().zip(&ids) {
        assert_got(&agent, text, *id);
    }
    assert!(
        shown < SHOWN_WITHIN && exited < AT_ONCE,
        "claude's wait showed them after {shown:?}, exited {exited:?} later: {agent:?}"
    );
}

/// The device has run as claude in `~/work/acme`, and the person's `skald recv` has shown what came
/// so far. While nothing comes, the server sees no request of the person's `skald recv --wait` in
/// one-shot mode, nor, once claude's `skald recv --wait` in `~/work/acme` has started a helper, of
/// the two waits. Bob's device then sends a message to the channel: claude's wait shows it once,
/// with its `msg_id`, and so does the person's.
#[test]
fn waits_reach_the_server_for_nothing_while_nothing_comes() {
    let mut waits = Waits::new(
        [4_148_091_001, 4_148_091_002],
        ["ada-waits-quietly", "bob-writes-at-last"],
    );
    waits.agents.run_as(CLAUDE);
    received(&waits.agents.talk.ada.machine);
    let talk = &waits.agents.talk;
    let cloud = talk.cloud();
    let since = cloud.relay.mark();
    let mut person = talk.ada.machine.start(&[RECV, WAIT_FLAG]);
    await_connections(talk, &mut person, (since, 1));
    // This local read takes the state lock once the one-shot receive has finished.
    let fingerprint = run(&talk.ada.machine, &["fingerprint"]);
    assert!(fingerprint.success, "{fingerprint:?}");
    assert_idle(talk, &mut [&mut person]);

    let since = cloud.relay.mark();
    let mut session = waits.session(CLAUDE);
    // Either wait can reach the helper first. The person's receive lists its spaces, and the
    // helper uploads presence after the agent's receive joins it.
    await_running(
        &mut person,
        "the person's receive through the helper",
        || cloud.count_sent(since, "GET /spaces ") > 0,
    );
    let presence = format!(
        "PUT /spaces/{}/channels/{}/presence ",
        talk.space.id, talk.channel
    );
    await_running(&mut session, "the agent's presence upload", || {
        cloud.count_sent(since, &presence) > 0
    });
    // The helper greets this command after the initial receives. The older command ends at the
    // greeting, leaving both waits idle.
    let probe = waits
        .helped
        .run_with(&[(VERSION_SETTING, &older())], &["fingerprint"]);
    assert!(
        !probe.success && probe.shown().contains("is older than the helper"),
        "{probe:?}"
    );
    assert_idle(talk, &mut [&mut person, &mut session]);
    let bob = waits.agents.bob();

    let id = waits.agents.bob_sends(&Line::new(&bob, 1, AFTER_QUIET));

    assert_got(&session.finish(), AFTER_QUIET, id);
    let shown = person.finish();
    assert!(shown.success, "{shown:?}");
    let channel = waits.agents.talk.channel;
    assert_shows_once(&shown.shown(), AFTER_QUIET, (channel, &bob, BOB_HOST));
}

/// The device has run as claude in `~/work/acme`; Bob's device sends a message to the channel, and
/// the person's `skald recv` shows it, keeping it for claude. claude's `skald recv --wait` in
/// `~/work/other`, in the personal space, starts a helper and keeps waiting. `skald add-rule` binds
/// `~/work/other` to `acme` through the helper: the session shows the message once, with its
/// `msg_id`, and exits.
#[test]
fn session_gets_the_messages_a_command_through_its_helper_makes_its_own() {
    let mut waits = Waits::new(
        [4_148_092_001, 4_148_092_002],
        ["ada-moves-her-agent", "bob-writes-to-acme"],
    );
    waits.agents.run_as(CLAUDE);
    let bob = waits.agents.bob();
    let id = waits.agents.bob_sends(&Line::new(&bob, 1, KEPT_FOR_CLAUDE));
    received(&waits.agents.talk.ada.machine);
    let other = dir_of(&waits.agents.talk.ada.machine, "work/other");
    let mut session = (waits.helped).start_in(&other, &[], &[AGENT, CLAUDE, RECV, WAIT_FLAG]);
    assert_keeps_waiting(&mut session);

    let bound = waits.helped.run(&[ADD_RULE, path(&other), ACME]);

    assert!(bound.success, "not bound: {bound:?}");
    assert_got(&session.finish(), KEPT_FOR_CLAUDE, id);
}

/// The device has run as claude in `~/work/acme`. Through one helper, claude waits in
/// `~/work/acme`, and codex in `~/work/other`, in the personal space. Bob's device sends a message
/// to acme's channel: claude's session shows it once, with its `msg_id`, and exits, the helper
/// having asked for no KeyPackages, GroupInfo, device list or server name, nor long-polled, on the
/// way; codex's keeps
/// waiting, the server seeing no request meanwhile, and its trace shows no intake: the message woke
/// no session of another space.
#[test]
fn message_wakes_no_session_of_another_space() {
    let mut waits = Waits::new(
        [4_148_093_001, 4_148_093_002],
        ["ada-runs-two-agents", "bob-writes-to-acme-only"],
    );
    waits.agents.run_as(CLAUDE);
    let other = dir_of(&waits.agents.talk.ada.machine, "work/other");
    let mut claude = waits.session(CLAUDE);
    let traced = [(TRACE_SETTING, "1")];
    let mut codex = (waits.helped).start_in(&other, &traced, &[AGENT, CODEX, RECV, WAIT_FLAG]);
    assert_idle(&waits.agents.talk, &mut [&mut claude, &mut codex]);
    let bob = waits.agents.bob();
    let sending = waits.agents.talk.cloud().relay.mark();

    let id = waits.agents.bob_sends(&Line::new(&bob, 1, FOR_ACME));

    assert_got(&claude.finish(), FOR_ACME, id);
    let relay = &waits.agents.talk.cloud().relay;
    let on_the_way: Vec<String> = (relay.sent_since(sending).iter())
        .filter_map(|sent| sent.head.lines().next().map(str::to_owned))
        .filter(|line| {
            [
                "/key-packages",
                "/group-info",
                "get /devices ",
                "get /server ",
                "post /mailbox/poll",
            ]
            .iter()
            .any(|path| line.contains(path))
        })
        .collect();
    assert!(on_the_way.is_empty(), "asked on the way: {on_the_way:?}");
    let from = relay.mark();
    assert_keeps_waiting(&mut codex);
    let sent: Vec<String> = (relay.sent_since(from).iter())
        .filter_map(|sent| sent.head.lines().next().map(str::to_owned))
        .collect();
    assert!(sent.is_empty(), "codex's session ran: {sent:?}");
    let shown = codex.shown();
    assert!(
        !shown.contains(FOR_ACME) && !shown.contains(TRACED),
        "{shown}"
    );
}

/// The device has run as claude and codex in `~/work/acme`, and both wait there through one helper.
/// Bob's device sends a message to codex: codex's session shows it once, with its `msg_id`, and
/// exits; claude's keeps waiting, having shown nothing of it but, as it is traced, the intake's
/// steps.
#[test]
fn message_to_another_agent_of_the_space_wakes_no_session_of_its_own() {
    let mut waits = Waits::new(
        [4_148_094_001, 4_148_094_002],
        ["ada-runs-claude-and-codex", "bob-writes-to-codex"],
    );
    waits.agents.run_as(CLAUDE);
    waits.agents.run_as(CODEX);
    let mut claude = waits.session_with(&[(TRACE_SETTING, "1")], CLAUDE);
    let mut codex = waits.session(CODEX);
    assert_keeps_waiting(&mut claude);
    assert_keeps_waiting(&mut codex);
    let mut line = Line::new(&waits.agents.bob(), 1, TO_CODEX);
    line.to = Some(waits.agents.address(CODEX));

    let id = waits.agents.bob_sends(&line);

    let got = codex.finish();
    assert_got(&got, TO_CODEX, id);
    assert!(
        !got.shown().contains(TRACED),
        "untraced, it traces: {got:?}"
    );
    assert_keeps_waiting(&mut claude);
    let shown = claude.shown();
    assert!(!shown.contains(TO_CODEX), "{shown}");
    assert!(
        shown.contains(&format!("{TRACED}handing over at ")),
        "{shown}"
    );
}
