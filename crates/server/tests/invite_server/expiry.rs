//! An invite ends at its expiry: from then on the server gives and takes neither it nor its
//! acceptance, and deletes both with no request for them. The end of an invite is moved back in the
//! database.

use std::thread;
use std::time::{Duration, Instant};

use crate::api::assert_refused;
use crate::common::TestDb;
use crate::common::devices::assert_taken;
use crate::scene::{Scene, acceptance, gets_no_invite};

/// Whether the database holds the invite ID `id`.
fn held(db: &TestDb, id: &str) -> bool {
    let id = id.to_owned();
    db.query(async |conn| {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM invites WHERE id::text = $1)")
            .bind(id)
            .fetch_one(conn)
            .await
    })
    .expect("read the invites")
}

/// Moves the end of the invite `id` back to a second ago.
fn end(db: &TestDb, id: &str) {
    db.execute(&format!(
        "UPDATE invites SET expires_at = now() - interval '1 second' WHERE id = '{id}'"
    ));
}

/// Right after the end of an accepted invite is moved back, before a sweep deletes it: alice's
/// session gets no waiting acceptance and stores the invite no more, eve's gets neither the invite
/// nor `acme`'s revisions and devices, and frank's acceptance is refused.
#[test]
fn an_invite_past_its_end_is_neither_given_nor_taken() {
    let scene = Scene::new(3_817_600_101);
    let (alice, eve, frank) = (&scene.team().alice, &scene.f.eve, &scene.frank);
    let space = scene.space();
    let invite = scene.invite();
    assert_taken(&eve.session.accept(invite.id, &acceptance(&invite, eve)));

    end(scene.f.db(), &invite.id.to_string());

    let refused = [
        ("alice's again", alice.session.put_invite(space, &invite)),
        ("eve's revisions", eve.session.revisions_response(space)),
        ("eve's devices", eve.session.devices_response(space)),
        (
            "frank's",
            frank.session.accept(invite.id, &acceptance(&invite, frank)),
        ),
    ];
    for (what, response) in &refused {
        assert_refused(response, what);
    }
    gets_no_invite(eve, invite.id, Some(&invite), "past its end");
    assert!(scene.waiting_bytes().is_empty(), "waiting");
}

/// Of two invite IDs alice's session gets, the one whose end the database moved back is deleted
/// within a few sweeps; the other stays.
#[test]
fn an_invite_past_its_end_is_deleted() {
    let scene = Scene::new(3_817_600_001);
    let alice = &scene.team().alice;
    let (ended, open) = (
        alice.session.issue(scene.space()).to_string(),
        alice.session.issue(scene.space()).to_string(),
    );
    end(scene.f.db(), &ended);

    let deadline = Instant::now() + Duration::from_secs(30);
    while held(scene.f.db(), &ended) {
        assert!(Instant::now() < deadline, "the ended invite is still held");
        thread::sleep(Duration::from_millis(200));
    }
    assert!(held(scene.f.db(), &open), "the open invite");
}
