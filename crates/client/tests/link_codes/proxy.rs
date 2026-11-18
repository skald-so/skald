//! A proxy between the devices and the server that answers the new device's hello itself, or
//! changes a reply, a request or a grant, fails the run on both sides, and the run spends one of
//! the new device's attempts: after two such runs and one with a wrong code, its code has failed 3
//! times. The relay plays the proxy on the answers that carry a run's later messages.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_COMPRESSED;
use serde_json::{Value, json};

use crate::cloud::{Cloud, unb64};
use crate::linking::{
    MESSAGES, NewDevice, STORE_CERT, THREE_TIMES, YES, link_failing, link_with, other_code, says,
    typed,
};
use crate::world::{Person, World};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// The types of `link-reply`, `link-request` and `link-grant`.
const REPLY: u8 = 2;
const REQUEST: u8 = 3;
const GRANT: u8 = 4;

/// The type of `message`, a `link-message` of three items, if it is one.
fn kind_of(message: &[u8]) -> Option<u8> {
    (message.len() > 2 && message[0] == 0x83).then(|| message[1])
}

/// From now on, the relay gives the later messages of each run as `change` makes the list the
/// server gives: each message's bytes, in order.
fn tamper(cloud: &Cloud, change: impl Fn(Vec<Vec<u8>>) -> Vec<Vec<u8>> + Send + Sync + 'static) {
    cloud.relay.rewrite(MESSAGES, move |body| {
        let mut answer: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let Some(listed) = answer["messages"].as_array() else {
            return body.to_owned();
        };
        let messages: Vec<Vec<u8>> = listed.iter().filter_map(Value::as_str).map(unb64).collect();
        let changed: Vec<String> = change(messages)
            .iter()
            .map(|message| URL_SAFE_NO_PAD.encode(message))
            .collect();
        answer["messages"] = json!(changed);
        answer.to_string()
    });
}

/// From now on, the relay gives the server's answers as they are.
fn honest(cloud: &Cloud) {
    cloud.relay.rewrite(MESSAGES, str::to_owned);
}

/// From now on, the relay changes each message of type `kind` by `change`.
fn tamper_with(cloud: &Cloud, kind: u8, change: fn(Vec<u8>) -> Vec<u8>) {
    tamper(cloud, move |messages| {
        messages
            .into_iter()
            .map(|message| {
                if kind_of(&message) == Some(kind) {
                    change(message)
                } else {
                    message
                }
            })
            .collect()
    });
}

/// `message` with its last byte, in its sealed bytes' tag, flipped.
fn flipped(mut message: Vec<u8>) -> Vec<u8> {
    *message.last_mut().expect("a message") ^= 1;
    message
}

/// `link-reply` `message` with another share, a valid ristretto255 element: the generator's.
fn other_share(message: Vec<u8>) -> Vec<u8> {
    let share = RISTRETTO_BASEPOINT_COMPRESSED.to_bytes();
    [&message[..message.len() - share.len()], &share[..]].concat()
}

/// The `session` of the account's latest hello, as `first`'s session gets it.
fn latest_session(cloud: &Cloud, first: &Person) -> [u8; 16] {
    let (status, body) = cloud.get_as("/account/links/hello", &first.token);
    assert_eq!(status, 200, "GET /account/links/hello: {body}");
    let hello = unb64(body["hello"].as_str().expect("a hello"));
    // `[1, session, share]`: the array's head, the type, the session's head, then its 16 bytes.
    hello[3..19].try_into().expect("a session of 16 bytes")
}

/// `link-reply = [2, session, share]` to the run `session`, as the proxy makes it.
fn forged_reply(session: &[u8; 16]) -> Vec<u8> {
    let share = RISTRETTO_BASEPOINT_COMPRESSED.to_bytes();
    [&[0x83, REPLY, 0x50][..], session, &[0x58, 0x20], &share].concat()
}

/// Ada's new machine, showing its code, and her account.
fn start(id: u64, login: &str) -> (World, Person, NewDevice) {
    let world = World::start();
    let ada = world.sign_up(id, login, "ada-workstation");
    let new = NewDevice::start(&world, &ada, LAPTOP);
    (world, ada, new)
}

/// Ends the test's third run with a wrong code and checks that the new device's code then has
/// failed 3 times and that it holds no certificate.
fn assert_three_spent(world: &World, ada: &Person, new: &mut NewDevice) {
    let cloud = &world.cloud;
    new.open_run(cloud, ada);
    link_failing(ada, &other_code(&new.code()));
    new.run().wait_for("that the code failed 3 times", |shown| {
        says(shown, &THREE_TIMES)
    });
    assert_eq!(
        cloud.count_sent(new.started, STORE_CERT),
        0,
        "a certificate stored"
    );
}

/// The proxy answers the new device's hello before the linked device: the linked device, typed
/// the right code, fails, and so does the new device. Then the proxy changes the linked device's
/// reply to another valid share: both fail again.
#[test]
fn a_proxy_that_answers_the_hello_or_changes_the_reply_fails_both_sides() {
    let (world, ada, mut new) = start(4_175_501_001, "ada-behind-a-proxy");
    let cloud = &world.cloud;

    new.open_run(cloud, &ada);
    let forged = forged_reply(&latest_session(cloud, &ada));
    tamper(cloud, move |messages| {
        [vec![forged.clone()], messages].concat()
    });
    link_failing(&ada, &typed(&new.code()));
    new.wait_for_failures(1);
    honest(cloud);

    new.open_run(cloud, &ada);
    tamper_with(cloud, REPLY, other_share);
    link_failing(&ada, &typed(&new.code()));
    new.wait_for_failures(2);
    honest(cloud);

    assert_three_spent(&world, &ada, &mut new);
}

/// The proxy changes the new device's request: the linked device, typed the right code, fails,
/// and so does the new device. Then it changes the linked device's grant, which its person
/// confirmed: the new device fails, stores no certificate, and the linked device fails too.
#[test]
fn a_proxy_that_changes_the_request_or_the_grant_fails_both_sides() {
    let (world, ada, mut new) = start(4_175_501_011, "ada-behind-another-proxy");
    let cloud = &world.cloud;

    new.open_run(cloud, &ada);
    tamper_with(cloud, REQUEST, flipped);
    link_failing(&ada, &typed(&new.code()));
    new.wait_for_failures(1);
    honest(cloud);

    new.open_run(cloud, &ada);
    tamper_with(cloud, GRANT, flipped);
    let granted = link_with(&ada, &typed(&new.code()), LAPTOP, YES);
    assert!(!granted.success, "{}", granted.text);
    new.wait_for_failures(2);
    honest(cloud);

    assert_three_spent(&world, &ada, &mut new);
}
