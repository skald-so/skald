//! The web app's Usage section with `skald` on the machine: the chosen space's limits and usage and
//! the spaces the account owns, as the server counts them, with the cloud's settings and without
//! them. Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL,
//! behind the recording relay and the front of the web app's harness (`server/tests/common/spa.rs`),
//! on which the machine reaches it too; a test plays the person in headless Chrome.
//!
//! The web app, as these tests use it, beside `common/spa.rs`: the side menu's link "Usage" opens
//! the section for the space the switcher chose. Each count with a limit has a bar, an element of
//! role `progressbar` or `meter`, whose accessible name starts with what it counts, "Spaces",
//! "Devices" or "Messages this month", and whose value is the count and maximum the limit; the
//! section lists the spaces the account owns by name, and says the largest message, as "64 KB",
//! and how long an undelivered message is kept, as "7 days". A count without a limit has no bar,
//! and the section says "no limit" or "unlimited". The section is the page's main area, `main`.
//! It gets them from `GET /account/usage` and `GET /spaces/{space}/usage`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The person's browser and the web app's harness, and the press of a button or link.
#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;

// The server, the relay and the machine of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

use std::collections::BTreeSet;
use std::time::Duration;

use access::press;
use browser::{Browser, Page};
use cloud::Cloud;
use serde::Deserialize;
use spa::{Spa, answers_without_secrets, wait_for_cabinet, wait_until};

/// A host of the server's public address that the machine resolves, as Chrome does.
const HOST: &str = "localhost";
/// How long a machine's code works, in seconds.
const CODE_LIFETIME: u64 = 300;

/// The cloud's values (`docs/dev/architecture.md`, "Cloud limits"): 2 owned spaces, 50 devices,
/// about 10M messages a month, a message text of 64 KB and 7 days for an undelivered message.
const OWNED_2: u32 = 2;
const DEVICES_50: u32 = 50;
const MESSAGES_10M: u64 = 10_000_000;
const KB_64: u64 = 64 * 1024;
const DAYS_7: Duration = Duration::from_secs(7 * 86_400);

/// What the section's bars count, as their names start, in lower case.
const SPACES: &str = "spaces";
const DEVICES: &str = "devices";
const MESSAGES: &str = "messages this month";

/// A bar of the page: its accessible name, in lower case, its value and its maximum.
#[derive(Debug, Deserialize, PartialEq)]
struct Bar {
    name: String,
    now: f64,
    max: f64,
}

/// The bars of `page`.
fn bars(page: &Page) -> Vec<Bar> {
    let found = page.evaluate(
        r#"[...document.querySelectorAll("[role=progressbar], progress, [role=meter], meter")]
            .map((bar) => {
                const labelled = (bar.getAttribute("aria-labelledby") ?? "")
                    .split(/\s+/)
                    .filter(Boolean)
                    .map((id) => document.getElementById(id)?.textContent ?? "")
                    .join(" ");
                const labels = [...(bar.labels ?? [])].map((label) => label.textContent).join(" ");
                const name = bar.getAttribute("aria-label") ?? (labelled || labels);
                const number = (aria, plain) =>
                    Number(bar.getAttribute(aria) ?? bar.getAttribute(plain) ?? NaN);
                return {
                    name: name.trim().toLowerCase(),
                    now: number("aria-valuenow", "value"),
                    max: number("aria-valuemax", "max"),
                };
            })"#,
    );
    serde_json::from_value(found).expect("the bars")
}

/// The value and maximum of the one bar of `page` whose name starts with `counted`; `None` unless
/// exactly one does.
fn bar_of(page: &Page, counted: &str) -> Option<(f64, f64)> {
    let mut found: Vec<Bar> = bars(page);
    found.retain(|bar| bar.name.starts_with(counted));
    match found.as_slice() {
        [bar] => Some((bar.now, bar.max)),
        _ => None,
    }
}

