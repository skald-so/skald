//! A session's account as `GET /session` gives it, and the form of a username.

use serde_json::Value;

use super::Response;

/// The usernames no account takes.
pub const RESERVED: [&str; 4] = ["skald", "admin", "support", "root"];

/// The most characters of a `name` in an agent address.
pub const NAME_MAX: usize = 63;

/// A session's account as `GET /session` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
    pub account: String,
    pub username: String,
    /// The login of the account's GitHub method, while it has one.
    pub github_login: Option<String>,
}

impl Who {
    /// The account in `response`, a `200` answer to `GET /session`.
    pub fn of(response: &Response) -> Self {
        let body = String::from_utf8_lossy(&response.body);
        assert_eq!(response.status, 200, "{body}");
        Who::of_body(&serde_json::from_str(&body).expect("JSON"))
    }

    /// The account in `body`, the JSON of a `200` answer to `GET /session`.
    pub fn of_body(body: &Value) -> Self {
        let account = body["account"].as_str().expect("an account ID").to_owned();
        let username = body["username"]
            .as_str()
            .unwrap_or_else(|| panic!("no username in {body}"))
            .to_owned();
        let github_login = match body.get("github_login") {
            None => None,
            Some(Value::String(login)) => Some(login.clone()),
            Some(other) => panic!("github_login {other} in {body}"),
        };
        Who {
            account,
            username,
            github_login,
        }
    }
}

/// Whether `text` has the form of a `name` in an agent address.
pub fn is_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    let alnum = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    (1..=NAME_MAX).contains(&bytes.len())
        && alnum(&bytes[0])
        && alnum(&bytes[bytes.len() - 1])
        && bytes.iter().all(|byte| alnum(byte) || *byte == b'-')
}

/// Checks that `username` is one an account can have: a `name`, not reserved.
#[track_caller]
pub fn assert_valid(username: &str) {
    assert!(is_name(username), "not a name: {username:?}");
    assert!(!RESERVED.contains(&username), "reserved: {username:?}");
}
