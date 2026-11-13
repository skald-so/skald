//! Checks responses against the OpenAPI spec in `api/openapi.json`.

use std::path::Path;

use jsonschema::Registry;
use serde_json::{Value, json};

use super::Response;

/// The base URI the spec is registered under to resolve `$ref`s.
const SPEC_URI: &str = "urn:skald:openapi";

pub fn spec_bytes() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../api/openapi.json");
    std::fs::read(path).expect("read api/openapi.json")
}

pub fn spec() -> Value {
    serde_json::from_slice(&spec_bytes()).expect("api/openapi.json is JSON")
}

/// Returns how `response` to `GET <path>` deviates from `spec`, as [`operation_violations`] does.
pub fn spec_violations(spec: &Value, path: &str, response: &Response) -> Vec<String> {
    operation_violations(spec, "get", path, response)
}

/// Returns how `response` to `<method> <path>`, `method` in lowercase, deviates from `spec`: a
/// status or a content type the spec does not list for the operation, a missing or invalid response
/// header the spec lists, a body that does not match the response schema, or a body where the spec
/// lists no content.
pub fn operation_violations(
    spec: &Value,
    method: &str,
    path: &str,
    response: &Response,
) -> Vec<String> {
    let documented = format!(
        "/paths/{}/{method}/responses/{}",
        pointer_token(path),
        response.status
    );
    // How the messages name the operation, e.g. `GET /session`.
    let path = &format!("{} {path}", method.to_ascii_uppercase());
    if spec.pointer(&documented).is_none() {
        return vec![format!(
            "{path}: status {} is not in the spec",
            response.status
        )];
    }
    let mut violations = header_violations(spec, path, &documented, response);
    if spec.pointer(&format!("{documented}/content")).is_none() {
        if !response.body.is_empty() || !response.content_type.is_empty() {
            violations.push(format!(
                "{path} {}: a body where the spec lists no content",
                response.status
            ));
        }
        return violations;
    }
    let media_type = response.content_type.split(';').next().unwrap_or("").trim();
    let schema = format!("{documented}/content/{}/schema", pointer_token(media_type));
    if spec.pointer(&schema).is_none() {
        violations.push(format!(
            "{path} {}: content type {media_type:?} is not in the spec",
            response.status
        ));
        return violations;
    }
    let body = if media_type.contains("json") {
        match serde_json::from_slice(&response.body) {
            Ok(body) => body,
            Err(err) => {
                violations.push(format!("{path}: body is not JSON: {err}"));
                return violations;
            }
        }
    } else {
        Value::String(String::from_utf8_lossy(&response.body).into_owned())
    };
    violations.extend(
        schema_violations(spec, &schema, &body)
            .into_iter()
            .map(|err| format!("{path}: {err}")),
    );
    violations
}

/// The headers of `response` that break the `headers` of the documented response at `documented`;
/// `path` names the operation, e.g. `GET /session`.
fn header_violations(
    spec: &Value,
    path: &str,
    documented: &str,
    response: &Response,
) -> Vec<String> {
    let headers = format!("{documented}/headers");
    let Some(listed) = spec.pointer(&headers).and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut violations = Vec::new();
    for (name, header) in listed {
        // A header object, or a `$ref` to one in `components/headers`.
        let at = match header.get("$ref").and_then(Value::as_str) {
            Some(reference) => reference
                .strip_prefix('#')
                .expect("a local $ref")
                .to_owned(),
            None => format!("{headers}/{}", pointer_token(name)),
        };
        let header = spec.pointer(&at).expect("the $ref resolves");
        let Some(value) = response.header(&name.to_ascii_lowercase()) else {
            if header["required"] == true {
                violations.push(format!("{path} {}: no {name} header", response.status));
            }
            continue;
        };
        let instance = if header["schema"]["type"] == "integer" {
            match value.parse::<i64>() {
                Ok(number) => json!(number),
                Err(_) => Value::String(value.to_owned()),
            }
        } else {
            Value::String(value.to_owned())
        };
        violations.extend(
            schema_violations(spec, &format!("{at}/schema"), &instance)
                .into_iter()
                .map(|err| format!("{path} {}: header {name}: {err}", response.status)),
        );
    }
    violations
}

/// How `instance` breaks the schema at the JSON pointer `schema` in `spec`.
fn schema_violations(spec: &Value, schema: &str, instance: &Value) -> Vec<String> {
    let registry = Registry::new()
        .add(SPEC_URI, spec)
        .expect("register the spec")
        .prepare()
        .expect("index the spec");
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&json!({ "$ref": format!("{SPEC_URI}#{}", fragment(schema)) }))
        .expect("compile the schema");
    validator
        .iter_errors(instance)
        .map(|err| format!("{err} at {:?}", err.instance_path().as_str()))
        .collect()
}

/// The JSON pointer `pointer` as a URI fragment (RFC 6901, §6): the characters a fragment cannot
/// hold that the spec's paths have, such as `{` of a path parameter, percent-encoded.
fn fragment(pointer: &str) -> String {
    pointer
        .replace('%', "%25")
        .replace('{', "%7B")
        .replace('}', "%7D")
}

/// Escapes `token` for a JSON pointer (RFC 6901).
fn pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}
