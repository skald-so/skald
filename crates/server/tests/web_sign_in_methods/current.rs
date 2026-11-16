//! The list marks the method the request's web session signed in with.

use crate::common::device_flow::json;
use crate::{METHODS, Provider, Stand, Via, listed_in, machine_of};

/// The providers of the methods that the session `headers` carry lists as `current`.
fn current(stand: &Stand, headers: &[(&str, &str)]) -> Vec<Provider> {
    let response = stand.api().call("get", METHODS, METHODS, headers);
    assert_eq!(response.status, 200);
    let body = json(&response);
    let listed = listed_in(&body);
    body["methods"]
        .as_array()
        .expect("a list of methods")
        .iter()
        .zip(&listed)
        .filter(|(item, _)| match &item["current"] {
            serde_json::Value::Bool(current) => *current,
            other => panic!("current {other} in {item}"),
        })
        .map(|(_, method)| method.provider)
        .collect()
}

/// Ada, with Google and GitHub: her Google session's list marks Google, her GitHub session's marks
/// GitHub, and her machine's marks none.
#[test]
fn list_marks_the_method_of_this_web_session() {
    let stand = Stand::launch(&[]);
    let ada = stand.with_both(501, "ada-current");
    let by_github = stand.signed_in(Via::GitHub(&ada.user));
    let token = machine_of(&stand.google, &ada.session.page);
    let bearer = format!("Bearer {token}");

    let google = [("cookie", ada.session.cookie.as_str())];
    assert_eq!(current(&stand, &google), [Provider::Google]);
    let github = [("cookie", by_github.cookie.as_str())];
    assert_eq!(current(&stand, &github), [Provider::GitHub]);
    let device = [("authorization", bearer.as_str())];
    assert_eq!(current(&stand, &device), []);
}
