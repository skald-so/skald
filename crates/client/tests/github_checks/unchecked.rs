//! GitHub unreachable, answering errors or at its rate limit: every run succeeds, both machines
//! trust the laptop, show no login for it and warn of nothing; a later run with GitHub back
//! checks it.

use serde_json::json;

use crate::{LOGIN, Listed, pair, set_faults};

#[test]
fn with_github_unreachable_the_laptop_is_not_checked_until_a_later_run() {
    let (api, scene) = pair(Listed::Authentication);

    api.unreachable();
    for _ in 0..2 {
        let looked = scene.look();
        scene.assert_not_checked(&looked, LOGIN);
    }
    api.reachable();
    let looked = scene.look();
    scene.assert_passed(&looked, LOGIN);
}

#[test]
fn github_errors_and_its_rate_limit_leave_the_laptop_not_checked_until_a_later_run() {
    let (api, scene) = pair(Listed::Authentication);

    let faults = [
        json!([{ "fault": "api_status", "status": 500 }]),
        json!([{ "fault": "api_status", "status": 503 }]),
        json!([{ "fault": "rate_limited" }]),
    ];
    for fault in &faults {
        set_faults(&api.github, fault);
        let looked = scene.look();
        scene.assert_not_checked(&looked, LOGIN);
    }
    set_faults(&api.github, &json!([]));
    let looked = scene.look();
    scene.assert_passed(&looked, LOGIN);
}
