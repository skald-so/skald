//! A separate Rust application sends and receives through the SDK on an enrolled device.
//! The other device runs `skald`. Both use the loopback server and ordinary MLS messages.

#![allow(dead_code, unused_imports)]

#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../recovery_key/grants.rs"]
mod terminal_grants;

use std::env::consts::EXE_SUFFIX;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::{MlsContent, MlsFraming};
use skald_proto::{ChannelId, Frame, MessageId, Scheme};
use skald_server::Settings;

use crate::fixtures::{await_running, fronted_with, msg_id};
use crate::front::{Front, Sent};
use crate::kit::{create_space, received};
use crate::linking::{NewDevice, link};
use crate::machine::{Finished, Run};
use crate::talk::{ADA_HOST, BOB_HOST, delivered, is_message, received_until};
use crate::world::{ACME, Person};

const FIRST: &str = "The SDK asks when the morning shipment leaves.";
const REPLY: &str = "The CLI replies that it leaves at ten.";
const TIMEOUT: u64 = 30;

struct Scene {
    _world: world::World,
    front: Front,
    sdk: Person,
    cli: Person,
    channel: ChannelId,
}

impl Scene {
    fn new(settings: impl FnOnce(&mut Settings)) -> Self {
        let (world, front) = fronted_with(settings);
        let sdk = world.sign_up(4_540_001_001, "ada-sdk-fixture", ADA_HOST);
        let channel = create_space(&world.cloud, &sdk, ACME).default;
        let mut new = NewDevice::start(&world, &sdk, BOB_HOST);
        link(&world.cloud, &sdk, &mut new);
        let cli = new.into_person(&world.cloud, &sdk);
        received(&sdk.machine);
        received(&cli.machine);
        Self {
            _world: world,
            front,
            sdk,
            cli,
            channel,
        }
    }

    fn fixture(&self, timeout: u64, args: &[&str]) -> Run {
        let executable = Path::new(common::CLIENT)
            .parent()
            .expect("Cargo's output directory")
            .join("examples")
            .join(format!("rust_message_fixture{EXE_SUFFIX}"));
        assert!(
            executable.is_file(),
            "missing {executable:?}; build the rust_message_fixture example"
        );
        let mut command = Command::new(executable);
        command
            .env_clear()
            .current_dir(self.sdk.machine.root())
            .arg(self.sdk.machine.state_dir())
            .arg(timeout.to_string())
            .args(args)
            .stdin(Stdio::null());
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        Run::spawn(&mut command)
    }

    fn send(&self, timeout: u64, body: &str) -> Run {
        self.fixture(timeout, &["send", &self.channel.to_string(), body])
    }

    fn cli_receive(&self) -> Vec<Value> {
        objects(&self.cli.machine.start(&["recv", "--json"]).finish())
    }

    fn sent_after(&self, start: usize) -> Vec<Sent> {
        self.front
            .sent()
            .into_iter()
            .skip(start)
            .filter(|send| {
                let envelope = send.envelope.envelope();
                envelope.header.channel == self.channel && is_message(envelope)
            })
            .collect()
    }

    fn only_send(&self, start: usize) -> Sent {
        let [send] = self.sent_after(start).try_into().expect("one message send");
        send
    }
}

fn objects(run: &Finished) -> Vec<Value> {
    assert!(run.success, "{run:?}");
    run.stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON output"))
        .collect()
}

fn message_objects(output: &[Value]) -> Vec<&Value> {
    output
        .iter()
        .filter(|item| item["type"] == "message")
        .collect()
}

fn sent_id(run: &Finished) -> MessageId {
    let output = objects(run);
    let [sent] = output.as_slice() else {
        panic!("expected one send result: {output:?}");
    };
    sent["msg_id"]
        .as_str()
        .expect("message ID")
        .parse()
        .expect("valid message ID")
}

#[track_caller]
fn assert_message(output: &[Value], id: MessageId, body: &str) {
    let messages = message_objects(output);
    let [message] = messages.as_slice() else {
        panic!("expected one message: {output:?}");
    };
    assert_eq!(message["msg_id"], id.to_string());
    assert_eq!(message["text"], body);
}

#[track_caller]
fn assert_encrypted(send: &Sent, id: MessageId, body: &str) {
    let envelope = send.envelope.envelope();
    assert_eq!(envelope.header.msg_id, id);
    assert_eq!(envelope.header.scheme, Scheme::Mls);
    assert!(matches!(
        MlsFraming::read(&envelope.payload),
        Some(MlsFraming::Message {
            public: false,
            content: MlsContent::Application,
            ..
        })
    ));
    assert!(
        !send
            .envelope
            .bytes()
            .windows(body.len())
            .any(|bytes| bytes == body.as_bytes()),
        "plaintext on the wire for {id}"
    );
}

