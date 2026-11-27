//! `skald` behind an HTTPS proxy (`docs/dev/architecture.md`, "Network"): with `HTTPS_PROXY`, or
//! `https_proxy`, set to a proxy's URL, every HTTPS request and the WebSocket of the client to its
//! server go through that proxy by an HTTP `CONNECT` tunnel to the server's host and port; TLS, its
//! certificate checks and the device's proof of its key stay end to end. A server whose host
//! `NO_PROXY`, or `no_proxy`, lists, as curl reads it, and plain HTTP to a server on this machine
//! take no proxy; other proxy settings than these two are not read. Credentials in the proxy's URL,
//! percent-decoded as curl decodes them, go to the proxy as `Proxy-Authorization: Basic` on each
//! `CONNECT`; the client answers no other authentication scheme. A proxy that refuses the tunnel or
//! cannot be reached fails the command with a message that says `proxy` and names the proxy by its
//! host and port, never the credentials its URL holds; the client never connects to the server
//! directly instead.
//!
//! The server sits behind the TLS endpoint of `https/front.rs`, with a certificate of a test CA that
//! the machines trust through `SSL_CERT_FILE`, and the recording proxy of `proxy.rs` tunnels to it.
//! The machines reach the server at `https://skald.test:<port>`, a name no resolver knows, so only
//! a proxy reaches it there; at `https://127.0.0.1:<port>` and `https://localhost:<port>` they
//! could reach it directly too. For sign-in, `skald send` and `skald recv`, the server has GitHub
//! sign-in through the GitHub stand-in, and its public address is `https://skald.test:<port>`,
//! which headless Chrome, where a test plays the person, reaches without the proxy. The other
//! member's device is played by the test, as in the tests of channel messages.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod proxy;
mod trust;

// A proxy that is down.
#[path = "../../src/closed_port.rs"]
mod closed_port;

// The TLS endpoint and test CA of the tests of HTTPS.
#[path = "../common/tls.rs"]
mod tls;
#[path = "../https/front.rs"]
mod tls_front;

// The server, the relay and the machines of the tests of signing in, people in spaces, and the
// person's browser.
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The channel of the tests of channel messages, the device they play, and what they need: the kit
// of channel groups, the front and fixtures of the tests of sending, the peer of the tests of
// receiving, and the server's tests' readers of the KeyPackages API's answers.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// Finding keys in bytes, and the helpers of every client test.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browser::Browser;
use closed_port::closed_port;
use cloud::Cloud;
use common::Server;
use fixtures::{TIMEOUT_SETTING, send};
use kit::{RECV, create_space, received, with_channel};
use machine::{Finished, Machine, SERVER, SIGN_IN};
use played::Played;
use proxy::{BAD_GATEWAY, BASIC_ASKED, FORBIDDEN, OTHER_SCHEMES_ASKED, Proxy, Request};
use skald_proto::{PROTOCOL_VERSION, Role};
use talk::{ADA_HOST, BOB_HOST, Line, assert_read, assert_shows_once, encrypted, stored};
use tls_front::Front;
use trust::{HOST, HTTPS_PROXY, Trust, signed_up};
use world::{ACME, Person, World};

/// The setting of the proxy in lowercase, and those of the hosts that take none, in both cases.
const HTTPS_PROXY_LOWER: &str = "https_proxy";
const NO_PROXY: &str = "NO_PROXY";
const NO_PROXY_LOWER: &str = "no_proxy";
/// The proxy settings the client does not read.
const OTHER_PROXIES: [&str; 4] = ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"];
/// The names of the endpoint's certificate: the hosts the tests reach it at.
const NAMES: &str = "DNS:skald.test, DNS:localhost, IP:127.0.0.1";
/// The names of a certificate for another server than the endpoint.
const OTHER_NAMES: &str = "DNS:other.skald.test";
/// What the client says of a certificate of a CA it does not trust, and of one for another name.
const UNKNOWN_ISSUER: &str = "invalid peer certificate: UnknownIssuer";
const NOT_VALID_FOR_NAME: &str = "invalid peer certificate: certificate not valid for name";

