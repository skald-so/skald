//! A machine's own keys and the first device's certificate: `skald sign-in` generates the device's
//! keys, registers the device and, on the account's first device, creates the account master key
//! and issues its own certificate with it (`docs/protocol/membership.md`, "Linking a device").
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind a
//! recording relay that can lose answers or hold requests, with a test playing the person on the
//! verification page in headless Chrome. Each machine keeps its state in a directory of its own and
//! its secrets in a file there, never in the system's key storage.
//!
//! The CLI, as these tests use it:
//! - `skald sign-in --server <url> --first-device yes|no`: the flag answers whether this is the
//!   account's first device; without it, `skald` asks on its input;
//! - `--name <name>`: the device's `name`, instead of the host name;
//! - `SKALD_HOST_NAME`: the host name the client reads, for tests.

#[allow(dead_code)]
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[allow(dead_code)]
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
mod keys;
#[allow(dead_code)]
#[path = "../sign_in/machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "../sign_in/relay.rs"]
mod relay;

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use browser::{APPROVE, Browser, Page};
use cloud::{ASK_MASTER, Cloud, Device, GIVE_MASTER, REGISTER, WAIT, certified, ids, shown_grant};
use machine::{FIRST_DEVICE, Finished, Machine, NAME, NO, SERVER, SIGN_IN, YES};
use relay::Lose;
use serde_json::Value;
use skald_github_stand_in::User;
use skald_proto::{CERT_NAME_MAX, Cert, DeviceId, DeviceKind};

/// `skald sign-in --server <cloud> <extra>`.
fn args<'a>(cloud: &'a Cloud, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![SIGN_IN, SERVER, &cloud.url];
    args.extend_from_slice(extra);
    args
}

/// A server whose codes live for 2 minutes and poll every second, and a browser for the person.
fn setup() -> (Cloud, Browser) {
    (Cloud::start(120, 1), Browser::launch())
}

/// The account's first device, signed in, as the server shows it.
struct First {
    /// What its sign-in printed.
    finished: Finished,
    /// Its session token.
    token: String,
    /// The account's master public key.
    master: [u8; 32],
    /// The account's devices.
    devices: Vec<Device>,
    /// Its ID.
    id: DeviceId,
    /// Its valid certificate.
    cert: Cert,
}

/// Signs `machine` in as `user`'s account's first device, with the arguments `extra` too; fails
/// unless the run succeeds and the device holds a valid certificate.
fn first_device(
    cloud: &Cloud,
    page: &Page,
    machine: &Machine,
    user: &User,
    extra: &[&str],
) -> First {
    let finished = cloud.signed_in(page, machine, user, &[&[FIRST_DEVICE, YES], extra].concat());
    assert!(finished.success, "{finished:?}");
    let (_, token) = secrets_of(cloud, machine);
    let master = cloud.master(&token).expect("the account's master key");
    let devices = cloud.devices(&token);
    let id = device_of(cloud, machine, &devices);
    let cert = checked(&master, &devices, id);
    First {
        finished,
        token,
        master,
        devices,
        id,
        cert,
    }
}

/// The signing and HPKE private keys of `cert` that the secrets file of `machine` holds, and the
/// file's bytes.
fn private_keys(cloud: &Cloud, machine: &Machine, cert: &Cert) -> ([u8; 32], [u8; 32], Vec<u8>) {
    let (file, _) = secrets_of(cloud, machine);
    let secrets = fs::read(file).expect("read the secrets file");
    let body = cert.body();
    let signing = keys::private_key(&secrets, &body.signing, keys::ed25519)
        .expect("the secrets file holds the signing private key");
    let hpke = keys::private_key(&secrets, &body.hpke, keys::x25519)
        .expect("the secrets file holds the HPKE private key");
    (signing, hpke, secrets)
}

/// Runs `skald sign-in` with `extra` on `machine`, signed in before.
fn again(cloud: &Cloud, machine: &Machine, extra: &[&str]) -> Finished {
    machine.start(&args(cloud, extra)).finish()
}

/// Whether the machine said it created the account's master key and holds its certificate.
fn says_created(finished: &Finished) -> bool {
    let said = finished.shown().to_lowercase();
    said.contains("created") && said.contains("master key") && said.contains("certificate")
}