#[test]
fn an_sdk_application_and_the_cli_exchange_an_encrypted_message_and_reply() {
    let scene = Scene::new(|_| {});
    let before = scene.front.sent().len();
    let id = sent_id(&scene.send(TIMEOUT, FIRST).finish());
    assert_encrypted(&scene.only_send(before), id, FIRST);
    assert_message(&scene.cli_receive(), id, FIRST);

    let before = scene.front.sent().len();
    let reply = scene
        .cli
        .machine
        .start(&["reply", "--json", &id.to_string(), REPLY])
        .finish();
    let reply_id = sent_id(&reply);
    assert_encrypted(&scene.only_send(before), reply_id, REPLY);
    let output = objects(&scene.fixture(TIMEOUT, &["receive"]).finish());
    assert_message(&output, reply_id, REPLY);
    assert_eq!(message_objects(&output)[0]["reply_to"], id.to_string());
}

#[test]
fn an_sdk_send_retries_a_lost_stored_response_with_one_stored_message() {
    let scene = Scene::new(|_| {});
    let before = scene.front.sent().len();
    scene.front.lose_next_stored();

    let id = sent_id(&scene.send(TIMEOUT, FIRST).finish());
    assert_eq!(scene.front.lost(), 1, "expected one lost stored response");
    let sends = scene.sent_after(before);
    assert!(sends.len() >= 2, "no retry: {sends:?}");
    assert_eq!(msg_id(&sends[0]), id);
    assert!(
        sends
            .iter()
            .all(|send| send.envelope.bytes() == sends[0].envelope.bytes())
    );

    scene.front.watch_delivery(scene.cli.device, id);
    assert_message(&scene.cli_receive(), id, FIRST);
    assert_eq!(
        scene.front.delivery().0.len(),
        1,
        "the server delivered more than one mailbox entry for {id}"
    );
}

#[test]
fn a_server_refusal_reaches_the_application_as_a_typed_error() {
    let scene = Scene::new(|settings| {
        settings.send_rate_limit = Some(1);
        settings.send_rate_window = Some(Duration::from_secs(600));
    });
    let first = sent_id(&scene.send(TIMEOUT, FIRST).finish());
    let before = scene.front.sent().len();

    let refused = scene
        .send(TIMEOUT, "A second report within the send-rate window.")
        .finish();

    assert!(!refused.success, "{refused:?}");
    let refused_id = msg_id(&scene.only_send(before));
    assert!(scene.front.answered().iter().any(|answer| matches!(answer,
        Frame::SendRefused { send_key, problem } if send_key.msg_id == refused_id && problem == "rate-limited"
    )));
    assert_message(&scene.cli_receive(), first, FIRST);
    let error: Value = serde_json::from_str(&refused.stderr).expect("typed send refusal");
    assert_eq!(
        error,
        json!({"type": "send-refused", "problem": "rate-limited"})
    );
}

#[test]
fn a_restart_hands_over_the_original_message_after_an_ack_before_handover() {
    let scene = Scene::new(|_| {});
    let sent = scene
        .cli
        .machine
        .start(&[
            "send",
            "--json",
            "--all",
            "--channel",
            &scene.channel.to_string(),
            REPLY,
        ])
        .finish();
    let id = sent_id(&sent);
    scene.front.watch_delivery(scene.sdk.device, id);
    // The device writes and acks the mailbox entry while catch-up still waits for its final batch.
    scene.front.hold_end(true);
    let mut killed = scene.fixture(TIMEOUT, &["receive"]);
    await_running(
        &mut killed,
        "native intake acknowledged the message",
        || scene.front.delivery().1,
    );
    let killed = killed.kill();
    assert!(!killed.success, "{killed:?}");
    assert!(
        killed.stdout.is_empty(),
        "handed over before the interruption: {killed:?}"
    );

    scene.front.hold_end(false);
    // A server receipt proves it got the ack. The restarted process must read from local state.
    received_until(&scene.cli.machine, |shown| {
        delivered(shown, id, scene.sdk.device) == 1
    });
    let restarted = objects(&scene.fixture(TIMEOUT, &["receive"]).finish());
    assert_message(&restarted, id, REPLY);
    assert_eq!(
        scene.front.delivery().0.len(),
        1,
        "the server redelivered {id}"
    );
    let next = objects(&scene.fixture(TIMEOUT, &["receive"]).finish());
    assert!(
        message_objects(&next).is_empty(),
        "handed over again: {next:?}"
    );
}

#[test]
fn a_second_application_respects_the_native_lock_on_the_same_device_state() {
    let scene = Scene::new(|_| {});
    let before = scene.front.sent().len();
    let held = scene.front.held_auths();
    scene.front.hold_auth_of(Some(scene.sdk.device));
    let mut first = scene.send(TIMEOUT, FIRST);
    await_running(
        &mut first,
        "first application's device proof at the server",
        || scene.front.held_auths() > held,
    );
    let held = scene.front.held_auths();

    let short = 2;
    let started = Instant::now();
    let waiting = scene
        .send(short, "A competing application request.")
        .finish();
    assert!(
        !waiting.success && waiting.stderr.to_lowercase().contains("lock"),
        "{waiting:?}"
    );
    assert!(
        started.elapsed() >= Duration::from_secs(short),
        "gave up before the lock timeout: {waiting:?}"
    );
    first.assert_running();
    assert_eq!(
        scene.front.held_auths(),
        held,
        "the app waiting for the lock reached the server"
    );

    scene.front.hold_auth_of(None);
    let id = sent_id(&first.finish());
    assert_eq!(msg_id(&scene.only_send(before)), id);
    assert_message(&scene.cli_receive(), id, FIRST);
}
