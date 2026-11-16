//! Which agents count: a `to` names one of this device's agents only by the space, the account and
//! the server; an agent is the device's in the space from its first run there, a `send` too.

use crate::fixtures::SEND;
use crate::kit::received;
use crate::talk::Line;
use crate::world::{ACME, AGENT, GLOBEX, HOST};
use crate::{Agents, CLAUDE, CODEX, assert_handed_once, assert_not_handed};

/// The texts Bob's device sends; none is anywhere else.
const TO_BOBS: &str = "for bob's own claude: rotate the deploy key";
const TO_GLOBEX: &str = "for claude in globex: close the sprint";
const TO_ELSEWHERE: &str = "for claude on another server: archive the logs";
const TO_CLAUDE: &str = "for ada's claude: review the pull request";
const TO_ALL: &str = "the build cache was cleared";
const SAID: &str = "codex starts on the release notes";

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends to the channel a message to
/// an agent of the same name of Bob's account in `acme`, one to Ada's in `globex`, one to Ada's in
/// `acme` on another server, and one to Ada's claude. claude's next run shows only the last.
#[test]
fn to_names_this_device_s_agent_only_by_its_space_account_and_server() {
    let mut agents = Agents::new(
        [4_147_076_001, 4_147_076_002],
        ["ada-agent-by-address", "bob-same-agent-name"],
    );
    agents.run_as(CLAUDE);
    let (ada, bob) = (
        agents.talk.ada.handle.clone(),
        agents.talk.played[0].person.handle.clone(),
    );
    let from = agents.bob();
    let mut ids = Vec::new();
    for (counter, (text, to)) in [
        (TO_BOBS, format!("{}/{bob}/{CLAUDE}@{HOST}", ACME)),
        (TO_GLOBEX, format!("{GLOBEX}/{ada}/{CLAUDE}@{HOST}")),
        (
            TO_ELSEWHERE,
            format!("{}/{ada}/{CLAUDE}@elsewhere.example", ACME),
        ),
        (TO_CLAUDE, agents.address(CLAUDE)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut line = Line::new(&from, counter as u64 + 1, text);
        line.to = Some(to);
        ids.push(agents.bob_sends(&line));
    }

    let claude = agents.run_as(CLAUDE).shown();

    assert_handed_once(&claude, TO_CLAUDE, ids[3]);
    for text in [TO_BOBS, TO_GLOBEX, TO_ELSEWHERE] {
        assert_not_handed(&claude, text);
    }
}

/// Ada's device's only run as codex in `~/work/acme` is a `skald send` to the channel. Bob's
/// device then sends a message without `to`, which the person's `skald recv` shows; codex's first
/// `skald recv` shows it once.
#[test]
fn agent_whose_only_run_is_a_send_gets_the_messages_that_come_after() {
    let mut agents = Agents::new(
        [4_147_077_001, 4_147_077_002],
        ["ada-agent-sends-first", "bob-answers-the-agent"],
    );
    let channel = agents.talk.channel.to_string();
    let sent = (agents.talk.ada.machine)
        .start_in(&agents.folder, &[AGENT, CODEX, SEND, &channel, SAID])
        .finish();
    assert!(sent.success, "{sent:?}");
    let bob = agents.bob();
    let id = agents.bob_sends(&Line::new(&bob, 1, TO_ALL));
    let person = received(&agents.talk.ada.machine).shown();
    assert!(person.contains(TO_ALL), "{person}");

    let codex = agents.run_as(CODEX).shown();

    assert_handed_once(&codex, TO_ALL, id);
}