/// The text of `page`'s main area, as a person sees it; empty without one.
fn main_text(page: &Page) -> String {
    let text = page.evaluate(r#"document.querySelector("main, [role=main]")?.innerText ?? """#);
    serde_json::from_value(text).expect("a text")
}

/// Chooses the space `name` in the switcher of the cabinet in `page`.
fn choose(page: &Page, name: &str) {
    press(page, None, "button", "^space\\b");
    press(page, None, "[role=option]", &format!("\\b{name}\\b"));
}

/// Ida's laptop signs in with `skald` as her account's first device on a server with the settings
/// `change` gives, approved in the browser she then opens the web app in, and signs her personal
/// space. She opens Usage and chooses that space; gives the page, the web app's harness, the
/// server and the space's name.
fn usage_of_first_machine<'b>(
    browser: &'b Browser,
    github: u64,
    change: impl FnOnce(&mut skald_server::Settings),
) -> (Page<'b>, Spa, Cloud, String) {
    let page = browser.page();
    page.record_answers();
    let spa = Spa::start(browser, HOST);
    let cloud = Cloud::fronted_with(HOST, spa.port, CODE_LIFETIME, 1, change);
    spa.serve(browser, &cloud.relay.url);
    let user = cloud.user(github, "Ida-Usage-Standin");
    let (_laptop, signed_in) = cloud.first_machine(&page, &user, "ida-laptop");
    assert!(signed_in.success, "{signed_in:?}");
    let spaces = cloud.spaces(&cloud.last_token());
    let [(_, personal)] = spaces.as_slice() else {
        panic!("the personal space alone: {spaces:?}");
    };
    let personal = personal.clone();

    page.goto(&spa.at("/"));
    wait_for_cabinet(&page, "ida-usage-standin");
    press(&page, None, "a, [role=link]", "^usage\\b");
    choose(&page, &personal);
    (page, spa, cloud, personal)
}

/// The paths of the usage requests of `cloud`'s only space: the account's and the space's.
fn usage_paths(cloud: &Cloud) -> [String; 2] {
    let (space, _) = cloud.spaces(&cloud.last_token()).remove(0);
    [
        "/account/usage".to_owned(),
        format!("/spaces/{space}/usage"),
    ]
}

/// Whether the web app got each of `paths` answered to `GET` with `200`, checking that no answer
/// it got holds a secret.
fn answered(page: &Page, paths: &[String]) -> bool {
    let answers = answers_without_secrets(page);
    let got: BTreeSet<&str> = answers
        .iter()
        .filter(|answer| answer.method == "GET" && answer.status == 200)
        .filter_map(|answer| answer.url.split('?').next())
        .collect();
    paths
        .iter()
        .all(|path| got.iter().any(|url| url.ends_with(path.as_str())))
}

/// With the cloud's settings, Ida's new account with one machine sees her personal space's usage:
/// she owns 1 of 2 spaces, her personal space listed; her laptop is 1 of 50 devices; the space has
/// sent 0 of 10,000,000 messages this month; a message holds up to 64 KB, and an undelivered one
/// is kept 7 days. The web app got them from the server, and no answer it got holds a secret.
#[test]
fn usage_shows_the_personal_spaces_counts_and_limits_and_1_of_2_owned_spaces() {
    let browser = Browser::launch();
    let (page, _spa, cloud, personal) = usage_of_first_machine(&browser, 4_226_100_001, |s| {
        s.owned_spaces_limit = Some(OWNED_2);
        s.devices_limit = Some(DEVICES_50);
        s.monthly_messages_limit = Some(MESSAGES_10M);
        s.message_size_limit = Some(KB_64);
        s.mailbox_retention = Some(DAYS_7);
    });
    let expected = [
        (SPACES, (1.0, f64::from(OWNED_2))),
        (DEVICES, (1.0, f64::from(DEVICES_50))),
        (MESSAGES, (0.0, MESSAGES_10M as f64)),
    ];
    let paths = usage_paths(&cloud);
    let what = format!("Usage with the bars {expected:?}, from {paths:?}");
    wait_until(&page, &what, |page| {
        answered(page, &paths)
            && expected
                .iter()
                .all(|(counted, shown)| bar_of(page, counted) == Some(*shown))
    });
    let text = main_text(&page);
    let main = text.to_lowercase().replace(char::is_whitespace, "");
    for part in ["64kb", "7days", personal.as_str()] {
        assert!(main.contains(part), "{part}: {text:?}");
    }
    assert!(
        !main.contains("nolimit") && !main.contains("unlimited"),
        "{text:?}"
    );
}

/// Without the settings, as on a self-hosted server, Ida's Usage shows her owned personal space and
/// no limit: no bar, and it says no limit applies. The web app got them from the server.
#[test]
fn usage_without_the_settings_shows_no_limits() {
    let browser = Browser::launch();
    let (page, _spa, cloud, personal) = usage_of_first_machine(&browser, 4_226_200_001, |_| {});
    let paths = usage_paths(&cloud);
    let what = format!("Usage saying no limit applies, from {paths:?}");
    wait_until(&page, &what, |page| {
        let text = main_text(page).to_lowercase();
        answered(page, &paths) && (text.contains("no limit") || text.contains("unlimited"))
    });
    let text = main_text(&page);
    assert!(text.contains(&personal), "{personal}: {text:?}");
    assert_eq!(bars(&page), Vec::<Bar>::new(), "{text:?}");
    for limit in ["of 2", "of 50", "64 kb", "7 days"] {
        assert!(!text.to_lowercase().contains(limit), "{limit}: {text:?}");
    }
}
