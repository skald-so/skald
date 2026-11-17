//! The spec's answer for the account's devices names no key or certificate.

use serde_json::Value;

use crate::common::spec::spec;
use crate::shown::ACCOUNT_DEVICES;

/// The property names that would carry a key or a certificate.
const SECRETS: [&str; 10] = [
    "key",
    "keys",
    "master",
    "signing",
    "hpke",
    "cert",
    "certs",
    "certificate",
    "key_package",
    "revocation",
];

/// Adds the name of every property of `schema` in `spec`, at any depth, through its `$ref`s, to
/// `names`; `seen` holds the `$ref`s already followed.
fn properties(spec: &Value, schema: &Value, names: &mut Vec<String>, seen: &mut Vec<String>) {
    match schema {
        Value::Object(fields) => {
            if let Some(Value::String(reference)) = fields.get("$ref")
                && !seen.contains(reference)
            {
                seen.push(reference.clone());
                let pointer = reference.strip_prefix('#').expect("a local $ref");
                let target = spec.pointer(pointer).expect("the $ref resolves");
                properties(spec, target, names, seen);
            }
            if let Some(Value::Object(listed)) = fields.get("properties") {
                names.extend(listed.keys().cloned());
            }
            for field in fields.values() {
                properties(spec, field, names, seen);
            }
        }
        Value::Array(items) => {
            for item in items {
                properties(spec, item, names, seen);
            }
        }
        _ => {}
    }
}

/// `GET /account/devices` answers `200` with a JSON schema that has `devices`, and no property of
/// it, at any depth, names a key or a certificate.
#[test]
fn the_spec_gives_the_accounts_devices_without_a_key_or_certificate() {
    let spec = spec();
    let pointer = format!(
        "/paths/{}/get/responses/200/content/application~1json/schema",
        ACCOUNT_DEVICES.replace('/', "~1")
    );
    let schema = spec
        .pointer(&pointer)
        .unwrap_or_else(|| panic!("no JSON answer 200 to GET {ACCOUNT_DEVICES} in the spec"));
    let mut names = Vec::new();
    properties(&spec, schema, &mut names, &mut Vec::new());
    assert!(names.iter().any(|name| name == "devices"), "{names:?}");
    for name in &names {
        assert!(
            !SECRETS.contains(&name.to_lowercase().as_str()),
            "a key or certificate: {name}"
        );
    }
}
