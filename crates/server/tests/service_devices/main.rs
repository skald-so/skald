//! `service` devices on the server: a session of an account registers a `service` device of the
//! account, as it registers a `client` one (`docs/protocol/membership.md`, "Linking a device"), and
//! the server takes the device's certificate, issued by a `client` device of the same account, only
//! after checks 1 and 2 of "Device checks" against the account's master key and the certificates
//! its devices hold, and only if it matches the registration. Registering a `service` device leaves
//! the session's device as it was, so the account's sessions still name the device the session
//! registered before. `service` devices stay outside `--devices-limit`, at registration and at the
//! certificate; the limit applies where the server takes a certificate, not at registration.
//! Against a `skald-server` with the device flow, through the GitHub stand-in; every response is
//! checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, each request stating the protocol version, with the
//! session's `Authorization: Bearer <token>`:
//! - `POST /devices` of `{"device": <ID>, "kind": "service", "signing": <bytes>, "hpke": <bytes>,
//!   "name": <name>}`: registers a `service` device of the session's account; `201`, and `200` for
//!   the same registration again.
//! - `PUT /devices/{device}/cert` of `{"cert": <bytes>}`, the certificate's CBOR bytes: stores the
//!   certificate of a device the session's account registered without one; `201`, and `200` for
//!   the same certificate again. Refused with a `4xx` and a problem details object, storing
//!   nothing: a certificate that fails check 1 or 2, one issued by a device of another account, and
//!   one whose account, kind, keys or `name` differ from the registration. With `--devices-limit`,
//!   the certificate of a `client` device of an account that has that many certified devices,
//!   `service` and revoked ones aside, is refused with `403` and the problem
//!   `https://skald.so/problems/devices-limit`.
//! - `GET /devices`: each device of the account with its certificate as the device sent it, or
//!   `null` while it has none.
//! - `GET /account/sessions` with the web session's `Cookie`: each device session with the
//!   `device_name` of the device it registered or connected as.
//!
//! Bytes are base64url without padding.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests.
#[path = "../spaces/api.rs"]
mod api;

mod certificates;
mod limit;
mod registering;

use serde_json::{Value, json};
use skald_proto::{AccountId, CertBody, DeviceId, DeviceKind, PROTOCOL_VERSION, VERSION_HEADER};

use api::{Session, assert_refused, the_spec};
use common::device_flow::{DeviceFlow, json, text};
use common::devices::{
    DEVICES, Device, Key, Machine, account_id, assert_taken, b64, cert_of, devices_of, machine,
    write_devices,
};
use common::spec::operation_violations;
use common::{Response, get_url};

/// The account's sessions: `GET`, with a web session.
pub const SESSIONS: &str = "/account/sessions";
/// The `--devices-limit` setting, and its problem.
pub const DEVICES_LIMIT: &str = "--devices-limit";
pub const OVER_DEVICES: &str = "https://skald.so/problems/devices-limit";

/// Where a device's certificate goes: `PUT`.
pub fn cert_path(device: DeviceId) -> String {
    format!("/devices/{device}/cert")
}

/// The registration of `device` as a `service` device.
pub fn service_registration(device: &Device) -> Value {
    let mut registration = device.registration();
    registration["kind"] = json!("service");
    registration
}

/// The certificate body of `device` as a device of `kind` on `account`, issued by `issuer`.
pub fn body_by(
    device: &Device,
    account: AccountId,
    kind: DeviceKind,
    issuer: DeviceId,
) -> CertBody {
    let mut body = device.body_as(account, kind);
    body.issuer = Some(issuer);
    body
}

/// An account signed in on a machine whose session registered its laptop, which the master key
/// certified, and nothing after it.
pub struct Owner<'f> {
    pub machine: Machine<'f>,
    pub session: Session,
    pub master: Key,
    pub laptop: Device,
}

