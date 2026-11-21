//! With `--browser`, the local page of `skald sign-in` on a machine without a device shows both
//! ways, the browser sign-in's link and a link code, and the code links the machine as at a
//! terminal; the command's output holds no code, as its person's run or an agent's.

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::at_terminal::as_agent;
use crate::linking::{OWN_DEVICE, YES, link_with, typed};
use crate::machine::{Finished, Machine, RUN_LIMIT, Run, SERVER, SIGN_IN};
use crate::pages::{Opener, PAGE, Paged, confirm_on_page, link_on_page, start_with};
use crate::world::{Person, World};
use crate::{
    assert_key_session, assert_no_code, certified_named, complete_uri, issuer_of, key_sessions,
    link_codes_in, named_file, until_no_run_open,
};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// Whether `text`, an opened file, takes the browser to a page on a loopback address.
fn local(text: &str) -> bool {
    ["http://127.", "http://localhost", "http://[::1]"]
        .iter()
        .any(|start| text.contains(start))
}

/// Waits until the opener has been given the command's own page, for up to [`RUN_LIMIT`], while
/// `run` goes on; gives how many runs of the opener came before it.
#[track_caller]
pub fn local_page_at(opener: &Opener, run: &mut Run) -> usize {
    let deadline = Instant::now() + RUN_LIMIT;
    loop {
        let found = (opener.openings().iter()).position(|args| {
            args.first()
                .and_then(|arg| fs::read_to_string(named_file(arg)).ok())
                .is_some_and(|text| local(&text))
        });
        if let Some(at) = found {
            return at;
        }
        assert!(
            !run.exited(),
            "skald exited opening no page: {}",
            run.shown()
        );
        assert!(
            Instant::now() < deadline,
            "skald opened no page: {}",
            run.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Whether the page `paged` shows links to `link`, as its text or a link's target.
fn links_to(paged: &Paged, link: &str) -> bool {
    let script = format!(
        "[...document.querySelectorAll('a[href]')].some((a) => a.href.includes({0})) \
         || document.body.innerText.includes({0})",
        Value::from(link)
    );
    paged.page.evaluate(&script) == Value::Bool(true)
}

/// A new machine `host` of Ada's account runs `skald <before> sign-in --server <cloud> --browser`
/// with its input empty and its outputs pipes; gives the run, the opener, its page once the page
/// shows a link code and links to the device flow's verification page, and the code.
#[track_caller]
pub fn on_its_page<'b>(
    world: &'b World,
    machine: &Machine,
    before: &[&str],
) -> (Run, Opener, Paged<'b>, String) {
    let cloud = &world.cloud;
    let opener = Opener::on(machine);
    let grants = cloud.grants().len();
    let args = [before, &[SIGN_IN, SERVER, &cloud.url, PAGE]].concat();
    let mut run = start_with(machine, opener.program(), &args);
    let at = local_page_at(&opener, &mut run);
    let paged = Paged::opened(&world.browser, &opener, at, &mut run);
    cloud.next_grant(grants, || run.shown());
    let link = complete_uri(cloud, grants);
    let text = paged.wait_for(&mut run, "both ways", |text| {
        !link_codes_in(cloud, text).is_empty() && links_to(&paged, &link)
    });
    let code = link_codes_in(cloud, &text).pop().expect("a link code");
    (run, opener, paged, code)
}

/// Fails unless the new machine `host`, whose run printed `finished`, exited with success, holds a
/// certificate `ada`'s first device issued and a device session of her account by its key, which
/// no device flow gave, and its output holds no form of `code`.
#[track_caller]
fn assert_linked_by_code(
    world: &World,
    ada: &Person,
    finished: &Finished,
    code: &str,
    since: Instant,
) {
    let cloud = &world.cloud;
    assert!(finished.success, "{finished:?}");
    assert_no_code("skald sign-in", &finished.shown(), code);
    let device = certified_named(cloud, &ada.token, LAPTOP);
    assert_eq!(issuer_of(cloud, &ada.token, device), Some(ada.device));
    let token = key_sessions(cloud, since)
        .pop()
        .expect("a device session by the key");
    assert_key_session(cloud, &token, ada.account);
    until_no_run_open(cloud);
}

/// Ada's new machine runs `skald sign-in --browser`. Its page shows a link code and links to the
/// device flow's verification page; nobody approves that. Her first machine's `skald link` at a
/// terminal takes the code from the page: the command exits with success, its output holding no
/// code, and the new machine holds its certificate and a device session of her account.
#[test]
fn the_page_shows_both_ways_and_its_code_links_the_machine() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_301_001, "ada-links-a-page-by-code", "ada-workstation");
    let machine = Machine::new().with_host_name(LAPTOP);
    let since = cloud.relay.mark();
    let tokens = cloud.tokens().len();
    let (run, _opener, _paged, code) = on_its_page(&world, &machine, &[]);

    let linked = link_with(&ada, &typed(&code), LAPTOP, YES);
    assert!(linked.success, "skald link failed: {}", linked.text);
    let finished = run.finish();

    assert_eq!(
        cloud.tokens().len(),
        tokens,
        "the device flow gave a session"
    );
    assert_linked_by_code(&world, &ada, &finished, &code, since);
}

/// As an agent, Ada's new machine runs `skald sign-in --browser`: its page shows a link code and
/// links to the verification page. As an agent, her first machine runs `skald link --browser`, and
/// she enters the code and confirms the new device on its page. Both commands exit with success,
/// neither output holding the code, and the new machine holds its certificate and a device session
/// of her account.
#[test]
fn as_an_agent_the_code_shows_only_on_the_page_and_links_the_machine() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_302_001, "ada-agent-links-a-page", "ada-workstation");
    let machine = Machine::new().with_host_name(LAPTOP);
    let since = cloud.relay.mark();
    let tokens = cloud.tokens().len();
    let (run, _opener, _paged, code) = on_its_page(&world, &machine, &as_agent(&[]));

    let at_first = Opener::on(&ada.machine);
    let (linking, page) = link_on_page(&world, &ada, &at_first, OWN_DEVICE);
    let linked = confirm_on_page(linking, &page, &typed(&code), LAPTOP);
    assert!(linked.success, "skald link: {linked:?}");
    assert_no_code("skald link", &linked.shown(), &code);
    let finished = run.finish();

    assert_eq!(
        cloud.tokens().len(),
        tokens,
        "the device flow gave a session"
    );
    assert_linked_by_code(&world, &ada, &finished, &code, since);
}
