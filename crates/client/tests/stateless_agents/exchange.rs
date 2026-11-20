//! A one-shot run with only the two secrets sends and receives the channel's messages, with only
//! ciphertext on the server and nothing left on its machine's disk.

use std::collections::BTreeSet;

use skald_proto::{DeviceId, Scheme};

use crate::agents::{CLAUDE, run_as_in};
use crate::fixtures::send;
use crate::front::Acks;
use crate::kit::RECV;
use crate::member::JSON;
use crate::member::texts_of;
use crate::scene::{Scene, killed_when, shown_with};
use crate::senders::{assert_labelled, assert_warned_before_others};
use crate::service_kit::BOT;

/// What Ada and the agent say; none is anywhere else.
const AGENT_SAYS: &str = "stateless bot test: deploy-bot starts the rollout";
const ADA_SAYS: [&str; 2] = [
    "stateless bot test: ada asks for the staging deploy",
    "stateless bot test: ada asks for the release notes",
];

/// The agent's run sends a line in the channel and succeeds, leaving nothing on its machine's disk.
/// The front saw it send one `hpke` envelope to each member device of the channel's group, Ada's
/// and Bob's, and no `mls` one. The server's database holds each envelope and not the text. Bob's
/// device opens the one to it: from the agent's address, without `counter`, with `hops` 0, the
/// text and a grant that passes Bob's device's checks. Ada's run shows the line once, from the
/// agent's address and device, named `ci/deploy-bot`, at the `service` level.
#[test]
fn a_run_with_only_the_two_secrets_sends_to_each_member_device_once() {
    let scene = Scene::new(
        [3_135_760_001, 3_135_760_002],
        ["ada-stateless-send-standin", "bob-stateless-send-standin"],
    );
    let mark = scene.mark();

    scene.agent.sent(scene.channel(), AGENT_SAYS);

    let sent = scene.sent_since(mark);
    let mut recipients = BTreeSet::new();
    for envelope in &sent {
        let header = &envelope.envelope().header;
        assert_eq!(header.scheme, Scheme::Hpke, "{header:?}");
        let [one] = header.recipients.as_deref().unwrap_or_default() else {
            panic!("not one recipient: {header:?}");
        };
        assert!(recipients.insert(*one), "two envelopes to {one}");
    }
    let wanted: BTreeSet<DeviceId> = [scene.ada().device, scene.bob().device].into();
    assert_eq!(recipients, wanted, "the devices the agent sealed to");
    scene.assert_only_ciphertext(&sent, AGENT_SAYS);

    let opened = scene.bob_opens();
    let [content] = opened.as_slice() else {
        panic!("not one envelope of the agent for Bob's device: {opened:?}");
    };
    assert_eq!(content.from.to_string(), scene.address, "{content:?}");
    assert_eq!(content.counter, None, "{content:?}");
    assert_eq!(content.hops, 0, "{content:?}");
    assert_eq!(texts_of(content), [AGENT_SAYS], "{content:?}");
    scene.assert_granted(content);

    let got = scene.ada_receives();
    scene.assert_shows_the_agent(&got, AGENT_SAYS);
}

/// Ada says two lines in the channel. The agent's next run shows each once, from Ada's address and
/// device; the run after it shows neither. Neither leaves anything on its machine's disk.
#[test]
fn a_members_messages_reach_the_next_run_once() {
    let scene = Scene::new(
        [3_135_761_001, 3_135_761_002],
        ["ada-stateless-recv-standin", "bob-stateless-recv-standin"],
    );
    for text in ADA_SAYS {
        let sent = send(&scene.ada().machine, scene.channel(), text);
        assert!(sent.success, "Ada's send failed: {sent:?}");
    }

    let got = scene.agent.received();

    for text in ADA_SAYS {
        scene.assert_agent_shows(&got, text, scene.ada());
    }
    let again = scene.agent.received();
    for text in ADA_SAYS {
        assert_eq!(
            shown_with(&again, text),
            0,
            "{text:?} shown again: {again:?}"
        );
    }
}

/// Ada says a line. The front swallows the agent's acks, and the agent's run is killed once it
/// has sent one: it showed the line before. The next run, its acks forwarded, shows the line again
/// once; the one after shows it no more. None leaves anything on its machine's disk.
#[test]
fn a_run_killed_after_its_output_and_before_its_ack_shows_the_message_again() {
    let scene = Scene::new(
        [3_135_762_001, 3_135_762_002],
        ["ada-stateless-kill-standin", "bob-stateless-kill-standin"],
    );
    let sent = send(&scene.ada().machine, scene.channel(), ADA_SAYS[0]);
    assert!(sent.success, "Ada's send failed: {sent:?}");
    let front = &scene.talk.front;
    front.acks(Acks::Swallow);
    let acked = front.acked();

    let killed = killed_when(scene.agent.start(&[RECV, JSON]), || front.acked() > acked);

    front.acks(Acks::Forward);
    scene.agent.assert_left_nothing(&killed);
    scene.assert_agent_shows(&killed, ADA_SAYS[0], scene.ada());
    let again = scene.agent.received();
    scene.assert_agent_shows(&again, ADA_SAYS[0], scene.ada());
    let after = scene.agent.received();
    assert_eq!(
        shown_with(&after, ADA_SAYS[0]),
        0,
        "shown a third time: {after:?}"
    );
}

/// With `~/work/acme` on Ada's machine bound to `acme`, where her agent has run once, the agent
/// sends a line. Ada's agent's run
/// there shows it on a line `[ci/deploy-bot · ci/deploy-bot · service]` after the one warning line
/// that its text is data.
#[test]
fn a_run_s_message_reaches_a_member_s_agent_as_a_service_line_after_the_warning() {
    let scene = Scene::new(
        [3_135_763_001, 3_135_763_002],
        ["ada-stateless-level-standin", "bob-stateless-level-standin"],
    );
    let machine = &scene.ada().machine;
    let folder = scene.ada_acme();

    scene.agent.sent(scene.channel(), AGENT_SAYS);

    let read = run_as_in(machine, &folder, CLAUDE);
    assert_labelled(&read.stdout, AGENT_SAYS, (BOT, BOT, crate::scene::SERVICE));
    assert_warned_before_others(&read);
}
