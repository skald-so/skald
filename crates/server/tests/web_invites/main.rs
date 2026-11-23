//! Web sessions of a space's admins list and revoke its active invites. Device sessions still
//! issue, store and accept invites through their own operations.

#![allow(dead_code)]

#[path = "../spaces/api.rs"]
mod api;
#[path = "../common/mod.rs"]
mod common;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../invites/scene.rs"]
mod scene;
#[path = "../web_sessions/sessions.rs"]
mod sessions;
#[path = "../mailbox/wire.rs"]
mod wire;

use serde_json::{Value, json};
use skald_proto::{InviteId, Name, Role, SpaceId};

use api::{Person, assert_refused};
use common::Response;
use common::device_flow::{json, text};
use common::devices::assert_taken;
use rosters::{first, member, next};
use scene::{Made, Scene, acceptance, gets_no_invite, wait_past};
use sessions::{Api, epoch};
use wire::now;

const LIST: &str = "/spaces/{space}/invites";
const REVOKE: &str = "/spaces/{space}/invites/{invite}";

fn cookie(scene: &Scene, person: &Person, github: u64) -> String {
    let flow = &scene.team().flow;
    let user = flow
        .stand
        .user(github, &format!("{}-spaces-standin", person.handle));
    flow.web_cookie(&user)
}

fn list(api: &Api<'_>, space: SpaceId, headers: &[(&str, &str)]) -> Response {
    api.call("get", &format!("/spaces/{space}/invites"), LIST, headers)
}

fn revoke(api: &Api<'_>, space: SpaceId, invite: InviteId, headers: &[(&str, &str)]) -> Response {
    api.call(
        "delete",
        &format!("/spaces/{space}/invites/{invite}"),
        REVOKE,
        headers,
    )
}