/// The status command, and the request timeout in seconds of the runs a proxy fails.
const STATUS: &str = "status";
const TIMEOUT: &str = "4";

/// Credentials in the proxy's URL as they go there, and the user and password they stand for:
/// plain, and percent-encoded with characters a URL's userinfo cannot hold.
const PLAIN: (&str, &str, &str) = (
    "ada-proxy-k7w2:Pr0xy-pass-3vq9",
    "ada-proxy-k7w2",
    "Pr0xy-pass-3vq9",
);
const ENCODED: (&str, &str, &str) = (
    "ada%40corp-m4x8:p%3Ass%2Fw0rd-8jd1",
    "ada@corp-m4x8",
    "p:ss/w0rd-8jd1",
);

/// The texts the tests send; none is anywhere else.
const ADA_SAYS: &str = "the proxy lets this one through, ship the release";
const BOB_SAYS: &str = "got it behind the corporate proxy, thanks";

/// A `skald-server` without sign-in behind the TLS endpoint, whose certificate the CA of `trust`
/// issued for [`NAMES`].
fn site(trust: &Trust) -> (Server, Front) {
    let server = Server::start(None);
    let address = server.url.trim_start_matches("http://").to_owned();
    let front = Front::start(&address, vec![trust.ca.issue(NAMES)]);
    (server, front)
}

/// Runs `skald status` on `machine` against `url`.
fn status(machine: &Machine, url: &str) -> Finished {
    machine.start(&[STATUS, SERVER, url]).finish()
}

/// Fails unless `finished` found the server at `url` reachable and compatible.
#[track_caller]
fn assert_reachable(finished: &Finished, url: &str) {
    assert!(finished.success, "{finished:?}");
    assert_eq!(
        finished.stdout,
        format!("Server {url}: reachable\nProtocol: version {PROTOCOL_VERSION}, compatible\n")
    );
}

/// Fails unless `finished` failed with a message that says `proxy` and names the proxy at
/// `address`, its host and port, and unless what it printed holds none of `secrets`.
#[track_caller]
fn assert_fails_naming(finished: &Finished, address: &str, secrets: &[String]) {
    assert!(!finished.success, "{finished:?}");
    assert!(finished.stderr.contains(address), "{finished:?}");
    assert!(
        finished.stderr.to_lowercase().contains("proxy"),
        "{finished:?}"
    );
    let shown = finished.shown();
    for secret in secrets {
        assert!(!shown.contains(secret.as_str()), "{secret:?}: {finished:?}");
    }
}

/// Runs `skald status` and `skald sign-in` on `machine` against `front` at `127.0.0.1`; fails
/// unless each fails as [`assert_fails_naming`] says, naming the proxy at `address`, and unless the
/// front took no connection.
#[track_caller]
fn assert_commands_fail(machine: &Machine, front: &Front, address: &str, secrets: &[String]) {
    let url = front.url("127.0.0.1");
    for command in [STATUS, SIGN_IN] {
        let finished = machine.start(&[command, SERVER, &url]).finish();
        assert_fails_naming(&finished, address, secrets);
    }
    assert_eq!(front.connections(), 0, "a direct connection");
}

/// What a message must not hold of the credentials `(userinfo, user, password)`: each of them, as
/// in the URL and decoded, and their Basic encoding.
fn secrets((userinfo, user, password): (&str, &str, &str)) -> Vec<String> {
    vec![
        userinfo.to_owned(),
        user.to_owned(),
        password.to_owned(),
        userinfo.split(':').nth(1).expect("a password").to_owned(),
        STANDARD.encode(format!("{user}:{password}")),
    ]
}

/// Fails unless the proxy got some requests and each of them is a `CONNECT` to `authority` that
/// carries no credentials.
#[track_caller]
fn assert_only_tunnels_to(proxy: &Proxy, authority: &str) -> Vec<Request> {
    let requests = proxy.requests();
    assert!(!requests.is_empty(), "nothing went through the proxy");
    for request in &requests {
        assert_eq!(request.line(), format!("CONNECT {authority} HTTP/1.1"));
        assert_eq!(
            request.headers("proxy-authorization"),
            Vec::<&str>::new(),
            "{request:?}"
        );
    }
    requests
}

