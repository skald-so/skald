//! The account records of a space's members (`docs/protocol/membership.md`, "Account" and
//! "Wraps"): with a space's devices, the server gives the account record of each member account of
//! the space that has one, as the account's first device sent it, so that member devices wrap each
//! epoch's key to those accounts' recovery keys. An account whose master key came before account
//! records has none to give. Against a `skald-server` with GitHub sign-in through the stand-in, as
//! in the spaces tests; every HTTP response is checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The interface, as these tests use it, each request stating the protocol version with a
//! session's token: `GET /spaces/{space}/devices` gives, beside `devices` and `revocations`,
//! `records`: a list of account records, each its CBOR bytes in base64url without padding (RFC
//! 4648, §5), each once. Each account of the tests gives its master key with a record of revision 1
//! (`PUT /account/master`); the table `accounts`, whose `record` and `backup` a test clears.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests, and their revisions; this binary uses some of
// them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

use std::collections::BTreeSet;

use crate::api::{Person, Team};
use crate::common::device_flow::{json, text};
use crate::common::devices::{master_request, unb64};

/// The account record `person`'s machine gave with its master key.
fn record_of(person: &Person) -> Vec<u8> {
    let request = master_request(person.account, &person.master, &person.cert);
    unb64(request["record"].as_str().expect("an account record"))
}

/// The account records the server gives `person`'s session with the devices of `team`'s space, in
/// order; fails unless the answer is `200`.
fn records_given(team: &Team, person: &Person) -> Vec<Vec<u8>> {
    let response = person.session.devices_response(team.space);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    body["records"]
        .as_array()
        .unwrap_or_else(|| panic!("no records in {body}"))
        .iter()
        .map(|record| unb64(record.as_str().expect("an account record's bytes")))
        .collect()
}

/// Fails unless `given` holds each of `expected` once and nothing else.
#[track_caller]
fn assert_records(given: &[Vec<u8>], expected: &[Vec<u8>], whose: &str) {
    let distinct: BTreeSet<&Vec<u8>> = given.iter().collect();
    assert_eq!(distinct.len(), given.len(), "{whose}: a record twice");
    let wanted: BTreeSet<&Vec<u8>> = expected.iter().collect();
    assert_eq!(distinct, wanted, "{whose}: the records given");
}

/// With revision 2 of `acme` listing alice, bob and carol, each of their sessions gets with the
/// space's devices the account records of the three, byte for byte, each once, and not dave's,
/// whom no revision lists.
#[test]
fn a_space_s_devices_come_with_the_account_records_of_its_members() {
    let team = Team::new(3_177_701_001);
    team.with_members();
    let expected: Vec<Vec<u8>> = [&team.alice, &team.bob, &team.carol]
        .into_iter()
        .map(record_of)
        .collect();

    for person in [&team.alice, &team.bob, &team.carol] {
        let given = records_given(&team, person);

        assert_records(&given, &expected, person.handle.as_str());
        assert!(
            !given.contains(&record_of(&team.dave)),
            "{}: dave's record given",
            person.handle
        );
    }
}

/// The server's database then holds no record or master backup of carol's account, as for an
/// account whose master key came before records: the space's devices come with alice's and bob's
/// records alone, and still with carol's devices.
#[test]
fn a_member_without_an_account_record_has_none_among_them() {
    let team = Team::new(3_177_702_001);
    team.with_members();
    let carol = team.carol.account;
    team.flow.stand.db.execute(&format!(
        "UPDATE accounts SET record = NULL, backup = NULL WHERE id = '{carol}'::uuid"
    ));
    let expected = [record_of(&team.alice), record_of(&team.bob)];

    let given = records_given(&team, &team.alice);

    assert_records(&given, &expected, "alice");
    let devices = team.alice.session.devices(team.space);
    assert_eq!(
        devices.get(&team.carol.laptop.id.to_string()),
        Some(&Some(team.carol.cert.clone())),
        "carol's laptop in {devices:?}"
    );
}
