//! The local page of `skald sign-in --browser` asks nothing before the recovery key of the
//! account's first device or the link code of any other.

use std::fs;

use skald_github_stand_in::User;

use crate::at_terminal::{as_agent, keys_in};
use crate::linking::{codes_in, says};
use crate::machine::{Machine, Run, SERVER, SIGN_IN};
use crate::pages::{Opener, PAGE, Paged, WRITTEN_DOWN_BUTTON, approve_next_as, start_with};
use crate::world::World;
use crate::{asks_whether_first, opened_file, openings, with_opener};

/// Which of the opener's runs opened the local page: the first whose file takes the browser to a
/// loopback address. Waits for it while `run` goes on.
#[track_caller]
fn local_page(opener: &Opener, run: &mut Run) -> usize {
    let mut count = 1;
    loop {
        let opened = openings(opener, count, || {
            run.assert_running();
            run.shown()
        });
        let found = opened.iter().position(|args| {
            fs::read_to_string(opened_file(args)).is_ok_and(|html| {
                html.contains("http://127.0.0.1") || html.contains("http://localhost")
            })
        });
        if let Some(found) = found {
            return found;
        }
        count = opened.len() + 1;
    }
}

/// Starts an agent's `skald sign-in --browser` on a new machine of `user`, the stand-in opening
/// its pages, and approves its code; gives the machine, the run and the local page once opened.
fn signed_in_on_page<'b>(world: &'b World, user: &User, host: &str) -> (Machine, Run, Paged<'b>) {
    let cloud = &world.cloud;
    let (machine, opener) = with_opener(host);
    let before = cloud.grants().len();
    let args = as_agent(&[SIGN_IN, SERVER, &cloud.url, PAGE]);
    let mut run = start_with(&machine, opener.program(), &args);
    approve_next_as(world, user, before, &mut run);
    let index = local_page(&opener, &mut run);
    let paged = Paged::opened(&world.browser, &opener, index, &mut run);
    (machine, run, paged)
}

/// On a new account, the page of an agent's `skald sign-in --browser` shows the recovery key with
/// nothing pressed, asking nothing before it; once Ada presses the button under it, the command
/// exits with success and the account has its master key.
#[test]
fn the_page_shows_a_new_accounts_recovery_key_asking_nothing_first() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_312_640_041, "ada-page-unasked");

    let (_machine, mut run, paged) = signed_in_on_page(&world, &ada, "ada-workstation");
    let text = paged.wait_for(&mut run, "a recovery key", |text| !keys_in(text).is_empty());

    assert!(!asks_whether_first(&text), "the page asks: {text}");
    paged.press(WRITTEN_DOWN_BUTTON);
    let finished = run.finish();
    assert!(finished.success, "{finished:?}");
    assert!(cloud.master(&cloud.last_token()).is_some(), "no master key");
}

/// On Ada's account with a master key, the page of an agent's `skald sign-in --browser` on a new
/// machine shows a link code and where it goes with nothing pressed, asking nothing before it.
#[test]
fn the_page_shows_a_link_code_on_an_account_with_a_master_key_asking_nothing_first() {
    let world = World::start();
    let ada = world.sign_up(4_312_640_042, "ada-page-code-unasked", "ada-desktop");

    let (_machine, mut run, paged) = signed_in_on_page(&world, &ada.user, "ada-laptop");
    let text = paged.wait_for(&mut run, "a link code and where it goes", |text| {
        !codes_in(text).is_empty() && says(text, &["skald link"])
    });

    assert!(!asks_whether_first(&text), "the page asks: {text}");
    run.kill();
}
