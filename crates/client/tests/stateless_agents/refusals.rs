//! What a run refuses, sending nothing and leaving nothing on its machine's disk: a grant whose
//! signer is revoked or that has expired, a revoked token, a latest revision without its entry,
//! revisions that do not chain to its anchor, certificates against another master key, a GroupInfo
//! that fails its checks; and a store the server refuses or does not answer fails it.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use skald_crypto::{SigningKey, StateKey, issue};
use skald_proto::{Cbor as _, Grant, GrantBody, NonEmpty, Roster, Unknown};

use crate::devices::second_machine;
use crate::fixtures::cert_of;
use crate::front::Answers;
use crate::kit::{RECV, group_info_request, received};
use crate::member::{JSON, texts_of};
use crate::scene::{Agent, OWN_RECORD_REQUEST, Scene, names_revision};
use crate::service_kit::{
    BOT, CREATE_SERVICE_TOKEN, REVOKE_SERVICE_TOKEN, Secrets, id_of, own_record, run_for_channels,
    token_of,
};
use crate::world::{
    ACME, change_devices, device_entry, devices_request, next, next_with, now, revisions_request,
    run, serve_revisions,
};

/// What the agent says; nowhere else.
const AGENT_SAYS: &str = "stateless bot test: deploy-bot tries to report";

/// What a run refused for its grant names: the command of a new token.
const NEW_TOKEN: &str = "create-service-token";

/// The problem the front gives in place of the server's `stored`.
const PROBLEM: &str = "not-a-member";

/// The setting of how long a request may take, in seconds, and the short one the tests set.
const TIMEOUT: &str = "SKALD_TIMEOUT";
const QUICK: &str = "5";

/// Runs the agent's send on `agent` and fails unless it fails, sends nothing and leaves nothing on
/// its machine's disk; `what` names the case. Gives the run.
#[track_caller]
fn refused_on(scene: &Scene, agent: &Agent, what: &str) -> crate::machine::Finished {
    let mark = scene.mark();
    let refused = agent.send(scene.channel(), AGENT_SAYS);
    assert!(!refused.success, "{what}: the send succeeded: {refused:?}");
    scene.assert_none_sent(mark, what);
    refused
}

/// As [`refused_on`], on the scene's agent.
#[track_caller]
fn refused(scene: &Scene, what: &str) -> crate::machine::Finished {
    refused_on(scene, &scene.agent, what)
}

/// From now on, the relay gives the answers to requests whose lines start with `request` as the
/// server gives them; then the agent's send succeeds, and Bob's device opens what it sent.
#[track_caller]
fn sends_once_given_as_held(scene: &Scene, request: &str) {
    scene.cloud().relay.rewrite(request, str::to_owned);
    scene.agent.sent(scene.channel(), AGENT_SAYS);
    let opened: Vec<Vec<String>> = scene.bob_opens().iter().map(texts_of).collect();
    assert_eq!(
        opened,
        [[AGENT_SAYS]],
        "what Bob's device opens of the agent"
    );
}

/// From now on, the relay gives the agent `revisions` as the space's.
fn serve(scene: &Scene, revisions: &[Roster]) {
    let given: Vec<&Roster> = revisions.iter().collect();
    serve_revisions(scene.cloud(), scene.space(), &given);
}

/// From now on, the relay gives `group_info` as the channel's GroupInfo, `change` applied to its
/// bytes.
fn change_group_info(scene: &Scene, change: impl Fn(&mut Vec<u8>) + Send + Sync + 'static) {
    let request = group_info_request(scene.space(), scene.channel());
    scene.cloud().relay.rewrite(&request, move |body| {
        let mut held: Value = serde_json::from_str(body).expect("a GroupInfo answer");
        let given = held["group_info"].as_str().expect("a GroupInfo");
        let mut bytes = URL_SAFE_NO_PAD
            .decode(given)
            .expect("a GroupInfo in base64url");
        change(&mut bytes);
        held["group_info"] = json!(URL_SAFE_NO_PAD.encode(&bytes));
        held.to_string()
    });
}

/// Ada's second machine, linked by hand, joins the channel; it revokes Ada's first device, the one
/// that signed the agent's grant, keeping itself and the agent's device, and takes the first device
/// out of the channel's group. The agent's send is refused, saying a new token is needed, with
/// nothing sent. With a token the second machine creates, the agent sends, and the second machine's
/// run shows it once at the `service` level.
#[test]
fn a_run_whose_grant_signer_is_revoked_says_a_new_token_is_needed() {
    let mut scene = Scene::new(
        [3_135_780_001, 3_135_780_002],
        [
            "ada-stateless-signer-standin",
            "bob-stateless-signer-standin",
        ],
    );
    let laptop = second_machine(&scene.talk.world, scene.ada(), "ada-laptop");
    received(&laptop.machine);
    scene.talk.played[0].take_in(&scene.talk.world.cloud);
    let (first, kept) = (scene.ada().device.to_string(), laptop.device.to_string());
    let agent = scene.device.to_string();
    let revoked = run(
        &laptop.machine,
        &["revoke-device", &first, "--keep", &kept, "--keep", &agent],
    );
    assert!(revoked.success, "{revoked:?}");
    received(&laptop.machine);
    scene.talk.played[0].take_in(&scene.talk.world.cloud);

    let said = refused(&scene, "a grant whose signer is revoked");

    assert!(said.stderr.contains(NEW_TOKEN), "{said:?}");
    let command = (CREATE_SERVICE_TOKEN, ACME, BOT);
    let token = token_of(&run_for_channels(
        &laptop.machine,
        command,
        &[scene.channel()],
        &[],
    ));
    let renewed = Agent::of(&Secrets {
        key: scene.secrets.key.clone(),
        token,
    });
    renewed.sent(scene.channel(), AGENT_SAYS);
    let got = run(&laptop.machine, &[RECV, JSON]);
    assert!(got.success, "{got:?}");
    scene.assert_shows_the_agent(&got, AGENT_SAYS);
}

