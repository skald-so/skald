//! Bob's resolver over HTTPS at a host that only the proxy `HTTPS_PROXY` names reaches: the lookups
//! go through that proxy by a `CONNECT` tunnel to the resolver's host. The server stays at its
//! plain HTTP address on this machine, which takes no proxy.

use crate::doh::Doh;
use crate::machine::Machine;
use crate::proxy::Proxy;
use crate::tls::Ca;
use crate::tls_front::Front;
use crate::{LONG_TTL, RECORDS, RESOLVER, Scene, listed};

/// The setting of the proxy, and of the root certificates the client trusts instead of the
/// platform's.
const HTTPS_PROXY: &str = "HTTPS_PROXY";
const CERT_FILE: &str = "SSL_CERT_FILE";

/// The resolver's host, which no resolver knows; the proxy tunnels to this machine.
const HOST: &str = "dns.resolver.test";

#[test]
fn lookups_go_through_https_proxy() {
    // The CA's files, in a directory removed on drop.
    let keeper = Machine::new();
    let ca = Ca::new(&keeper.root().join("ca"));
    let doh = Doh::start();
    let front = Front::start(&doh.address(), vec![ca.issue(&format!("DNS:{HOST}"))]);
    let proxy = Proxy::tunneling();
    let env = [
        (RESOLVER, format!("{}/dns-query", front.url(HOST))),
        (HTTPS_PROXY, proxy.url()),
        (CERT_FILE, ca.cert.to_string_lossy().into_owned()),
    ];
    let env = env.map(|(name, value)| (name.to_owned(), value));
    let scene = Scene::with(
        [4_187_401_001, 4_187_401_002],
        ["bob-behind-a-proxy", "ada-looked-up-by-proxy"],
        doh,
        &env,
    );
    scene.name_domain(|_| {});
    scene.records(LONG_TTL, &[listed(&scene.bob), listed(&scene.ada)]);

    let shown = scene.members();

    scene.assert_listed(&shown, &scene.ada, true);
    let connect = format!("CONNECT {HOST}:{} ", front.port());
    let tunnels = proxy.requests();
    assert!(
        tunnels
            .iter()
            .any(|request| request.line().starts_with(&connect)),
        "no {connect}through the proxy: {tunnels:?}"
    );
    assert!(
        scene.doh.txt_queries(RECORDS) > 0,
        "no lookup reached the resolver: {:?}",
        scene.doh.queries()
    );
}
