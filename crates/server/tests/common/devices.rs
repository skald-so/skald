//! Devices on a `skald-server` with the device flow, against the GitHub stand-in: a machine signs
//! in by plain HTTP, registers its device, gives the account's master key with its first device's
//! certificate, and reads the account's master key and devices. Every response is checked against
//! the OpenAPI spec in `api/openapi.json`.
//!
//! The API, each request stating the protocol version:
//! - `POST /devices` of `{"device": <ID>, "kind": "client", "signing": <bytes>, "hpke": <bytes>,
//!   "name": <name>}`: registers a device of the session's account.
//! - `GET /devices`: `{"devices": [{"device": <ID>, "cert": <bytes> | null}, ...]}`, the account's
//!   devices, each with its certificate as the device sent it.
//! - `PUT /account/master` of `{"master": <bytes>, "cert": <bytes>}`: the master public key and the
//!   first device's certificate, issued by that key.
//! - `GET /account/master`: `{"master": <bytes> | null}`.
//!
//! Bytes are base64url without padding (RFC 4648, §5); a certificate is its CBOR bytes. Taken:
//! `200` or `201`. Refused, with a problem details object: `400` for an invalid request, `401`
//! without a valid session, `409` for an ID the server has or an account that has a master key.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use skald_crypto::{SigningKey, issue};
use skald_github_stand_in::User;
use skald_proto::{AccountId, Cbor, CertBody, DeviceId, DeviceKind, Unknown};

use super::device_flow::{DeviceFlow, json, text};
use super::sign_in::Account;
use super::{Response, delete_url, get_url, post_form, send};

/// Registers a device: `POST`; the account's devices: `GET`.
pub const DEVICES: &str = "/devices";
/// Takes the master key and the first device's certificate: `PUT`; gives the master key: `GET`.
pub const MASTER: &str = "/account/master";
/// The device flow's verification page, where the person approves a code.
const PAGE: &str = "/device";

/// `bytes` as the API carries them: base64url without padding.
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The bytes of `text`, base64url without padding.
pub fn unb64(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(text)
        .unwrap_or_else(|err| panic!("{text:?} is not base64url without padding: {err}"))
}

/// 32 bytes from the operating system's cryptographically secure generator.
pub fn random() -> [u8; 32] {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).expect("the operating system's random generator");
    bytes
}

/// An Ed25519 key pair with its secret, so that a test can look for the secret where it must not
/// be.
pub struct Key {
    pub secret: [u8; 32],
    pub key: SigningKey,
}

impl Key {
    pub fn generate() -> Self {
        let secret = random();
        Key {
            secret,
            key: SigningKey::from_bytes(&secret),
        }
    }

    pub fn public(&self) -> [u8; 32] {
        self.key.public_key()
    }
}

/// A `client` device's keys as its machine makes them: an Ed25519 signing key, an X25519 public
/// key, the ID derived from the signing key, and its `name`.
pub struct Device {
    pub signing: Key,
    pub hpke: [u8; 32],
    pub id: DeviceId,
    pub name: String,
}

impl Device {
    pub fn new(name: &str) -> Self {
        let signing = Key::generate();
        let id = DeviceId::from_signing_key(&signing.public());
        Device {
            signing,
            hpke: random(),
            id,
            name: name.to_owned(),
        }
    }

    /// The registration of this device.
    pub fn registration(&self) -> Value {
        json!({
            "device": self.id.to_string(),
            "kind": "client",
            "signing": b64(&self.signing.public()),
            "hpke": b64(&self.hpke),
            "name": self.name,
        })
    }

    /// The certificate body that matches this device's registration on `account`, issued by the
    /// master key.
    pub fn body(&self, account: &Account) -> CertBody {
        CertBody {
            account: account_id(account),
            device: self.id,
            kind: DeviceKind::Client,
            signing: self.signing.public(),
            hpke: self.hpke,
            issuer: None,
            issued: 1_794_000_000,
            ssh: None,
            name: self.name.clone(),
            unknown: Unknown::default(),
        }
    }

    /// The CBOR bytes of this device's certificate on `account`, issued by `master`.
    pub fn cert(&self, account: &Account, master: &Key) -> Vec<u8> {
        cert_of(&self.body(account), &master.key, &self.signing.key)
    }
}

/// The CBOR bytes of the certificate of `body`, signed by `issuer` and proved by `device`.
pub fn cert_of(body: &CertBody, issuer: &SigningKey, device: &SigningKey) -> Vec<u8> {
    issue(body, issuer, device)
        .expect("a certificate body the spec allows")
        .to_cbor()
}

/// The ID of `account`.
pub fn account_id(account: &Account) -> AccountId {
    account.id.parse().expect("an account ID")
}

/// A machine signed in to an account through the device flow.
pub struct Machine<'f> {
    flow: &'f DeviceFlow,
    /// The session token, sent as `Authorization: Bearer <token>`.
    pub token: String,
    pub account: Account,
    /// The `Cookie` header of the web session of the person who approved the machine.
    pub cookie: String,
}

