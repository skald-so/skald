//! What the tests share: the web app's harness before the server, Ada, an admin of `acme`, and Bob,
//! a member, each signed in to the web app in a browser of their own, `ci/deploy-bot` that Ada
//! creates with `skald`, and the token's row in Access.

use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::access::{open_access, rows_holding};
use crate::browser::{Browser, Page};
use crate::cloud::Cloud;
use crate::kit::{
    AGENT_NAME, Acme, BOT, Secrets, create_agent, own_record, secrets_of, sign_up_on,
};
use crate::spa::{Spa, wait_for_cabinet, wait_until};
use crate::world::{ACME, HOST, Person, World};

/// The months as a date names them.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The people's world, its server's public address that of the web app's harness, on which the
/// machines reach the server too.
pub struct Harness {
    pub world: World,
    pub spa: Spa,
}

impl Harness {
    pub fn start() -> Self {
        let browser = Browser::launch();
        let spa = Spa::start(&browser, HOST);
        let world = World {
            cloud: Cloud::fronted(HOST, spa.port, 120, 1),
            browser,
        };
        spa.serve(&world.browser, &world.cloud.relay.url);
        Harness { world, spa }
    }

    /// Opens Access in `page`, `person`'s browser: the cabinet first, then the section from the
    /// side menu, waiting for the row of the page's own session.
    pub fn open_access(&self, page: &Page, person: &Person) {
        page.goto(&self.spa.at("/"));
        wait_for_cabinet(page, person.handle.as_str());
        open_access(page);
    }
}

/// Ada and Bob, each with a browser signed in to the web app, `acme` with Bob as a member, and the
/// service agent `ci/deploy-bot` Ada creates for its default channel with the default options, its
/// token's own record as the server gives it to the token.
pub struct Stage<'h> {
    pub ada: Person,
    pub ada_page: Page<'h>,
    pub bob: Person,
    pub bob_page: Page<'h>,
    pub acme: Acme,
    pub secrets: Secrets,
    pub record: Value,
}

impl<'h> Stage<'h> {
    /// The stage on `harness`; GitHub user IDs from `github` on.
    pub fn set(harness: &'h Harness, github: u64) -> Self {
        let world = &harness.world;
        let ada_page = world.browser.page();
        ada_page.record_answers();
        let ada = sign_up_on(
            world,
            &ada_page,
            (github, "Ada-Tokens-Standin"),
            "ada-laptop",
        );
        let bob_page = world.browser.page();
        bob_page.record_answers();
        let bob = sign_up_on(
            world,
            &bob_page,
            (github + 1, "Bob-Tokens-Standin"),
            "bob-laptop",
        );
        let acme = Acme::with_member(&world.cloud, &ada, &bob);
        let created = create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]);
        let secrets = secrets_of(&created);
        let record =
            own_record(&world.cloud, &secrets.token).expect("the server refuses the token");
        Stage {
            ada,
            ada_page,
            bob,
            bob_page,
            acme,
            secrets,
            record,
        }
    }

    /// The forms in which a row may say when the token expires: the days left, rounded either way,
    /// or its date.
    pub fn expiry_forms(&self, days: u64) -> Vec<String> {
        let expires = self.record["expires_at"]
            .as_str()
            .unwrap_or_else(|| panic!("no expiry in {}", self.record));
        let date = &expires[..10];
        let month: usize = date[5..7].parse().expect("a month");
        let day: u32 = date[8..10].parse().expect("a day");
        let month = MONTHS[month - 1];
        vec![
            format!("{days} days"),
            format!("{} days", days - 1),
            date.to_owned(),
            format!("{month} {day}"),
            format!("{day} {month}"),
        ]
    }

    /// The forms in which a row may name the token's one channel, `acme`'s default channel.
    pub fn channel_forms(&self) -> Vec<String> {
        let id = self.acme.default.to_string();
        vec![id[..8].to_owned(), id, "1 channel".to_owned()]
    }
}

/// The one row of the Access section in `page` that names `deploy-bot`, once it shows.
#[track_caller]
pub fn token_row(page: &Page) -> String {
    wait_until(page, "the row of ci/deploy-bot's token", |page| {
        rows_holding(page, AGENT_NAME).len() == 1
    });
    rows_holding(page, AGENT_NAME).remove(0)
}

/// Whether `page`'s Access section, open with its own session's row, keeps showing no row that
/// names `deploy-bot` and no failure for 2 s.
pub fn shows_no_token(page: &Page) -> bool {
    for _ in 0..20 {
        if !rows_holding(page, AGENT_NAME).is_empty() || page.count("[role=alert]") > 0 {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
    true
}
