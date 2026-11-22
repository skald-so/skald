//! Issuing invite IDs and storing invites: only for sessions of the space's admins, signed by a
//! certified, unrevoked `client` device of the session's account, of the space with an ID issued for
//! it, expiring within 30 days, naming channels of the latest revision with the default channel
//! first; once, also when sent twice.

use skald_proto::{ChannelId, DeviceKind, InviteId, Role};

use crate::api::{ACME, Person, assert_refused};
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::assert_taken;
use crate::common::sign_in::{SignIn, WebAddress};
use crate::rosters::first;
use crate::scene::{
    DAY, MALFORMED, Scene, assert_stored, broken, gets_no_invite, made, signed_invite,
    signed_invite_with_later_field,
};
use crate::wire::now;

/// Checks that `person`'s session is refused `bytes` as the invite `id` of `acme`, and that the
/// server gives no invite `id`; `what` names the case.
#[track_caller]
fn refused(scene: &Scene, person: &Person, id: InviteId, bytes: &[u8], what: &str) {
    let response = person.session.store_invite(scene.space(), id, bytes);
    assert_refused(&response, what);
    gets_no_invite(&scene.f.eve, id, None, what);
}

/// alice's session gets an ID and stores an invite, `201`, and the same bytes again, `200`; eve's
/// session then gets them byte for byte. So are taken: bob's, another admin's, signed by his
/// laptop; one signed by alice's tablet, certified by her laptop; one for an admin with only the
/// default channel and a field of a later version; one expiring a little under 30 days ahead.
#[test]
fn an_admins_session_gets_an_id_and_stores_an_invite_once_also_when_sent_twice() {
    let scene = Scene::new(3_817_100_001);
    let t = scene.team();
    let (alice, bob) = (&t.alice, &t.bob);
    let space = scene.space();
    let general = vec![(scene.f.general, true)];

    let by_alice = scene.invite_with(|_| {});
    assert_stored(&alice.session.put_invite(space, &by_alice), true, "first");
    assert_stored(&alice.session.put_invite(space, &by_alice), false, "again");
    scene.gets_invite(&scene.f.eve, &by_alice);

    let tablet = alice.certified_by(scene.f.db(), &alice.laptop, DeviceKind::Client, "Tablet");
    let by_bob = made(space, bob, &bob.laptop, general.clone(), |_| {});
    let by_tablet = made(space, alice, &tablet, general.clone(), |_| {});
    let mut later = made(space, alice, &alice.laptop, general, |body| {
        body.role = Role::Admin;
    });
    later.bytes = signed_invite_with_later_field(&later.body, &alice.laptop.signing.key);
    let near = scene.invite_with(|body| body.expires = now() + 30 * DAY - 3_600);
    for (person, invite, what) in [
        (bob, &by_bob, "bob's"),
        (alice, &by_tablet, "by alice's tablet"),
        (alice, &later, "for an admin, with a later field"),
        (alice, &near, "expiring in a little under 30 days"),
    ] {
        assert_stored(&person.session.put_invite(space, invite), true, what);
        assert_stored(&person.session.put_invite(space, invite), false, what);
        scene.gets_invite(&scene.f.eve, invite);
    }
}

/// carol, a member who is no admin, and eve, outside the space, get no invite ID; an invite with an
/// ID issued to alice, signed by their own laptops, is refused from their sessions and not stored:
/// alice's session then stores one under that ID.
#[test]
fn sessions_of_accounts_that_are_no_admins_get_no_id_and_store_no_invite() {
    let scene = Scene::new(3_817_100_101);
    let space = scene.space();
    let t = scene.team();
    for person in [&t.carol, &scene.f.eve] {
        let what = format!("an ID for {}", person.handle);
        assert_refused(&person.session.issue_response(space), &what);

        let id = t.alice.session.issue(space);
        let theirs = scene.invite_with(|body| {
            body.invite = id;
            body.signer = person.laptop.id;
        });
        let bytes = signed_invite(&theirs.body, &person.laptop.signing.key);
        let what = format!("an invite from {}", person.handle);
        refused(&scene, person, id, &bytes, &what);

        let valid = scene.invite_with(|body| body.invite = id);
        assert_stored(&t.alice.session.put_invite(space, &valid), true, &what);
    }
}

/// From alice's session, invites signed by bob's laptop, a device of another account, by her
/// revoked tablet, by her device without a certificate and by her `service` device, and one with a
/// broken signature, are refused and not stored.
#[test]
fn invites_signed_by_a_device_that_may_not_sign_them_are_refused() {
    let scene = Scene::new(3_817_100_201);
    let t = scene.team();
    let alice = &t.alice;
    let db = scene.f.db();
    let tablet = alice.certified_by(db, &alice.laptop, DeviceKind::Client, "Tablet");
    alice.revoke(&alice.by_device(&alice.laptop, tablet.id));
    let bot = alice.certified_by(db, &alice.laptop, DeviceKind::Service, "Bot");

    for (signer, what) in [
        (&t.bob.laptop, "by a device of another account"),
        (&tablet, "by a revoked device"),
        (&alice.uncertified, "by a device without a certificate"),
        (&bot, "by a service device"),
    ] {
        let invite = scene.invite_with(|body| body.signer = signer.id);
        let bytes = signed_invite(&invite.body, &signer.signing.key);
        refused(&scene, alice, invite.id, &bytes, what);
    }
    let invite = scene.invite_with(|_| {});
    let what = "with a broken signature";
    refused(&scene, alice, invite.id, &broken(&invite.bytes), what);
    assert_stored(
        &alice.session.put_invite(scene.space(), &invite),
        true,
        what,
    );
}

