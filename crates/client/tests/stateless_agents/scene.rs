//! What the tests share: the talk of Ada and Bob with the service agent `ci/deploy-bot` that Ada's
//! machine created for the talk's channel, the machines the agent runs on and what they leave,
//! what the front saw them send, what Bob's device opens of the agent, and the fixture revisions
//! and anchors objects of the tests of the roster's start.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{fs, thread};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use skald_crypto::{open_message, sign_anchors};
use skald_proto::{
    AnchorsBody, Cbor as _, Cert, ChannelId, Content, DeviceId, Push, RawEnvelope, Role, Roster,
    Scheme, SpaceId, Unknown,
};

use crate::agents::{ACME_DIR, CLAUDE, run_as_in};
use crate::cloud::{Cloud, WAIT};
use crate::fixtures::{Issuer, Keyed, add_device, database, hex, keyed, send_args, strs};
use crate::kit::{RECV, latest, received};
use crate::machine::{Finished, Machine, ONE_SHOT, Run, STATE_DIR};
use crate::member::{JSON, messages, shown_once, talk_with_bot};
use crate::service_kit::{BOT, Secrets, bot_address, service_of};
use crate::talk::{Talk, messages_sent};
use crate::world::{
    ACME, ADD_RULE, Person, dir_of, next, next_by, path, post_as, set_role, store, warnings, words,
};

/// The level of a `service` device's messages.
pub const SERVICE: &str = "service";

/// The directories every machine of the tests starts with, empty.
const MACHINE_DIRS: [&str; 4] = ["home", "state", "tmp", "bin"];

/// The account's anchors objects: `POST` stores one, `GET` gives them all.
pub const ANCHORS: &str = "/account/anchors";

/// The request for the account's anchors objects, as its line starts.
pub const ANCHORS_REQUEST: &str = "GET /account/anchors";

/// The request for a token's own record and grant, as its line starts.
pub const OWN_RECORD_REQUEST: &str = "GET /service-token";

/// A machine the service agent runs on: of its own, empty, with the settings `vars` and none of
/// the state directory or one-shot mode, its working directory its home.
pub struct Agent {
    pub machine: Machine,
}

impl Agent {
    /// A new machine whose runs have the settings `vars`, by name.
    pub fn with(vars: &[(&str, &str)]) -> Self {
        let mut machine = Machine::new();
        for (name, value) in vars {
            machine = machine.with_env(name, *value);
        }
        Agent { machine }
    }

    /// A new machine whose runs have both of `secrets` as their environment variables, and
    /// nothing else of the agent's.
    pub fn of(secrets: &Secrets) -> Self {
        Self::with(&secrets.variables())
    }

    /// As [`Agent::of`], with the setting `name` set to `value` too.
    pub fn of_with(secrets: &Secrets, (name, value): (&str, &str)) -> Self {
        let [key, token] = secrets.variables();
        Self::with(&[key, token, (name, value)])
    }

    /// Its home.
    pub fn home(&self) -> PathBuf {
        self.machine.root().join("home")
    }

    /// `skald` with `args`, as the agent's runs are.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = self.machine.command_in(&self.home(), args);
        command
            .env_remove(STATE_DIR)
            .env_remove(ONE_SHOT)
            .stdin(Stdio::null());
        command
    }

    /// Starts `skald` with `args`.
    pub fn start(&self, args: &[&str]) -> Run {
        Run::spawn(&mut self.command(args))
    }

    /// Runs `skald` with `args`; fails if the run left anything on the machine's disk.
    #[track_caller]
    pub fn run(&self, args: &[&str]) -> Finished {
        let finished = self.start(args).finish();
        self.assert_left_nothing(&finished);
        finished
    }

    /// Runs `skald send --all --channel <channel> <text>`, as [`Agent::run`].
    #[track_caller]
    pub fn send(&self, channel: ChannelId, text: &str) -> Finished {
        self.run(&strs(&send_args(channel, text)))
    }

    /// As [`Agent::send`]; fails unless the run succeeds.
    #[track_caller]
    pub fn sent(&self, channel: ChannelId, text: &str) -> Finished {
        let finished = self.send(channel, text);
        assert!(finished.success, "the agent's send failed: {finished:?}");
        finished
    }

    /// Runs `skald recv --json`, as [`Agent::run`].
    #[track_caller]
    pub fn recv(&self) -> Finished {
        self.run(&[RECV, JSON])
    }

    /// As [`Agent::recv`]; fails unless the run succeeds.
    #[track_caller]
    pub fn received(&self) -> Finished {
        let finished = self.recv();
        assert!(finished.success, "the agent's recv failed: {finished:?}");
        finished
    }

    /// Fails unless the machine's disk holds nothing but its empty home, state, temporary and
    /// program directories: no file, no directory, no secrets file of a key storage. `after` is the
    /// run that must have left it so.
    #[track_caller]
    pub fn assert_left_nothing(&self, after: &Finished) {
        let root = self.machine.root();
        let mut left = Vec::new();
        for dir in MACHINE_DIRS {
            entries(&root.join(dir), &mut left);
        }
        let extra: Vec<PathBuf> = fs::read_dir(root)
            .expect("read the machine's directory")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                !MACHINE_DIRS
                    .iter()
                    .any(|dir| path.file_name() == Some(dir.as_ref()))
            })
            .collect();
        left.extend(extra);
        assert_eq!(
            left,
            Vec::<PathBuf>::new(),
            "the agent's run left these on disk: {after:?}"
        );
    }
}

