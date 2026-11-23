//! Deleting an account on the web app's Account page with `skald` on the machines: the page shows
//! the spaces a deletion deletes and those that block it, and deletes the account only after a
//! recent sign-in, the typed username and while no space blocks it. Against a `skald-server` with
//! GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind the recording relay and the
//! front of the web app's harness (`server/tests/common/spa.rs`), on which the machines reach it
//! too; a test plays each person in headless Chrome. The fixture revision that adds a second
//! account is signed with the key of the admin's device, read from its secrets file, and stored
//! through the API, as in the client's tests of roster revisions; the admin's machine hands over
//! its rights with `skald hand-over-admin <space> <handle>`, as in the tests of admin rights.
//!
//! The web app, as these tests use it, beside `common/spa.rs` and `web_members/main.rs`:
//! - the account menu, the footer's button "Account", has the item "Account", which opens the
//!   Account page; it names each space that blocks the deletion with the command that hands over
//!   admin rights, `skald hand-over-admin <space> …`. While one blocks, the page has no button
//!   whose name starts with "Delete"; otherwise it has one;
//! - Delete opens a dialog, role `dialog` or `alertdialog`, that says what goes at once and what 30
//!   days later and holds a textbox for the username and a button whose name starts with "Delete",
//!   which deletes the account once the textbox holds its username;
//! - after the deletion, the web app shows the sign-in page, which says the account was "deleted";
//! - with a sign-in older than `--link-sign-in-age`, the deletion deletes nothing and the page asks
//!   to sign in again with a link or button whose text says "sign in";
//! - in Members, the row of a member the server reports deleted says "deleted".
//!
//! `sessions.created_at` in the database is a session's sign-in.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The people, their machines, keys and spaces, and the revisions they sign, of the tests of
// roster revisions.
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../roster_revisions/world.rs"]
mod world;
use world::{ACME, ADMIN, Person, World, create_with_member, run};

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
use cloud::Cloud;
use machine::DEVICES;

// The web app's harness, the person's browser and the rows of the web app's tables.
#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;
use access::{press, rows_holding};
use browser::{Browser, Page};
use spa::{
    Spa, answers_without_secrets, space_options, wait_for_cabinet, wait_for_sign_in_page,
    wait_until,
};

/// A host of the server's public address that the machines resolve, as Chrome does.
const HOST: &str = "localhost";
/// How long a machine's code works, in seconds.
const CODE_LIFETIME: u64 = 300;
/// The command that hands this account's admin rights in a space to a member.
const HAND_OVER_ADMIN: &str = "hand-over-admin";
/// The dialogs a confirmation opens, and the textbox in one.
const DIALOG: &str = "[role=dialog], [role=alertdialog]";
const DIALOG_FIELD: &str = "[role=dialog] input, [role=alertdialog] input";
/// What marks a deleted account on the pages.
const DELETED: &str = "deleted";

/// A script that counts the buttons outside a dialog whose name starts with "Delete".
const DELETES: &str = r#"[...document.querySelectorAll("button")]
    .filter((button) => !button.closest("[role=dialog], [role=alertdialog]"))
    .filter((button) =>
        /^delete\b/i.test((button.getAttribute("aria-label") ?? button.textContent).trim()))
    .length"#;

/// A script that counts the links and buttons whose text says "sign in".
const SIGN_IN_PROMPTS: &str = r#"[...document.querySelectorAll("a, button")]
    .filter((element) => /sign in/i.test(element.textContent)).length"#;

/// How many buttons outside a dialog whose name starts with "Delete" `page` shows.
fn delete_count(page: &Page) -> usize {
    page.evaluate(DELETES).as_u64().expect("a count") as usize
}

/// How many links and buttons whose text says "sign in" `page` shows.
fn sign_in_prompts(page: &Page) -> usize {
    page.evaluate(SIGN_IN_PROMPTS).as_u64().expect("a count") as usize
}

/// Whether `page` shows the command that hands over admin rights in `space`.
fn shows_hand_over(page: &Page, space: &str) -> bool {
    let text = page.text();
    let command = format!("skald {HAND_OVER_ADMIN} ");
    text.match_indices(&command).any(|(at, _)| {
        text[at + command.len()..]
            .trim_start_matches(['"', '\''])
            .starts_with(space)
    })
}

