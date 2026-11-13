//! `skald` to a server behind TLS, as Caddy serves `skald-server`: a test TLS endpoint before it
//! ([`front`]) presents a certificate of a test CA, which the client trusts only when
//! `SSL_CERT_FILE` names it instead of the platform's root certificates. Its HTTPS requests and its
//! WebSocket, over WSS, check the certificate each.

#[allow(dead_code)]
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[allow(dead_code)]
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
mod front;
#[allow(dead_code)]
#[path = "../sign_in/machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../common/tls.rs"]
mod tls;

use browser::{APPROVE, Browser};
use cloud::{Cloud, shown_grant};
use common::{Server, Stub, response};
use front::Front;
use machine::{DEVICES, FIRST_DEVICE, Finished, Machine, SERVER, SIGN_IN, THIS_DEVICE, YES};
use skald_proto::PROTOCOL_VERSION;
use tls::Ca;

/// The status command.
const STATUS: &str = "status";
/// The setting of the root certificates the client trusts instead of the platform's.
const CERT_FILE: &str = "SSL_CERT_FILE";
/// The names of the endpoint's certificate: the address the tests reach it at, and a name no
/// resolver knows.
const NAMES: &str = "DNS:skald.test, IP:127.0.0.1";
/// The names of a certificate for another server than the endpoint.
const OTHER_NAMES: &str = "DNS:other.skald.test";
/// What the client says of a certificate of a CA it does not trust, and of one for another name.
const UNKNOWN_ISSUER: &str = "invalid peer certificate: UnknownIssuer";
const NOT_VALID_FOR_NAME: &str = "invalid peer certificate: certificate not valid for name";

/// A machine that trusts the test CA it gives, and nothing else.
fn trusting() -> (Machine, Ca) {
    let machine = Machine::new();
    let ca = Ca::new(&machine.root().join("ca"));
    let machine = machine.with_env(CERT_FILE, &ca.cert);
    (machine, ca)
}

/// The address `url`, a local server's `http://` URL, listens on.
fn address(url: &str) -> &str {
    url.strip_prefix("http://").expect("a local server's URL")
}

/// A stub that answers every request with an empty JSON object.
fn stub() -> Stub {
    Stub::start(|_| response("200 OK", "application/json", "{}"))
}

/// Fails unless `wire` holds none of `texts`, in any case.
fn assert_unreadable(wire: &[u8], texts: &[&str]) {
    let wire = String::from_utf8_lossy(wire).to_lowercase();
    for text in texts {
        assert!(
            !wire.contains(&text.to_lowercase()),
            "{text:?} crossed the wire in the clear"
        );
    }
}

/// Fails unless `finished` failed, said `says` and printed nothing else to stdout but `stdout`.
fn assert_refused(finished: &Finished, stdout: &str, says: &str) {
    assert!(!finished.success, "{finished:?}");
    assert_eq!(finished.stdout, stdout, "{finished:?}");
    assert!(finished.stderr.contains(says), "{finished:?}");
}

/// Runs `skald status` on `machine` against `url`; fails unless it finds the server unreachable
/// and says `says`.
fn assert_status_refused(machine: &Machine, url: &str, says: &str) {
    let status = machine.start(&[STATUS, SERVER, url]).finish();
    assert_refused(&status, &format!("Server {url}: unreachable\n"), says);
}

/// Fails unless `front` took `connections` and none of them carried anything to `stub`.
fn assert_nothing_forwarded(front: &Front, stub: &Stub, connections: usize) {
    assert_eq!(front.connections(), connections);
    assert_eq!(front.plain(), "");
    assert_eq!(stub.requests(), Vec::<String>::new());
}

#[test]
fn status_goes_over_https_and_its_websocket_over_wss() {
    let server = Server::start(None);
    let (machine, ca) = trusting();
    let front = Front::start(address(&server.url), vec![ca.issue(NAMES)]);
    let url = front.url("127.0.0.1");

    let finished = machine.start(&[STATUS, SERVER, &url]).finish();

    assert!(finished.success, "{finished:?}");
    assert_eq!(
        finished.stdout,
        format!("Server {url}: reachable\nProtocol: version {PROTOCOL_VERSION}, compatible\n")
    );
    // The request and the WebSocket's opening request, each on a connection of its own.
    let plain = front.plain();
    assert!(plain.starts_with("get /protocol http/1.1\r\n"), "{plain}");
    assert!(plain.contains("\r\nget /ws http/1.1\r\n"), "{plain}");
    assert!(plain.contains("\r\nupgrade: websocket\r\n"), "{plain}");
    assert_eq!(front.connections(), 2);
    assert_unreadable(&front.wire(), &["http/1.1", "/protocol", "websocket"]);
}