/// Whether the machine said what `part`, in lowercase, says.
fn says(finished: &Finished, part: &str) -> bool {
    finished.shown().to_lowercase().contains(part)
}

/// The secrets file of `machine` and its session token: the one file that holds one of the tokens
/// the server gave.
fn secrets_of(cloud: &Cloud, machine: &Machine) -> (PathBuf, String) {
    let mut found = cloud.token_files(machine);
    assert_eq!(found.len(), 1, "files holding a session token: {found:?}");
    found.remove(0)
}

/// The valid certificate of `device` among the certificates of `devices`, as a server returns them,
/// against `master`: the checks of `skald_crypto::check_device`. Fails if it has none.
fn checked(master: &[u8; 32], devices: &[Device], device: DeviceId) -> Cert {
    let certs: Vec<Cert> = devices.iter().filter_map(|d| d.cert.clone()).collect();
    match skald_crypto::check_device(master, &certs, device) {
        Ok(cert) => cert.clone(),
        Err(err) => panic!("{device} has no valid certificate: {err}; devices: {devices:?}"),
    }
}

/// The device of `devices` whose signing key's private key the secrets of `machine` hold.
fn device_of(cloud: &Cloud, machine: &Machine, devices: &[Device]) -> DeviceId {
    let (file, _) = secrets_of(cloud, machine);
    let secrets = fs::read(&file).expect("read the secrets file");
    let found: Vec<DeviceId> = keys::public_keys(&secrets, keys::ed25519)
        .iter()
        .map(DeviceId::from_signing_key)
        .filter(|id| ids(devices).contains(id))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "devices whose private key the machine's secrets hold: {found:?}, of {devices:?}"
    );
    found[0]
}

/// Every row of every table of the machine's local state, as text, sorted.
fn state_rows(machine: &Machine) -> Vec<String> {
    use rusqlite::OpenFlags;
    use rusqlite::types::Value as Sql;

    let mut rows = Vec::new();
    for file in machine.sqlite_files() {
        let bytes = fs::read(&file).expect("read the local state");
        if !bytes.starts_with(b"SQLite format 3\0") {
            continue;
        }
        let db = rusqlite::Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .expect("open the local state");
        let tables: Vec<String> = db
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get(0))?
                    .collect::<Result<_, _>>()
            })
            .expect("the local state's tables");
        for table in tables {
            let sql = format!("SELECT * FROM \"{}\"", table.replace('"', "\"\""));
            let mut statement = db.prepare(&sql).expect("read a table of the local state");
            let columns = statement.column_count();
            let found: Vec<Vec<Sql>> = statement
                .query_map([], |row| (0..columns).map(|at| row.get(at)).collect())
                .and_then(Iterator::collect)
                .expect("read a table of the local state");
            rows.extend(found.iter().map(|row| format!("{table}: {row:?}")));
        }
    }
    rows.sort();
    rows
}

/// The account's first machine says it created the master key and holds a certificate that the
/// master key issued for its own keys, which passes the checks against it; the local state pins
/// that key and holds the certificate, the secrets file holds the private keys with the session
/// token, and a run again keeps the device ID and sends no master key.
#[test]
fn first_machine_creates_the_master_key_and_holds_a_certificate_chained_to_it() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_120_666_001, "ada-first-device-standin");
    let machine = Machine::new().with_host_name("ada-workstation");

    let first = first_device(&cloud, &browser.page(), &machine, &ada, &[]);

    assert!(says_created(&first.finished), "{:?}", first.finished);
    assert_eq!(ids(&first.devices), vec![first.id]);
    let body = first.cert.body();
    assert_eq!(body.account, cloud.account_of(&first.token));
    assert_eq!(body.kind, DeviceKind::Client);
    assert_eq!(body.issuer, None, "issued by the master key");
    assert_eq!(body.name, "ada-workstation", "the host name");
    let state = machine.state_bytes();
    assert!(keys::holds(&state, &first.master), "no pinned master key");
    for part in [
        first.cert.body_bytes(),
        first.cert.signature(),
        first.cert.proof(),
    ] {
        assert!(keys::holds(&state, part), "the certificate is not kept");
    }
    let (signing, hpke, _) = private_keys(&cloud, &machine, &first.cert);
    for secret in [signing, hpke] {
        assert!(!keys::holds(&state, &secret), "a private key in the state");
    }

    let mark = cloud.relay.mark();
    let run = again(&cloud, &machine, &[FIRST_DEVICE, YES]);
    assert!(run.success, "{run:?}");
    assert_eq!(
        cloud.count_sent(mark, GIVE_MASTER),
        0,
        "the master key sent again"
    );
    let devices = cloud.devices(&first.token);
    assert_eq!(ids(&devices), vec![first.id], "the device ID changed");
    checked(&first.master, &devices, first.id);
    assert_eq!(cloud.master(&first.token), Some(first.master));
}

