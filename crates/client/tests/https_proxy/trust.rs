//! What the tests of `skald` behind a proxy share: the settings of the proxy and of the root
//! certificates the client trusts, the server's host that only the proxy reaches, a test CA the
//! machines trust, and signing up through the proxy.

use crate::browser::APPROVE;
use crate::cloud::shown_grant;
use crate::machine::{Machine, SERVER, SIGN_IN};
use crate::tls::Ca;
use crate::world::{Person, World};

/// The setting of the proxy.
pub const HTTPS_PROXY: &str = "HTTPS_PROXY";
/// The setting of the root certificates the client trusts instead of the platform's.
pub const CERT_FILE: &str = "SSL_CERT_FILE";

/// The server's host that only the proxy reaches; Chrome maps it to this machine.
pub const HOST: &str = "skald.test";

/// A test CA, its files in the directory of a machine of its own, removed on drop.
pub struct Trust {
    pub ca: Ca,
    _keeper: Machine,
}

impl Trust {
    pub fn new() -> Self {
        let keeper = Machine::new();
        Trust {
            ca: Ca::new(&keeper.root().join("ca")),
            _keeper: keeper,
        }
    }

    /// A new machine that trusts the CA and nothing else.
    pub fn machine(&self) -> Machine {
        Machine::new().with_env(CERT_FILE, &self.ca.cert)
    }

    /// A new machine that trusts the CA and whose proxy is at `url`.
    pub fn proxied(&self, url: &str) -> Machine {
        self.machine().with_env(HTTPS_PROXY, url)
    }
}

/// A new account of GitHub user `id` with `login`, whose first machine, `machine` on the host
/// `host`, signs in at the server's public address through its proxy, the person approving its
/// code in Chrome.
pub fn signed_up(
    world: &World,
    machine: Machine,
    (id, login): (u64, &str),
    host: &'static str,
) -> Person {
    let (cloud, page) = (&world.cloud, world.browser.page());
    let grants = cloud.grants().len();
    let user = cloud.user(id, login);
    let args = [SIGN_IN, SERVER, cloud.public.as_str()];
    let mut signing_in = machine.start(&args);
    let grant = shown_grant(cloud, &mut signing_in, grants);
    cloud.decide(&page, &user, &grant, APPROVE);
    let finished = signing_in.finish();
    assert!(finished.success, "{login}: {finished:?}");
    world.person(user, machine, host)
}
