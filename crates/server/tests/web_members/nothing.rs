//! Who gets a space's members: only sessions of its members.

use skald_proto::{Role, SpaceId};

use crate::api::{Person, assert_refused};
use crate::common::Response;
use crate::common::device_flow::text;
use crate::common::devices::assert_refused as assert_status;
use crate::listed::{Web, expected, login_of, members_path, members_response, roles};
use crate::rosters::first;
use crate::sessions::COOKIE;

/// The space dave creates beside `acme`.
const GLOBEX: &str = "globex";

/// Checks that `response` gives nothing of a space whose members are `people`: refused, its body
/// naming none of them; `what` names the case.
#[track_caller]
fn nothing_of(response: &Response, people: &[&Person], what: &str) {
    assert_refused(response, what);
    let body = text(response);
    for person in people {
        let handle = person.handle.as_str();
        assert!(!body.contains(handle), "{what}: {body}");
        assert!(!body.contains(&login_of(handle)), "{what}: {body}");
    }
}

/// `acme` has alice and bob as admins and carol as a member; dave created `globex`, whose only
/// member he is. Each gets the members of their own space. Dave's web and device sessions get
/// nothing of `acme`, nor alice's web session anything of `globex`, nor a request without a session
/// or with an unknown one, `401`; no space has an unknown ID.
#[test]
fn a_session_of_a_non_member_gets_nothing_of_the_space() {
    let web = Web::new(3_225_400_001);
    web.team.with_members();
    let (team, api) = (&web.team, web.api());
    let [alice, bob, carol, dave] = [&team.alice, &team.bob, &team.carol, &team.dave];
    let globex = dave.create_space(GLOBEX);
    let revision = dave.sign(&first(globex, GLOBEX, dave));
    crate::common::devices::assert_taken(&dave.session.store(globex, &revision));

    let in_acme = [alice, bob, carol];
    assert_eq!(
        roles(&api, team.space, &web.bob),
        expected(&[
            (alice, Role::Admin),
            (bob, Role::Admin),
            (carol, Role::Member)
        ]),
        "bob, of acme"
    );
    assert_eq!(
        roles(&api, globex, &web.dave),
        expected(&[(dave, Role::Admin)]),
        "dave, of globex"
    );

    let acme = members_path(team.space);
    nothing_of(
        &members_response(&api, team.space, &[("cookie", &web.dave)]),
        &in_acme,
        "dave's web session, of acme",
    );
    nothing_of(
        &dave.session.call("get", &acme, None),
        &in_acme,
        "dave's device session, of acme",
    );
    nothing_of(
        &members_response(&api, globex, &[("cookie", &web.alice)]),
        &[dave],
        "alice's web session, of globex",
    );
    nothing_of(
        &members_response(&api, SpaceId::generate(), &[("cookie", &web.alice)]),
        &[],
        "an unknown space",
    );

    let unknown = format!("{COOKIE}=bm8tc3VjaC1zZXNzaW9uLWF0LWFsbC0wMDAwMDAwMDAwMDA");
    for (headers, what) in [
        (vec![], "without a session"),
        (vec![("cookie", unknown.as_str())], "an unknown session"),
    ] {
        let response = members_response(&api, team.space, &headers);
        assert_status(&response, 401);
        nothing_of(&response, &in_acme, what);
    }
}
