//! A copy the server refuses because the agent has no live token that receives is not sent again,
//! and the run says so.

use serde_json::json;

use crate::agents::own_record;
use crate::scene::Scene;

/// The problem of an envelope to a device that cannot receive in its channel.
const UNKNOWN_RECIPIENT: &str = "unknown-recipient";

/// `ci/deploy-bot`'s token is revoked, but the relay gives Ada's machine, as the space's grants,
/// that token's grant, as a compromised server could. Her message sends the agent one copy, which
/// the server refuses with `unknown-recipient`: the run shows the problem, and neither it nor her
/// next run sends that copy again.
#[test]
fn a_copy_the_server_refuses_is_not_sent_again_and_the_run_says_so() {
    let scene = Scene::new(
        [3_155_540_001, 3_155_540_002],
        ["ada-stale-grant-standin", "bob-stale-grant-standin"],
    );
    let record = own_record(scene.cloud(), &scene.secrets.token).expect("a live token");
    let grant = record["grant"]
        .as_str()
        .expect("the token's grant")
        .to_owned();
    scene.revoke(&scene.secrets.token);
    let answer = json!({ "grants": [grant] }).to_string();
    let request = format!("GET /spaces/{}/service-grants", scene.space());
    (scene.cloud().relay).rewrite(&request, move |_| answer.clone());
    let mark = scene.mark();

    let refused = scene.talk.send("to a revoked token's grant");

    let sent = scene.copies_sent(mark);
    assert_eq!(sent.len(), 1, "not one copy sent: {sent:?}");
    let msg_id = sent[0].envelope().header.msg_id;
    assert!(
        refused.shown().contains(UNKNOWN_RECIPIENT),
        "the refusal is not shown: {refused:?}"
    );
    scene.assert_none_stored(mark, "a revoked token's grant");

    scene.talk.send("the next message");

    let again: Vec<_> = scene
        .copies_sent(mark)
        .into_iter()
        .filter(|envelope| envelope.envelope().header.msg_id == msg_id)
        .collect();
    assert_eq!(again.len(), 1, "the refused copy is sent again: {again:?}");
}
