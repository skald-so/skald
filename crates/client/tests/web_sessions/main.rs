//! A CLI session ended in the web app's Access section: the machine's next command is refused, its
//! `skald recv` connection closes, and `skald sign-in` then resumes the same device. Against a
//! `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind the
//! recording relay, the front of the tests of sending (`sending/front.rs`), and before it the front
//! of the web app's harness (`server/tests/common/spa.rs`) at the server's public address, on which
//! the machines reach it too; a test plays the person in headless Chrome. The web app as these
//! tests use it: `server/tests/web_sessions/access.rs`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sending/front.rs"]
mod front;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

use std::thread;
use std::time::{Duration, Instant};

use access::{WHEN_REVOKED, open_access, revoke, rows_holding};
use browser::Browser;
use cloud::{Cloud, GIVE_MASTER, WAIT};
use front::Front;
use machine::{DEVICES, FIRST_DEVICE, YES};
use spa::{Spa, answers_without_secrets, wait_for_cabinet, wait_until};

/// A host of the server's public address that the machines resolve, as Chrome does.
const HOST: &str = "localhost";
/// The command that receives messages.
const RECV: &str = "recv";
/// How soon a `skald recv` whose session ended exits.
const EXITS: Duration = Duration::from_secs(30);

/// Ada's laptop signs in with `skald` as her account's first device, which she approves in the
/// browser she then opens the web app in. While its `skald recv` waits for the end of its catch-up, which the front holds back,
/// she revokes the laptop's CLI session in Access, its row naming the laptop and expiring "when
/// revoked". The `skald recv` exits with a failure, the server refuses the old session, and
/// `skald devices` fails. `skald sign-in` then resumes the same device: a new session, the same
/// devices and master key, none given again; `skald recv` connects as that device, and Access
/// lists the new CLI session by the laptop's name.
#[test]
fn cli_session_revoked_in_access_is_refused_and_sign_in_resumes_the_device() {
    let browser = Browser::launch();
    let spa = Spa::start(&browser, HOST);
    let front = Front::bind();
    let cloud = Cloud::fronted(HOST, spa.port, 120, 1);
    front.forward_to(cloud.relay.address());
    spa.serve(&browser, &format!("http://127.0.0.1:{}", front.port()));
    let user = cloud.user(4_222_700_001, "Ada-Access-Cli-Standin");
    let username = "ada-access-cli-standin";
    let page = browser.page();
    page.record_answers();

    // Ada signs in to the server on the verification page as she approves the laptop's code.
    let (laptop, signed_in) = cloud.first_machine(&page, &user, "ada-laptop");
    assert!(signed_in.success, "{signed_in:?}");
    let token = cloud.last_token();
    let devices = cloud.devices(&token);
    let master = cloud.master(&token);
    assert_eq!(devices.len(), 1, "{devices:?}");

    front.hold_end_until_notice(true);
    let held = front.held_ends();
    let recv = laptop.start(&[RECV]);
    let deadline = Instant::now() + WAIT;
    while front.held_ends() == held {
        assert!(Instant::now() < deadline, "no catch-up: {}", recv.shown());
        thread::sleep(Duration::from_millis(50));
    }

    page.goto(&spa.at("/"));
    wait_for_cabinet(&page, username);
    open_access(&page);
    let row = rows_holding(&page, "ada-laptop");
    assert_eq!(row.len(), 1, "the laptop's CLI session");
    assert!(row[0].to_lowercase().contains(WHEN_REVOKED), "{row:?}");
    revoke(&page, "ada-laptop");
    let revoked = Instant::now();
    wait_until(&page, "the CLI session gone", |page| {
        rows_holding(page, "ada-laptop").is_empty()
    });

    let received = recv.finish();
    assert!(
        revoked.elapsed() < EXITS,
        "skald recv exited {:?} after the revocation",
        revoked.elapsed()
    );
    assert!(!received.success, "{received:?}");
    let (status, body) = cloud.session_of(&token);
    assert_eq!(status, 401, "the revoked session: {body}");
    let listing = laptop.start(&[DEVICES]).finish();
    assert!(!listing.success, "{listing:?}");

    front.hold_end_until_notice(false);
    let mark = cloud.relay.mark();
    let again = cloud.signed_in(&page, &laptop, &user, &[FIRST_DEVICE, YES]);
    assert!(again.success, "{again:?}");
    let resumed = cloud.last_token();
    assert_ne!(resumed, token);
    assert_eq!(cloud.devices(&resumed), devices, "the same device");
    assert_eq!(cloud.master(&resumed), master);
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    let listing = laptop.start(&[DEVICES]).finish();
    assert!(listing.success, "{listing:?}");
    let received = laptop.start(&[RECV]).finish();
    assert!(received.success, "{received:?}");

    page.goto(&spa.at("/"));
    wait_for_cabinet(&page, username);
    open_access(&page);
    assert_eq!(
        rows_holding(&page, "ada-laptop").len(),
        1,
        "the new session"
    );
    answers_without_secrets(&page);
}