#[track_caller]
fn listed(api: &Api<'_>, space: SpaceId, cookie: &str) -> Vec<Value> {
    let response = list(api, space, &[("cookie", cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    json(&response)["invites"]
        .as_array()
        .unwrap_or_else(|| panic!("no invites list: {}", text(&response)))
        .clone()
}

#[track_caller]
fn assert_record(
    api: &Api<'_>,
    row: &Value,
    invite: &Made,
    username: &Value,
    device: &str,
    stored: std::ops::RangeInclusive<i64>,
    waiting: Option<&Name>,
) {
    let mut metadata = row.clone();
    let dates = metadata.as_object_mut().expect("an invite record");
    let created = dates.remove("created_at").expect("when stored");
    let expires = dates.remove("expires_at").expect("when it expires");
    let handle = dates.remove("acceptance_handle").unwrap_or(Value::Null);
    assert_eq!(
        handle,
        json!(waiting.map(Name::as_str)),
        "the waiting handle"
    );
    let role = match invite.body.role {
        Role::Admin => "admin",
        Role::Member => "member",
    };
    assert_eq!(
        metadata,
        json!({
            "invite": invite.id.to_string(),
            "role": role,
            "username": username,
            "device_name": device,
            "acceptance_waiting": waiting.is_some(),
        }),
        "unexpected invite metadata"
    );
    let created = epoch(api.db, created.as_str().expect("a creation date"));
    assert!(stored.contains(&created), "{created} outside {stored:?}");
    assert_eq!(
        epoch(api.db, expires.as_str().expect("an expiry date")),
        i64::try_from(invite.body.expires).expect("a Unix time"),
    );
}

#[test]
fn a_web_admin_lists_active_invites_with_only_their_metadata_and_waiting_state() {
    const GITHUB: u64 = 3_819_810_001;
    let scene = Scene::new(GITHUB);
    let team = scene.team();
    let api = Api::of(&team.flow);
    let alice = cookie(&scene, &team.alice, GITHUB);
    let bob = cookie(&scene, &team.bob, GITHUB + 1);
    let username =
        json(&api.call("get", "/session", "/session", &[("cookie", &alice)]))["username"].clone();
    assert!(username.is_string(), "alice's username: {username}");

    // The reserved ID has no stored invite yet.
    team.alice.session.issue(scene.space());
    let expires = now() + 6;
    scene.invite_until(expires);
    let before = now() as i64;
    let open = scene.invite_with(|body| body.role = Role::Admin);
    assert_taken(&team.alice.session.put_invite(scene.space(), &open));
    let waiting = scene.invite();
    let after = now() as i64;
    let accepted = acceptance(&waiting, &scene.f.eve);
    assert_taken(&scene.f.eve.session.accept(waiting.id, &accepted));
    wait_past(expires);

    for admin in [&alice, &bob] {
        let rows = listed(&api, scene.space(), admin);
        assert_eq!(rows.len(), 2, "{rows:?}");
        for (invite, handle) in [(&open, None), (&waiting, Some(&scene.f.eve.handle))] {
            let id = invite.id.to_string();
            let row = rows
                .iter()
                .find(|row| row["invite"] == id)
                .unwrap_or_else(|| panic!("{id} is absent from {rows:?}"));
            assert_record(
                &api,
                row,
                invite,
                &username,
                &team.alice.laptop.name,
                before..=after,
                handle,
            );
        }
    }
}

#[test]
fn invite_listing_is_private_and_only_a_web_admin_from_the_server_can_revoke() {
    const GITHUB: u64 = 3_819_810_011;
    let scene = Scene::new(GITHUB);
    let team = scene.team();
    let api = Api::of(&team.flow);
    let space = scene.space();
    let invite = scene.invite();
    let eve = &scene.f.eve;
    let globex = eve.create_space("globex");
    assert_taken(
        &eve.session
            .store(globex, &eve.sign(&first(globex, "globex", eve))),
    );
    let other_admin = cookie(&scene, eve, GITHUB + 4);
    let member = cookie(&scene, &team.carol, GITHUB + 2);
    let device = format!("Bearer {}", team.alice.session.token());
    let own = ("origin", api.public);
    let member_headers = [("cookie", member.as_str()), own];
    let other_headers = [("cookie", other_admin.as_str()), own];
    let device_headers = [("authorization", device.as_str()), own];
    for (who, headers) in [
        ("member", &member_headers[..]),
        ("admin of another space", &other_headers),
        ("admin's device", &device_headers),
        ("signed out", &[]),
    ] {
        let response = list(&api, space, headers);
        if response.status == 200 {
            assert_eq!(json(&response), json!({ "invites": [] }), "{who}");
        } else {
            assert_refused(&response, who);
        }
        assert_refused(&revoke(&api, space, invite.id, headers), who);
        scene.gets_invite(&scene.frank, &invite);
    }

    assert_refused(
        &revoke(&api, globex, invite.id, &other_headers),
        "wrong space",
    );
    scene.gets_invite(&scene.frank, &invite);

    let admin = cookie(&scene, &team.bob, GITHUB + 1);
    let session = ("cookie", admin.as_str());
    for headers in [
        &[session, ("origin", "https://other.test")][..],
        &[session, ("origin", "null")],
        &[session],
    ] {
        assert_refused(
            &revoke(&api, space, invite.id, headers),
            "outside the web app",
        );
        scene.gets_invite(&scene.frank, &invite);
    }
}

fn admission(scene: &Scene, invite: InviteId, guest: &Person) -> Vec<u8> {
    let admin = &scene.team().alice;
    let current = scene.f.revisions.last().expect("a roster revision");
    let mut revision = next(current, &admin.laptop);
    let mut newcomer = member(guest, Role::Member);
    newcomer.invite = Some(invite);
    revision.members.push(newcomer);
    admin.sign(&revision)
}

#[test]
fn revoking_an_invite_removes_its_waiting_acceptance_and_refuses_it_at_once() {
    const GITHUB: u64 = 3_819_810_021;
    let scene = Scene::new(GITHUB);
    let team = scene.team();
    let api = Api::of(&team.flow);
    let space = scene.space();
    let admin = cookie(&scene, &team.bob, GITHUB + 1);
    let headers = [("cookie", admin.as_str()), ("origin", api.public)];
    let invite = scene.invite();
    let other = scene.invite();
    let eve = &scene.f.eve;
    let frank = &scene.frank;
    let accepted = acceptance(&invite, eve);
    assert_taken(&eve.session.accept(invite.id, &accepted));
    assert_eq!(scene.waiting_bytes(), std::slice::from_ref(&accepted));
    scene.gets_invite(eve, &invite);
    let revision = admission(&scene, invite.id, eve);

    let response = revoke(&api, space, invite.id, &headers);
    assert_eq!(response.status, 204, "{}", text(&response));
    gets_no_invite(eve, invite.id, Some(&invite), "revoked");
    assert_refused(
        &eve.session.accept(invite.id, &accepted),
        "acceptance retry",
    );
    assert_refused(
        &frank.session.accept(invite.id, &acceptance(&invite, frank)),
        "another acceptance",
    );
    assert!(
        scene.waiting_bytes().is_empty(),
        "the revoked acceptance is still waiting"
    );
    assert_refused(
        &team.alice.session.store(space, &revision),
        "admission after revocation",
    );
    assert_refused(
        &eve.session.revisions_response(space),
        "waiting newcomer's access",
    );

    let rows = listed(&api, space, &admin);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["invite"], other.id.to_string());
    scene.gets_invite(frank, &other);
}

#[test]
fn a_revoke_request_after_admission_preserves_the_roster_and_the_new_members_access() {
    const GITHUB: u64 = 3_819_810_031;
    let scene = Scene::new(GITHUB);
    let team = scene.team();
    let space = scene.space();
    let invite = scene.invite();
    let guest = &scene.f.eve;
    let accepted = acceptance(&invite, guest);
    assert_taken(&guest.session.accept(invite.id, &accepted));
    assert_taken(
        &team
            .alice
            .session
            .store(space, &admission(&scene, invite.id, guest)),
    );

    let api = Api::of(&team.flow);
    let admin = cookie(&scene, &team.alice, GITHUB);
    let headers = [("cookie", admin.as_str()), ("origin", api.public)];
    assert!(
        listed(&api, space, &admin).is_empty(),
        "the used invite is still listed"
    );
    let revisions = team.alice.session.revisions(space);

    let response = revoke(&api, space, invite.id, &headers);
    if response.status != 204 {
        assert_refused(&response, "the invite already ended on admission");
    }
    assert_eq!(team.alice.session.revisions(space), revisions);
    assert_eq!(scene.f.eve.session.revisions(space), revisions);
}
