//! Device registration and the account's master key on the server: a machine signed in through the
//! device flow registers its device by its public keys, the account's first device gives the master
//! public key with its certificate, and the account's sessions get the master key and the devices.
//! Every response is checked against the OpenAPI spec in `api/openapi.json`.

mod common;

use std::collections::BTreeMap;
use std::sync::Barrier;
use std::thread;

use common::device_flow::{DeviceFlow, json, text};
use common::devices::{
    DEVICES, Device, Key, MASTER, Machine, account_id, assert_refused, assert_taken, b64, cert_of,
    devices_of, machine, master_of, request,
};
use common::get_url;
use serde_json::{Value, json};
use skald_proto::{CERT_LABEL, Cbor, Cert, CertBody, DeviceKind, PROOF_LABEL};

/// Runs `send` from `count` threads at once; gives the results in the threads' order.
fn at_once<T: Send>(count: usize, send: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    let barrier = Barrier::new(count);
    thread::scope(|scope| {
        let sent: Vec<_> = (0..count)
            .map(|at| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    send(at)
                })
            })
            .collect();
        sent.into_iter()
            .map(|answer| answer.join().expect("a request"))
            .collect()
    })
}

/// Opens the server's database connections with requests at once, so that requests sent at once
/// later run side by side.
fn warm_up(flow: &DeviceFlow, count: usize) {
    let health = format!("{}/health", flow.stand.server.base());
    at_once(count, &|_| get_url(&health, &[]).status);
}

/// `registration` with `field` set to `value`, or removed if `value` is `None`.
fn with(registration: &Value, field: &str, value: Option<Value>) -> Value {
    let mut changed = registration.clone();
    let fields = changed.as_object_mut().expect("a JSON object");
    match value {
        Some(value) => fields.insert(field.to_owned(), value),
        None => fields.remove(field),
    };
    changed
}

/// A device list as `GET /devices` gives it: each of `ids` without a certificate, and each of
/// `certs` with its certificate.
fn listed(ids: &[&Device], certs: &[(&Device, &[u8])]) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut devices: BTreeMap<_, _> = ids
        .iter()
        .map(|device| (device.id.to_string(), None))
        .collect();
    for (device, cert) in certs {
        devices.insert(device.id.to_string(), Some(cert.to_vec()));
    }
    devices
}

#[test]
fn device_registers_and_the_same_registration_again_is_taken() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_001, "ada-register-standin");
    let machine = machine(&flow, &ada);
    // A name of 64 bytes in 32 characters: the limit counts bytes.
    let laptop = Device::new(&"é".repeat(32));
    let phone = Device::new("p");

    for device in [&laptop, &phone] {
        let registration = device.registration();
        assert_taken(&machine.register(&registration));
        // A retry after a lost answer.
        assert_taken(&machine.register(&registration));
    }
    assert_eq!(machine.devices(), listed(&[&laptop, &phone], &[]));
    assert_eq!(machine.master(), None);
}

#[test]
fn an_id_the_server_has_is_refused_with_other_keys_or_name_or_on_another_account() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_002, "ada-taken-standin");
    let bob = flow.stand.user(3_120_665_003, "bob-taken-standin");
    let first = machine(&flow, &ada);
    let other = machine(&flow, &bob);
    let device = Device::new("Laptop");
    let registration = device.registration();
    first.register_device(&device);

    let other_hpke = with(&registration, "hpke", Some(json!(b64(&[7; 32]))));
    let other_name = with(&registration, "name", Some(json!("Phone")));
    for changed in [&other_hpke, &other_name] {
        assert_refused(&first.register(changed), 409);
    }
    for sent in [&registration, &other_hpke, &other_name] {
        assert_refused(&other.register(sent), 409);
    }
    assert_eq!(other.devices(), BTreeMap::new());

    // The registration stays as it was: a certificate that matches it is taken.
    assert_eq!(first.devices(), listed(&[&device], &[]));
    let master = Key::generate();
    let cert = device.cert(&first.account, &master);
    assert_taken(&first.give_master(&master, &cert));
    assert_eq!(first.devices(), listed(&[], &[(&device, &cert)]));
}

