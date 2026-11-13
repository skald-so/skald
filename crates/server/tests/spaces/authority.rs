//! Who stores a revision: revision 1 only a session of the space's creator, listing that account as
//! an admin; revision n+1 only a session of an admin in revision n. Its `signer` is a device of the
//! session's account with a certificate on the server, and its signature verifies with that key.

use skald_crypto::SigningKey;
use skald_proto::{CERT_LABEL, Cbor, ROSTER_LABEL, Role, RosterBody};

use crate::api::{ACME, Person, Team};
use crate::rosters::{encoded, first, member, next, signed};

/// `body` with alice's account the member `role`, or not a member if `None`, and bob's an admin.
fn first_with_alice(team: &Team, role: Option<Role>) -> RosterBody {
    let mut body = first(team.space, ACME, &team.bob);
    body.signer = team.alice.laptop.id;
    if let Some(role) = role {
        body.members.push(member(&team.alice, role));
    }
    body
}

#[test]
fn revision_one_is_stored_only_from_the_creators_session_listing_it_as_an_admin() {
    let team = Team::new(3_120_830_001);
    let (alice, bob) = (&team.alice, &team.bob);

    let bobs = bob.sign(&first(team.space, ACME, bob));
    team.refused(bob, &bobs, &[], "bob's revision 1 of alice's space");
    let alices = alice.sign(&first(team.space, ACME, alice));
    team.refused(bob, &alices, &[], "alice's revision 1 from bob's session");
    let as_member = alice.sign(&first_with_alice(&team, Some(Role::Member)));
    team.refused(alice, &as_member, &[], "the creator listed as a member");
    let absent = alice.sign(&first_with_alice(&team, None));
    team.refused(alice, &absent, &[], "the creator not listed");

    team.stored_by(alice, &alices);
    assert_eq!(alice.session.revisions(team.space), [alices]);
}

#[test]
fn a_next_revision_is_stored_only_from_an_admins_session() {
    let team = Team::new(3_120_830_011);
    let (alice, bob, carol, dave) = (&team.alice, &team.bob, &team.carol, &team.dave);
    let stored = team.with_members().to_vec();
    // Revision 3 signed by `person`'s laptop, taking dave in as a member.
    let by = |person: &Person| {
        let mut body = next(&stored[1], &person.laptop);
        body.members.push(member(dave, Role::Member));
        person.sign(&body)
    };

    team.refused(carol, &by(carol), &stored, "a member that is no admin");
    team.refused(dave, &by(dave), &stored, "an account outside the space");
    let alices = by(alice);
    team.refused(
        carol,
        &alices,
        &stored,
        "an admin's revision from a member's session",
    );
    team.refused(
        dave,
        &alices,
        &stored,
        "an admin's revision from an outsider's session",
    );

    let bobs = by(bob);
    team.stored_by(bob, &bobs);
    assert_eq!(
        alice.session.revisions(team.space),
        [stored, vec![bobs]].concat()
    );
}

/// `body` signed by `key` over `label`.
fn signed_over(label: &str, body: &RosterBody, key: &SigningKey) -> Vec<u8> {
    let bytes = body.to_cbor();
    encoded(&bytes, &key.sign(label, &[&bytes]))
}

/// The revisions of `body` whose `signer` or signature is wrong for `signer`'s session: each with
/// what it breaks.
fn wrongly_signed(team: &Team, signer: &Person, body: &RosterBody) -> Vec<(String, Vec<u8>)> {
    let other = if signer.account == team.alice.account {
        &team.bob
    } else {
        &team.alice
    };
    let mut cases = Vec::new();

    let mut broken = signer.sign(body);
    let last = broken.len() - 1;
    broken[last] ^= 0x01;
    cases.push(("a broken signature", broken));

    let key = &signer.laptop.signing.key;
    cases.push((
        "a signature over another label",
        signed_over(CERT_LABEL, body, key),
    ));
    let key = &other.laptop.signing.key;
    cases.push((
        "a signature by another device",
        signed_over(ROSTER_LABEL, body, key),
    ));

    let mut uncertified = body.clone();
    uncertified.signer = signer.uncertified.id;
    let key = &signer.uncertified.signing.key;
    cases.push(("a signer without a certificate", signed(&uncertified, key)));

    let mut others = body.clone();
    others.signer = other.laptop.id;
    cases.push(("a signer of another account", other.sign(&others)));

    cases
        .into_iter()
        .map(|(what, revision)| (what.to_owned(), revision))
        .collect()
}

#[test]
fn the_signer_is_a_certified_device_of_the_sessions_account_and_its_signature_verifies() {
    let team = Team::new(3_120_830_021);
    let (alice, bob) = (&team.alice, &team.bob);

    let body = first(team.space, ACME, alice);
    for (what, revision) in wrongly_signed(&team, alice, &body) {
        team.refused(alice, &revision, &[], &format!("revision 1: {what}"));
    }

    let stored = team.with_members().to_vec();
    for signer in [alice, bob] {
        let body = next(&stored[1], &signer.laptop);
        for (what, revision) in wrongly_signed(&team, signer, &body) {
            let what = format!("revision 3 from {}'s session: {what}", signer.handle);
            team.refused(signer, &revision, &stored, &what);
        }
    }

    team.stored_by(bob, &bob.sign(&next(&stored[1], &bob.laptop)));
}