/// The server's database holds the device's public keys and certificate and none of its private
/// keys or the master private key; no request the machine sent carries them; nothing on the
/// machine holds the master private key.
#[test]
fn server_and_requests_get_only_public_keys_and_no_master_private_key_is_kept() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_120_666_002, "ada-public-keys-standin");
    let machine = Machine::new().with_host_name("ada-public-keys");
    let first = first_device(&cloud, &browser.page(), &machine, &ada, &[]);
    let (signing, hpke, _) = private_keys(&cloud, &machine, &first.cert);
    let (cert, body) = (&first.cert, first.cert.body());

    let rows = cloud.rows().join("\n").into_bytes();
    let sent = cloud.relay.sent_bytes();
    let publics = [
        &body.signing[..],
        &body.hpke[..],
        cert.body_bytes(),
        cert.signature(),
        cert.proof(),
        &first.master[..],
    ];
    for (place, bytes) in [("the database", &rows), ("the requests", &sent)] {
        for public in publics {
            assert!(keys::holds(bytes, public), "{place} lack a public key");
        }
        for secret in [signing, hpke] {
            assert!(!keys::holds(bytes, &secret), "{place} hold a private key");
        }
        let found = keys::private_key(bytes, &first.master, keys::ed25519);
        assert_eq!(found, None, "{place} hold the master private key");
    }
    for file in machine.files() {
        let bytes = fs::read(&file).expect("read a machine's file");
        let found = keys::private_key(&bytes, &first.master, keys::ed25519);
        assert_eq!(found, None, "{file:?} holds the master private key");
    }
}

/// The relay loses the server's answer to `request` as `how` says: it closes the connection, or it
/// withholds the answer and the test kills the CLI. The next run leaves the device, registered once,
/// with its certificate, and the account with the master key it took, if it took one.
fn recovers_from_a_lost_answer(request: &str, how: Lose, user: (u64, &str)) {
    let (cloud, browser) = setup();
    let ada = cloud.user(user.0, user.1);
    let machine = Machine::new().with_host_name("ada-flaky-network");
    cloud.relay.lose_answer(request, how);

    let mut run = machine.start(&args(&cloud, &[FIRST_DEVICE, YES]));
    let grant = shown_grant(&cloud, &mut run, 0);
    cloud.decide(&browser.page(), &ada, &grant, APPROVE);
    match how {
        // Its result does not matter.
        Lose::Close => drop(run.finish()),
        Lose::Withhold => cloud.kill_once_lost(run, request),
    }
    assert_eq!(cloud.relay.lost(), 1, "the server never answered {request}");
    let token = cloud.last_token();
    let registered = cloud.devices(&token);
    assert_eq!(registered.len(), 1, "the server took no registration");
    let taken = cloud.master(&token);
    if request == GIVE_MASTER {
        assert!(taken.is_some(), "the server took no master key");
    }

    let next = again(&cloud, &machine, &[FIRST_DEVICE, YES]);

    assert!(next.success, "{next:?}");
    let since = cloud.relay.sent().first().expect("requests").at;
    assert!(
        cloud.count_sent(since, request) >= 2,
        "{request} was not sent again"
    );
    let master = cloud.master(&token).expect("the account's master key");
    if let Some(taken) = taken {
        assert_eq!(master, taken, "another master key");
    }
    let devices = cloud.devices(&token);
    assert_eq!(ids(&devices), ids(&registered), "another device");
    checked(&master, &devices, registered[0].id);
    assert_eq!(device_of(&cloud, &machine, &devices), registered[0].id);
}

