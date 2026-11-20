//! What a stateless run does beyond what `main.rs` lists (`docs/service-agents.md`, "Running
//! without a disk"; `docs/protocol/delivery.md`, "Mailbox" and "Gap detection"):
//!
//! - it keeps no counters, so a run that gets a counter it never saw before from a sending device
//!   does no gap detection and warns of no missing message;
//! - over the event stream and over long-poll, as over a WebSocket, it acks a message only once it
//!   has shown it, and over long-poll it takes one answer;
//! - a run whose GroupInfo request fails, so it cannot check a message's sender, fails and acks
//!   nothing, so the next run shows the message;
//! - every command but `skald send` and `skald recv` without `--agent` or `--wait` refuses, leaving
//!   nothing on the machine's disk.
//!
//! As a proxy that blocks WebSocket or streaming would (`transport_fallback/main.rs`), the relay
//! answers `GET /ws`, and `GET /mailbox/events`, with `403 Forbidden` and no problem details.

use crate::fixtures::send;
use crate::kit::RECV;
use crate::member::JSON;
use crate::scene::{Scene, killed_when, shown_with};
use crate::world::ACME;

/// What Ada says; none is anywhere else.
const ADA_SAYS: [&str; 2] = [
    "stateless runs test: ada asks for the canary",
    "stateless runs test: ada asks for the rollback plan",
];

/// The requests of a WebSocket and of the mailbox's event stream, as their lines start.
const WEBSOCKET: &str = "GET /ws";
const EVENTS: &str = "GET /mailbox/events";

/// The heads of an event stream's request and of a long-poll, in lowercase, as the relay keeps them.
const EVENTS_HEAD: &str = "get /mailbox/events";
const POLL_HEAD: &str = "post /mailbox/poll";

/// An ack over HTTPS, as its line starts.
const ACK: &str = "POST /mailbox/ack";

/// Ada says `text` in the channel; fails unless her send succeeds.
#[track_caller]
fn ada_says(scene: &Scene, text: &str) {
    let sent = send(&scene.ada().machine, scene.channel(), text);
    assert!(sent.success, "Ada's send failed: {sent:?}");
}

/// Ada says a line, and the agent's run takes it; Ada says another, and the next run, which never
/// saw the first one's counter, shows the second without a warning that a message is missing.
#[test]
fn a_run_does_no_gap_detection_on_counters_an_earlier_run_took() {
    let scene = Scene::new(
        [3_135_800_001, 3_135_800_002],
        ["ada-stateless-gaps-standin", "bob-stateless-gaps-standin"],
    );
    ada_says(&scene, ADA_SAYS[0]);
    let first = scene.agent.received();
    scene.assert_agent_shows(&first, ADA_SAYS[0], scene.ada());
    ada_says(&scene, ADA_SAYS[1]);

    let second = scene.agent.received();

    scene.assert_agent_shows(&second, ADA_SAYS[1], scene.ada());
    assert!(
        !second.stderr.contains("missing"),
        "a missing message warned of: {second:?}"
    );
}

/// With Ada's line in the agent's mailbox, the relay holds the agent's acks over HTTPS, and the
/// agent's run is killed once it has sent one; the held ack never reaches the server. The run
/// asked over `transport`, the head of its request, and showed the line before it acked. The next
/// run shows it again; the one after, its ack let through, shows it no more.
#[track_caller]
fn shows_before_it_acks_over(scene: &Scene, transport: &str) {
    let relay = &scene.cloud().relay;
    relay.hold(ACK, 2);
    let mark = relay.mark();

    let killed = killed_when(scene.agent.start(&[RECV, JSON]), || relay.held() > 0);

    // The held ack waits for a request that never comes.
    relay.hold("POST /never", 1);
    scene.agent.assert_left_nothing(&killed);
    let sent = relay.sent_since(mark);
    assert!(
        sent.iter()
            .any(|request| request.head.starts_with(transport)),
        "no {transport}: {sent:?}"
    );
    scene.assert_agent_shows(&killed, ADA_SAYS[0], scene.ada());
    for (run, times) in [("next", 1), ("third", 0)] {
        let got = scene.agent.received();
        let shown = shown_with(&got, ADA_SAYS[0]);
        assert_eq!(shown, times, "the {run} run's: {got:?}");
    }
}