#[test]
fn invalid_registrations_are_refused() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_004, "ada-invalid-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    let registration = device.registration();
    let another = Device::new("Other");

    let invalid = [
        // An ID not derived from the signing key.
        with(&registration, "device", Some(json!(another.id.to_string()))),
        // The ID in a text form other than the lowercase hyphenated one.
        with(
            &registration,
            "device",
            Some(json!(device.id.to_string().to_uppercase())),
        ),
        with(&registration, "device", None),
        // Other kinds come with later stories.
        with(&registration, "kind", Some(json!("browser"))),
        with(&registration, "kind", Some(json!("service"))),
        with(&registration, "kind", Some(json!("toaster"))),
        with(&registration, "kind", None),
        // `name` is 1 to 64 bytes.
        with(&registration, "name", Some(json!(""))),
        with(&registration, "name", Some(json!("n".repeat(65)))),
        with(&registration, "name", Some(json!("é".repeat(33)))),
        with(&registration, "name", None),
        // Keys of 32 bytes.
        with(&registration, "signing", Some(json!(b64(&[1; 31])))),
        with(&registration, "signing", Some(json!("not base64url!"))),
        with(&registration, "signing", None),
        with(&registration, "hpke", Some(json!(b64(&[1; 33])))),
        with(&registration, "hpke", None),
    ];
    for sent in &invalid {
        assert_refused(&machine.register(sent), 400);
    }
    assert_refused(&machine.register_raw(b"{\"device\":"), 400);
    assert_eq!(machine.devices(), BTreeMap::new());

    // None of them held the ID back.
    machine.register_device(&device);
    assert_eq!(machine.devices(), listed(&[&device], &[]));
}

#[test]
fn requests_without_a_valid_session_are_refused() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_005, "ada-nosession-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    let master = Key::generate();
    let registration = serde_json::to_vec(&device.registration()).expect("JSON");
    let given = serde_json::to_vec(&json!({
        "master": b64(&master.public()),
        "cert": b64(&device.cert(&machine.account, &master)),
    }))
    .expect("JSON");
    let unknown = format!("Bearer {}x", machine.token);
    let basic = format!("Basic {}", b64(b"ada:secret"));

    for headers in [
        vec![],
        vec![("authorization", unknown.as_str())],
        vec![("authorization", basic.as_str())],
        vec![("authorization", "Bearer")],
        vec![("cookie", "skald_session=unknown")],
    ] {
        for (method, path, body) in [
            ("post", DEVICES, Some(registration.as_slice())),
            ("put", MASTER, Some(given.as_slice())),
            ("get", DEVICES, None),
            ("get", MASTER, None),
        ] {
            let response = request(&flow, method, path, &headers, body, true);
            assert_eq!(
                response.status,
                401,
                "{method} {path} {headers:?}: {}",
                text(&response)
            );
        }
    }
    assert_eq!(machine.devices(), BTreeMap::new());
    assert_eq!(machine.master(), None);
}

#[test]
fn requests_state_the_protocol_version() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_006, "ada-version-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    let registration = serde_json::to_vec(&device.registration()).expect("JSON");
    let bearer = format!("Bearer {}", machine.token);
    let headers = [("authorization", bearer.as_str())];

    for (method, path, body) in [
        ("post", DEVICES, Some(registration.as_slice())),
        ("put", MASTER, Some(b"{}".as_slice())),
        ("get", DEVICES, None),
        ("get", MASTER, None),
    ] {
        let response = request(&flow, method, path, &headers, body, false);
        assert_eq!(response.status, 400, "{method} {path}: {}", text(&response));
        assert_eq!(
            json(&response)["type"],
            "https://skald.so/problems/protocol-version-invalid",
            "{method} {path}"
        );
    }
    assert_eq!(machine.devices(), BTreeMap::new());
}

#[test]
fn one_id_registered_by_two_accounts_at_once_goes_to_one() {
    const ROUNDS: usize = 5;
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_007, "ada-race-id-standin");
    let bob = flow.stand.user(3_120_665_008, "bob-race-id-standin");
    let machines = [machine(&flow, &ada), machine(&flow, &bob)];
    warm_up(&flow, 8);

    for round in 0..ROUNDS {
        let device = Device::new("Laptop");
        let registration = device.registration();
        let statuses = at_once(2, &|at| machines[at].register(&registration).status);
        let taken: Vec<_> = statuses
            .iter()
            .map(|status| matches!(status, 200 | 201))
            .collect();
        assert_eq!(
            taken.iter().filter(|taken| **taken).count(),
            1,
            "round {round}: {statuses:?}"
        );
        for ((machine, taken), status) in machines.iter().zip(&taken).zip(&statuses) {
            let held = machine.devices().contains_key(&device.id.to_string());
            assert_eq!(held, *taken, "round {round}: {statuses:?}");
            if !taken {
                assert_eq!(*status, 409, "round {round}: {statuses:?}");
            }
        }
    }
}

