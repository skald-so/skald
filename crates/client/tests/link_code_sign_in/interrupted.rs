//! A run linked by its code that does not end linked: killed after it took its grant, before it
//! has its session, or refused its certificate over the devices limit. A later run of `skald
//! sign-in` on the machine stores the certificate and gets the session, with no sign-in.

use std::time::Instant;

use skald_proto::DeviceId;
use skald_server::Settings;

use crate::cloud::Cloud;
use crate::devices::second_machine;
use crate::fixtures::fronted_with;
use crate::linking::{YES, link_with, says, start_link, typed};
use crate::machine::{SERVER, SIGN_IN};
use crate::relay::strings_named;
use crate::revocations::revoked_by_cli;
use crate::terminal::{Shown, Terminal};
use crate::world::Person;
use crate::{
    Fresh, assert_key_session, certified_named, issuer_of, key_sessions, until, until_no_run_open,
};

/// The host names of Ada's machines.
const WORKSTATION: &str = "ada-workstation";
const DESKTOP: &str = "ada-desktop";
const LAPTOP: &str = "ada-laptop";

/// The device the hello of `fresh`'s run names, as its request to `POST /links` sends it.
#[track_caller]
fn hello_device(cloud: &Cloud) -> DeviceId {
    let sent = String::from_utf8_lossy(&cloud.relay.sent_bytes()).into_owned();
    let at = sent
        .find("POST /links ")
        .expect("a hello without a session");
    let named = strings_named(&sent[at..], "device");
    named
        .first()
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("the hello names no device: {named:?}"))
}

/// Whether `text` says on a line that the account has as many devices as it may: a line holding
/// `as many devices`, or both `device` and `limit`.
fn says_at_the_limit(text: &str) -> bool {
    says(text, &["as many devices"]) || says(text, &["device", "limit"])
}

/// Whether the server holds a certificate of a device named `name` of `first`'s account.
fn certified(cloud: &Cloud, first: &Person, name: &str) -> bool {
    cloud.devices(&first.token).iter().any(|device| {
        device
            .cert
            .as_ref()
            .is_some_and(|cert| cert.body().name.as_str() == name)
    })
}

/// For a failure: each request the relay passed from `since` on, its line and when it came, and
/// each device challenge the server holds, its device, expiry and whether a request used it,
/// beside the time now.
fn seen_since(cloud: &Cloud, since: Instant) -> String {
    let requests: Vec<String> = (cloud.relay.sent_since(since).iter())
        .map(|sent| {
            let line = sent.head.lines().next().unwrap_or_default();
            format!("  {:?} {line}", sent.at.duration_since(since))
        })
        .collect();
    let value = |row: &str, tag: &str| {
        let open = format!("<{tag}>");
        row.find(&open).map_or_else(String::new, |at| {
            let rest = &row[at + open.len()..];
            rest[..rest.find('<').unwrap_or(rest.len())].to_owned()
        })
    };
    let challenges: Vec<String> = (cloud.rows().iter())
        .filter(|table| table.contains("<challenge_hash>"))
        .flat_map(|table| table.split("<row>").skip(1).map(str::to_owned))
        .map(|row| {
            let used = !row.contains("<request xsi:nil");
            let (device, expires) = (value(&row, "device_id"), value(&row, "expires_at"));
            format!("  {device} expires {expires}, used: {used}")
        })
        .collect();
    format!(
        "requests:\n{}\ndevice challenges at {:?}:\n{}",
        requests.join("\n"),
        jiff::Timestamp::now(),
        challenges.join("\n")
    )
}

/// `skald sign-in --server <cloud>` on `fresh`'s machine again, at a terminal, with nobody
/// approving anything; gives what it showed once it exits.
fn signs_in_again(cloud: &Cloud, fresh: &Fresh) -> Shown {
    Terminal::start(&fresh.machine, &[SIGN_IN, SERVER, &cloud.url]).finish()
}