/// Fails unless what the client sent through each tunnel of `requests` is TLS, from its first
/// byte, and none of it holds any of `texts` in the clear, in any case.
#[track_caller]
fn assert_only_tls(requests: &[Request], texts: &[&str]) {
    for request in requests {
        assert_eq!(
            request.tunneled.first(),
            Some(&0x16),
            "no TLS handshake through the tunnel: {request:?}"
        );
        let wire = String::from_utf8_lossy(&request.tunneled).to_lowercase();
        for text in texts {
            assert!(
                !wire.contains(&text.to_lowercase()),
                "{text:?} crossed the proxy in the clear"
            );
        }
    }
}

/// `person`'s person address in `space`, on a server named after its host, [`HOST`].
fn address_of(space: &str, person: &Person) -> String {
    format!("{space}/{}/user@{HOST}", person.handle)
}

/// Ada signs in through the proxy, creates a space with a channel with Bob, whose device the test
/// plays, and sends Bob a message, which he reads; then her `skald recv` shows his answer. The
/// proxy only tunnels to the server's host and port and sees nothing but TLS: no request, no
/// session token, no text. With the proxy down, `skald send` and `skald recv` fail, naming it,
/// and reach the server no other way.
#[test]
fn sign_in_send_and_recv_go_through_the_proxy_which_sees_only_tls() {
    let trust = Trust::new();
    let front = Front::bind(vec![trust.ca.issue(NAMES)]);
    let cloud = Cloud::fronted_over_tls(HOST, front.port(), 120, 1);
    front.forward_to(cloud.relay.address());
    let world = World {
        cloud,
        browser: Browser::launch(),
    };
    let cloud = &world.cloud;
    let proxy = Proxy::tunneling();
    let on = |host| trust.proxied(&proxy.url()).with_host_name(host);
    let mut ada = signed_up(
        &world,
        on(ADA_HOST),
        (4_143_031_001, "ada-behind-proxy"),
        ADA_HOST,
    );
    let bob = signed_up(
        &world,
        on(BOB_HOST),
        (4_143_031_002, "bob-behind-proxy"),
        BOB_HOST,
    );
    let mut bob = Played::new(cloud, bob);
    let space = create_space(cloud, &ada, ACME);
    let channel = with_channel(cloud, space.id, &ada, &[(&bob.person, Role::Member)]);
    received(&ada.machine);
    bob.take_in(cloud);

    let said = send(&ada.machine, channel, ADA_SAYS);

    assert!(said.success, "{said:?}");
    bob.take_in(cloud);
    let read = bob.read_in(channel);
    assert_eq!(read.len(), 1, "Bob read {read:?}");
    let msg_id = read[0].0;
    assert!(said.shown().contains(&msg_id.to_string()), "{said:?}");
    let from_ada = address_of(&space.name, &ada);
    assert_read(
        &bob,
        channel,
        (ada.device, &from_ada),
        &[(msg_id, 1, ADA_SAYS)],
    );

    let from_bob = address_of(&space.name, &bob.person);
    let line = Line::new(&from_bob, 1, BOB_SAYS);
    let answer = encrypted(&mut bob, (space.id, channel), channel, &line);
    stored(cloud, &bob, &answer);
    let shown = received(&ada.machine).shown();
    assert_shows_once(&shown, BOB_SAYS, (channel, &from_bob, BOB_HOST));

    let authority = format!("{HOST}:{}", front.port());
    let requests = assert_only_tunnels_to(&proxy, &authority);
    let tokens = cloud.tokens();
    let mut texts = vec!["http/1.1", "websocket", "authorization", ADA_SAYS, BOB_SAYS];
    texts.extend(tokens.iter().map(String::as_str));
    assert_only_tls(&requests, &texts);
    let plain = front.plain();
    assert!(plain.contains("get /ws "), "no WebSocket: {plain}");

    let down = closed_port().to_string();
    ada.machine = (ada.machine)
        .with_env(HTTPS_PROXY, format!("http://{down}"))
        .with_env(TIMEOUT_SETTING, TIMEOUT);
    let connections = front.connections();
    let recv = ada.machine.start(&[RECV]).finish();
    assert_fails_naming(&recv, &down, &[]);
    let sent = send(&ada.machine, channel, "said while the proxy is down");
    assert_fails_naming(&sent, &down, &[]);
    assert_eq!(front.connections(), connections);
}