/// Signs a machine in to `user`'s account on `flow`'s server: the person, signed in by plain
/// HTTP, approves its code on the verification page.
pub fn machine<'f>(flow: &'f DeviceFlow, user: &User) -> Machine<'f> {
    let cookie = flow.web_cookie(user);
    let grant = flow.authorize();
    let decided = post_form(
        &format!("{}{PAGE}", flow.stand.server.base()),
        &[("cookie", &cookie), ("origin", &flow.stand.public)],
        &[("user_code", &grant.user_code), ("decision", "approve")],
    );
    assert_eq!(decided.status, 200, "{}", text(&decided));
    let token = flow.token(&grant);
    let account = flow.account_of(&token);
    Machine {
        flow,
        token,
        account,
        cookie,
    }
}

/// `<method> <path>` to `flow`'s server, `method` in lowercase, with the request headers `headers`
/// and the JSON `body` if given, stating the protocol version if `stated`; checked against the spec.
pub fn request(
    flow: &DeviceFlow,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    stated: bool,
) -> Response {
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let mut all = headers.to_vec();
    if stated {
        all.push((skald_proto::VERSION_HEADER, &version));
    }
    let url = format!("{}{path}", flow.stand.server.base());
    let response = match body {
        Some(body) => {
            all.push(("content-type", "application/json"));
            send(method, &url, &all, body)
        }
        None if method == "delete" => delete_url(&url, &all),
        None => get_url(&url, &all),
    };
    flow.conforms(method, path, &response);
    response
}

impl Machine<'_> {
    /// `<method> <path>` with this machine's session and the JSON `body` if given.
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Response {
        let body = body.map(|body| serde_json::to_vec(body).expect("JSON"));
        self.send_raw(method, path, body.as_deref())
    }

    /// `<method> <path>` with this machine's session and the bytes `body` as they are, such as a
    /// body that is not JSON.
    fn send_raw(&self, method: &str, path: &str, body: Option<&[u8]>) -> Response {
        let bearer = format!("Bearer {}", self.token);
        let headers = [("authorization", bearer.as_str())];
        request(self.flow, method, path, &headers, body, true)
    }

    /// `POST /devices` of `registration`.
    pub fn register(&self, registration: &Value) -> Response {
        self.send("post", DEVICES, Some(registration))
    }

    /// `POST /devices` of `body`'s bytes as they are.
    pub fn register_raw(&self, body: &[u8]) -> Response {
        self.send_raw("post", DEVICES, Some(body))
    }

    /// Registers `device` and checks that the server takes it.
    pub fn register_device(&self, device: &Device) {
        assert_taken(&self.register(&device.registration()));
    }

    /// `PUT /account/master` of `request`.
    pub fn give(&self, request: &Value) -> Response {
        self.send("put", MASTER, Some(request))
    }

    /// `PUT /account/master` of `master`'s public key and the certificate `cert`.
    pub fn give_master(&self, master: &Key, cert: &[u8]) -> Response {
        self.give(&json!({ "master": b64(&master.public()), "cert": b64(cert) }))
    }

    /// `DELETE /session`: ends this machine's session.
    pub fn sign_out(&self) -> Response {
        self.send("delete", super::sign_in::SESSION, None)
    }

    /// The account's master public key as `GET /account/master` gives it.
    pub fn master(&self) -> Option<Vec<u8>> {
        master_of(&self.send("get", MASTER, None))
    }

    /// The account's devices as `GET /devices` gives them: each device's certificate by its ID.
    pub fn devices(&self) -> BTreeMap<String, Option<Vec<u8>>> {
        devices_of(&self.send("get", DEVICES, None))
    }
}

/// The master key in `response`, a `GET /account/master` answer.
pub fn master_of(response: &Response) -> Option<Vec<u8>> {
    assert_eq!(response.status, 200, "{}", text(response));
    let body = json(response);
    match &body["master"] {
        Value::Null => None,
        Value::String(master) => {
            let master = unb64(master);
            assert_eq!(master.len(), 32, "{body}");
            Some(master)
        }
        _ => panic!("no master in {body}"),
    }
}

/// The devices in `response`, a `GET /devices` answer: each device's certificate by its ID.
pub fn devices_of(response: &Response) -> BTreeMap<String, Option<Vec<u8>>> {
    assert_eq!(response.status, 200, "{}", text(response));
    let body = json(response);
    let listed = body["devices"]
        .as_array()
        .unwrap_or_else(|| panic!("no devices in {body}"));
    let mut devices = BTreeMap::new();
    for device in listed {
        let id = device["device"].as_str().expect("a device ID").to_owned();
        let cert = match &device["cert"] {
            Value::Null => None,
            Value::String(cert) => Some(unb64(cert)),
            _ => panic!("no cert in {device}"),
        };
        assert!(
            devices.insert(id.clone(), cert).is_none(),
            "{id} twice: {body}"
        );
    }
    devices
}

/// Checks that the server took what `response` answers.
pub fn assert_taken(response: &Response) {
    assert!(
        matches!(response.status, 200 | 201),
        "taken: {}",
        text(response)
    );
}

/// Checks that `response` refuses with `status` and a problem details object.
pub fn assert_refused(response: &Response, status: u16) {
    assert_eq!(response.status, status, "{}", text(response));
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "{}: {}",
        response.content_type,
        text(response)
    );
}