/// alice is the admin of `globex` too. An invite of `globex` under an ID issued for `acme`, one of
/// `acme` under an ID issued for `globex`, one under an ID never issued and one sent under another
/// issued ID than its own are refused as invites of `acme` and not stored.
#[test]
fn invites_of_another_space_or_with_an_id_not_issued_for_it_are_refused() {
    let scene = Scene::new(3_817_100_301);
    let alice = &scene.team().alice;
    let globex = alice.create_space("globex");
    assert_taken(
        &alice
            .session
            .store(globex, &alice.sign(&first(globex, "globex", alice))),
    );

    let of_globex = scene.invite_with(|body| body.space = globex);
    let what = "of another space";
    refused(&scene, alice, of_globex.id, &of_globex.bytes, what);

    let globex_id = alice.session.issue(globex);
    let with_globex_id = scene.invite_with(|body| body.invite = globex_id);
    let what = "with an ID issued for another space";
    refused(&scene, alice, globex_id, &with_globex_id.bytes, what);

    let never = InviteId::generate();
    let with_unknown_id = scene.invite_with(|body| body.invite = never);
    refused(
        &scene,
        alice,
        never,
        &with_unknown_id.bytes,
        "with an ID never issued",
    );

    let invite = scene.invite_with(|_| {});
    let other = alice.session.issue(scene.space());
    refused(&scene, alice, other, &invite.bytes, "under another ID");
}

/// Invites that expired a minute ago or expire an hour after 30 days from now are refused and not
/// stored.
#[test]
fn invites_expired_or_expiring_after_30_days_are_refused() {
    let scene = Scene::new(3_817_100_401);
    let alice = &scene.team().alice;
    for (expires, what) in [
        (now() - 60, "expired"),
        (now() + 30 * DAY + 3_600, "expiring after 30 days"),
    ] {
        let invite = scene.invite_with(|body| body.expires = expires);
        refused(&scene, alice, invite.id, &invite.bytes, what);
    }
}

/// Invites that name a channel of `acme` that the latest revision lacks, `ops` before `general`,
/// only `ops`, or no channel, are refused and not stored.
#[test]
fn invites_off_the_latest_revisions_channels_or_without_the_default_channel_first_are_refused() {
    let scene = Scene::new(3_817_100_501);
    let alice = &scene.team().alice;
    let (general, ops) = (scene.f.general, scene.ops);
    let unlisted: ChannelId = alice.session.channel(scene.space());
    for (channels, what) in [
        (
            vec![(general, true), (unlisted, false)],
            "a channel the revision lacks",
        ),
        (vec![(ops, false), (general, true)], "another channel first"),
        (vec![(ops, false)], "without the default channel"),
        (vec![], "no channel"),
    ] {
        let invite = scene.invite_with(|body| body.channels = channels);
        refused(&scene, alice, invite.id, &invite.bytes, what);
    }
}

/// Bytes that are no invite are refused under an issued ID, which then takes an invite; other bytes
/// under that ID are refused, and the invite stored first is kept.
#[test]
fn malformed_invites_and_other_bytes_under_a_stored_id_are_refused() {
    let scene = Scene::new(3_817_100_601);
    let alice = &scene.team().alice;
    let invite = scene.invite_with(|_| {});
    refused(&scene, alice, invite.id, MALFORMED, "malformed");

    assert_stored(
        &alice.session.put_invite(scene.space(), &invite),
        true,
        "valid",
    );
    let mut body = invite.body.clone();
    body.expires -= 60;
    let other = signed_invite(&body, &alice.laptop.signing.key);
    let response = alice.session.store_invite(scene.space(), invite.id, &other);
    assert_refused(&response, "other bytes under a stored ID");
    scene.gets_invite(&scene.f.eve, &invite);
}

/// The answer that issues an invite ID names the server's web address, which an invite link starts
/// with: its public address by default, and the address `--web-url` sets.
#[test]
fn the_answer_that_issues_an_id_names_the_servers_web_address() {
    let scene = Scene::new(3_817_100_701);
    let response = scene.team().alice.session.issue_response(scene.space());
    let public = &scene.team().flow.stand.public;
    assert_eq!(
        json(&response)["web"],
        public.as_str(),
        "{}",
        text(&response)
    );

    let (stand, _) = SignIn::with_web_address(WebAddress::Argument, |port| {
        (
            format!("http://api.skald.test:{port}"),
            format!("http://app.skald.test:{port}"),
        )
    });
    let flow = DeviceFlow::of(stand);
    let alice = Person::new(&flow, 3_817_100_801, "alice");
    let space = alice.create_space(ACME);
    assert_taken(
        &alice
            .session
            .store(space, &alice.sign(&first(space, ACME, &alice))),
    );
    let response = alice.session.issue_response(space);
    assert_eq!(
        json(&response)["web"],
        flow.stand.public.as_str(),
        "{}",
        text(&response)
    );
}