#[test]
fn first_device_gives_the_master_key_and_its_certificate() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_009, "ada-master-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    machine.register_device(&device);
    assert_eq!(machine.master(), None);

    let master = Key::generate();
    let cert = device.cert(&machine.account, &master);
    let holds = || {
        assert_eq!(machine.master(), Some(master.public().to_vec()));
        assert_eq!(machine.devices(), listed(&[], &[(&device, &cert)]));
    };
    assert_taken(&machine.give_master(&master, &cert));
    holds();

    // A retry after a lost answer, and the registration's retry after it.
    assert_taken(&machine.give_master(&master, &cert));
    machine.register_device(&device);
    holds();
}

#[test]
fn device_list_gives_each_certificate_as_sent() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_010, "ada-bytes-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    let other = Device::new("Phone");
    machine.register_device(&device);
    machine.register_device(&other);

    // A body with a field of a later version, 10 => "later": the map's head counts one more pair,
    // and the field goes last, as deterministic encoding orders the keys.
    let mut body = device.body(&machine.account).to_cbor();
    assert_eq!(body[0], 0xa7, "a map of 7 pairs");
    body[0] = 0xa8;
    body.extend_from_slice(&[0x0a, 0x65]);
    body.extend_from_slice(b"later");
    let master = Key::generate();
    let signature = master.key.sign(CERT_LABEL, &[&body]);
    let proof = device.signing.key.sign(PROOF_LABEL, &[&body]);
    let cert = Cert::from_parts(body, signature, proof)
        .expect("a certificate")
        .to_cbor();

    assert_taken(&machine.give_master(&master, &cert));
    assert_eq!(
        machine.devices(),
        listed(&[&other], &[(&device, cert.as_slice())])
    );
}

#[test]
fn invalid_certificates_are_refused() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_011, "ada-badcert-standin");
    let bob = flow.stand.user(3_120_665_012, "bob-badcert-standin");
    let other = machine(&flow, &bob);
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    let issuer = Device::new("Desktop");
    let elsewhere = Device::new("Bob's laptop");
    let unregistered = Device::new("Unregistered");
    machine.register_device(&device);
    machine.register_device(&issuer);
    other.register_device(&elsewhere);
    let account = &machine.account;
    let master = Key::generate();
    let stranger = Key::generate();
    let body = device.body(account);
    let bytes = body.to_cbor();
    let parts = |signature: [u8; 64], proof: [u8; 64]| {
        Cert::from_parts(bytes.clone(), signature, proof)
            .expect("a certificate")
            .to_cbor()
    };
    let by_master = master.key.sign(CERT_LABEL, &[&bytes]);
    let proved = device.signing.key.sign(PROOF_LABEL, &[&bytes]);
    let signed = |change: &dyn Fn(&mut CertBody)| {
        let mut changed = body.clone();
        change(&mut changed);
        cert_of(&changed, &master.key, &device.signing.key)
    };
    let genuine = device.cert(account, &master);

    let refused: Vec<(&str, Vec<u8>)> = vec![
        // The proof check.
        (
            "proof by another key",
            parts(by_master, stranger.key.sign(PROOF_LABEL, &[&bytes])),
        ),
        (
            "proof over another label",
            parts(by_master, device.signing.key.sign(CERT_LABEL, &[&bytes])),
        ),
        // The master key's signature.
        (
            "signed by another key",
            cert_of(&body, &stranger.key, &device.signing.key),
        ),
        (
            "signed by the device itself",
            cert_of(&body, &device.signing.key, &device.signing.key),
        ),
        (
            "signed over another label",
            parts(master.key.sign(PROOF_LABEL, &[&bytes]), proved),
        ),
        (
            "issued by a device",
            cert_of(
                &CertBody {
                    issuer: Some(issuer.id),
                    ..body.clone()
                },
                &issuer.signing.key,
                &device.signing.key,
            ),
        ),
        (
            "issuer named, signed by the master key",
            signed(&|body| body.issuer = Some(issuer.id)),
        ),
        // The registration.
        (
            "another account",
            signed(&|body| body.account = account_id(&other.account)),
        ),
        ("another hpke key", signed(&|body| body.hpke = [9; 32])),
        (
            "another name",
            signed(&|body| body.name = "Phone".to_owned()),
        ),
        (
            "another kind",
            signed(&|body| body.kind = DeviceKind::Browser),
        ),
        (
            "an unregistered device",
            unregistered.cert(account, &master),
        ),
        (
            "a device of another account",
            cert_of(
                &CertBody {
                    account: account_id(account),
                    ..elsewhere.body(&other.account)
                },
                &master.key,
                &elsewhere.signing.key,
            ),
        ),
        (
            "another account's device and account",
            elsewhere.cert(&other.account, &master),
        ),
        // Malformed.
        ("truncated", genuine[..genuine.len() - 1].to_vec()),
        ("empty", Vec::new()),
        ("the body alone", bytes.clone()),
    ];
    for (what, cert) in &refused {
        let response = machine.give_master(&master, cert);
        assert_eq!(response.status, 400, "{what}: {}", text(&response));
        assert_refused(&response, 400);
    }
    let cert = b64(&genuine);
    for (what, sent) in [
        (
            "a master key of 31 bytes",
            json!({ "master": b64(&master.public()[..31]), "cert": cert }),
        ),
        ("no master key", json!({ "cert": cert })),
        ("no certificate", json!({ "master": b64(&master.public()) })),
        (
            "a certificate not in base64url",
            json!({ "master": b64(&master.public()), "cert": "not base64url!" }),
        ),
    ] {
        let response = machine.give(&sent);
        assert_eq!(response.status, 400, "{what}: {}", text(&response));
        assert_refused(&response, 400);
    }
    // The account's genuine certificate, from another account's session.
    assert_refused(&other.give_master(&master, &genuine), 400);
    assert_eq!(machine.master(), None);
    assert_eq!(machine.devices(), listed(&[&device, &issuer], &[]));
    assert_eq!(other.master(), None);
    assert_eq!(other.devices(), listed(&[&elsewhere], &[]));

    // None of them held the account back.
    assert_taken(&machine.give_master(&master, &genuine));
    assert_eq!(machine.master(), Some(master.public().to_vec()));
}