#[test]
fn registration_answer_dropped_by_the_proxy_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(REGISTER, Lose::Close, (4_120_666_003, "ada-lost-register"));
}

#[test]
fn registration_answer_lost_with_the_cli_killed_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(
        REGISTER,
        Lose::Withhold,
        (4_120_666_004, "ada-killed-register"),
    );
}

#[test]
fn master_key_answer_dropped_by_the_proxy_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(GIVE_MASTER, Lose::Close, (4_120_666_005, "ada-lost-master"));
}

#[test]
fn master_key_answer_lost_with_the_cli_killed_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(
        GIVE_MASTER,
        Lose::Withhold,
        (4_120_666_006, "ada-killed-master"),
    );
}

/// After the account's first device, `other` signs in to the same account answering `answer`:
/// gives what its run printed, with the first device. Fails if `other` sent a master key or the
/// account's master key or certified devices changed.
fn other_machine(answer: &str, user: (u64, &str)) -> (Finished, First, Cloud, Machine) {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(user.0, user.1);
    let first = first_device(
        &cloud,
        &page,
        &Machine::new().with_host_name("ada-first"),
        &ada,
        &[],
    );
    let other = Machine::new().with_host_name("ada-other");

    let mark = cloud.relay.mark();
    let finished = cloud.signed_in(&page, &other, &ada, &[FIRST_DEVICE, answer]);

    assert!(!says_created(&finished), "{finished:?}");
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    assert_eq!(cloud.master(&first.token), Some(first.master));
    let devices = cloud.devices(&first.token);
    assert_eq!(certified(&devices), vec![first.id], "{devices:?}");
    (finished, first, cloud, other)
}

/// A second machine of the account, answering that it is not the first device, registers, stays
/// without a certificate and says it waits for linking, also when run again.
#[test]
fn second_machine_of_the_account_waits_for_linking_without_a_certificate() {
    let (finished, first, cloud, second) =
        other_machine(NO, (4_120_666_007, "ada-second-machine-standin"));

    assert!(says(&finished, "link"), "{finished:?}");
    let devices = cloud.devices(&first.token);
    assert_eq!(devices.len(), 2, "the second device is not registered");
    assert_ne!(device_of(&cloud, &second, &devices), first.id);

    let mark = cloud.relay.mark();
    let run = again(&cloud, &second, &[FIRST_DEVICE, NO]);
    assert!(says(&run, "link"), "{run:?}");
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    assert_eq!(certified(&cloud.devices(&first.token)), vec![first.id]);
}

/// A machine that says it is the first device of an account with a master key warns that someone
/// else may have created it, creates no master key and stays without a certificate.
#[test]
fn yes_on_an_account_with_a_master_key_warns_and_creates_none() {
    let (finished, ..) = other_machine(YES, (4_120_666_009, "ada-says-yes-standin"));

    assert!(says(&finished, "someone else"), "{finished:?}");
}