/// As a proxy that blocks WebSocket, the relay refuses it: the agent's runs read the mailbox's
/// event stream, and each acks a line only once it has shown it.
#[test]
fn a_run_over_the_event_stream_acks_only_what_it_showed() {
    let scene = Scene::new(
        [3_135_801_001, 3_135_801_002],
        ["ada-stateless-sse-standin", "bob-stateless-sse-standin"],
    );
    ada_says(&scene, ADA_SAYS[0]);
    scene.cloud().relay.block(WEBSOCKET);

    shows_before_it_acks_over(&scene, EVENTS_HEAD);
}

/// As a proxy that blocks WebSocket and streaming, the relay refuses both: the agent's runs
/// long-poll, and each acks a line only once it has shown it, so it takes one long-poll answer.
#[test]
fn a_run_over_long_poll_acks_only_what_it_showed() {
    let scene = Scene::new(
        [3_135_802_001, 3_135_802_002],
        ["ada-stateless-poll-standin", "bob-stateless-poll-standin"],
    );
    ada_says(&scene, ADA_SAYS[0]);
    let relay = &scene.cloud().relay;
    relay.block(WEBSOCKET);
    relay.block(EVENTS);

    shows_before_it_acks_over(&scene, POLL_HEAD);
}

/// With Ada's line in the agent's mailbox, the relay refuses the agent's requests for the
/// channel's GroupInfo, so its run cannot check Ada's device: the run fails and acks nothing. Once
/// the relay lets them through, the next run shows the line.
#[test]
fn a_run_whose_group_info_request_fails_acks_nothing() {
    let scene = Scene::new(
        [3_135_803_001, 3_135_803_002],
        [
            "ada-stateless-groupinfo-standin",
            "bob-stateless-groupinfo-standin",
        ],
    );
    ada_says(&scene, ADA_SAYS[0]);
    let relay = &scene.cloud().relay;
    let request = format!(
        "GET /spaces/{}/channels/{}/group-info",
        scene.space(),
        scene.channel()
    );
    relay.refuse(&request, "403 Forbidden");

    let failed = scene.agent.recv();

    assert!(!failed.success, "{failed:?}");
    assert_eq!(shown_with(&failed, ADA_SAYS[0]), 0, "{failed:?}");
    relay.stop_refusing();
    let next = scene.agent.received();
    scene.assert_agent_shows(&next, ADA_SAYS[0], scene.ada());
}

/// With only the two secrets, `skald spaces`, `skald channels`, and `skald send` and `skald recv`
/// with `--agent`, fail, saying the run sends and receives only; `skald recv --wait` fails, naming
/// the state directory's variable. None leaves anything on the machine's disk.
#[test]
fn a_stateless_run_refuses_every_other_command() {
    let scene = Scene::new(
        [3_135_804_001, 3_135_804_002],
        [
            "ada-stateless-commands-standin",
            "bob-stateless-commands-standin",
        ],
    );
    let channel = scene.channel().to_string();
    let send_as = [
        "send",
        "--agent",
        "other",
        "--all",
        "--channel",
        &channel,
        "a line",
    ];
    let others: [&[&str]; 4] = [
        &["spaces"],
        &["channels", ACME],
        &[RECV, "--agent", "other"],
        &send_as,
    ];
    for args in others {
        let refused = scene.agent.run(args);
        assert!(!refused.success, "{args:?}: {refused:?}");
        assert!(
            refused.stderr.contains("only sends and receives"),
            "{args:?}: {refused:?}"
        );
    }

    let waits = scene.agent.run(&[RECV, "--wait"]);

    assert!(!waits.success, "{waits:?}");
    assert!(waits.stderr.contains("SKALD_STATE_DIR"), "{waits:?}");
}
