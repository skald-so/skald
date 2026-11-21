//! A deleted account's spaces: each space where it is the only member, its personal space included,
//! is deleted with its channels; a space with other members stays, keeps the account in its
//! revisions, still gives its devices' certificates and revocations, marks it deleted, and its
//! devices get a `space-notice`.

use std::collections::BTreeSet;

use crate::api::{Person, Team};
use crate::common::device_flow::{DeviceFlow, INTERVAL};
use crate::common::devices::devices_of;
use crate::deletion::{
    Preview, acme_with_member, assert_holds_space, assert_space_gone, deleted_accounts, deletes,
    ids, members, personal_space, preview, space_alone,
};
use crate::fixture::Fixture;
use crate::listener::{Listener, second_device};
use crate::sessions::Api;

/// Ada's personal space and her team space `solo`, each with her its only member, `solo` with a
/// channel: the server says a deletion deletes both and nothing blocks it. Once she deletes her
/// account, no row of the database holds either space or the channel.
#[test]
fn personal_space_and_team_space_of_only_the_account_are_deleted_with_their_channels() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let api = Api::of(&flow);
    let ada = Person::new(&flow, 3_303_050_001, "ada");
    let personal = personal_space(&ada);
    let (solo, general) = space_alone(&ada, "solo");
    let expected = Preview {
        deletes: ids(&[personal, solo]),
        blocks: BTreeSet::new(),
    };
    assert_eq!(preview(&api, &ada), expected);
    let db = &flow.stand.db;
    assert_holds_space(db, personal, "the personal space");
    assert_holds_space(db, solo, "solo");

    deletes(&api, &ada);

    assert_space_gone(db, personal, &[], "the personal space");
    assert_space_gone(db, solo, &[general], "solo");
}

/// alice created `acme`, where bob is the other admin and carol and dave are members, and has a
/// revocation of a second device stored. Once she deletes her account, `acme` stays with the same
/// revisions; bob's and carol's sessions get her marked deleted among its members and its devices,
/// her devices' certificates and her revocation still given; bob's and carol's connections get a
/// `space-notice` of `acme`, and eve's, outside it, none.
#[test]
fn space_with_other_members_stays_and_marks_the_account_deleted() {
    let f = Fixture::new(3_303_050_011);
    let t = &f.team;
    let flow = &t.flow;
    let space = t.space;
    let phone = second_device(f.db(), &t.alice, "Phone");
    let revocation = t.alice.by_master(phone.id, &[]);
    t.alice.revoke(&revocation);
    let revisions = t.bob.session.revisions(space);
    let devices = devices_of(&t.bob.session.devices_response(space));
    assert!(
        devices.contains_key(&t.alice.laptop.id.to_string()),
        "alice's laptop among the space's devices"
    );
    assert_eq!(
        deleted_accounts(&t.bob.session.devices_response(space)),
        BTreeSet::new()
    );
    let mut bob = Listener::of(flow, &t.bob);
    let mut carol = Listener::of(flow, &t.carol);
    let mut eve = Listener::of(flow, &f.eve);
    let api = Api::of(flow);
    assert!(
        !preview(&api, &t.alice).deletes.contains(&space.to_string()),
        "acme is not deleted"
    );

    deletes(&api, &t.alice);

    bob.assert_notice(space);
    carol.assert_notice(space);
    eve.assert_no_notice();
    let alice = t.alice.account.to_string();
    for session in [&t.bob.session, &t.carol.session] {
        assert_eq!(session.revisions(space), revisions, "the same revisions");
        let marked: Vec<(String, bool)> = members(session, space);
        let expected: Vec<(String, bool)> = ["alice", "bob", "carol", "dave"]
            .map(|handle| (handle.to_owned(), handle == "alice"))
            .to_vec();
        assert_eq!(marked, expected, "only alice marked deleted");
        let response = session.devices_response(space);
        assert_eq!(
            devices_of(&response),
            devices,
            "the same devices and certificates"
        );
        assert_eq!(deleted_accounts(&response), BTreeSet::from([alice.clone()]));
        assert!(
            session.space_revocations(space).contains(&revocation),
            "alice's revocation still given"
        );
        assert!(
            session.spaces().contains_key(&space.to_string()),
            "acme still listed"
        );
    }
}

/// alice's space `acme` has dave as its only other member. Once dave deletes his account, which
/// neither deletes `acme` nor is blocked by it, the server says alice's deletion deletes `acme` and
/// nothing blocks it; once she deletes hers, no row of the database holds `acme`.
#[test]
fn space_of_the_account_and_an_already_deleted_one_is_deleted_with_it() {
    let team = Team::new(3_303_050_021);
    let (alice, dave) = (&team.alice, &team.dave);
    acme_with_member(&team, dave);
    let api = Api::of(&team.flow);
    let daves = preview(&api, dave);
    let acme = team.space.to_string();
    assert!(!daves.deletes.contains(&acme), "{daves:?}");
    assert!(!daves.blocks.contains(&acme), "{daves:?}");

    deletes(&api, dave);

    let expected = Preview {
        deletes: ids(&[team.space]),
        blocks: BTreeSet::new(),
    };
    assert_eq!(preview(&api, alice), expected);
    let db = &team.flow.stand.db;
    assert_holds_space(db, team.space, "acme");
    deletes(&api, alice);
    assert_space_gone(db, team.space, &[], "acme");
}
