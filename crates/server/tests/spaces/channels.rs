//! Channel IDs: only an admin's session gets one, and a revision adds only channels whose IDs the
//! server issued for that space.

use std::collections::BTreeSet;

use skald_proto::ChannelId;

use crate::api::{Session, Team, assert_refused};
use crate::common::device_flow::text;
use crate::common::devices::assert_taken;
use crate::rosters::{channel, first, next};

#[test]
fn only_an_admins_session_gets_a_channel_id() {
    let team = Team::new(3_120_840_001);
    team.with_members();

    let mut issued = BTreeSet::new();
    for admin in [&team.alice, &team.bob, &team.alice] {
        assert!(issued.insert(admin.session.channel(team.space)), "a new ID");
    }
    let member = team.carol.session.channel_response(team.space);
    assert_refused(&member, "a member that is no admin");
    let outsider = team.dave.session.channel_response(team.space);
    assert_refused(&outsider, "an account outside the space");
    let none = Session::none(&team.flow).channel_response(team.space);
    assert_eq!(none.status, 401, "{}", text(&none));
}

#[test]
fn a_revision_adds_only_channels_the_server_issued_for_its_space() {
    let team = Team::new(3_120_840_011);
    let (alice, bob) = (&team.alice, &team.bob);
    let other = bob.create_space("other");
    assert_taken(
        &bob.session
            .store(other, &bob.sign(&first(other, "other", bob))),
    );
    let of_other = bob.session.channel(other);

    let mut stored = team.with_members().to_vec();
    let with = |stored: &[Vec<u8>], ids: &[ChannelId]| {
        let mut body = next(stored.last().expect("a revision"), &alice.laptop);
        for id in ids {
            body.channels.push(channel(*id, &[alice, bob]));
        }
        alice.sign(&body)
    };

    let what = "a channel ID issued for another space";
    team.refused(alice, &with(&stored, &[of_other]), &stored, what);
    let unissued = ChannelId::generate();
    let what = "a channel ID the server never issued";
    team.refused(alice, &with(&stored, &[unissued]), &stored, what);

    let own = alice.session.channel(team.space);
    let third = with(&stored, &[own]);
    team.stored_by(alice, &third);
    stored.push(third);
    // The next revision keeps the channel.
    let fourth = alice.sign(&next(&stored[2], &alice.laptop));
    team.stored_by(alice, &fourth);
    stored.push(fourth);

    let mut body = next(&stored[3], &alice.laptop);
    body.channels.push(channel(of_other, &[alice]));
    let what = "a channel of another space next to its own";
    team.refused(alice, &alice.sign(&body), &stored, what);
}
