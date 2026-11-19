//! The web app's Usage section shows the space's service agents with their limit, also at 360 px.

use skald_github_stand_in::User;

use crate::agents::{LIMIT, agent_of};
use crate::api::{ACME, Person};
use crate::browser::{Browser, Page};
use crate::common::device_flow::{DeviceFlow, INTERVAL};
use crate::common::devices::assert_taken;
use crate::common::google::HOST;
use crate::common::sign_in::SignIn;
use crate::rosters::{first, next};
use crate::spa::{Spa, answers_without_secrets, wait_for_cabinet, wait_until};

/// A phone's width, in CSS pixels.
const PHONE: u32 = 360;

/// The text of `page`'s main area as a person sees it, each run of white space one space, in lower
/// case; empty without one.
fn main_text(page: &Page) -> String {
    let text = page.evaluate(r#"document.querySelector("main, [role=main]")?.innerText ?? """#);
    let text = text.as_str().expect("a text").to_lowercase();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `page`'s main area says the service agents are `count` of `limit`, with one bar of that
/// value and maximum.
fn shows_agents(page: &Page, count: u32, limit: u32) -> bool {
    let bar = [
        format!(r#"[role=progressbar][aria-valuenow="{count}"][aria-valuemax="{limit}"]"#),
        format!(r#"[role=meter][aria-valuenow="{count}"][aria-valuemax="{limit}"]"#),
        format!(r#"progress[value="{count}"][max="{limit}"]"#),
        format!(r#"meter[value="{count}"][max="{limit}"]"#),
    ]
    .join(", ");
    let text = main_text(page);
    text.contains("service agents")
        && text.contains(&format!("{count} of {limit}"))
        && page.count(&bar) == 1
}

/// How far `page` scrolls sideways, in CSS pixels.
fn sideways(page: &Page) -> i64 {
    let overflow = page
        .evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth");
    overflow.as_i64().expect("a number of pixels")
}

/// On a server with the limit 3, alice's machine creates `acme`, her only space, whose revision 2
/// lists two service agents. Signed in to the web app, she opens Usage: it says `2 of 3` service
/// agents, with a bar, and no answer the web app got holds a secret. In a browser 360 px wide with
/// her session, Usage shows the same and does not scroll sideways.
#[test]
fn usage_in_the_web_app_shows_the_service_agents_of_their_limit_also_at_360_px() {
    let browser = Browser::launch();
    let spa = Spa::start(&browser, HOST);
    let stand = SignIn::behind(&spa.url, &[INTERVAL, "1", LIMIT, "3"]);
    spa.serve(&browser, stand.server.base());
    let flow = DeviceFlow::of(stand);
    let github = 3_359_400_001;
    let alice = Person::new(&flow, github, "alice");
    let space = alice.create_space(ACME);
    let revision = alice.sign(&first(space, ACME, &alice));
    assert_taken(&alice.session.store(space, &revision));
    let mut body = next(&revision, &alice.laptop);
    body.service_agents = (1..=2)
        .map(|n| agent_of(&alice, &flow.stand.db, n))
        .collect();
    assert_taken(&alice.session.store(space, &alice.sign(&body)));

    let login = "alice-spaces-standin";
    let page = browser.page();
    page.record_answers();
    flow.stand.sign_in(&page, &User::new(github, login));
    page.goto(&spa.at("/usage"));
    wait_for_cabinet(&page, login);
    wait_until(&page, "Usage with 2 of 3 service agents", |page| {
        shows_agents(page, 2, 3)
    });
    answers_without_secrets(&page);

    let phone = browser.page_of_width(PHONE);
    phone.add_cookies(&page.cookies());
    phone.goto(&spa.at("/usage"));
    wait_until(
        &phone,
        "Usage at 360 px with 2 of 3 service agents",
        |page| shows_agents(page, 2, 3),
    );
    assert_eq!(sideways(&phone), 0, "{}", main_text(&phone));
}
