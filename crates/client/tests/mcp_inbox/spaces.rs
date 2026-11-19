//! A session covers only the space of its folder: in `~/work/clientA` its `inbox` names nothing of
//! clientb, and in a folder in no space it says the agent sees no space.

use serde_json::json;

use crate::client::INBOX;
use crate::fixtures::person_address;
use crate::helped::with_client_b;
use crate::kit::bind_client_a;
use crate::lines::assert_handed_line;
use crate::talk::{Line, Talk, encrypted, stored};
use crate::world::{ADD_RULE, CLIENT_A, CLIENT_B, dir_of, path, run};
use crate::{CLIENT, session_in};

/// What Bob's device says in each space; none is anywhere else.
const IN_A: &str = "space test: clienta's invoices are sent";
const IN_B: &str = "space test: clientb's contract is signed";

/// A space Ada's device is not in.
const INITECH: &str = "initech";

/// What a session in a folder in no space says.
const NO_SPACE: &str = "no space";

/// Ada's machine follows `clienta` and `clientb`, each with a channel that holds Bob's device;
/// `~/work/clientA` is bound to `clienta`, `~/work/clientB` to `clientb` and `~/work/initech` to
/// `initech`, a space the device is not in. A session of `claude-code` starts in each folder. Bob's
/// device says something in each channel. The session in `~/work/clientA` shows clienta's message
/// and names nothing of clientb, and the one in `~/work/clientB` the other way round; the one in
/// `~/work/initech` says the agent sees no space and shows neither.
#[test]
fn inbox_names_nothing_of_another_space_and_in_no_space_says_so() {
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_627_031_001, 4_627_031_002],
        &["ada-keeps-clients-apart", "bob-writes-to-both-clients"],
        false,
    );
    let (other, elsewhere) = with_client_b(&mut talk);
    let machine = &talk.ada.machine;
    let in_a = bind_client_a(machine);
    let in_b = dir_of(machine, "work/clientB");
    let nowhere = dir_of(machine, "work/initech");
    for (folder, space) in [(&in_b, CLIENT_B), (&nowhere, INITECH)] {
        let bound = run(machine, &[ADD_RULE, path(folder), space]);
        assert!(bound.success, "not bound: {bound:?}");
    }
    let mut session_a = session_in(machine, &in_a, &[], CLIENT);
    let mut session_b = session_in(machine, &in_b, &[], CLIENT);
    let mut lost = session_in(machine, &nowhere, &[], CLIENT);
    let targets = [
        (CLIENT_A, (talk.space.id, talk.channel), IN_A),
        (CLIENT_B, (other.id, elsewhere), IN_B),
    ];
    let bob = &mut talk.played[0];
    let mut sent = Vec::new();
    for (space, at, text) in targets {
        let from = person_address(space, &bob.person);
        let said = encrypted(bob, at, at.1, &Line::new(&from, 1, text));
        stored(&talk.world.cloud, bob, &said);
        sent.push(said.envelope().header.msg_id);
    }

    let shown_a = session_a.inbox();
    let shown_b = session_b.inbox();
    let said = lost.call(INBOX, json!({}));

    assert_handed_line(&shown_a, IN_A, sent[0]);
    assert_handed_line(&shown_b, IN_B, sent[1]);
    for (shown, other_text, other_space) in [(&shown_a, IN_B, CLIENT_B), (&shown_b, IN_A, CLIENT_A)]
    {
        assert!(
            !shown.contains(other_text) && !shown.to_lowercase().contains(other_space),
            "it names {other_space}: {shown}"
        );
    }
    assert!(
        said.text.to_lowercase().contains(NO_SPACE),
        "does not say it sees no space: {said:?}"
    );
    assert!(
        !said.text.contains(IN_A) && !said.text.contains(IN_B),
        "{said:?}"
    );
}
