//! Invite links created at the terminal and opened or managed in Chrome against the real API.

#![allow(dead_code)]

#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../web_service_tokens/stage.rs"]
mod harness;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../service_agents/kit.rs"]
mod kit;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../roster_revisions/world.rs"]
mod world;

use access::{open_access, press, revoke, rows_holding};
use browser::Page;
use harness::Harness;
use jiff::{Timestamp, tz::TimeZone};
use kit::sign_up_on;
use skald_crypto::InviteLink;
use spa::{Spa, wait_until};
use terminal::Terminal;
use world::{ACME, Person, create, create_with_member, now, words};

const DEVICE: &str = "ada-invite-laptop";
const PHONE: u32 = 360;
const DIALOG: &str = "[role=dialog], [role=alertdialog]";
const LIFETIME_DAYS: u64 = 12;

fn create_invite(ada: &Person, spa: &Spa) -> String {
    let shown = Terminal::start(
        &ada.machine,
        &["invite", ACME, "--days", &LIFETIME_DAYS.to_string()],
    )
    .finish();
    assert!(shown.success, "skald invite failed: {}", shown.text);
    let prefix = spa.at("/invite/");
    let link = shown
        .text
        .split_whitespace()
        .find(|word| word.starts_with(&prefix))
        .expect("skald invite shows a link at the web address");
    assert!(
        InviteLink::parse(link).is_ok(),
        "the CLI gave an invalid invite link"
    );
    link.to_owned()
}

