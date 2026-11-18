//! The agent of a session without `--agent`: its default name joins the client's name, the
//! device's certificate `name` and the device's reference, so the account's devices give one
//! client's sessions addresses of their own; with `--agent`, the agent it names.

use skald_proto::{DeviceId, MessageId};

use crate::agents::{ACME_DIR, Agents, CODEX};
use crate::devices::second_machine;
use crate::kit::received;
use crate::lines::{assert_handed_line, reference};
use crate::machine::Machine;
use crate::talk::{ADA_HOST, Line};
use crate::world::{ACME, ADD_RULE, AGENT, Person, dir_of, path, run};
use crate::{CLIENT, Session, session_in};

/// The length of a device's reference in a default name.
const REFERENCE: usize = 6;

/// The certificate `name` of a machine of Ada's that holds no character a name allows.
const NO_NAME: &str = "★ ★";

/// The default name's part of `device`: the first [`REFERENCE`] characters of the base32 of
/// SHA-256 of its ID's 16 bytes, as a message's reference is made of its `msg_id`'s.
pub fn device_reference(device: DeviceId) -> String {
    reference(MessageId::from_bytes(*device.as_bytes()))[..REFERENCE].to_owned()
}

/// The sessions, each with the name its agent has; Bob's device sends a message to each of those
/// agents, each its own text. Fails unless each session's `inbox` shows its own message once, by
/// its reference, and none of the others.
#[track_caller]
fn assert_each_gets_its_own(agents: &mut Agents, sessions: &mut [(Session, String)]) {
    let bob = agents.bob();
    let texts: Vec<String> = (1..=sessions.len())
        .map(|at| {
            format!(
                "name test: note {at} of {} for its own agent",
                sessions.len()
            )
        })
        .collect();
    let mut sent = Vec::new();
    for (((_, name), text), counter) in sessions.iter().zip(&texts).zip(1..) {
        let mut line = Line::new(&bob, counter, text);
        line.to = Some(agents.address(name));
        sent.push(agents.bob_sends(&line));
    }
    for (at, (session, name)) in sessions.iter_mut().enumerate() {
        let shown = session.inbox();
        assert_handed_line(&shown, &texts[at], sent[at]);
        for (other, text) in texts.iter().enumerate() {
            assert!(
                other == at || !shown.contains(text.as_str()),
                "{name} got {text:?}: {shown}"
            );
        }
    }
}

/// Ada's machine, whose certificate's `name` is `ada-workstation`, starts sessions in
/// `~/work/acme` without `--agent`, as clients named `Claude Code`, `  Codex__CLI!!  v2 `, one of
/// 51 characters, one of 70 and an empty one, and one with `--agent codex-m3t9xa`. Bob's device
/// sends a message to the agent of each name the rule gives: each session's `inbox` shows its own
/// message once, and none of the others.
#[test]
fn default_names_join_the_client_name_the_device_name_and_the_device_reference() {
    let mut agents = Agents::new(
        [4_627_021_001, 4_627_021_002],
        ["ada-runs-many-clients", "bob-writes-to-each-name"],
    );
    let r = device_reference(agents.talk.ada.device);
    let fifty_one = format!("cursor-{}", "z".repeat(44));
    let seventy = "o".repeat(70);
    let cases = [
        ("Claude Code", format!("claude-code-{ADA_HOST}-{r}"), None),
        (
            "  Codex__CLI!!  v2 ",
            format!("codex-cli-v2-{ADA_HOST}-{r}"),
            None,
        ),
        // 51 + 1 + 15 + 1 + 6 is 74: the device's name is cut to `ada-`, then trimmed.
        (fifty_one.as_str(), format!("{fifty_one}-ada-{r}"), None),
        // The device's name goes whole, and the client's is cut to 56.
        (seventy.as_str(), format!("{}-{r}", "o".repeat(56)), None),
        ("", format!("{ADA_HOST}-{r}"), None),
        ("Claude Code", CODEX.to_owned(), Some(CODEX)),
    ];
    let mut sessions: Vec<(Session, String)> = cases
        .into_iter()
        .map(|(client, name, agent)| {
            assert!(name.len() <= 63, "{name}");
            let args: Vec<&str> = agent.map(|agent| vec![AGENT, agent]).unwrap_or_default();
            let machine = &agents.talk.ada.machine;
            (session_in(machine, &agents.folder, &args, client), name)
        })
        .collect();

    assert_each_gets_its_own(&mut agents, &mut sessions);
}

/// A machine of Ada's account, linked by hand, with the certificate `name` `host`, its
/// `~/work/acme` bound to `acme`; it has joined the account's channels, and Bob's device has taken
/// that in.
fn linked(agents: &mut Agents, host: &'static str) -> Person {
    let other = second_machine(&agents.talk.world, &agents.talk.ada, host);
    bound_to_acme(&other.machine);
    received(&other.machine);
    agents.talk.played[0].take_in(&agents.talk.world.cloud);
    other
}

/// Binds `machine`'s `~/work/acme` to `acme`.
fn bound_to_acme(machine: &Machine) {
    let folder = dir_of(machine, ACME_DIR);
    let bound = run(machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "not bound: {bound:?}");
}

/// Ada's second machine, linked by hand, has the certificate `name` of her first,
/// `ada-workstation`. A session of `claude-code` without `--agent` starts in `~/work/acme` on each:
/// their agents' names differ, each ending in its device's reference. Bob's device sends a message
/// to each: each session's `inbox` shows its own and not the other's.
#[test]
fn sessions_of_one_client_on_two_devices_of_one_name_get_two_addresses() {
    let mut agents = Agents::new(
        [4_627_022_001, 4_627_022_002],
        ["ada-has-two-workstations", "bob-writes-to-both"],
    );
    let second = linked(&mut agents, ADA_HOST);
    let names: Vec<String> = [agents.talk.ada.device, second.device]
        .into_iter()
        .map(|device| format!("{CLIENT}-{ADA_HOST}-{}", device_reference(device)))
        .collect();
    assert_ne!(names[0], names[1], "one reference for two devices");
    let first = session_in(&agents.talk.ada.machine, &agents.folder, &[], CLIENT);
    let other = dir_of(&second.machine, ACME_DIR);
    let other = session_in(&second.machine, &other, &[], CLIENT);
    let mut sessions = [(first, names[0].clone()), (other, names[1].clone())];

    assert_each_gets_its_own(&mut agents, &mut sessions);
}

/// Ada's second machine, linked by hand, has a certificate `name` with no character a name allows.
/// Sessions without `--agent` start in `~/work/acme` there as `Claude Code` and as a client with an
/// empty name: their agents are `claude-code-<reference>` and `<reference>`, the device's name
/// left out. Bob's device sends a message to each: each session's `inbox` shows its own.
#[test]
fn device_name_with_nothing_a_name_allows_is_left_out_and_the_reference_stays() {
    let mut agents = Agents::new(
        [4_627_023_001, 4_627_023_002],
        ["ada-has-an-odd-device-name", "bob-writes-to-the-odd-one"],
    );
    let second = linked(&mut agents, NO_NAME);
    let r = device_reference(second.device);
    let folder = dir_of(&second.machine, ACME_DIR);
    let mut sessions = [
        (
            session_in(&second.machine, &folder, &[], "Claude Code"),
            format!("claude-code-{r}"),
        ),
        (session_in(&second.machine, &folder, &[], ""), r.clone()),
    ];

    assert_each_gets_its_own(&mut agents, &mut sessions);
}
