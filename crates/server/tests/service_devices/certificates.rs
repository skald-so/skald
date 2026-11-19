//! The certificate of a `service` device is stored only after checks 1 and 2 against the account's
//! master key and the certificates its devices hold, and only if it matches the registration.
//! Refused, nothing is stored.

use skald_proto::{CertBody, DeviceKind};

use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::common::devices::{Device, Key, cert_of, write_devices};
use crate::{Owner, assert_cert_refused, body_by};

/// Alice, Bob and Alice's registered `service` device, which has no certificate yet.
struct Stage<'f> {
    flow: &'f DeviceFlow,
    alice: Owner<'f>,
    bob: Owner<'f>,
    bot: Device,
}

impl<'f> Stage<'f> {
    fn new(flow: &'f DeviceFlow, github: u64) -> Self {
        let alice = Owner::new(flow, github, "alice-certs-standin");
        let bob = Owner::new(flow, github + 1, "bob-certs-standin");
        let bot = alice.register_service("Deploy bot");
        Stage {
            flow,
            alice,
            bob,
            bot,
        }
    }

    /// Uploads `cert` for the bot from Alice's session and checks that it is refused and nothing
    /// stored; `what` names the case.
    #[track_caller]
    fn assert_refused(&self, cert: &[u8], what: &str) {
        let response = self.alice.upload(self.bot.id, cert);
        assert_cert_refused(&self.alice, self.bot.id, &response, what);
    }

    /// Then the bot's genuine certificate, by Alice's laptop, is still taken.
    #[track_caller]
    fn assert_genuine_taken(&self) {
        let cert = self.alice.service_cert(&self.bot, &self.alice.laptop);
        let response = self.alice.upload(self.bot.id, &cert);
        assert_eq!(
            response.status,
            201,
            "the genuine certificate: {}",
            text(&response)
        );
    }
}

/// A certificate of Alice's `service` device issued by Bob's laptop, a device of another account,
/// is refused, as is one that names Bob's account; neither is stored.
#[test]
fn certificate_from_a_device_of_another_account_is_refused() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let s = Stage::new(&flow, 3_135_302_001);
    let (alice, bob, bot) = (&s.alice, &s.bob, &s.bot);

    let by_bob = body_by(bot, alice.account(), DeviceKind::Service, bob.laptop.id);
    let by_bob = cert_of(&by_bob, &bob.laptop.signing.key, &bot.signing.key);
    s.assert_refused(&by_bob, "issued by another account's device");
    let on_bob = bob.service_cert(bot, &bob.laptop);
    s.assert_refused(&on_bob, "on another account, issued by its device");
    s.assert_genuine_taken();
}

/// Certificates that fail check 1 are refused: one whose signature is not the issuer's, by Alice's
/// laptop as `issuer` but signed with another key, and one whose proof is not by the device's
/// signing key.
#[test]
fn certificate_that_fails_check_1_is_refused() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let s = Stage::new(&flow, 3_135_302_101);
    let (alice, bot) = (&s.alice, &s.bot);
    let body = body_by(bot, alice.account(), DeviceKind::Service, alice.laptop.id);

    let signed_otherwise = cert_of(&body, &Key::generate().key, &bot.signing.key);
    s.assert_refused(&signed_otherwise, "a signature not by the issuer");
    let proved_otherwise = cert_of(&body, &alice.laptop.signing.key, &Key::generate().key);
    s.assert_refused(&proved_otherwise, "a proof not by the device's key");
    s.assert_genuine_taken();
}

/// Certificates that fail check 2 are refused: issued by Alice's other `service` device, which is
/// no `client` device; by her desktop, which has no certificate; by her device certified by another
/// master key; by a device the server does not know; and by the master key itself, which certifies
/// only `client` devices.
#[test]
fn certificate_that_fails_check_2_is_refused() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let s = Stage::new(&flow, 3_135_302_201);
    let (alice, bot) = (&s.alice, &s.bot);
    let account = alice.account();
    let issued_by = |issuer: &Device| {
        let body = body_by(bot, account, DeviceKind::Service, issuer.id);
        cert_of(&body, &issuer.signing.key, &bot.signing.key)
    };

    let other_bot = alice.written_service(s.flow, "Other bot");
    s.assert_refused(&issued_by(&other_bot), "issued by a service device");

    let desktop = Device::new("Desktop");
    alice.machine.register_device(&desktop);
    s.assert_refused(
        &issued_by(&desktop),
        "issued by a device without a certificate",
    );

    let foreign = Device::new("Foreign");
    let foreign_cert = foreign.cert_as(account, DeviceKind::Client, &Key::generate());
    write_devices(
        &s.flow.stand.db,
        account,
        DeviceKind::Client,
        &[(&foreign, Some(&foreign_cert))],
    );
    s.assert_refused(
        &issued_by(&foreign),
        "issued by a device of another master key",
    );

    let unknown = Device::new("Unknown");
    s.assert_refused(&issued_by(&unknown), "issued by an unknown device");

    let by_master = bot.cert_as(account, DeviceKind::Service, &alice.master);
    s.assert_refused(
        &by_master,
        "a service device's certificate by the master key",
    );
    s.assert_genuine_taken();
}

/// A certificate that does not match the `service` registration is refused: one of a `client`
/// device, one with another `name`, and one with another HPKE key.
#[test]
fn certificate_unlike_the_registration_is_refused() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let s = Stage::new(&flow, 3_135_302_301);
    let (alice, bot) = (&s.alice, &s.bot);
    let laptop = &alice.laptop;
    let sign = |change: &dyn Fn(&mut CertBody)| {
        let mut body = body_by(bot, alice.account(), DeviceKind::Service, laptop.id);
        change(&mut body);
        cert_of(&body, &laptop.signing.key, &bot.signing.key)
    };

    s.assert_refused(
        &sign(&|body| body.kind = DeviceKind::Client),
        "of a client device",
    );
    s.assert_refused(
        &sign(&|body| body.name = "Other bot".to_owned()),
        "another name",
    );
    s.assert_refused(&sign(&|body| body.hpke = [0x42; 32]), "another HPKE key");
    s.assert_genuine_taken();
}