/// Fails unless `fresh`'s machine is linked to `ada`'s account by her first device and the server
/// gave it a device session of her account by its key since it started, and no device flow gave
/// it a session.
#[track_caller]
fn assert_linked(cloud: &Cloud, ada: &Person, fresh: &Fresh) {
    let device = certified_named(cloud, &ada.token, LAPTOP);
    assert_eq!(issuer_of(cloud, &ada.token, device), Some(ada.device));
    assert_key_session(cloud, &fresh.key_session(cloud), ada.account);
    assert!(!fresh.signed_in(cloud), "the device flow gave a session");
    until_no_run_open(cloud);
}

/// Ada's new machine shows its code, and the relay answers each of its requests to `/devices/` with
/// `503`. Her first machine's `skald link` takes the code and she confirms it; once the new
/// machine has sent such a request, after its grant, both runs are killed: the server holds no
/// certificate of the new device. With the server reachable again, the new machine's next `skald
/// sign-in`, which nobody approves, exits with success: the server holds its certificate, issued
/// by her first device, and it holds a device session of her account.
#[test]
fn killed_after_the_grant_the_next_run_stores_the_certificate_and_gets_the_session() {
    let (world, _front) = fronted_with(|_| {});
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_401_001, "ada-kills-the-laptop-run", WORKSTATION);
    let mut fresh = Fresh::start(cloud, LAPTOP, &[]);
    let code = fresh.code(cloud);
    let device = hello_device(cloud);
    let requests = format!("POST /devices/{device}/");
    cloud.relay.refuse(&requests, "503 Service Unavailable");

    let mut linking = start_link(&ada, &[]);
    linking.type_line(&typed(&code));
    linking.wait_for("the new device's name and kind", |shown| {
        says(shown, &[LAPTOP]) && says(shown, &["client"])
    });
    linking.type_line(YES);
    until("a request of the new device after its grant", || {
        cloud.count_sent(fresh.started, &requests) >= 1
    });
    fresh.kill();
    linking.kill();
    assert!(!certified(cloud, &ada, LAPTOP), "a certificate stored");

    cloud.relay.stop_refusing();
    let later = signs_in_again(cloud, &fresh);
    if !later.success {
        let seen = seen_since(cloud, fresh.started);
        let again = signs_in_again(cloud, &fresh);
        panic!(
            "the next run failed: {}\n{seen}\nthe run after it: success {}: {}",
            later.text, again.success, again.text
        );
    }
    assert_linked(cloud, &ada, &fresh);
}

/// Under a limit of 2 devices, Ada's account has 2 certified: her first machine and one linked by
/// hand. Her new machine shows its code, and her first machine's `skald link` takes it: the server
/// refuses the new device's certificate, so the new machine exits failing, saying on a line that
/// the account is at the devices limit, the server holds no certificate of it
/// and gave it no session. Her first machine revokes the one linked by hand; the new machine's next
/// `skald sign-in`, which nobody approves, exits with success: it holds its certificate, issued by
/// her first device, and a device session of her account.
#[test]
fn at_the_devices_limit_it_says_so_and_the_next_run_links_once_a_place_is_free() {
    let (world, _front) = fronted_with(|settings: &mut Settings| settings.devices_limit = Some(2));
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_402_001, "ada-links-at-the-limit", WORKSTATION);
    let desktop = second_machine(&world, &ada, DESKTOP);
    assert!(certified(cloud, &ada, DESKTOP), "the desktop's certificate");
    let mut fresh = Fresh::start(cloud, LAPTOP, &[]);
    let code = fresh.code(cloud);

    link_with(&ada, &typed(&code), LAPTOP, YES);
    let refused = fresh.finish();

    assert!(
        !refused.success,
        "the new machine succeeded: {}",
        refused.text
    );
    assert!(says_at_the_limit(&refused.text), "{}", refused.text);
    assert!(
        !certified(cloud, &ada, LAPTOP),
        "a certificate over the limit"
    );
    assert_eq!(
        key_sessions(cloud, fresh.started),
        Vec::<String>::new(),
        "a session over the limit"
    );

    revoked_by_cli(&ada.machine, desktop.device);
    let later = signs_in_again(cloud, &fresh);
    assert!(later.success, "the next run failed: {}", later.text);
    assert_linked(cloud, &ada, &fresh);
}
