//! A run's token sends at most 8 later messages of at most 4096 bytes each; an account enters at
//! most 10 pairings the server does not hold within 15 minutes; a pairing is refused once the run's
//! device registered on another account.

use crate::assert_problem;
use crate::common::cbor::{array_of, byte_item, uint};
use crate::common::devices::{Device, assert_taken};
use crate::kit::{Fresh, pair, register, send_pairing};
use crate::relay::{abort, reply, request};

/// `link-request = [3, session, sealed]` with `len` bytes sealed.
fn request_of(session: &[u8; 16], len: usize) -> Vec<u8> {
    array_of(&[uint(3), byte_item(session), byte_item(&vec![7; len])])
}

#[test]
fn a_runs_token_sends_at_most_8_later_messages() {
    let f = Fresh::start(3_175_390_011);
    let run = f.open(&Device::new("Tablet"));
    pair(&f.alice.session, &run.pairing);
    let sent: Vec<Vec<u8>> = (0..8).map(|_| request(&run.session)).collect();
    for message in &sent {
        assert_taken(&run.send(message));
    }
    assert_problem(
        &run.send(&request(&run.session)),
        429,
        "too-many-link-messages",
        "the 9th",
    );
    assert_eq!(run.messages(), sent, "the 9th is not relayed");
    // The issuing side is not bound so, and the token still ends the run.
    assert_taken(&f.alice.session.send_message(&reply(&run.session)));
    assert_taken(&run.send(&abort(&run.session)));
    run.until_ended("aborted after its 8 messages");
}

#[test]
fn a_message_of_more_than_4096_bytes_from_a_runs_token_is_refused() {
    let f = Fresh::start(3_175_390_021);
    let run = f.open(&Device::new("Tablet"));
    let largest = request_of(&run.session, 4096 - 22);
    assert_eq!(largest.len(), 4096, "the message, heads and all");
    assert_taken(&run.send(&largest));
    let over = request_of(&run.session, 4096 - 21);
    assert_problem(&run.send(&over), 400, "invalid-link-message", "4097 bytes");
    assert_eq!(run.messages(), vec![largest], "only the 4096 bytes");
}

#[test]
fn an_account_enters_at_most_10_pairings_the_server_does_not_hold() {
    let f = Fresh::start(3_175_390_031);
    let run = f.open(&Device::new("Tablet"));
    for at in 0..10 {
        assert_problem(
            &send_pairing(&f.alice.session, "AAAAAA"),
            404,
            "no-link-run",
            &format!("wrong pairing {at}"),
        );
    }
    assert_problem(
        &send_pairing(&f.alice.session, &run.pairing),
        429,
        "too-many-pairings",
        "the right pairing after 10 wrong ones",
    );
    f.flow
        .stand
        .db
        .execute("UPDATE wrong_pairings SET entered_at = now() - interval '15 minutes'");
    assert_eq!(
        pair(&f.alice.session, &run.pairing),
        run.hello,
        "once they are 15 minutes old"
    );

    // Another account is not bound by alice's.
    let other = f.open(&Device::new("Phone"));
    for _ in 0..10 {
        send_pairing(&f.alice.session, "AAAAAA");
    }
    assert_eq!(pair(&f.bob.session, &other.pairing), other.hello);
}

#[test]
fn a_pairing_is_refused_once_the_runs_device_registered_on_another_account() {
    let f = Fresh::start(3_175_390_041);
    let tablet = Device::new("Tablet");
    let run = f.open(&tablet);
    register(&f.bob.session, &tablet);
    assert_problem(
        &send_pairing(&f.alice.session, &run.pairing),
        404,
        "no-link-run",
        "alice, the device on bob's account",
    );
    assert_eq!(
        pair(&f.bob.session, &run.pairing),
        run.hello,
        "bob pairs it"
    );
}

#[test]
fn a_paired_run_ends_once_its_device_registers_on_another_account() {
    let f = Fresh::start(3_175_390_051);
    let tablet = Device::new("Tablet");
    let run = f.open(&tablet);
    pair(&f.alice.session, &run.pairing);
    register(&f.bob.session, &tablet);
    run.until_ended("its device on bob's account");
    crate::kit::until_refused(
        &f.alice.session,
        &reply(&run.session),
        "alice's reply after the device registered on bob's account",
    );
}