#[test]
fn a_second_master_key_is_refused_and_a_retry_of_the_first_is_taken() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_013, "ada-second-standin");
    let first = machine(&flow, &ada);
    let second = machine(&flow, &ada);
    assert_eq!(first.account, second.account);
    let laptop = Device::new("Laptop");
    let phone = Device::new("Phone");
    first.register_device(&laptop);
    second.register_device(&phone);
    let master = Key::generate();
    let other_master = Key::generate();
    let cert = laptop.cert(&first.account, &master);
    assert_taken(&first.give_master(&master, &cert));

    // Another master key, from the other device or from the first one, and another certificate by
    // the account's master key: the account has its master key.
    let account = &first.account;
    for (what, key, sent) in [
        (
            "the phone's own master key",
            &other_master,
            phone.cert(account, &other_master),
        ),
        (
            "the phone's certificate by the master key",
            &master,
            phone.cert(account, &master),
        ),
        (
            "another master key for the laptop",
            &other_master,
            laptop.cert(account, &other_master),
        ),
    ] {
        let response = second.give_master(key, &sent);
        assert_eq!(response.status, 409, "{what}: {}", text(&response));
        assert_refused(&response, 409);
    }

    // A retry of the first, from either session of the account.
    assert_taken(&second.give_master(&master, &cert));
    assert_taken(&first.give_master(&master, &cert));
    let devices = listed(&[&phone], &[(&laptop, &cert)]);
    for machine in [&first, &second] {
        assert_eq!(machine.master(), Some(master.public().to_vec()));
        assert_eq!(machine.devices(), devices);
    }
}

