//! How long the server keeps a channel's history, as a limit without a count: the value of
//! `--history-retention`, absent without it.

use serde_json::Value;

use crate::common::device_flow::{json, text};
use crate::usage::{Web, space_usage_response};

/// The cloud's value (`docs/dev/architecture.md`, "Cloud limits"): 90 days.
const DAYS_90: u64 = 90 * 86_400;

/// The `history_retention` that the web session `cookie` gets of `web`'s space.
fn history_retention(web: &Web, cookie: &str) -> Value {
    let response = space_usage_response(&web.api(), web.fixture.team.space, &[("cookie", cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    json(&response)["history_retention"].clone()
}

/// With the setting, each member's web session gets its value; without it, no limit.
#[test]
fn the_history_retention_is_its_settings_value_and_absent_without_it() {
    let days = DAYS_90.to_string();
    let web = Web::with(3_226_700_001, &["--history-retention", &days]);
    for (cookie, who) in [(&web.alice, "alice"), (&web.carol, "carol")] {
        let limit = serde_json::json!({ "limit": DAYS_90 });
        assert_eq!(history_retention(&web, cookie), limit, "{who}");
    }

    let web = Web::with(3_226_800_001, &[]);
    assert_eq!(
        history_retention(&web, &web.alice),
        serde_json::json!({}),
        "without the setting"
    );
}