/// Every file and directory under `dir`, at any depth, into `found`.
fn entries(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        found.push(path.clone());
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            entries(&path, found);
        }
    }
}

/// The talk of Ada and Bob in `acme`, and the service agent `ci/deploy-bot` that Ada's machine
/// created for the talk's channel, not as a member, with the creation's default permissions.
pub struct Scene {
    pub talk: Talk,
    pub secrets: Secrets,
    /// The agent's `service` device and its certificate.
    pub device: DeviceId,
    pub cert: Cert,
    /// Its address, `acme/ci/deploy-bot@<server>`.
    pub address: String,
    /// A machine it runs on, with both secrets.
    pub agent: Agent,
}

impl Scene {
    /// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones; Ada's
    /// machine creates `ci/deploy-bot`, which must succeed, and takes in its mailbox.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let (talk, secrets) = talk_with_bot(ids, logins, &[]);
        received(&talk.ada.machine);
        let (device, cert) = service_of(talk.cloud(), &talk.ada);
        let (address, agent) = (bot_address(), Agent::of(&secrets));
        Scene {
            talk,
            secrets,
            device,
            cert,
            address,
            agent,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        self.talk.cloud()
    }

    pub fn space(&self) -> SpaceId {
        self.talk.space.id
    }

    pub fn channel(&self) -> ChannelId {
        self.talk.channel
    }

    pub fn ada(&self) -> &Person {
        &self.talk.ada
    }

    pub fn bob(&self) -> &Person {
        &self.talk.played[0].person
    }

    /// How many `send` frames the front has seen.
    pub fn mark(&self) -> usize {
        self.talk.front.sent().len()
    }

    /// Fails if the front saw any `send` frame since `mark`; `what` names the case.
    #[track_caller]
    pub fn assert_none_sent(&self, mark: usize, what: &str) {
        let sent = self.talk.front.sent().split_off(mark);
        assert!(sent.is_empty(), "{what}: sent {sent:?}");
    }

    /// The distinct message envelopes of the channel the front saw sent since `mark`.
    pub fn sent_since(&self, mark: usize) -> Vec<RawEnvelope> {
        (messages_sent(&self.talk.front, self.channel(), mark).into_iter())
            .map(|sent| sent.envelope)
            .collect()
    }

    /// `~/work/acme` on Ada's machine, once bound to `acme` and her agent claude has run there
    /// once; fails unless both succeed.
    #[track_caller]
    pub fn ada_acme(&self) -> PathBuf {
        let machine = &self.ada().machine;
        let folder = dir_of(machine, ACME_DIR);
        let bound = crate::world::run(machine, &[ADD_RULE, path(&folder), ACME]);
        assert!(bound.success, "not bound: {bound:?}");
        run_as_in(machine, &folder, CLAUDE);
        folder
    }

    /// Ada's `skald recv --json`; fails unless it succeeds.
    #[track_caller]
    pub fn ada_receives(&self) -> Finished {
        let got = crate::world::run(&self.ada().machine, &[RECV, JSON]);
        assert!(got.success, "Ada's recv failed: {got:?}");
        got
    }

