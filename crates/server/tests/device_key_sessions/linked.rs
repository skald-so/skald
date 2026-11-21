//! What the tests share: alice's account, her laptop certified by her master key, bob's, and
//! alice's new tablet, whose run without a session alice's session paired, with the certificate
//! her laptop issued it; the session the tablet asks for, and the account's sessions of it.

use skald_proto::DeviceKind;

use crate::api::Session;
use crate::common::Response;
use crate::common::devices::Device;
use crate::kit::{sign_in_by_key, token_of};
use crate::runs::{Fresh, pair};
use crate::upload::issued_by;
use crate::web::{Api, Kind, Listed};

/// The name alice's new device registers with.
pub const TABLET: &str = "Tablet";

pub struct Linked {
    pub fresh: Fresh,
    pub tablet: Device,
    /// The tablet's complete certificate, issued by alice's laptop.
    pub cert: Vec<u8>,
    /// The `Cookie` header of a web session of alice's.
    pub cookie: String,
}

impl Linked {
    /// The accounts on a server whose GitHub user IDs start at `github`.
    pub fn start(github: u64) -> Self {
        Self::with(github, &[])
    }

    /// As [`Linked::start`], on a server with the extra arguments `args`.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let fresh = Fresh::with(github, args);
        let tablet = Device::new(TABLET);
        let run = fresh.open(&tablet);
        pair(&fresh.alice.session, &run.pairing);
        let alice = &fresh.alice;
        let cert = issued_by(&tablet, alice.account, DeviceKind::Client, &alice.laptop);
        let cookie = fresh.flow.web_cookie(&fresh.alice_user());
        Linked {
            fresh,
            tablet,
            cert,
            cookie,
        }
    }

    /// Requests to the server without a session.
    pub fn nobody(&self) -> Session {
        self.fresh.nobody()
    }

    /// The tablet asks for a session with its certificate, on a fresh challenge.
    pub fn ask(&self) -> Response {
        sign_in_by_key(&self.fresh.flow, &self.nobody(), &self.tablet, &self.cert)
    }

    /// The tablet's session token; the server must give one.
    #[track_caller]
    pub fn session(&self) -> String {
        token_of(&self.ask())
    }

    /// The server's API, as the tests of sessions on the web reach it.
    pub fn api(&self) -> Api<'_> {
        Api::of(&self.fresh.flow)
    }

    /// The device sessions alice's web session lists for the tablet.
    #[track_caller]
    pub fn tablet_sessions(&self) -> Vec<Listed> {
        let listed = self.api().list(&self.cookie);
        listed
            .into_iter()
            .filter(|session| {
                session.kind == Kind::Device && session.device_name.as_deref() == Some(TABLET)
            })
            .collect()
    }

    /// The account's sessions alice's web session lists.
    #[track_caller]
    pub fn sessions(&self) -> Vec<Listed> {
        self.api().list(&self.cookie)
    }
}
