//! What the members' answer leaves out: the space's revisions and its members' master keys, in the
//! spec and in what the server gives.

use serde_json::Value;

use crate::common::browser::Answer;
use crate::common::device_flow::text;
use crate::common::devices::b64;
use crate::common::spa::assert_no_secrets;
use crate::common::spec::spec;
use crate::listed::{MEMBERS, Web, members_path, members_response};
use crate::sessions::token_of;

/// The property names that would carry a revision or a key.
const LEFT_OUT: [&str; 9] = [
    "revision",
    "revisions",
    "roster",
    "master",
    "key",
    "keys",
    "signing",
    "signature",
    "cert",
];

/// The names of the properties of `schema` in `spec`, at any depth, its `$ref`s followed once each.
fn property_names(spec: &Value, schema: &Value) -> Vec<String> {
    let mut names = Vec::new();
    let mut followed = Vec::new();
    let mut pending = vec![schema.clone()];
    while let Some(next) = pending.pop() {
        if let Value::Array(items) = &next {
            pending.extend(items.iter().cloned());
            continue;
        }
        let Value::Object(fields) = &next else {
            continue;
        };
        if let Some(Value::Object(listed)) = fields.get("properties") {
            names.extend(listed.keys().cloned());
        }
        if let Some(Value::String(target)) = fields.get("$ref")
            && !followed.contains(target)
        {
            followed.push(target.clone());
            let at = target.trim_start_matches('#');
            pending.push(spec.pointer(at).cloned().expect("a $ref that resolves"));
        }
        pending.extend(fields.values().cloned());
    }
    names
}

/// `GET /spaces/{space}/members` answers `200` with a JSON schema that has `members`, and no
/// property of it, at any depth, names a revision or a key.
#[test]
fn the_spec_gives_a_spaces_members_without_a_revision_or_key() {
    let spec = spec();
    let path = MEMBERS.replace('/', "~1");
    let pointer = format!("/paths/{path}/get/responses/200/content/application~1json/schema");
    let schema = spec
        .pointer(&pointer)
        .unwrap_or_else(|| panic!("no JSON answer 200 to GET {MEMBERS} in the spec"));
    let names = property_names(&spec, schema);
    assert!(names.iter().any(|name| name == "members"), "{names:?}");
    for name in &names {
        assert!(
            !LEFT_OUT.contains(&name.to_lowercase().as_str()),
            "a revision or key: {name}"
        );
    }
}

/// The members' answer to alice's web session holds none of the space's revisions, none of its
/// members' master keys or device keys, and no session token.
#[test]
fn the_answer_holds_no_revision_master_key_or_session_token() {
    let web = Web::new(3_225_500_001);
    let team = &web.team;
    let revisions = team.with_members();
    let response = members_response(&web.api(), team.space, &[("cookie", &web.alice)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = String::from_utf8_lossy(&response.body).into_owned();

    let answer = Answer {
        method: "GET".to_owned(),
        url: members_path(team.space),
        status: response.status,
        headers: Default::default(),
        body: body.clone(),
    };
    assert_no_secrets(&[answer], &[token_of(&web.alice).to_owned()]);
    let mut secrets: Vec<String> = revisions.iter().map(|revision| b64(revision)).collect();
    for person in [&team.alice, &team.bob, &team.carol] {
        secrets.push(b64(&person.master.public()));
        secrets.push(b64(&person.laptop.signing.public()));
        secrets.push(b64(&person.cert));
    }
    for secret in &secrets {
        assert!(!body.contains(secret.as_str()), "a secret: {body}");
    }
}
