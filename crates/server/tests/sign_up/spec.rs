//! The OpenAPI spec names a refused sign-up: its problem type on both callbacks, and the token
//! endpoint's answer to the device code it was for.

use super::common::spec::spec;
use super::*;

/// Every description in `value`, at any depth, joined.
fn descriptions(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, value)| match (key.as_str(), value) {
                ("description", serde_json::Value::String(text)) => text.clone(),
                _ => descriptions(value),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Array(items) => items
            .iter()
            .map(descriptions)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[test]
fn the_spec_names_a_refused_sign_up() {
    let spec = spec();
    for path in ["/auth/github/callback", "/auth/google/callback"] {
        let responses = &spec["paths"][path]["get"]["responses"];
        assert!(
            descriptions(responses).contains(REFUSED),
            "GET {path} names {REFUSED}"
        );
    }
    let token = descriptions(&spec["paths"]["/auth/token"]["post"]["responses"]["400"]);
    assert!(
        token.contains("sign-up") || token.contains("Sign-up"),
        "POST /auth/token names the answer to a refused sign-up: {token}"
    );
}