/// The relay gives the token's own record with a grant Ada's device signed for the agent and the
/// channel that expired a minute ago: the agent's send is refused, saying a new token is needed.
/// Given the record as the server holds it, the agent sends.
#[test]
fn a_run_with_an_expired_grant_says_a_new_token_is_needed() {
    let scene = Scene::new(
        [3_135_781_001, 3_135_781_002],
        [
            "ada-stateless-expired-standin",
            "bob-stateless-expired-standin",
        ],
    );
    let ada = scene.ada();
    let body = GrantBody {
        service: scene.device,
        channels: NonEmpty::from_vec(vec![scene.channel()]).expect("a channel"),
        signer: ada.device,
        expires: now() - 60,
        unknown: Unknown::default(),
    };
    let signature = ada.key.sign("skald grant", &[&body.to_cbor()]);
    let expired = URL_SAFE_NO_PAD.encode(Grant { body, signature }.to_cbor());
    scene
        .cloud()
        .relay
        .rewrite(OWN_RECORD_REQUEST, move |answer| {
            let mut record: Value = serde_json::from_str(answer).expect("a token's own record");
            record["grant"] = json!(expired);
            record.to_string()
        });

    let said = refused(&scene, "an expired grant");

    assert!(said.stderr.contains(NEW_TOKEN), "{said:?}");
    sends_once_given_as_held(&scene, OWN_RECORD_REQUEST);
}

/// Ada's machine revokes the agent's token: the agent's send is refused.
#[test]
fn a_run_with_a_revoked_token_is_refused() {
    let scene = Scene::new(
        [3_135_782_001, 3_135_782_002],
        ["ada-stateless-token-standin", "bob-stateless-token-standin"],
    );
    let record = own_record(scene.cloud(), &scene.secrets.token).expect("a live token");
    let revoked = run(
        &scene.ada().machine,
        &[REVOKE_SERVICE_TOKEN, ACME, &id_of(&record)],
    );
    assert!(revoked.success, "{revoked:?}");

    refused(&scene, "a revoked token");
}

/// The relay gives, after the space's revisions, one Ada's device signed that drops the agent's
/// entry: the agent's send is refused, naming that revision. Given the revisions the server holds,
/// the agent sends.
#[test]
fn a_run_whose_latest_revision_has_no_entry_of_its_device_is_refused() {
    let scene = Scene::new(
        [3_135_783_001, 3_135_783_002],
        ["ada-stateless-entry-standin", "bob-stateless-entry-standin"],
    );
    let ada = scene.ada();
    let mut given = scene.revisions();
    let last = given.last().expect("a revision");
    let dropped = next_with(last, (ada.device, &ada.key), |body| {
        body.service_agents.clear();
    });
    let number = dropped.body().revision;
    given.push(dropped);
    serve(&scene, &given);

    let said = refused(&scene, "no entry");

    let named =
        (said.stderr.lines().map(str::to_lowercase)).any(|line| names_revision(&line, number));
    assert!(named, "revision {number} not named: {said:?}");
    sends_once_given_as_held(&scene, &revisions_request(scene.space()));
}

/// The relay gives the space's revisions with another revision in place of the key secret's
/// anchor, which Ada's device signed on the revision before without the agent's entry: the
/// revisions do not chain to the anchor, and the agent's send is refused. Given the revisions the
/// server holds, the agent sends.
#[test]
fn a_run_whose_revisions_do_not_chain_to_its_anchor_is_refused() {
    let scene = Scene::new(
        [3_135_784_001, 3_135_784_002],
        ["ada-stateless-chain-standin", "bob-stateless-chain-standin"],
    );
    let ada = scene.ada();
    let mut given = scene.revisions();
    given.pop().expect("the anchor");
    let before = given.last().expect("a revision before the anchor");
    let other = next(before, (ada.device, &ada.key), |_| {});
    given.push(other);
    serve(&scene, &given);

    refused(&scene, "revisions off the anchor");
    sends_once_given_as_held(&scene, &revisions_request(scene.space()));
}

