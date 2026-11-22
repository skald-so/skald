//! The signature goes to the local state with the certificate before it is sent: a run killed
//! after that leaves the next run the same certificate to send.

use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_proto::Cbor as _;

use crate::at_terminal::first_at_terminal;
use crate::cloud::{GIVE_MASTER, WAIT};
use crate::machine::{SERVER, SIGN_IN};
use crate::relay::Lose;
use crate::{ADA_GITHUB, ADA_LOGIN, Kind, Setup, assert_signed_by};

/// How many times `needle` is in `haystack`.
fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// A listed ECDSA key in Ada's SSH directory, whose signatures differ each time it signs. The relay
/// withholds the server's answer to her first machine's master key and certificate, and the run at
/// her terminal is killed. With the key's files gone, the next run sends the certificate again, the same bytes,
/// which carry the signature by that key, and the server holds that one certificate.
#[test]
fn killed_after_writing_its_certificate_the_next_run_sends_the_same_one() {
    let setup = Setup::with_key(true, ("id_ecdsa", Kind::Ecdsa), "");
    setup.list_key();
    let (world, machine, key) = (&setup.world, &setup.machine, &setup.key);
    let cloud = &world.cloud;
    let user = cloud.user(ADA_GITHUB, ADA_LOGIN);
    let since = cloud.relay.mark();
    cloud.relay.lose_answer(GIVE_MASTER, Lose::Withhold);

    let run = first_at_terminal(cloud, &world.browser.page(), machine, &user, &[]);
    let deadline = Instant::now() + WAIT;
    while cloud.relay.lost() == 0 {
        assert!(
            Instant::now() < deadline,
            "no {GIVE_MASTER}: {}",
            run.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
    run.kill();
    assert_eq!(
        cloud.relay.lost(),
        1,
        "the server never answered {GIVE_MASTER}"
    );
    let token = cloud.last_token();
    let devices = cloud.devices(&token);
    assert_eq!(devices.len(), 1, "the account's devices: {devices:?}");
    let taken = devices[0]
        .cert
        .clone()
        .expect("the certificate the server took");
    assert_signed_by(&taken, ADA_GITHUB, key, &[]);
    key.remove_files();

    let next = machine
        .start(&[SIGN_IN, SERVER, cloud.url.as_str()])
        .finish();

    assert!(next.success, "the next run failed: {next:?}");
    let sent = cloud.count_sent(since, GIVE_MASTER);
    assert!(sent >= 2, "{GIVE_MASTER} sent {sent} times: {next:?}");
    let encoded = URL_SAFE_NO_PAD.encode(taken.to_cbor());
    let carried = count(&cloud.relay.sent_bytes(), encoded.as_bytes());
    assert!(
        carried >= 2,
        "the certificate taken was sent {carried} times: {next:?}"
    );
    assert_eq!(cloud.cert(&token, devices[0].id), Some(taken));
}