/// Opens the cabinet of `username` in `page` afresh, then the Account page from the account menu.
fn open_account(page: &Page, spa: &Spa, username: &str) {
    page.goto(&spa.at("/"));
    wait_for_cabinet(page, username);
    press(page, None, "button", "^account$");
    press(page, None, "[role=menuitem]", "^account$");
}

/// Clicks Delete on the Account page in `page`, then, in the dialog it opens, types `typed` and
/// clicks the dialog's Delete; gives what the dialog said.
fn delete_typing(page: &Page, typed: &str) -> String {
    press(page, None, "button", r"^delete\b");
    wait_until(page, "the dialog of Delete", |page| page.count(DIALOG) == 1);
    let said = page.evaluate(&format!("document.querySelector({DIALOG:?}).innerText"));
    page.fill(DIALOG_FIELD, typed);
    press(page, Some(DIALOG), "button", r"^delete\b");
    said.as_str().unwrap_or_default().to_owned()
}

/// Opens Members in the cabinet of `username` in `page` and chooses the space `name`.
fn open_members(page: &Page, spa: &Spa, username: &str, name: &str) {
    page.goto(&spa.at("/"));
    wait_for_cabinet(page, username);
    press(page, None, "a, [role=link]", "^members\\b");
    let listed = space_options(page).unwrap_or_default();
    let offered = listed.iter().filter(|option| option.contains(name)).count();
    assert_eq!(offered, 1, "{name} once in the space switcher: {listed:?}");
    press(page, None, "[role=option]", &format!("\\b{name}\\b"));
}

/// Moves back by an hour the sign-in of every session of `person`'s account, beyond the server's
/// `--link-sign-in-age`.
fn move_sign_ins_back(cloud: &Cloud, person: &Person) {
    cloud.execute(&format!(
        "UPDATE sessions SET created_at = created_at - interval '1 hour'
         WHERE account_id = '{}'",
        person.account
    ));
}

/// The server, with the front of the web app before it, and its browser.
struct Stage {
    browser: Browser,
    spa: Spa,
    world: World,
}

impl Stage {
    fn start() -> Self {
        let people = Browser::launch();
        let front = Spa::start(&people, HOST);
        let cloud = Cloud::fronted(HOST, front.port, CODE_LIFETIME, 1);
        front.serve(&people, &cloud.relay.url);
        // The world's own browser approves no code here: each person approves in their page.
        let approvals = Browser::launch();
        Stage {
            world: World {
                cloud,
                browser: approvals,
            },
            spa: front,
            browser: people,
        }
    }

    /// A new account of GitHub user `id` with `login`, whose first machine, on the host `host`,
    /// signs in with `skald`, approved in `page`, which then holds the account's web session.
    fn sign_up(&self, page: &Page, id: u64, login: &str, host: &'static str) -> Person {
        let cloud = &self.world.cloud;
        let user = cloud.user(id, login);
        let (machine, signed_in) = cloud.first_machine(page, &user, host);
        assert!(signed_in.success, "{signed_in:?}");
        self.world.person(user, machine, host)
    }
}