#[test]
fn signs_in_over_https_and_keeps_the_session_on_it() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let page = browser.page();
    let ada = cloud.user(4_120_665_101, "ada-https-standin");
    let (machine, ca) = trusting();
    let front = Front::start(address(&cloud.url), vec![ca.issue(NAMES)]);
    let url = front.url("127.0.0.1");

    let mut run = machine.start(&[SIGN_IN, SERVER, &url, FIRST_DEVICE, YES]);
    let grant = shown_grant(&cloud, &mut run, 0);
    cloud.decide(&page, &ada, &grant, APPROVE);
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    let signed_in = format!("Signed in to {url} as {}.", ada.login);
    assert!(finished.stdout.contains(&signed_in), "{finished:?}");
    // The stored session goes to the server over HTTPS too.
    let devices = machine.start(&[DEVICES]).finish();
    assert!(devices.success, "{devices:?}");
    assert!(devices.stdout.contains(THIS_DEVICE), "{devices:?}");
    let listed = cloud.relay.sent().into_iter().rev().find(|sent| {
        sent.head.starts_with("get /devices ") && sent.head.contains("\nauthorization: bearer ")
    });
    assert!(listed.is_some(), "no device list: {:?}", cloud.relay.sent());
    // The session token reached the server only inside TLS.
    let token = cloud.last_token();
    let sent = String::from_utf8_lossy(&cloud.relay.sent_bytes()).into_owned();
    assert!(sent.contains(&token), "the token never reached the server");
    assert!(front.plain().contains(&token.to_lowercase()));
    assert_unreadable(&front.wire(), &[&token, "authorization", "http/1.1"]);
}

#[test]
fn certificate_of_a_ca_the_client_does_not_trust_is_refused() {
    let stub = stub();
    let (machine, _) = trusting();
    let other = Ca::new(&machine.root().join("other-ca"));
    let front = Front::start(address(&stub.url), vec![other.issue(NAMES)]);
    let url = front.url("127.0.0.1");
    // Nor do the platform's root certificates have the CA.
    let platform = Machine::new();

    for machine in [&machine, &platform] {
        assert_status_refused(machine, &url, UNKNOWN_ISSUER);
        let sign_in = machine.start(&[SIGN_IN, SERVER, &url]).finish();
        assert_refused(&sign_in, "", UNKNOWN_ISSUER);
    }

    assert_nothing_forwarded(&front, &stub, 4);
}

#[test]
fn certificate_for_another_name_is_refused() {
    let stub = stub();
    let (machine, ca) = trusting();
    let front = Front::start(address(&stub.url), vec![ca.issue(OTHER_NAMES)]);

    assert_status_refused(&machine, &front.url("127.0.0.1"), NOT_VALID_FOR_NAME);

    assert_nothing_forwarded(&front, &stub, 1);
}

#[test]
fn websocket_checks_the_certificate_too() {
    let (machine, ca) = trusting();
    let other = Ca::new(&machine.root().join("other-ca"));
    let wrong = [
        (other.issue(NAMES), UNKNOWN_ISSUER),
        (ca.issue(OTHER_NAMES), NOT_VALID_FOR_NAME),
    ];

    for (certificate, says) in wrong {
        let stub = stub();
        // The request's connection gets the right certificate, the WebSocket's the wrong one.
        let front = Front::start(address(&stub.url), vec![ca.issue(NAMES), certificate]);

        assert_status_refused(&machine, &front.url("127.0.0.1"), says);

        assert_eq!(front.connections(), 2);
        let plain = front.plain();
        assert!(plain.starts_with("get /protocol "), "{plain}");
        assert!(!plain.contains("/ws"), "{plain}");
        let requests = stub.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
    }
}
