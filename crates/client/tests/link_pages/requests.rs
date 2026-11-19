//! Who the page answers: only requests that carry its secret, with its own Host and its own Origin
//! or none, so that no other site, DNS rebinding or local process without the secret drives it;
//! every answer is not to be stored; it listens only on loopback, and only while its command runs.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use crate::at_terminal::as_agent;
use crate::linking::LINK;
use crate::pages::{Opener, PAGE, Reply, outward_address, request, start_with, wait_for_opening};
use crate::world::World;

/// A site that is not the page's, and a host name that a rebinding DNS server points at loopback.
const OTHER_SITE: &str = "http://attacker.example";
const REBOUND: &str = "attacker.example";

/// As an agent, `skald link --browser` on Ada's first machine hands the stand-in its file; the
/// page's URL names a loopback address. Taken, each not to be stored: a `GET` of the URL with no
/// Origin and with the page's own. Refused, or not answered, and if answered not to be stored: the
/// URL with another Origin, with `null` and with the page's host on another port as its Origin, a
/// `POST` of it with another Origin, the URL with a rebound host name as its Host, the URL without
/// its query, with its query changed, and the root. The page's port takes no connection on an
/// address of the machine that is not loopback. Once the command ends, the page answers nothing.
#[test]
fn the_page_answers_only_its_secret_with_its_own_host_and_origin() {
    let world = World::start();
    let ada = world.sign_up(4_179_401_001, "ada-guards-her-page", "ada-workstation");
    let opener = Opener::on(&ada.machine);
    let mut linking = start_with(&ada.machine, opener.program(), &as_agent(&[LINK, PAGE]));
    let opening = wait_for_opening(&opener, 0, &mut linking);
    assert!(opening.address.ip().is_loopback(), "{}", opening.url);

    let (address, host, target) = (opening.address, &opening.authority, opening.target());
    let own = opening.origin();
    let other_port = format!(
        "http://{}:{}",
        opening.address.ip(),
        opening.address.port().wrapping_add(1).max(1)
    );
    let mut changed = opening.query.clone();
    let last = changed.pop().expect("a query");
    changed.push(if last == 'a' { 'b' } else { 'a' });
    let changed = format!("{}?{changed}", opening.path);
    let rebound = format!("{REBOUND}:{}", opening.address.port());

    let refused: Vec<(&str, Option<Reply>)> = vec![
        (
            "another Origin",
            request(address, "GET", &target, host, Some(OTHER_SITE)),
        ),
        (
            "Origin null",
            request(address, "GET", &target, host, Some("null")),
        ),
        (
            "another port's Origin",
            request(address, "GET", &target, host, Some(&other_port)),
        ),
        (
            "a POST from another Origin",
            request(address, "POST", &target, host, Some(OTHER_SITE)),
        ),
        (
            "a rebound Host",
            request(address, "GET", &target, &rebound, None),
        ),
        (
            "no secret",
            request(address, "GET", &opening.path, host, None),
        ),
        (
            "a changed secret",
            request(address, "GET", &changed, host, None),
        ),
        ("the root", request(address, "GET", "/", host, None)),
    ];
    for (what, reply) in refused {
        if let Some(reply) = reply {
            assert!(!reply.taken(), "{what}: taken: {reply:?}");
            assert!(reply.not_stored(), "{what}: may be stored: {reply:?}");
        }
    }
    for (what, origin) in [("no Origin", None), ("its own Origin", Some(own.as_str()))] {
        let reply = request(address, "GET", &target, host, origin)
            .unwrap_or_else(|| panic!("{what}: no answer"));
        assert!(reply.taken(), "{what}: refused: {reply:?}");
        assert!(reply.not_stored(), "{what}: may be stored: {reply:?}");
    }

    let outward = SocketAddr::new(outward_address(), opening.address.port());
    assert!(
        TcpStream::connect_timeout(&outward, Duration::from_secs(5)).is_err(),
        "the page takes connections on {outward}"
    );

    linking.kill();
    let after = request(address, "GET", &target, host, None);
    assert!(
        after.as_ref().is_none_or(|reply| !reply.taken()),
        "the page answers once its command ended: {after:?}"
    );
}