    /// Fails unless `got`, a run of `skald recv --json` of a member device, shows `text` once, in
    /// the channel, from the agent's address and device, named `ci/deploy-bot`, at the `service`
    /// level.
    #[track_caller]
    pub fn assert_shows_the_agent(&self, got: &Finished, text: &str) {
        let message = shown_once(got, text, (self.channel(), &self.address, self.device));
        assert_eq!(message["device_name"], BOT, "{message}");
        assert_eq!(message["level"], SERVICE, "{message}");
    }

    /// Fails unless `got`, a run of the agent's `skald recv --json`, shows `text` once, in the
    /// channel, from `person`'s address and device.
    #[track_caller]
    pub fn assert_agent_shows(&self, got: &Finished, text: &str, person: &Person) {
        shown_once(
            got,
            text,
            (self.channel(), &self.talk.address(person), person.device),
        );
    }

    /// The content of each `hpke` envelope to Bob's device alone in his mailbox, opened with his
    /// device's HPKE key and its signature checked with the agent's certificate.
    #[track_caller]
    pub fn bob_opens(&self) -> Vec<Content> {
        let cloud = self.cloud();
        let bob = &self.talk.played[0];
        let keys: Keyed = keyed(cloud, &bob.person);
        let mut peer = bob.connect(cloud);
        let entries = peer.catch_up(0);
        entries
            .into_iter()
            .filter_map(|entry| match entry {
                Push::Message { envelope, .. } => Some(envelope),
                _ => None,
            })
            .filter(|envelope| {
                let header = &envelope.envelope().header;
                header.scheme == Scheme::Hpke
                    && header.recipients.as_deref() == Some(&[bob.device()][..])
            })
            .map(|envelope| {
                let envelope = envelope.envelope();
                let opened = open_message(&envelope.header, &envelope.payload, keys.id, &keys.hpke)
                    .unwrap_or_else(|err| panic!("an envelope Bob's device cannot open: {err:?}"));
                opened
                    .verify(&self.cert)
                    .unwrap_or_else(|err| panic!("an envelope not signed by the agent: {err:?}"))
            })
            .collect()
    }

    /// As [`crate::member::assert_granted`], of the agent's device.
    #[track_caller]
    pub fn assert_granted(&self, content: &Content) {
        crate::member::assert_granted(&self.talk, self.device, content);
    }

    /// Fails unless the server's database holds each of `envelopes` and not `text`.
    #[track_caller]
    pub fn assert_only_ciphertext(&self, envelopes: &[RawEnvelope], text: &str) {
        let held = database(self.cloud());
        for envelope in envelopes {
            assert!(
                held.contains(&hex(envelope.bytes())),
                "the server does not hold an envelope the agent sent"
            );
        }
        assert!(
            !crate::keys::holds(self.cloud().rows().concat().as_bytes(), text.as_bytes()),
            "the database holds the text {text:?}"
        );
    }

    /// The revisions of the space the server holds, in order.
    pub fn revisions(&self) -> Vec<Roster> {
        crate::service_kit::rosters(self.cloud(), &self.ada().token, self.space())
    }
}

/// What the tests of the roster's start set up: Ada's device makes Bob an admin, past the key
/// secret's anchor, and a `client` device of Bob's account that is in no group, his lost laptop,
/// signs the next revision; the server stores both.
pub struct Lost {
    /// Bob's lost laptop.
    pub laptop: Keyed,
    /// The revision Ada's device signed.
    pub promoted: Roster,
    /// The revision the laptop signed.
    pub signed: Roster,
}

impl Lost {
    #[track_caller]
    pub fn set_up(scene: &Scene) -> Self {
        let (cloud, ada, bob) = (scene.cloud(), scene.ada(), scene.bob());
        let promoted = next_by(&latest(cloud, &ada.token, scene.space()), ada, |members| {
            set_role(members, bob.account, Role::Admin);
        });
        store(cloud, &ada.token, &promoted);
        let laptop = add_device(cloud, bob.account, "bob-lost-laptop", &Issuer::Device(bob));
        let signed = next(&promoted, (laptop.id, &laptop.signing), |_| {});
        store(cloud, &bob.token, &signed);
        Lost {
            laptop,
            promoted,
            signed,
        }
    }

