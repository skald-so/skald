//! `skald sign-in` with no browser to open: the fallback line alone, and the person approves the
//! code on another device.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::browser::APPROVE;
use crate::cloud::shown_grant;
use crate::machine::{Machine, SERVER, SIGN_IN};
use crate::pages::{BROWSER, failing_program, missing_program};
use crate::world::World;
use crate::{assert_only_the_line, complete_uri};

/// How long a test lets a run print more before it reads what the run printed: past two polls of
/// the token endpoint, every second.
const SETTLE: Duration = Duration::from_secs(2);

/// Signs in a new machine whose `BROWSER` names what `program` gives as GitHub user `id`, `login`,
/// who types the code on the verification page; gives what the run printed before the approval and
/// the link it was to print. Fails unless the machine signs in to that user's account.
fn signed_in_with(id: u64, login: &str, program: fn(&Machine) -> PathBuf) -> (String, String) {
    let world = World::start();
    let cloud = &world.cloud;
    let user = cloud.user(id, login);
    let machine = Machine::new().with_host_name("ada-server");
    let named = program(&machine);
    let machine = machine.with_env(BROWSER, named);

    let mut run = machine.start(&[SIGN_IN, SERVER, &cloud.url]);
    let grant = shown_grant(cloud, &mut run, 0);
    thread::sleep(SETTLE);
    let before = run.shown();
    let link = complete_uri(cloud);
    cloud.decide(&world.browser.page(), &user, &grant, APPROVE);
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    assert_eq!(cloud.login_of(&cloud.last_token()), user.login);
    (before, link)
}

/// With `BROWSER` naming no program, `skald sign-in` prints only the line with the link until the
/// code is approved, and the approval signs the machine in.
#[test]
fn without_a_browser_to_open_it_prints_only_the_line_and_an_approval_signs_it_in() {
    let (before, link) = signed_in_with(4_312_640_011, "ada-no-browser", missing_program);

    assert_only_the_line(&before, &link);
}

/// With `BROWSER` naming a program that opens nothing and exits failing, `skald sign-in` also prints
/// only the line with the link, the fallback, and the approval signs the machine in.
#[test]
fn a_browser_that_fails_to_open_leaves_only_the_line_and_an_approval_signs_it_in() {
    let (before, link) = signed_in_with(4_312_640_012, "ada-failing-browser", failing_program);

    assert_only_the_line(&before, &link);
}