#[test]
fn of_master_keys_sent_at_once_one_is_taken() {
    const ACCOUNTS: u64 = 3;
    const AT_ONCE: usize = 6;
    let flow = DeviceFlow::start();
    warm_up(&flow, AT_ONCE * 2);

    for round in 0..ACCOUNTS {
        let user = flow
            .stand
            .user(3_120_665_100 + round, &format!("race-{round}-standin"));
        let machine: Machine = machine(&flow, &user);
        let devices: Vec<Device> = (0..AT_ONCE)
            .map(|at| Device::new(&format!("Machine {at}")))
            .collect();
        for device in &devices {
            machine.register_device(device);
        }
        let masters: Vec<Key> = (0..AT_ONCE).map(|_| Key::generate()).collect();
        let certs: Vec<Vec<u8>> = devices
            .iter()
            .zip(&masters)
            .map(|(device, master)| device.cert(&machine.account, master))
            .collect();

        let statuses = at_once(AT_ONCE, &|at| {
            machine.give_master(&masters[at], &certs[at]).status
        });
        let winners: Vec<usize> = (0..AT_ONCE)
            .filter(|at| matches!(statuses[*at], 200 | 201))
            .collect();
        assert_eq!(winners.len(), 1, "round {round}: {statuses:?}");
        let winner = winners[0];
        for (at, status) in statuses.iter().enumerate() {
            if at != winner {
                assert_eq!(*status, 409, "round {round}: {statuses:?}");
            }
        }
        assert_eq!(machine.master(), Some(masters[winner].public().to_vec()));
        let others: Vec<&Device> = (0..AT_ONCE)
            .filter(|at| *at != winner)
            .map(|at| &devices[at])
            .collect();
        let expected = listed(&others, &[(&devices[winner], &certs[winner])]);
        assert_eq!(machine.devices(), expected, "round {round}");

        // The winner's retry is taken; a loser's is not.
        assert_taken(&machine.give_master(&masters[winner], &certs[winner]));
        let loser = (winner + 1) % AT_ONCE;
        assert_refused(&machine.give_master(&masters[loser], &certs[loser]), 409);
        assert_eq!(machine.devices(), expected, "round {round}");
    }
}

#[test]
fn master_key_and_devices_go_only_to_the_account_sessions() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_014, "ada-sessions-standin");
    let bob = flow.stand.user(3_120_665_015, "bob-sessions-standin");
    let first = machine(&flow, &ada);
    let second = machine(&flow, &ada);
    let other = machine(&flow, &bob);
    let device = Device::new("Laptop");
    first.register_device(&device);
    let master = Key::generate();
    let cert = device.cert(&first.account, &master);
    assert_taken(&first.give_master(&master, &cert));
    let devices = listed(&[], &[(&device, &cert)]);
    let master_key = Some(master.public().to_vec());

    assert_eq!(second.master(), master_key);
    assert_eq!(second.devices(), devices);
    // The web session of the account's person.
    let web = [("cookie", first.cookie.as_str())];
    assert_eq!(
        master_of(&request(&flow, "get", MASTER, &web, None, true)),
        master_key
    );
    assert_eq!(
        devices_of(&request(&flow, "get", DEVICES, &web, None, true)),
        devices
    );

    // Another account's sessions get their own account's, and none of these.
    assert_eq!(other.master(), None);
    assert_eq!(other.devices(), BTreeMap::new());
    let web = [("cookie", other.cookie.as_str())];
    assert_eq!(
        master_of(&request(&flow, "get", MASTER, &web, None, true)),
        None
    );
    assert_eq!(
        devices_of(&request(&flow, "get", DEVICES, &web, None, true)),
        BTreeMap::new()
    );
}

#[test]
fn the_database_holds_only_public_keys_and_certificates() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_665_016, "ada-public-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    machine.register_device(&device);
    let master = Key::generate();
    let cert = device.cert(&machine.account, &master);
    assert_taken(&machine.give_master(&master, &cert));

    let rows = flow.stand.db.rows();
    let holds = |bytes: &[u8]| rows.iter().any(|row| held(row, bytes));
    for (what, public) in [
        (
            "the device's signing key",
            device.signing.public().as_slice(),
        ),
        ("the device's HPKE key", device.hpke.as_slice()),
        ("the master key", master.public().as_slice()),
        ("the certificate", cert.as_slice()),
    ] {
        assert!(holds(public), "{what} is not in the database: {rows:#?}");
    }
    for (what, secret) in [
        ("the device's private key", device.signing.secret),
        ("the master private key", master.secret),
    ] {
        assert!(!holds(&secret), "{what} is in the database: {rows:#?}");
    }
}

/// Whether `row`, a row in PostgreSQL's text form, holds `bytes`: as the hex of a `bytea`, or in
/// base64 or base64url, with or without padding.
fn held(row: &str, bytes: &[u8]) -> bool {
    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let mut forms = vec![hex];
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        forms.push(engine.encode(bytes));
    }
    forms.iter().any(|form| row.contains(form.as_str()))
}