    /// The number of the revision the laptop signed.
    pub fn number(&self) -> u64 {
        self.signed.body().revision
    }

    /// Bob's device revokes the laptop.
    pub fn revoke(&self, scene: &Scene) {
        crate::revocations::revoked_by(scene.cloud(), scene.bob(), self.laptop.id);
    }
}

/// Whether `line` names the revision `number` as `revision <number>`.
pub fn names_revision(line: &str, number: u64) -> bool {
    let number = number.to_string();
    words(line)
        .windows(2)
        .any(|pair| pair[0] == "revision" && pair[1] == number)
}

/// The warnings of `finished` that name the revision `number`.
pub fn warnings_naming_revision(finished: &Finished, number: u64) -> Vec<String> {
    (warnings(finished).into_iter())
        .filter(|line| names_revision(line, number))
        .collect()
}

/// Fails unless `finished` warns that the revision `number` has no effect, naming it and its
/// signer's device `signer`.
#[track_caller]
pub fn assert_stops_at(finished: &Finished, number: u64, signer: DeviceId) {
    let signer = signer.to_string();
    let warned: Vec<String> = (warnings_naming_revision(finished, number).into_iter())
        .filter(|line| line.contains(&signer))
        .collect();
    assert!(
        !warned.is_empty(),
        "no warning that revision {number}, signed by {signer}, has no effect: {finished:?}"
    );
}

/// Fails if `finished` warns of the revision `number`.
#[track_caller]
pub fn assert_takes(finished: &Finished, number: u64) {
    let warned = warnings_naming_revision(finished, number);
    assert_eq!(
        warned,
        Vec::<String>::new(),
        "a warning of revision {number}: {finished:?}"
    );
}

/// The anchors object of the device `(device, key)` of `account`, which anchors `space` at
/// `revision`, in CBOR.
pub fn anchors_object(
    (account, device, key): (skald_proto::AccountId, DeviceId, &skald_crypto::SigningKey),
    space: SpaceId,
    revision: &Roster,
) -> Vec<u8> {
    let body = AnchorsBody {
        account,
        device,
        anchors: vec![(space, revision.hash())],
        personal: None,
        unknown: Unknown::default(),
    };
    sign_anchors(&body, key)
        .expect("an anchors object")
        .to_cbor()
}

/// Stores `object`, an anchors object, as the session `token`; fails unless the server takes it.
#[track_caller]
pub fn store_anchors(cloud: &Cloud, token: &str, object: &[u8]) {
    let request = json!({ "anchors": URL_SAFE_NO_PAD.encode(object) });
    let (status, body) = post_as(cloud, token, ANCHORS, Some(&request));
    assert!(
        (200..300).contains(&status),
        "the anchors object is not stored: {status} {body}"
    );
}

/// From now on, the relay adds `objects` to the account's anchors objects it gives, as a
/// compromised server would.
pub fn add_anchors(cloud: &Cloud, objects: Vec<Vec<u8>>) {
    let added: Vec<String> = objects
        .iter()
        .map(|one| URL_SAFE_NO_PAD.encode(one))
        .collect();
    cloud.relay.rewrite(ANCHORS_REQUEST, move |body| {
        let mut held: Value = serde_json::from_str(body).expect("the account's anchors objects");
        let list = held["anchors"]
            .as_array_mut()
            .expect("a list of anchors objects");
        list.extend(added.iter().map(|one| json!(one)));
        held.to_string()
    });
}

/// A `client` device of `person`'s account that `person`'s device certified, written to the
/// server's database; it is in no group.
pub fn other_device(cloud: &Cloud, person: &Person, name: &str) -> Keyed {
    add_device(cloud, person.account, name, &Issuer::Device(person))
}

/// Waits for up to [`WAIT`] until `done` holds or `run` exits, then kills it; gives what it showed.
pub fn killed_when(mut run: Run, done: impl Fn() -> bool) -> Finished {
    let deadline = Instant::now() + WAIT;
    while !done() && !run.exited() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    run.kill()
}

/// The messages with `text` that `finished`, a run of `skald recv --json`, shows.
pub fn shown_with(finished: &Finished, text: &str) -> usize {
    (messages(finished).into_iter())
        .filter(|message| message["text"] == text)
        .count()
}
