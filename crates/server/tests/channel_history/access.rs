//! Who gets a channel's history: only sessions of accounts the latest revision lists as the
//! channel's members, as for its GroupInfo ("MLS profile").

use crate::api::Session;
use crate::kit::{
    ENTRIES, LINKS, WRAPS, assert_history, assert_none_given, bob_rotates, fetch, formed,
    of_epoch_2, stores,
};

#[test]
fn a_channels_history_is_given_only_to_sessions_of_its_members() {
    let (mut s, _) = formed(3_160_110_301);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let t = &f.team;
    let [link, wrap, entry] = of_epoch_2(f, &t.carol, &t.alice);
    stores(&mut s.bob, f, &link, "bob's link");
    stores(&mut s.alice, f, &wrap, "alice's wrap");
    stores(&mut s.carol, f, &entry, "carol's entry");

    for peer in [&s.alice, &s.bob, &s.carol, &s.dave] {
        let what = "a channel member's session";
        assert_history(&peer.session, f, [&[&link], &[&wrap], &[&entry]], what);
    }
    assert_none_given(&s.eve.session, f, "eve, outside the space");
    let none = Session::none(&t.flow);
    for kind in [LINKS, WRAPS, ENTRIES] {
        assert_eq!(
            fetch(&none, f, kind).status,
            401,
            "{kind} without a session"
        );
    }

    // A revision without carol and dave in the channel; both stay in the space, and carol's laptop
    // in the group.
    let general = s.fixture.general;
    let members = [t.alice.account, t.bob.account];
    s.fixture.revise(|body| {
        let channel = (body.channels.iter_mut())
            .find(|channel| channel.id == general)
            .expect("the channel");
        channel
            .members
            .retain(|member| members.contains(&member.account));
    });
    let f = &s.fixture;
    assert_none_given(&s.carol.session, f, "carol, out of the channel");
    assert_none_given(&s.dave.session, f, "dave, out of the channel");
    let what = "bob's, still a member";
    assert_history(&s.bob.session, f, [&[&link], &[&wrap], &[&entry]], what);
}