/// Two first machines of one account send their master keys at once: the relay holds the first
/// until the second comes. The account gets one master key and one certified device; the other
/// machine drops its own master key, sends it no more and says it waits for linking.
#[test]
fn two_first_machines_at_once_give_the_account_one_master_key() {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(4_120_666_008, "ada-race-standin");
    let one = Machine::new().with_host_name("ada-one");
    let two = Machine::new().with_host_name("ada-two");
    cloud.relay.hold(GIVE_MASTER, 2, Duration::from_secs(20));

    let mut run_one = one.start(&args(&cloud, &[FIRST_DEVICE, YES]));
    let grant_one = shown_grant(&cloud, &mut run_one, 0);
    let mut run_two = two.start(&args(&cloud, &[FIRST_DEVICE, YES]));
    let grant_two = shown_grant(&cloud, &mut run_two, 1);
    cloud.decide(&page, &ada, &grant_one, APPROVE);
    cloud.decide(&page, &ada, &grant_two, APPROVE);
    let finished = [run_one.finish(), run_two.finish()];

    assert_eq!(
        cloud.relay.held(),
        2,
        "both machines did not send a master key: {finished:?}"
    );
    let token = cloud.last_token();
    let master = cloud.master(&token).expect("the account's master key");
    let devices = cloud.devices(&token);
    assert_eq!(devices.len(), 2, "{devices:?}");
    let certified = certified(&devices);
    assert_eq!(certified.len(), 1, "{devices:?}");
    checked(&master, &devices, certified[0]);
    let (winner, loser) = if device_of(&cloud, &one, &devices) == certified[0] {
        (&one, &two)
    } else {
        (&two, &one)
    };
    assert_ne!(device_of(&cloud, loser, &devices), certified[0]);

    let run = again(&cloud, winner, &[FIRST_DEVICE, YES]);
    assert!(run.success, "{run:?}");
    let mark = cloud.relay.mark();
    let run = again(&cloud, loser, &[FIRST_DEVICE, YES]);
    assert!(says(&run, "link"), "{run:?}");
    assert_eq!(
        cloud.count_sent(mark, GIVE_MASTER),
        0,
        "the loser kept its key"
    );
    assert_eq!(cloud.master(&token), Some(master));
    assert_eq!(self::certified(&cloud.devices(&token)), certified);
}

/// A machine that says it is not the first device of an account without a master key warns that
/// the server reports none, creates no master key and stays without a certificate.
#[test]
fn no_on_an_account_without_a_master_key_warns_and_creates_none() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_120_666_010, "ada-says-no-standin");
    let machine = Machine::new().with_host_name("ada-says-no");

    let finished = cloud.signed_in(&browser.page(), &machine, &ada, &[FIRST_DEVICE, NO]);

    assert!(says(&finished, "no master key"), "{finished:?}");
    assert!(!says_created(&finished), "{finished:?}");
    let token = cloud.last_token();
    assert_eq!(cloud.master(&token), None, "a master key created");
    assert_eq!(certified(&cloud.devices(&token)), Vec::<DeviceId>::new());
    let since = cloud.relay.sent().first().expect("requests").at;
    assert_eq!(cloud.count_sent(since, GIVE_MASTER), 0, "a master key sent");
}

/// Without the flag, the machine asks its user whether this is the account's first device before
/// it asks the server whether the account has a master key; a "yes" typed on its input creates the
/// master key.
#[test]
fn asks_whether_this_is_the_first_device_before_asking_the_server() {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(4_120_666_011, "ada-asked-standin");
    let machine = Machine::new().with_host_name("ada-asked");

    let mut run = machine.start_asking(&args(&cloud, &[]));
    let deadline = Instant::now() + WAIT;
    let mut approved = false;
    loop {
        let shown = run.shown();
        if !approved
            && let Some(grant) = cloud.grants().last()
            && shown.contains(&grant.user_code)
        {
            cloud.decide(&page, &ada, grant, APPROVE);
            approved = true;
            continue;
        }
        if shown.to_lowercase().contains("first") {
            break;
        }
        run.assert_running();
        assert!(Instant::now() < deadline, "no question: {shown}");
        thread::sleep(Duration::from_millis(50));
    }
    let since = cloud.relay.sent().first().expect("requests").at;
    assert_eq!(
        cloud.count_sent(since, ASK_MASTER),
        0,
        "the server was asked first: {}",
        run.shown()
    );
    run.answer("yes\n");
    if !approved {
        let grant = shown_grant(&cloud, &mut run, 0);
        cloud.decide(&page, &ada, &grant, APPROVE);
    }
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    assert!(says_created(&finished), "{finished:?}");
    let token = cloud.last_token();
    let master = cloud.master(&token).expect("the account's master key");
    let devices = cloud.devices(&token);
    assert_eq!(devices.len(), 1, "{devices:?}");
    checked(&master, &devices, devices[0].id);
}

