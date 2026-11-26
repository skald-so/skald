//! Bob's messages from before he deleted his account still show once the server has erased it, on
//! Ada's machine and on a machine of hers linked after.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{AccountId, PROTOCOL_VERSION, SpaceId, VERSION_HEADER};

use crate::cloud::{Cloud, agent};
use crate::devices::{BOB, said_by};
use crate::fixtures::hex;
use crate::kit::received;
use crate::linking::{NewDevice, link};
use crate::reading::{assert_history, sends, shown_history};
use crate::talk::Talk;
use crate::world::ACME;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";
/// How soon the server erases an account once its deletion is 30 days old.
const ERASURE: Duration = Duration::from_secs(30);

/// The account of the device session `token` deletes itself through the API; fails unless the
/// server deletes it.
#[track_caller]
fn deletes_account(cloud: &Cloud, token: &str) {
    let response = agent()
        .delete(format!("{}/account", cloud.url))
        .header(VERSION_HEADER, PROTOCOL_VERSION.to_string())
        .header("authorization", format!("Bearer {token}"))
        .call();
    let status = match response {
        Ok(response) => response.status().as_u16(),
        Err(ureq::Error::StatusCode(status)) => status,
        Err(err) => panic!("DELETE /account: {err}"),
    };
    assert_eq!(status, 204, "DELETE /account");
}

/// Moves the deletion of `account` back 30 days in the server's database.
fn moves_deletion_back(cloud: &Cloud, account: AccountId) {
    cloud.execute(&format!(
        "UPDATE accounts SET deleted_at = deleted_at - interval '30 days'
         WHERE id = '{account}'::uuid AND deleted_at IS NOT NULL"
    ));
}

/// The rows of the server's database that hold `master`, a master public key, and not `space`'s
/// ID, as the rows of `space`'s revisions do; in lowercase.
fn holding_master(cloud: &Cloud, master: &[u8; 32], space: SpaceId) -> Vec<String> {
    let (master, space) = (hex(master), space.to_string());
    let tables = cloud.rows();
    let rows = tables.iter().flat_map(|table| table.split("<row>").skip(1));
    rows.map(str::to_lowercase)
        .filter(|row| row.contains(&master) && !row.contains(&space))
        .collect()
}

/// Waits for up to [`ERASURE`] until the server has erased the account of `master`, which shared
/// only `space`: no row holds the master key but those of `space`'s revisions.
#[track_caller]
fn until_erased(cloud: &Cloud, master: &[u8; 32], space: SpaceId) {
    let deadline = Instant::now() + ERASURE;
    loop {
        let rows = holding_master(cloud, master, space);
        if rows.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Bob's master key after {ERASURE:?}: {rows:#?}"
        );
        thread::sleep(Duration::from_millis(250));
    }
}

/// In the talk's channel Bob says a line and Ada's machine sends one. Bob deletes his account, and
/// 30 days later the server erases it. Ada's `skald history` still shows both lines, oldest first.
/// Then her new machine is linked with its code, and once it has run, and her first one, and it
/// again, its `skald history` shows both lines too.
#[test]
fn members_still_read_an_erased_accounts_messages_also_on_a_device_linked_after() {
    let mut talk = Talk::in_space(
        ACME,
        &[4_304_001_001, 4_304_001_002],
        &["ada-keeps-the-record", "bob-leaves-for-good"],
        false,
    );
    let at = talk.at();
    let said = [
        said_by(&mut talk, BOB, (at, 1), "bob hands over the runbook").0,
        sends(&talk.ada, (ACME, at.1), "ada thanks bob for the runbook"),
    ];
    let cloud = talk.cloud();
    let bob = &talk.played[BOB].person;
    deletes_account(cloud, &bob.token);
    moves_deletion_back(cloud, bob.account);
    until_erased(cloud, &bob.master, at.0);

    let shown = shown_history(&talk.ada.machine, at.1);
    assert_history(&shown, at.1, &said, &[]);

    let mut new = NewDevice::start(&talk.world, &talk.ada, LAPTOP);
    link(cloud, &talk.ada, &mut new);
    let laptop = new.into_person(cloud, &talk.ada);
    for machine in [&laptop.machine, &talk.ada.machine, &laptop.machine] {
        received(machine);
    }
    let shown = shown_history(&laptop.machine, at.1);
    assert_history(&shown, at.1, &said, &[]);
}