/// The relay gives Ada's device in the space's device list with a certificate that another master
/// key than her account's signed: the agent's send is refused. Given the devices the server holds,
/// the agent sends.
#[test]
fn a_run_given_another_master_key_s_certificates_is_refused() {
    let scene = Scene::new(
        [3_135_785_001, 3_135_785_002],
        [
            "ada-stateless-master-standin",
            "bob-stateless-master-standin",
        ],
    );
    let ada = scene.ada();
    let mut body = cert_of(scene.cloud(), ada).body().clone();
    body.issuer = None;
    let forged = issue(&body, &SigningKey::generate(), &ada.key).expect("a certificate");
    let (device, entry) = (ada.device.to_string(), device_entry(ada.device, &forged));
    change_devices(scene.cloud(), scene.space(), move |list| {
        for one in list.iter_mut() {
            if one["device"] == json!(device) {
                *one = entry.clone();
            }
        }
    });

    refused(&scene, "another master key");
    sends_once_given_as_held(&scene, &devices_request(scene.space()));
}

/// The relay gives the channel's GroupInfo with a byte of its signature changed: the agent's send
/// is refused. Given the GroupInfo the server holds, the agent sends.
#[test]
fn a_run_given_a_group_info_with_a_broken_signature_is_refused() {
    let scene = Scene::new(
        [3_135_786_001, 3_135_786_002],
        [
            "ada-stateless-signature-standin",
            "bob-stateless-signature-standin",
        ],
    );
    change_group_info(&scene, |bytes| {
        let last = bytes.last_mut().expect("a GroupInfo");
        *last ^= 1;
    });

    refused(&scene, "a GroupInfo with a broken signature");
    sends_once_given_as_held(&scene, &group_info_request(scene.space(), scene.channel()));
}

/// Bob's device adds Eve's device, of an account outside `acme`, to the channel's group without
/// sending the Commit, and the relay gives the new epoch's GroupInfo, signed by Bob's device: Eve's
/// device is not admitted, and the agent's send is refused. Given the GroupInfo the server holds,
/// the agent sends.
#[test]
fn a_run_given_a_group_info_with_a_device_not_admitted_is_refused() {
    let mut scene = Scene::new(
        [3_135_787_001, 3_135_787_002],
        [
            "ada-stateless-admitted-standin",
            "bob-stateless-admitted-standin",
        ],
    );
    let eve = (scene.talk.world).sign_up(3_135_787_003, "eve-stateless-standin", "eve-workstation");
    let eve_cert = cert_of(scene.cloud(), &eve);
    let mut eve_mls = skald_crypto::MlsDevice::new(&eve.key, &StateKey::generate());
    let package = eve_mls
        .key_package(now())
        .expect("a KeyPackage")
        .as_ref()
        .to_vec();
    let channel = scene.channel();
    let bob = &mut scene.talk.played[0];
    (bob.mls)
        .add_devices(channel, &[package], &[eve_cert], now())
        .expect("a Commit adding Eve's device");
    (bob.mls).commit_stored(channel).expect("the Commit merges");
    let group_info = bob.mls.group_info(channel).expect("a GroupInfo").to_vec();
    change_group_info(&scene, move |bytes| bytes.clone_from(&group_info));

    refused(&scene, "a GroupInfo with a device not admitted");
    sends_once_given_as_held(&scene, &group_info_request(scene.space(), scene.channel()));
}

/// The front gives a `send-refused` in place of each `stored`: the agent's send fails, naming the
/// problem. With the answers forwarded again, the agent's next run sends nothing.
#[test]
fn a_store_the_server_refuses_fails_the_run() {
    let scene = Scene::new(
        [3_135_788_001, 3_135_788_002],
        [
            "ada-stateless-refused-standin",
            "bob-stateless-refused-standin",
        ],
    );
    let front = &scene.talk.front;
    front.answer(Answers::Refuse(PROBLEM.to_owned()));

    let failed = scene.agent.send(scene.channel(), AGENT_SAYS);

    assert!(!failed.success, "{failed:?}");
    assert!(failed.stderr.contains(PROBLEM), "{failed:?}");
    front.answer(Answers::Forward);
    let mark = scene.mark();
    scene.agent.received();
    scene.assert_none_sent(mark, "the run after a refused store");
}

/// The front loses each `stored`, and the agent's requests may take 5 seconds: the agent's send
/// fails. With the answers forwarded again, the agent's next run sends nothing.
#[test]
fn a_store_the_server_does_not_answer_within_the_run_fails_it() {
    let scene = Scene::new(
        [3_135_789_001, 3_135_789_002],
        ["ada-stateless-lost-standin", "bob-stateless-lost-standin"],
    );
    let agent = Agent::of_with(&scene.secrets, (TIMEOUT, QUICK));
    let front = &scene.talk.front;
    front.answer(Answers::Lose);

    let failed = agent.send(scene.channel(), AGENT_SAYS);

    assert!(!failed.success, "{failed:?}");
    front.answer(Answers::Forward);
    let mark = scene.mark();
    agent.received();
    scene.assert_none_sent(mark, "the run after a lost store");
}
