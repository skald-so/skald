//! The web app's Machines section with `skald` on the machines: the account's devices with their
//! states, as the server's checks give them. Against a `skald-server` with GitHub sign-in through
//! the GitHub stand-in, on PostgreSQL, behind the recording relay and the front of the web app's
//! harness (`server/tests/common/spa.rs`), on which the machines reach it too; a test plays the
//! person in headless Chrome.
//!
//! The web app, as these tests use it, beside `common/spa.rs`: the cabinet opens on the Machines
//! section, a table with a row per device of the account, which names the device and says whether
//! it is "Linked", "Waiting for linking" or "Revoked". It gets them from `GET /account/devices`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The person's browser, the web app's harness and the rows of its tables.
#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;

use access::rows_holding;
use browser::{Browser, Page};
use cloud::{Cloud, certified};
use machine::{FIRST_DEVICE, Machine, NO};
use spa::{Spa, answers_without_secrets, wait_for_cabinet, wait_until};

/// A host of the server's public address that the machines resolve, as Chrome does.
const HOST: &str = "localhost";
/// The request of the account's devices.
const ACCOUNT_DEVICES: &str = "/account/devices";
/// How long a machine's code works, in seconds.
const CODE_LIFETIME: u64 = 300;
/// The command that revokes a device.
const REVOKE_DEVICE: &str = "revoke-device";

/// The words of a device's states, as a row says them in any case.
const STATES: [&str; 3] = ["linked", "waiting", "revoked"];

/// The states the one row of `page` naming `device` says; empty unless one row names it.
fn states_of(page: &Page, device: &str) -> Vec<&'static str> {
    let rows = rows_holding(page, device);
    if rows.len() != 1 {
        return Vec::new();
    }
    let row = rows[0].to_lowercase();
    STATES
        .into_iter()
        .filter(|state| row.contains(state))
        .collect()
}

/// Opens the web app in `page` and waits until the cabinet of `username` shows, on its own, each
/// of `devices` in one row saying its state, and only that one.
fn wait_for_machines(page: &Page, spa: &Spa, username: &str, devices: &[(&str, &str)]) {
    page.goto(&spa.at("/"));
    wait_for_cabinet(page, username);
    wait_until(page, &format!("Machines listing {devices:?}"), |page| {
        devices
            .iter()
            .all(|(device, state)| states_of(page, device) == [*state])
    });
}

/// Ida's laptop signs in with `skald` as her account's first device, approved in the browser she
/// then opens the web app in: the cabinet opens on Machines, which lists the laptop as linked. Her
/// desk machine then signs in as not the first device: Machines lists it as waiting for linking.
/// Once the laptop revokes itself, Machines lists it as revoked and the desk still waiting. The web
/// app got the devices from the server, and no answer it got holds a secret.
#[test]
fn machines_lists_the_first_machine_linked_a_second_waiting_and_a_revoked_one_revoked() {
    let browser = Browser::launch();
    let page = browser.page();
    page.record_answers();
    let spa = Spa::start(&browser, HOST);
    let cloud = Cloud::fronted(HOST, spa.port, CODE_LIFETIME, 1);
    spa.serve(&browser, &cloud.relay.url);
    let username = "ida-machines-standin";
    let user = cloud.user(4_224_400_001, "Ida-Machines-Standin");

    let (laptop, signed_in) = cloud.first_machine(&page, &user, "ida-laptop");
    assert!(signed_in.success, "{signed_in:?}");
    let token = cloud.last_token();
    wait_for_machines(&page, &spa, username, &[("ida-laptop", "linked")]);

    let desk = Machine::new().with_host_name("ida-desk");
    // The machine signs in and waits for linking.
    let _ = cloud.signed_in(&page, &desk, &user, &[FIRST_DEVICE, NO]);
    wait_for_machines(
        &page,
        &spa,
        username,
        &[("ida-laptop", "linked"), ("ida-desk", "waiting")],
    );

    let linked = certified(&cloud.devices(&token));
    assert_eq!(linked.len(), 1, "the laptop's device: {linked:?}");
    let revoked = laptop
        .start(&[REVOKE_DEVICE, &linked[0].to_string()])
        .finish();
    assert!(revoked.success, "{revoked:?}");
    wait_for_machines(
        &page,
        &spa,
        username,
        &[("ida-laptop", "revoked"), ("ida-desk", "waiting")],
    );

    let answers = answers_without_secrets(&page);
    assert!(
        answers.iter().any(|answer| answer.method == "GET"
            && answer.status == 200
            && answer
                .url
                .split('?')
                .next()
                .unwrap_or_default()
                .ends_with(ACCOUNT_DEVICES)),
        "no answer to GET {ACCOUNT_DEVICES}: {answers:?}"
    );
}
