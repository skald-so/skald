//! As an agent, the line names nothing of a key change in another space.

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{send_args, strs};
use crate::kit::{RECV, assert_names_none, create_space, latest};
use crate::world::{AGENT, GLOBEX, MEMBERS, next_by, run, store};
use crate::{CAROL_HOST, assert_says_waiting, other_master};

/// Ada's machine creates `globex`, and a revision her device signs adds Carol, who is in no space
/// of the talk; her `skald members globex` accepts it. A revision 4 changes Carol's `master`, and
/// her next `skald members globex` accepts it: a key change of Carol waits in `globex`. claude in
/// `~/work/acme`, bound to `acme`, runs `skald recv` and `skald send`: each prints the line, and
/// neither names `globex`, its ID, Carol's handle or her account's ID.
#[test]
fn an_agents_line_names_nothing_of_a_key_change_in_another_space() {
    let agents = Agents::new(
        [4_188_901_001, 4_188_901_002],
        ["ada-with-a-change-elsewhere", "bob-in-acme-only"],
    );
    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let carol = talk
        .world
        .sign_up(4_188_901_003, "carol-of-globex-only", CAROL_HOST);
    let globex = create_space(cloud, ada, GLOBEX);
    let third = next_by(&latest(cloud, &ada.token, globex.id), ada, |list| {
        list.push(carol.member(skald_proto::Role::Member));
    });
    store(cloud, &ada.token, &third);
    assert!(run(&ada.machine, &[MEMBERS, GLOBEX]).success);
    let fourth = next_by(&third, ada, |list| {
        let entry = (list.iter_mut())
            .find(|member| member.account == carol.account)
            .expect("Carol");
        entry.master = other_master();
    });
    store(cloud, &ada.token, &fourth);
    assert!(run(&ada.machine, &[MEMBERS, GLOBEX]).success);
    let hidden = [
        GLOBEX.to_owned(),
        globex.id.to_string(),
        carol.handle.to_string(),
        carol.account.to_string(),
    ];

    let mut send = vec![AGENT.to_owned(), CLAUDE.to_owned()];
    send.extend(send_args(
        talk.channel,
        "key changes test: claude sends beside globex",
    ));
    let ran = [
        (ada.machine)
            .start_in(&agents.folder, &[AGENT, CLAUDE, RECV])
            .finish(),
        (ada.machine)
            .start_in(&agents.folder, &strs(&send))
            .finish(),
    ];

    for finished in &ran {
        assert!(finished.success, "{finished:?}");
        assert_says_waiting(&finished.shown(), Some(1));
        assert_names_none(finished, &hidden);
    }
}
