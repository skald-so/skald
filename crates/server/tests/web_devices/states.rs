//! Each device's name, kind, registration and state, by the server's checks, and nothing of its
//! keys or certificate.

use std::collections::BTreeMap;

use skald_proto::DeviceKind;

use crate::common::browser::Answer;
use crate::common::device_flow::{json, text};
use crate::common::devices::{DEVICES, b64, devices_of};
use crate::common::spa::assert_no_secrets;
use crate::sessions::{Api, now, token_of};
use crate::shown::{ACCOUNT_DEVICES, State, flow, list_response, person, shown, states};

/// Ada's account has: `Laptop`, certified by her master key, and `Desktop`, registered without a
/// certificate; `Tablet`, `Agent`, a `service` device, and `Chrome`, a `browser` device, each
/// certified by `Laptop`; `Phone`, certified by `Laptop` and revoked by it, and `Watch`, certified
/// by `Phone`; and `Old laptop`, certified by her master key and revoked by it. Her web session
/// gets each of them once with its name, kind and registration: linked, waiting for linking, or
/// revoked, `Watch` through `Phone`. The answer holds no key or certificate of any of them, nor a
/// session token.
#[test]
fn each_device_is_given_with_its_name_kind_registration_and_state_and_no_key_or_certificate() {
    let flow = flow();
    let api = Api::of(&flow);
    let db = &flow.stand.db;
    let before = now();
    let (ada, cookie) = person(&flow, 3_224_100_001, "ada");
    let laptop = &ada.laptop;
    let tablet = ada.certified_by(db, laptop, DeviceKind::Client, "Tablet");
    let agent = ada.certified_by(db, laptop, DeviceKind::Service, "Agent");
    let chrome = ada.certified_by(db, laptop, DeviceKind::Browser, "Chrome");
    let phone = ada.certified_by(db, laptop, DeviceKind::Client, "Phone");
    let watch = ada.certified_by(db, &phone, DeviceKind::Client, "Watch");
    let old = ada.certified_by_key(db, &ada.master, "Old laptop");
    ada.revoke(&ada.by_device(laptop, phone.id));
    ada.revoke(&ada.by_master(old.id, &[]));
    let after = now();

    let response = list_response(&api, &[("cookie", &cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let shown = shown(&api, &cookie);
    let expected = [
        (laptop, "client", State::Linked),
        (&ada.uncertified, "client", State::Waiting),
        (&tablet, "client", State::Linked),
        (&agent, "service", State::Linked),
        (&chrome, "browser", State::Linked),
        (&phone, "client", State::Revoked),
        (&watch, "client", State::Revoked),
        (&old, "client", State::Revoked),
    ];
    assert_eq!(shown.len(), expected.len(), "{shown:#?}");
    for (device, kind, state) in expected {
        let listed = shown
            .get(&device.name)
            .unwrap_or_else(|| panic!("{} not listed: {shown:#?}", device.name));
        assert_eq!(listed.id, device.id, "{listed:?}");
        assert_eq!(
            (listed.kind.as_str(), listed.state),
            (kind, state),
            "{listed:?}"
        );
        assert!(
            (before - 5..=after + 5).contains(&listed.registered_at),
            "registered at {}..{}: {listed:?}",
            before,
            after
        );
    }

    let body = String::from_utf8_lossy(&response.body).into_owned();
    let answer = Answer {
        method: "GET".to_owned(),
        url: ACCOUNT_DEVICES.to_owned(),
        status: response.status,
        headers: Default::default(),
        body: body.clone(),
    };
    assert_no_secrets(&[answer], &[token_of(&cookie).to_owned()]);
    let certs = devices_of(&ada.session.call("get", DEVICES, None));
    assert_eq!(certs.len(), expected.len(), "{certs:?}");
    let mut secrets: Vec<String> = certs.values().flatten().map(|cert| b64(cert)).collect();
    for (device, _, _) in expected {
        secrets.push(b64(&device.signing.public()));
        secrets.push(b64(&device.hpke));
    }
    secrets.push(b64(&ada.master.public()));
    for secret in &secrets {
        assert!(
            !body.contains(secret.as_str()),
            "a key or certificate: {body}"
        );
    }
    assert!(json(&response)["devices"].is_array(), "{body}");
}

/// Ada's laptop revokes her phone, which it certified: the phone shows as revoked, the laptop as
/// linked. Once her master key revokes the laptop, keeping the phone, it voids the laptop's
/// revocation: the phone shows as linked again, and the laptop as revoked.
#[test]
fn a_revocation_that_a_master_key_revocation_voids_leaves_its_device_linked() {
    let flow = flow();
    let api = Api::of(&flow);
    let (ada, cookie) = person(&flow, 3_224_100_011, "ada");
    let phone = ada.certified_by(&flow.stand.db, &ada.laptop, DeviceKind::Client, "Phone");
    let shows = |laptop: State, phone: State| {
        BTreeMap::from([
            ("Desktop".to_owned(), State::Waiting),
            ("Laptop".to_owned(), laptop),
            ("Phone".to_owned(), phone),
        ])
    };
    assert_eq!(
        states(&api, &cookie),
        shows(State::Linked, State::Linked),
        "before any revocation"
    );

    ada.revoke(&ada.by_device(&ada.laptop, phone.id));
    assert_eq!(
        states(&api, &cookie),
        shows(State::Linked, State::Revoked),
        "the phone revoked by the laptop"
    );

    ada.revoke(&ada.by_master(ada.laptop.id, &[phone.id]));
    assert_eq!(
        states(&api, &cookie),
        shows(State::Revoked, State::Linked),
        "the laptop revoked by the master key, keeping the phone"
    );
}