/// `skald status` sends its request and opens its WebSocket each through a tunnel of the proxy
/// that `HTTPS_PROXY` names, and the same with `https_proxy`; the proxy sees only TLS.
#[test]
fn status_and_its_websocket_go_through_the_proxy_https_proxy_names_in_either_case() {
    for name in [HTTPS_PROXY, HTTPS_PROXY_LOWER] {
        let trust = Trust::new();
        let (_server, front) = site(&trust);
        let proxy = Proxy::tunneling();
        let machine = trust.machine().with_env(name, proxy.url());
        let url = front.url(HOST);

        let finished = status(&machine, &url);

        assert_reachable(&finished, &url);
        let requests = assert_only_tunnels_to(&proxy, &format!("{HOST}:{}", front.port()));
        assert_eq!(requests.len(), front.connections(), "{name}: {requests:?}");
        let plain = front.plain();
        assert!(plain.starts_with("get /protocol "), "{name}: {plain}");
        assert!(plain.contains("\r\nget /ws "), "{name}: {plain}");
        assert_only_tls(&requests, &["http/1.1", "/protocol", "websocket"]);
    }
}

/// A server whose address `NO_PROXY`, or `no_proxy`, lists, alone, among other hosts or by a CIDR
/// block, is reached directly, the proxy seeing nothing; a server it does not list still goes
/// through the proxy. So is `localhost` when it is listed.
#[test]
fn server_that_no_proxy_lists_is_reached_directly() {
    let listed = [
        (NO_PROXY, "127.0.0.1"),
        (NO_PROXY_LOWER, "127.0.0.1"),
        (NO_PROXY, "other.example, 127.0.0.1"),
        (NO_PROXY, "127.0.0.0/8"),
    ];
    let trust = Trust::new();
    let (_server, front) = site(&trust);
    for (name, value) in listed {
        let proxy = Proxy::tunneling();
        let machine = trust.proxied(&proxy.url()).with_env(name, value);

        let direct = front.url("127.0.0.1");
        assert_reachable(&status(&machine, &direct), &direct);
        assert_eq!(proxy.requests().len(), 0, "{name}={value}");

        let proxied = front.url(HOST);
        assert_reachable(&status(&machine, &proxied), &proxied);
        assert_only_tunnels_to(&proxy, &format!("{HOST}:{}", front.port()));
    }

    let proxy = Proxy::tunneling();
    let machine = trust.proxied(&proxy.url()).with_env(NO_PROXY, "localhost");
    let url = front.url("localhost");
    assert_reachable(&status(&machine, &url), &url);
    assert_eq!(proxy.requests().len(), 0);
}

/// `NO_PROXY` matches the server's host name as curl does: the name itself in any case, a domain
/// it ends in with or without a leading dot, or `*` for every host. A listed `skald.test` is
/// reached directly, which no resolver can, so the run fails, the proxy seeing nothing. A name
/// that only shares characters with it, or is longer, is no match: the run goes through the proxy.
#[test]
fn no_proxy_matches_host_names_as_curl_does() {
    let trust = Trust::new();
    let (_server, front) = site(&trust);
    let url = front.url(HOST);
    // `skald status` against `url` on a machine whose proxy is a new one and `NO_PROXY` `value`.
    let status_with = |value: &str| {
        let proxy = Proxy::tunneling();
        let machine = trust.proxied(&proxy.url()).with_env(NO_PROXY, value);
        (status(&machine, &url), proxy)
    };
    for value in [
        "skald.test",
        "SKALD.Test",
        ".skald.test",
        "test",
        ".test",
        "*",
    ] {
        let (finished, proxy) = status_with(value);
        assert!(!finished.success, "NO_PROXY={value}: {finished:?}");
        assert_eq!(proxy.requests().len(), 0, "NO_PROXY={value}");
    }
    for value in [
        "kald.test",
        "other.test",
        "skald.test.example",
        "skald",
        "127.0.0.1",
    ] {
        let (finished, proxy) = status_with(value);
        assert_reachable(&finished, &url);
        assert_only_tunnels_to(&proxy, &format!("{HOST}:{}", front.port()));
    }
}

