//! Who gets a space's usage: only sessions of its members; and an account's usage: only its own
//! sessions.

use skald_proto::SpaceId;

use crate::api::{ACME, assert_refused};
use crate::common::Response;
use crate::common::device_flow::text;
use crate::common::devices::assert_refused as assert_status;
use crate::limits::MONTHLY;
use crate::sessions::COOKIE;
use crate::usage::{
    Web, account_usage, account_usage_response, names, of, space_usage, space_usage_path,
    space_usage_response,
};

/// Checks that `response` gives nothing of a space's usage: refused, without its count or limit;
/// `what` names the case.
#[track_caller]
fn nothing_of(response: &Response, what: &str) {
    assert_refused(response, what);
    let body = text(response);
    for part in [
        "monthly_messages",
        "message_size",
        "mailbox_retention",
        "service_agents",
        "owned_spaces",
    ] {
        assert!(!body.contains(part), "{what}: {body}");
    }
}

/// `acme` has alice, bob, carol and dave as members; eve is in no space. Eve's web and device
/// sessions get nothing of `acme`, nor a space with an unknown ID anything, nor a request without
/// a session or with an unknown one, `401`. Once a revision removes carol, she gets nothing of it
/// either, while dave still gets it. Eve's own usage lists no space of the others.
#[test]
fn a_session_of_a_non_member_gets_nothing_of_the_space() {
    let mut web = Web::with(3_226_600_001, &[MONTHLY, "5"]);
    let api = web.api();
    let space = web.fixture.team.space;
    assert_eq!(
        space_usage(&api, space, &web.carol).messages,
        of(0, 5),
        "carol"
    );

    let response =
        |space: SpaceId, cookie: &str| space_usage_response(&api, space, &[("cookie", cookie)]);
    nothing_of(&response(space, &web.eve), "eve's web session");
    let eve = &web.fixture.eve;
    let path = space_usage_path(space);
    nothing_of(
        &eve.session.call("get", &path, None),
        "eve's device session",
    );
    nothing_of(
        &response(SpaceId::generate(), &web.alice),
        "an unknown space",
    );

    let unknown = format!("{COOKIE}=bm8tc3VjaC1zZXNzaW9uLWF0LWFsbC0wMDAwMDAwMDAwMDA");
    let unknown: &[(&str, &str)] = &[("cookie", &unknown)];
    for headers in [&[][..], unknown] {
        let refusals = [
            space_usage_response(&api, space, headers),
            account_usage_response(&api, headers),
        ];
        for refused in &refusals {
            assert_status(refused, 401);
            nothing_of(refused, "without a session or with an unknown one");
        }
    }

    let eves = account_usage(&api, &web.eve);
    assert_eq!(eves.owned_spaces, names(&[]), "eve owns no space");
    assert!(!format!("{eves:?}").contains(ACME), "{eves:?}");

    let carol = web.fixture.team.carol.account;
    web.fixture.revise(|body| {
        let kept: Vec<_> = body
            .members
            .iter()
            .filter(|member| member.account != carol)
            .cloned()
            .collect();
        body.members = skald_proto::NonEmpty::from_vec(kept).expect("members left");
        for channel in &mut body.channels {
            channel.members.retain(|member| member.account != carol);
        }
    });
    let api = web.api();
    let refused = space_usage_response(&api, space, &[("cookie", &web.carol)]);
    nothing_of(&refused, "carol, removed");
    assert_eq!(
        space_usage(&api, space, &web.dave).messages,
        of(0, 5),
        "dave"
    );
}