/// Ida signs up with her laptop, approved in the browser she then opens the web app in, and Bob
/// with his, in a browser of his own. Ida's laptop creates `acme`, and a revision it signs adds Bob
/// as a member. Ida's Account page names `acme` with the command that hands over her rights and
/// offers no Delete, and her account stays. Her laptop then hands her rights to Bob with `skald
/// hand-over-admin`: the page names the command no more and offers Delete, whose dialog says what
/// goes at once and what 30 days later. With her username typed, the account is deleted: the
/// browser gets the sign-in page, which says so; the server refuses the laptop's session, and its
/// next command, `skald devices`, fails. In Bob's browser, Members of `acme` marks Ida deleted,
/// and not Bob, its admin. No answer either browser got holds a secret.
#[test]
fn the_only_admin_deletes_nothing_until_handing_over_then_deletes_the_account() {
    let stage = Stage::start();
    let (spa, cloud) = (&stage.spa, &stage.world.cloud);
    let ida_page = stage.browser.page();
    ida_page.record_answers();
    let bob_page = stage.browser.page();
    bob_page.record_answers();
    let ida = stage.sign_up(
        &ida_page,
        4_307_100_001,
        "Ida-Deletes-Standin",
        "ida-laptop",
    );
    let bob = stage.sign_up(&bob_page, 4_307_100_002, "Bob-Stays-Standin", "bob-laptop");
    let (ida_name, bob_name) = (ida.handle.to_string(), bob.handle.to_string());
    create_with_member(cloud, &ida, ACME, &bob);

    open_account(&ida_page, spa, &ida_name);
    wait_until(&ida_page, "acme blocking, with the command", |page| {
        shows_hand_over(page, ACME)
    });
    assert_eq!(delete_count(&ida_page), 0, "a Delete while acme blocks");
    let (status, body) = cloud.session_of(&ida.token);
    assert_eq!(status, 200, "Ida's account after the blocked page: {body}");

    let handed = run(&ida.machine, &[HAND_OVER_ADMIN, ACME, &bob_name]);
    assert!(handed.success, "{handed:?}");
    open_account(&ida_page, spa, &ida_name);
    wait_until(&ida_page, "Delete, once nothing blocks", |page| {
        delete_count(page) == 1
    });
    assert!(!shows_hand_over(&ida_page, ACME), "{:?}", ida_page.text());

    let said = delete_typing(&ida_page, &ida_name);
    let said = said.to_lowercase();
    assert!(said.contains("30 days"), "the dialog: {said:?}");
    wait_for_sign_in_page(&ida_page);
    wait_until(
        &ida_page,
        "the sign-in page saying the account was deleted",
        |page| page.text().to_lowercase().contains(DELETED),
    );
    let (status, body) = cloud.session_of(&ida.token);
    assert_eq!(
        status, 401,
        "the laptop's session after the deletion: {body}"
    );
    let listed = run(&ida.machine, &[DEVICES]);
    assert!(!listed.success, "{listed:?}");

    open_members(&bob_page, spa, &bob_name, ACME);
    wait_until(&bob_page, "Ida marked deleted in acme's Members", |page| {
        let rows = rows_holding(page, &ida_name);
        rows.len() == 1 && rows[0].to_lowercase().contains(DELETED)
    });
    let bob_rows = rows_holding(&bob_page, &bob_name);
    assert_eq!(bob_rows.len(), 1, "Bob's row: {bob_rows:?}");
    let bob_row = bob_rows[0].to_lowercase();
    assert!(
        bob_row.contains(ADMIN) && !bob_row.contains(DELETED),
        "{bob_row:?}"
    );

    answers_without_secrets(&ida_page);
    answers_without_secrets(&bob_page);
}

/// Ida, in her personal space alone, has sign-ins older than the server's setting: Delete, with
/// her username typed, asks her to sign in again and deletes nothing; she stays in the cabinet, and
/// her account, its laptop's session and personal space stay.
#[test]
fn with_an_older_sign_in_delete_asks_to_sign_in_again_and_deletes_nothing() {
    let stage = Stage::start();
    let (spa, cloud) = (&stage.spa, &stage.world.cloud);
    let page = stage.browser.page();
    page.record_answers();
    let ida = stage.sign_up(&page, 4_307_100_101, "Ida-Older-Standin", "ida-laptop");
    let name = ida.handle.to_string();
    move_sign_ins_back(cloud, &ida);

    open_account(&page, spa, &name);
    wait_until(&page, "Delete on the Account page", |page| {
        delete_count(page) == 1
    });
    let asking = sign_in_prompts(&page);
    delete_typing(&page, &name);
    wait_until(&page, "a prompt to sign in again", |page| {
        sign_in_prompts(page) > asking
    });

    assert_eq!(page.count(spa::ACCOUNT_MENU), 1, "still in the cabinet");
    let (status, body) = cloud.session_of(&ida.token);
    assert_eq!(status, 200, "the laptop's session: {body}");
    let spaces = cloud.spaces(&ida.token);
    assert!(
        spaces.iter().any(|(_, listed)| *listed == name),
        "the personal space: {spaces:?}"
    );
    answers_without_secrets(&page);
}