/// Plain HTTP to a server on this machine takes no proxy, by address or as `localhost`; HTTPS to
/// the same server, through its TLS endpoint, does.
#[test]
fn plain_http_to_this_machine_takes_no_proxy() {
    let trust = Trust::new();
    let (server, front) = site(&trust);
    let proxy = Proxy::tunneling();
    let machine = trust.proxied(&proxy.url());
    let port = server.url.rsplit(':').next().expect("the server's port");

    for url in [server.url.clone(), format!("http://localhost:{port}")] {
        assert_reachable(&status(&machine, &url), &url);
    }
    assert_eq!(proxy.requests().len(), 0);

    let url = front.url(HOST);
    assert_reachable(&status(&machine, &url), &url);
    assert_only_tunnels_to(&proxy, &format!("{HOST}:{}", front.port()));
}

/// Without `HTTPS_PROXY` or `https_proxy` no proxy is used, whatever `HTTP_PROXY` and `ALL_PROXY`
/// say, in either case: a server is reached directly, or not at all; with `HTTPS_PROXY` added, the
/// proxy is used.
#[test]
fn without_https_proxy_no_proxy_is_used() {
    let trust = Trust::new();
    let (_server, front) = site(&trust);
    let proxy = Proxy::tunneling();
    let others = |machine: Machine| {
        OTHER_PROXIES
            .iter()
            .fold(machine, |machine, name| machine.with_env(name, proxy.url()))
    };
    let machine = others(trust.machine());

    let direct = front.url("127.0.0.1");
    assert_reachable(&status(&machine, &direct), &direct);
    let proxied = front.url(HOST);
    assert!(!status(&machine, &proxied).success);
    assert_eq!(proxy.requests().len(), 0);

    let machine = others(trust.proxied(&proxy.url()));
    assert_reachable(&status(&machine, &proxied), &proxied);
    assert_only_tunnels_to(&proxy, &format!("{HOST}:{}", front.port()));
}

/// Through the tunnel the client checks the server's certificate itself: one of a CA it does not
/// trust, or for another name, is refused, and nothing goes to the server inside TLS.
#[test]
fn certificate_checks_stay_end_to_end_through_the_tunnel() {
    let trust = Trust::new();
    let other = Trust::new();
    let wrong = [
        (other.ca.issue(NAMES), UNKNOWN_ISSUER),
        (trust.ca.issue(OTHER_NAMES), NOT_VALID_FOR_NAME),
    ];
    for (certificate, says) in wrong {
        let server = Server::start(None);
        let front = Front::start(server.url.trim_start_matches("http://"), vec![certificate]);
        let proxy = Proxy::tunneling();
        let url = front.url(HOST);

        let finished = status(&trust.proxied(&proxy.url()), &url);

        assert!(!finished.success, "{finished:?}");
        assert!(finished.stderr.contains(says), "{finished:?}");
        assert_only_tunnels_to(&proxy, &format!("{HOST}:{}", front.port()));
        assert_eq!(front.plain(), "");
    }
}

/// A proxy that refuses the tunnel, however it answers, fails `skald status` and `skald sign-in`
/// with a message naming it by its host and port, holding none of the credentials of its URL; the
/// client never connects to the server directly instead.
#[test]
fn proxy_that_refuses_the_tunnel_fails_the_command_without_a_direct_connection() {
    for answer in [FORBIDDEN, BASIC_ASKED, BAD_GATEWAY] {
        let trust = Trust::new();
        let (_server, front) = site(&trust);
        let proxy = Proxy::refusing(answer);
        let machine = trust
            .proxied(&proxy.url_as(PLAIN.0))
            .with_env(TIMEOUT_SETTING, TIMEOUT);

        assert_commands_fail(&machine, &front, &proxy.address(), &secrets(PLAIN));

        let authority = format!("127.0.0.1:{}", front.port());
        for request in proxy.requests() {
            assert_eq!(request.line(), format!("CONNECT {authority} HTTP/1.1"));
        }
    }
}

