//! GitHub's API over HTTPS at a host that only the proxy `HTTPS_PROXY` names reaches: the checks
//! go through that proxy by a `CONNECT` tunnel to GitHub's host, and nothing of the Skald session
//! reaches GitHub, neither in what the stand-in got nor in what the client sent inside TLS. The
//! server stays at its plain HTTP address on this machine, which takes no proxy.

use crate::machine::Machine;
use crate::proxy::Proxy;
use crate::tls::Ca;
use crate::tls_front::Front;
use crate::{GITHUB_API, LOGIN, Listed, Scene, start_github};
use crate::{assert_carry_nothing_of_the_session, shows_login};

/// The setting of the proxy, and of the root certificates the client trusts instead of the
/// platform's.
const HTTPS_PROXY: &str = "HTTPS_PROXY";
const CERT_FILE: &str = "SSL_CERT_FILE";

/// GitHub's API host, which no resolver knows; the proxy tunnels to this machine.
const HOST: &str = "api.github.test";

#[test]
fn checks_go_through_https_proxy_and_carry_nothing_of_the_session() {
    // The CA's files, in a directory removed on drop.
    let keeper = Machine::new();
    let ca = Ca::new(&keeper.root().join("ca"));
    let github = start_github();
    let front = Front::start(
        github.url().trim_start_matches("http://"),
        vec![ca.issue(&format!("DNS:{HOST}"))],
    );
    let proxy = Proxy::tunneling();
    let env = [
        (GITHUB_API, front.url(HOST)),
        (HTTPS_PROXY, proxy.url()),
        (CERT_FILE, ca.cert.to_string_lossy().into_owned()),
    ];
    let scene = Scene::alone(env.map(|(name, value)| (name.to_owned(), value)).to_vec());
    scene.list_ada(&github, LOGIN, Listed::Authentication);

    let own = scene.own();
    assert!(shows_login(&own, scene.laptop.id, LOGIN), "{own:?}");
    proxy.assert_tunneled(HOST, front.port());
    let requests = github.requests();
    assert!(!requests.is_empty(), "no request to GitHub's API");
    assert_carry_nothing_of_the_session(&requests, &scene.ada);
    assert!(
        !front.plain().contains(&scene.ada.token.to_lowercase()),
        "the session token went to GitHub"
    );
}