fn main_text(page: &Page<'_>) -> String {
    page.evaluate(r#"document.querySelector("main, [role=main]")?.innerText ?? """#)
        .as_str()
        .expect("the page's main text")
        .to_owned()
}

fn fits_phone(page: &Page<'_>) {
    assert_eq!(
        page.evaluate(
            "[window.innerWidth, document.documentElement.scrollWidth - document.documentElement.clientWidth]"
        ),
        serde_json::json!([PHONE, 0]),
        "the page scrolls sideways: {}",
        main_text(page)
    );
}

fn open_members(page: &Page<'_>, spa: &Spa, phone: bool) {
    page.goto(&spa.at("/members"));
    if phone {
        press(page, None, "button", "^menu$");
    }
    press(page, None, "button", "^space\\b");
    press(page, None, "[role=option]", "^acme\\b");
    if phone {
        press(page, None, "button", "^menu$");
    }
}

fn date_forms(seconds: u64) -> Vec<String> {
    let time =
        Timestamp::from_second(i64::try_from(seconds).expect("a Unix time")).expect("a timestamp");
    // Near midnight the date in the browser's time zone can differ from UTC.
    [-86_400, 0, 86_400]
        .into_iter()
        .flat_map(|offset| {
            let day = Timestamp::from_second(time.as_second() + offset)
                .expect("a timestamp")
                .to_zoned(TimeZone::UTC);
            [
                day.strftime("%Y-%m-%d").to_string(),
                day.strftime("%b %-d").to_string().to_lowercase(),
                day.strftime("%-d %b").to_string().to_lowercase(),
            ]
        })
        .collect()
}

fn assert_metadata(text: &str, created: u64) {
    let text = text.to_lowercase();
    assert!(words(&text).contains(&"member"), "role: {text}");
    assert!(
        text.contains("newcomer") || text.contains("waiting for acceptance"),
        "state: {text}"
    );
    let dates = date_forms(created);
    assert!(
        dates.iter().any(|date| text.contains(date))
            || text.contains("today")
            || text.contains("now"),
        "creation date: {text}"
    );
    let expires = date_forms(created + LIFETIME_DAYS * 86_400);
    assert!(
        expires.iter().any(|date| text.contains(date))
            || (LIFETIME_DAYS - 1..=LIFETIME_DAYS)
                .any(|days| text.contains(&format!("{days} days"))),
        "expiry: {text}"
    );
}

fn members_invite(page: &Page<'_>, username: &str) -> String {
    let invite_list = |page: &Page<'_>| {
        page.evaluate(
            r#"[...document.querySelectorAll("main section, main [role=region]")].find((section) =>
                [...section.querySelectorAll("h2, [role=heading]")].some((heading) =>
                    /^invite links$/i.test(heading.innerText.trim())))?.innerText ?? """#,
        )
        .as_str()
        .expect("the invite section's text")
        .to_owned()
    };
    wait_until(
        page,
        "the Members invite listing with its creator and device",
        |page| {
            let text = invite_list(page);
            text.contains(username) && text.contains(DEVICE)
        },
    );
    assert_eq!(page.count("main >> role=button[name=/^revoke\\b/i]"), 1);
    invite_list(page)
}

fn access_invite(page: &Page<'_>) -> String {
    wait_until(page, "the Access row for the invite to acme", |page| {
        rows_holding(page, ACME).len() == 1
    });
    let row = rows_holding(page, ACME).remove(0);
    assert!(row.to_lowercase().contains("invite link"), "{row}");
    row
}

#[test]
fn an_admin_lists_and_revokes_a_cli_invite_and_a_member_sees_none() {
    let Harness { world, spa } = Harness::start();
    let admin = world.browser.page();
    let member = world.browser.page();
    let ada = sign_up_on(&world, &admin, (3_819_811_001, "ada-web-invites"), DEVICE);
    let bob = sign_up_on(
        &world,
        &member,
        (3_819_811_002, "bob-web-invites"),
        "bob-invite-laptop",
    );
    create_with_member(&world.cloud, &ada, ACME, &bob);
    let created = now();
    let link = create_invite(&ada, &spa);
    let phone = world.browser.page_of_width(PHONE);
    phone.add_cookies(&admin.cookies());

    for (page, narrow) in [(&admin, false), (&phone, true)] {
        open_members(page, &spa, narrow);
        assert_metadata(&members_invite(page, ada.handle.as_str()), created);
        if narrow {
            fits_phone(page);
            page.goto(&spa.at("/access"));
        } else {
            open_access(page);
        }
        assert_metadata(&access_invite(page), created);
        if narrow {
            fits_phone(page);
        }
    }

    member.record_requests();
    open_members(&member, &spa, false);
    wait_until(&member, "Bob's Members list for acme", |page| {
        rows_holding(page, ada.handle.as_str()).len() == 1
            && rows_holding(page, bob.handle.as_str()).len() == 1
    });
    member.requests();
    assert!(!main_text(&member).to_lowercase().contains("invite links"));
    open_access(&member);
    member.requests();
    assert!(
        rows_holding(&member, ACME).is_empty(),
        "{}",
        main_text(&member)
    );

    revoke(&admin, ACME);
    wait_until(&admin, "the revoke confirmation", |page| {
        page.count(DIALOG) == 1
    });
    press(&admin, Some(DIALOG), "button", "revoke");
    wait_until(&admin, "the revoked row is gone", |page| {
        rows_holding(page, ACME).is_empty()
    });

    let mut cli = Terminal::start(&bob.machine, &["accept-invite"]);
    let asked = cli.wait_for("a prompt for the link", |shown| {
        shown.to_lowercase().contains("link")
    });
    cli.type_line(&link);
    let refused = cli.finish();
    let answer = refused
        .text
        .strip_prefix(&asked)
        .unwrap_or(&refused.text)
        .replace(&link, "");
    assert!(
        !refused.success && words(&answer.to_lowercase()).contains(&"ended"),
        "{answer}"
    );
}

#[test]
fn a_link_explains_acceptance_and_keeps_the_fragment_local_with_or_without_a_session() {
    let Harness { world, spa } = Harness::start();
    let signed_in = world.browser.page_of_width(PHONE);
    let ada = sign_up_on(
        &world,
        &signed_in,
        (3_819_812_001, "ada-opens-invite"),
        DEVICE,
    );
    create(&world.cloud, &ada, ACME);
    let link = create_invite(&ada, &spa);
    let (address, fragment) = link
        .split_once('#')
        .expect("an invite link with a fragment");
    let decoded = InviteLink::parse(&link).expect("a valid invite link");
    let secret = decoded.secret.to_bytes();
    let invite = decoded.invite.to_string();
    let signed_out = world.browser.page_of_width(PHONE);
    assert!(
        signed_in
            .cookies()
            .iter()
            .any(|cookie| cookie.name == "skald_session")
    );
    assert!(signed_out.cookies().is_empty());

    let mut pages = Vec::new();
    for (session, page) in [("signed in", &signed_in), ("signed out", &signed_out)] {
        page.record_requests();
        let visit = page.goto(&link);
        assert_eq!(visit.status, Some(200), "{session}: {:?}", visit.error);
        let requests = page.requests();
        assert!(
            requests
                .iter()
                .any(|request| request.method == "GET" && request.url == address),
            "{session}: invite navigation was not recorded"
        );
        for request in requests {
            for value in [request.url.as_str(), request.body.as_deref().unwrap_or("")]
                .into_iter()
                .chain(request.headers.keys().map(String::as_str))
                .chain(request.headers.values().map(String::as_str))
            {
                let bytes: Vec<u8> = value
                    .chars()
                    .filter_map(|ch| u8::try_from(ch as u32).ok())
                    .collect();
                assert!(
                    !value.contains(fragment) && !keys::holds(&bytes, secret.as_ref()),
                    "{session}: a recorded {} request carried the invite fragment or secret",
                    request.method
                );
            }
        }
        pages.push((session, page, main_text(page)));
    }

    for (session, page, text) in pages {
        for instruction in ["skald accept-invite", "paste", "terminal"] {
            assert!(
                text.to_lowercase().contains(instruction),
                "{session}: missing {instruction:?}: {text}"
            );
        }
        for value in [&invite, fragment, ACME, DEVICE] {
            assert!(
                !text.contains(value),
                "{session}: the page displays invite data"
            );
        }
        fits_phone(page);
    }
}