/// A proxy that is down fails `skald status` and `skald sign-in` with a message naming it by its
/// host and port, holding none of the credentials of its URL, plain or percent-encoded; the client
/// never connects to the server directly instead.
#[test]
fn proxy_that_is_down_fails_the_command_without_a_direct_connection() {
    for credentials in [PLAIN, ENCODED] {
        let trust = Trust::new();
        let (_server, front) = site(&trust);
        let down = closed_port().to_string();
        let machine = trust
            .proxied(&format!("http://{}@{down}", credentials.0))
            .with_env(TIMEOUT_SETTING, TIMEOUT);

        assert_commands_fail(&machine, &front, &down, &secrets(credentials));
    }
}

/// Credentials in the proxy's URL go to the proxy as `Proxy-Authorization: Basic` on each
/// `CONNECT`, percent-decoded as curl decodes them; the server never sees them.
#[test]
fn credentials_in_the_proxy_url_go_as_basic_on_each_connect() {
    for (userinfo, user, password) in [PLAIN, ENCODED] {
        let trust = Trust::new();
        let (_server, front) = site(&trust);
        let proxy = Proxy::tunneling();
        let url = front.url(HOST);

        let finished = status(&trust.proxied(&proxy.url_as(userinfo)), &url);

        assert_reachable(&finished, &url);
        let requests = proxy.requests();
        assert_eq!(requests.len(), front.connections(), "{requests:?}");
        let basic = STANDARD.encode(format!("{user}:{password}"));
        for request in &requests {
            assert_eq!(
                request.line(),
                format!("CONNECT {HOST}:{} HTTP/1.1", front.port())
            );
            let given = request.headers("proxy-authorization");
            assert_eq!(given.len(), 1, "{request:?}");
            let (scheme, credentials) = given[0].split_once(' ').expect("a scheme");
            assert!(scheme.eq_ignore_ascii_case("basic"), "{request:?}");
            assert_eq!(credentials.trim(), basic, "{request:?}");
        }
        let plain = front.plain();
        for secret in secrets((userinfo, user, password)) {
            assert!(
                !plain.contains(&secret.to_lowercase()),
                "{secret:?}: {plain}"
            );
        }
    }
}

/// A proxy that asks for NTLM, Negotiate or Digest gets no answer by them: the client sends Basic
/// with credentials in the proxy's URL and nothing without, then fails, naming the proxy.
#[test]
fn proxy_asking_for_another_scheme_gets_basic_or_nothing() {
    for credentials in [Some(PLAIN), None] {
        let trust = Trust::new();
        let (_server, front) = site(&trust);
        let proxy = Proxy::refusing(OTHER_SCHEMES_ASKED);
        let url = match credentials {
            Some((userinfo, ..)) => proxy.url_as(userinfo),
            None => proxy.url(),
        };
        let machine = trust.proxied(&url).with_env(TIMEOUT_SETTING, TIMEOUT);

        let finished = status(&machine, &front.url(HOST));

        let hidden = credentials.map(secrets).unwrap_or_default();
        assert_fails_naming(&finished, &proxy.address(), &hidden);
        let requests = proxy.requests();
        assert!(!requests.is_empty(), "nothing went to the proxy");
        let basic = credentials.map(|(_, user, password)| {
            format!("Basic {}", STANDARD.encode(format!("{user}:{password}")))
        });
        for request in &requests {
            let given: Vec<String> = request
                .headers("proxy-authorization")
                .into_iter()
                .map(str::to_owned)
                .collect();
            assert_eq!(given, Vec::from_iter(basic.clone()), "{request:?}");
        }
        assert_eq!(front.connections(), 0);
    }
}