impl<'f> Owner<'f> {
    /// Signs in the GitHub user `github` with `login` on `flow`'s server, registers its laptop and
    /// gives the master key with the laptop's certificate.
    pub fn new(flow: &'f DeviceFlow, github: u64, login: &str) -> Self {
        let user = flow.stand.user(github, login);
        let machine = machine(flow, &user);
        let master = Key::generate();
        let laptop = Device::new("Laptop");
        machine.register_device(&laptop);
        let cert = laptop.cert(&machine.account, &master);
        assert_taken(&machine.give_master(&master, &cert));
        let session = Session::of_token(flow, &machine.token);
        Owner {
            machine,
            session,
            master,
            laptop,
        }
    }

    pub fn account(&self) -> AccountId {
        account_id(&self.machine.account)
    }

    /// `POST /devices` of `registration` from this session.
    pub fn register(&self, registration: &Value) -> Response {
        self.session.call("post", DEVICES, Some(registration))
    }

    /// Registers a new `service` device named `name`, which the server must take.
    #[track_caller]
    pub fn register_service(&self, name: &str) -> Device {
        let device = Device::new(name);
        let response = self.register(&service_registration(&device));
        assert_eq!(
            response.status,
            201,
            "a new service device: {}",
            text(&response)
        );
        device
    }

    /// The certificate of `device` as a `service` device of this account, issued by `issuer`, a
    /// device of the account.
    pub fn service_cert(&self, device: &Device, issuer: &Device) -> Vec<u8> {
        let body = body_by(device, self.account(), DeviceKind::Service, issuer.id);
        cert_of(&body, &issuer.signing.key, &device.signing.key)
    }

    /// `PUT /devices/<device>/cert` of `cert` from this session.
    pub fn upload(&self, device: DeviceId, cert: &[u8]) -> Response {
        let body = json!({ "cert": b64(cert) });
        self.session.call("put", &cert_path(device), Some(&body))
    }

    /// The certificate `GET /devices` gives for `device`, which the account must list.
    #[track_caller]
    pub fn cert_of(&self, device: DeviceId) -> Option<Vec<u8>> {
        let devices = devices_of(&self.session.call("get", DEVICES, None));
        devices
            .get(&device.to_string())
            .unwrap_or_else(|| panic!("{device} is not listed: {devices:?}"))
            .clone()
    }

    /// Writes to the database a `service` device of this account certified by its laptop, which
    /// the server's API did not take; gives it.
    pub fn written_service(&self, flow: &DeviceFlow, name: &str) -> Device {
        let device = Device::new(name);
        let cert = self.service_cert(&device, &self.laptop);
        write_devices(
            &flow.stand.db,
            self.account(),
            DeviceKind::Service,
            &[(&device, Some(&cert))],
        );
        device
    }
}

/// Checks that `response` refuses a certificate and that `owner`'s account keeps `device` without
/// one; `what` names the case.
#[track_caller]
pub fn assert_cert_refused(owner: &Owner, device: DeviceId, response: &Response, what: &str) {
    assert_refused(response, what);
    assert_eq!(
        owner.cert_of(device),
        None,
        "{what}: the certificate is stored"
    );
}

/// The `device_name` of each device session the web session `cookie` lists on `flow`'s server.
#[track_caller]
pub fn device_session_names(flow: &DeviceFlow, cookie: &str) -> Vec<Option<String>> {
    let version = PROTOCOL_VERSION.to_string();
    let url = format!("{}{SESSIONS}", flow.stand.server.base());
    let answer = get_url(&url, &[("cookie", cookie), (VERSION_HEADER, &version)]);
    let checked = (
        answer.status,
        operation_violations(the_spec(), "get", SESSIONS, &answer),
    );
    assert_eq!(checked, (200, Vec::new()), "{}", text(&answer));
    let body = json(&answer);
    body["sessions"]
        .as_array()
        .unwrap_or_else(|| panic!("no sessions in {body}"))
        .iter()
        .filter(|session| session["kind"] == "device")
        .map(|session| session["device_name"].as_str().map(str::to_owned))
        .collect()
}