/// An empty host name and one longer than a `name` may be, cut in the middle of a character, each
/// give the device a valid `name` of 1 to 64 bytes.
#[test]
fn empty_and_overlong_host_names_give_valid_names() {
    let (cloud, browser) = setup();
    let overlong = format!("a{}", "ü".repeat(50));
    let cases = [
        (4_120_666_012, "ada-empty-host-standin", String::new()),
        (4_120_666_013, "ada-long-host-standin", overlong),
    ];
    for (id, login, host) in cases {
        let user = cloud.user(id, login);
        let machine = Machine::new().with_host_name(&host);
        // A page of its own: one signed in to the server as the user before approves as that user.
        let first = first_device(&cloud, &browser.page(), &machine, &user, &[]);
        let name = &first.cert.body().name;
        assert!(
            (1..=CERT_NAME_MAX).contains(&name.len()),
            "host name {host:?}: name {name:?}"
        );
    }
}

/// A `name` the user gives names the device instead of the host name.
#[test]
fn name_the_user_gives_is_the_certificates_name() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_120_666_014, "ada-names-standin");
    let machine = Machine::new().with_host_name("ada-host");

    let first = first_device(
        &cloud,
        &browser.page(),
        &machine,
        &ada,
        &[NAME, "Ada's laptop"],
    );

    assert_eq!(first.cert.body().name, "Ada's laptop");
}

/// The private keys of a device the local state knows, missing from the secrets file or unreadable
/// there, are an error: the run replaces no key, registers no device and leaves the secrets and the
/// local state as they were. With the keys back, the device signs in as before.
#[test]
fn missing_or_unreadable_keys_of_a_known_device_are_an_error_and_change_nothing() {
    let (cloud, browser) = setup();
    let ada = cloud.user(4_120_666_015, "ada-lost-keys-standin");
    let machine = Machine::new().with_host_name("ada-lost-keys");
    let first = first_device(&cloud, &browser.page(), &machine, &ada, &[]);
    let (signing, hpke) = (first.cert.body().signing, first.cert.body().hpke);
    let (file, token) = secrets_of(&cloud, &machine);
    let original = fs::read(&file).expect("read the secrets file");

    // The secrets that hold the device's private keys go, or become unreadable.
    let secrets: serde_json::Map<String, Value> =
        serde_json::from_slice(&original).expect("the secrets file holds a JSON object");
    let holds_key = |value: &Value| {
        let bytes = value.to_string().into_bytes();
        keys::private_key(&bytes, &signing, keys::ed25519).is_some()
            || keys::private_key(&bytes, &hpke, keys::x25519).is_some()
    };
    let missing: serde_json::Map<String, Value> = secrets
        .iter()
        .filter(|(_, value)| !holds_key(value))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    assert!(missing.len() < secrets.len(), "no secret holds a key");
    let unreadable: serde_json::Map<String, Value> = secrets
        .iter()
        .map(|(name, value)| {
            let value = if holds_key(value) {
                Value::String("not a key".to_owned())
            } else {
                value.clone()
            };
            (name.clone(), value)
        })
        .collect();

    for (case, broken) in [("missing", missing), ("unreadable", unreadable)] {
        let broken = serde_json::to_vec(&Value::Object(broken)).expect("encode the secrets");
        assert!(
            keys::holds(&broken, token.as_bytes()),
            "the keys are not apart from the session token"
        );
        fs::write(&file, &broken).expect("write the secrets file");
        let state = state_rows(&machine);
        let mark = cloud.relay.mark();

        let run = again(&cloud, &machine, &[FIRST_DEVICE, YES]);

        assert!(!run.success, "{case} keys: {run:?}");
        assert!(says(&run, "key"), "{case} keys: {run:?}");
        let now = fs::read(&file).expect("read the secrets file");
        assert_eq!(now, broken, "{case} keys: the secrets changed");
        assert_eq!(
            state_rows(&machine),
            state,
            "{case} keys: the state changed"
        );
        assert_eq!(ids(&cloud.devices(&token)), vec![first.id], "{case} keys");
        assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "{case} keys");
        assert_eq!(cloud.master(&token), Some(first.master), "{case} keys");
    }

    fs::write(&file, &original).expect("write the secrets file");
    let run = again(&cloud, &machine, &[FIRST_DEVICE, YES]);
    assert!(run.success, "{run:?}");
    let devices = cloud.devices(&token);
    assert_eq!(ids(&devices), vec![first.id]);
    checked(&first.master, &devices, first.id);
}
