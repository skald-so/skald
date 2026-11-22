//! A service agent that keeps no state between runs takes its tasks' results as the server
//! confirms them: the task is in a queue of the channel, `to` is its agent, and the device in the
//! task's `sched`, if any, sent the result ("Results").

use serde_json::json;
use skald_crypto::{HpkeKey, SigningKey, seal_message};
use skald_proto::{
    Cbor as _, ClaimId, DeviceId, Envelope, Outcome, QueueId, RawEnvelope, Sched, Scheme,
    ServiceKey, TaskId,
};

use crate::claiming::{Craft, sealed_by, uploads};
use crate::cloud::Cloud;
use crate::fixtures::CHANNEL;
use crate::kit::received;
use crate::machine::{Finished, Machine};
use crate::member::talk_with_bot;
use crate::played::mls_envelope;
use crate::queuing::{QUEUES, default_queue, enqueued, proved, queues_path};
use crate::returns::{Answer, COMPLETE_ACT, DONE, FAILED, holds_part, result_objects};
use crate::scene::Agent;
use crate::senders::BOB_AGENT;
use crate::service_kit::{BOT, PERMISSION, RECEIVE, SEND, bot_address, key_of, service_of};
use crate::states::{self, CLAIMED, Held};
use crate::talk::{Talk, stored};
use crate::world::{ACME, HOST, Person, dir_of};

/// What the tests enqueue and answer; no text is anywhere else.
const ROLLOUT: &str = "stateless result test: the bot asks for a canary check of the rollout";
const CANARY_DONE: &str = "stateless result test: the canary is healthy";
const ELSEWHERE: &str = "stateless result test: ada asks in the default channel for a backup";
const FORGED: &str = "stateless result test: a result the bot must not take";
const FOR_ADA: &str = "stateless result test: the bot asks ada's device for a log export";
const NOT_YET: &str = "stateless result test: the canary did not start";

/// The trust level a claim of Bob's states for the bot's tasks.
const SERVICE_LEVEL: &str = "service";

/// The talk of Ada and Bob in `acme`, and the service agent `ci/deploy-bot` that Ada's machine
/// created for the talk's channel, not as a member, with the permissions `send`, `receive` and
/// `queues`, and a machine it runs on without a disk.
struct Bot {
    talk: Talk,
    key: ServiceKey,
    /// Its `service` device as the requests of the tests prove it: on its token, with the signing
    /// key of its key secret.
    device: Person,
    /// Its address, `acme/ci/deploy-bot@<server>`.
    address: String,
    agent: Agent,
    /// The talk's channel's queue `default`.
    queue: QueueId,
    /// The counter of Bob's last message.
    said: u64,
}

impl Bot {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones.
    #[track_caller]
    fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let permissions = [PERMISSION, SEND, PERMISSION, RECEIVE, PERMISSION, QUEUES];
        let (mut talk, secrets) = talk_with_bot(ids, logins, &permissions);
        received(&talk.ada.machine);
        talk.played[0].take_in(&talk.world.cloud);
        let (id, _) = service_of(talk.cloud(), &talk.ada);
        let queue = default_queue(talk.cloud(), &talk.ada, talk.at());
        let key = key_of(&secrets.key);
        let device = Person {
            user: talk.ada.user.clone(),
            machine: Machine::new(),
            token: secrets.token.clone(),
            account: key.account,
            master: key.master,
            handle: talk.ada.handle.clone(),
            device: id,
            host: BOT,
            key: SigningKey::from_bytes(&key.signing),
        };
        Bot {
            agent: Agent::of(&secrets),
            address: bot_address(),
            talk,
            key,
            device,
            queue,
            said: 0,
        }
    }

    fn cloud(&self) -> &Cloud {
        self.talk.cloud()
    }

    fn bob(&self) -> &Person {
        &self.talk.played[0].person
    }

    /// The full address of Bob's agent `claude-q3x6np`.
    fn bob_agent(&self) -> String {
        format!("{ACME}/{}/{BOB_AGENT}@{HOST}", self.bob().handle)
    }

    /// A task of `text` from the bot to the talk's channel's queue `default`, with `sched`, signed
    /// with the bot's key secret, sealed to the channel's current epoch and uploaded with the bot's
    /// device's proof on its token; fails unless the server stores it.
    #[track_caller]
    fn writes(&self, sched: Sched, text: &str) -> TaskId {
        let mut craft = Craft::new(&self.address, self.queue, text);
        craft.sched = sched;
        let at = self.talk.at();
        let signer = (&self.device.key, self.device.device);
        let record = sealed_by(&self.talk.played[0], at, signer, &craft);
        uploads(self.cloud(), &self.device, at, &record);
        record.task
    }

    /// Bob's device claims a task of the talk's channel's queue `default`, stating `service` and
    /// taking tasks without a template, and completes it if `done`, otherwise fails it with retry;
    /// fails unless the server gives `task` claimed by Bob's device and takes the rest. Gives the
    /// claim.
    #[track_caller]
    fn bob_ends(&self, task: TaskId, done: bool) -> Held {
        let (cloud, bob, at) = (self.cloud(), self.bob(), self.talk.at());
        let path = format!("{}/{}/claims", queues_path(at), self.queue);
        let takes = json!({
            "trust_levels": [SERVICE_LEVEL],
            "templates": [],
            "without_template": true,
        });
        let (status, body) = proved(cloud, bob, ("POST", &path), Some(&takes));
        assert_eq!(status, 200, "Bob's claim stating service: {body}");
        let read = states::read(cloud, bob, (at, self.queue), task);
        let Some((claim_id, generation, device)) = read.claim.clone() else {
            panic!("{task} has no claim: {read:?}");
        };
        assert_eq!(
            (read.state.as_str(), device),
            (CLAIMED, bob.device.to_string()),
            "{task} not claimed by Bob's device: {read:?}"
        );
        let held = Held {
            at,
            queue: self.queue,
            task,
            claim_id,
            generation,
        };
        if done {
            states::taken(cloud, bob, &held, (COMPLETE_ACT, json!({})));
        } else {
            states::fails(cloud, bob, &held, true);
        }
        held
    }

    /// Bob's device sends `answer` in the talk's channel under `hpke` to the bot's device alone,
    /// sealed to the HPKE key of its key secret; fails unless the server stores it.
    #[track_caller]
    fn bob_answers(&mut self, answer: &Answer) {
        let to = self.device.device;
        let mut header = mls_envelope(self.talk.at(), &[], &[to])
            .envelope()
            .header
            .clone();
        header.scheme = Scheme::Hpke;
        self.said += 1;
        let content = answer.content(header.msg_id, self.said);
        let key = HpkeKey::from_bytes(&self.key.hpke).public_key();
        let bob = &self.talk.played[0];
        let payload =
            seal_message(&header, &content, &bob.person.key, &key).expect("a sealed payload");
        let envelope =
            RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope");
        stored(self.talk.cloud(), bob, &envelope);
    }

    /// The bot's `skald recv --json` without a disk; fails unless it succeeds and leaves nothing on
    /// its machine's disk.
    #[track_caller]
    fn reads(&self) -> Finished {
        self.agent.received()
    }

    /// Fails unless the bot's next run gives one result of `task` with `outcome`, holding `text`.
    #[track_caller]
    fn assert_shows(&self, task: TaskId, outcome: &str, text: &str) {
        let got = self.reads();
        let objects = result_objects(&got.stdout, task, outcome);
        let [object] = objects.as_slice() else {
            panic!("not one {outcome} result of {task}: {got:?}");
        };
        assert!(holds_part(object, text), "not the text {text:?}: {object}");
    }

    /// Fails if the bot's next run gives a result of `task` or shows `text`. `what` names the
    /// case.
    #[track_caller]
    fn assert_drops(&self, task: TaskId, text: &str, what: &str) {
        let got = self.reads();
        for outcome in [DONE, FAILED] {
            let objects = result_objects(&got.stdout, task, outcome);
            assert!(objects.is_empty(), "{what}: {objects:?}");
        }
        assert!(!got.shown().contains(text), "{what}: {got:?}");
    }
}

/// The bot's device writes a task to the queue `default`. Bob's device claims it, stating
/// `service`, completes it and sends a `done` result naming the claim, from Bob's agent to the
/// bot. The bot's next run, without a disk, gives one result of the task: a JSON object that names
/// the task, `done` and the result's text. The run after it gives no result of the task.
#[test]
fn a_stateless_run_shows_the_done_result_of_a_task_enqueued_as_it() {
    let mut bot = Bot::new(
        [5_873_051_001, 5_873_051_002],
        ["ada-runs-a-diskless-bot", "bob-answers-the-diskless-bot"],
    );
    let task = bot.writes(Sched::default(), ROLLOUT);
    let held = bot.bob_ends(task, true);
    let (from, to) = (bot.bob_agent(), bot.address.clone());

    bot.bob_answers(&Answer::of(&held, Outcome::Done, (&from, &to), CANARY_DONE));

    bot.assert_shows(task, DONE, CANARY_DONE);
    bot.assert_drops(task, CANARY_DONE, "the bot's second run");
}

/// The person on Ada's machine enqueues a task in `acme`'s default channel, for which the bot has
/// no grant; Bob's device claims and completes it there. Bob's `done` result of it, sent in the
/// talk's channel under `hpke` to the bot, is not shown by the bot's next run. A result of the
/// bot's own task, which Bob's device claims and completes next, is.
#[test]
fn a_stateless_run_drops_a_result_for_a_task_of_another_channel() {
    let mut bot = Bot::new(
        [5_873_052_001, 5_873_052_002],
        [
            "ada-enqueues-beside-the-bot",
            "bob-answers-the-bot-elsewhere",
        ],
    );
    let default = (bot.talk.space.id, bot.talk.space.default);
    let ada = &bot.talk.ada;
    let (home, named) = (dir_of(&ada.machine, "work"), default.1.to_string());
    enqueued(&ada.machine, (&home, None), &[CHANNEL, &named, ELSEWHERE]);
    let queue = default_queue(bot.cloud(), ada, default);
    let elsewhere = states::claims(bot.cloud(), bot.bob(), default, queue);
    states::taken(
        bot.cloud(),
        bot.bob(),
        &elsewhere,
        (COMPLETE_ACT, json!({})),
    );
    let (from, to) = (bot.bob_agent(), bot.address.clone());

    bot.bob_answers(&Answer::of(&elsewhere, Outcome::Done, (&from, &to), FORGED));

    bot.assert_drops(elsewhere.task, FORGED, "another channel's task");
    let task = bot.writes(Sched::default(), ROLLOUT);
    let held = bot.bob_ends(task, true);
    bot.bob_answers(&Answer::of(&held, Outcome::Done, (&from, &to), CANARY_DONE));
    bot.assert_shows(task, DONE, CANARY_DONE);
}

/// The bot's device writes a task whose `sched` names Ada's device, and one with no device in
/// `sched`. Bob's device sends a `failed` result of the first: the bot's next run does not show
/// it. Bob's device claims the second, stating `service`, fails it with retry and sends a `failed`
/// result naming the claim: the bot's next run shows it.
#[test]
fn a_stateless_run_drops_a_result_from_another_device_than_the_one_in_sched() {
    let mut bot = Bot::new(
        [5_873_053_001, 5_873_053_002],
        [
            "ada-device-gets-the-bot-task",
            "bob-answers-for-ada-s-device",
        ],
    );
    let for_ada: DeviceId = bot.talk.ada.device;
    let addressed = bot.writes(
        Sched {
            device: Some(for_ada),
            ..Sched::default()
        },
        FOR_ADA,
    );
    let task = bot.writes(Sched::default(), ROLLOUT);
    let (from, to) = (bot.bob_agent(), bot.address.clone());
    let unclaimed = Held {
        at: bot.talk.at(),
        queue: bot.queue,
        task: addressed,
        claim_id: ClaimId::generate().to_string(),
        generation: 1,
    };

    bot.bob_answers(&Answer::of(
        &unclaimed,
        Outcome::Failed,
        (&from, &to),
        FORGED,
    ));

    bot.assert_drops(addressed, FORGED, "a result from a device not in sched");
    let held = bot.bob_ends(task, false);
    bot.bob_answers(&Answer::of(&held, Outcome::Failed, (&from, &to), NOT_YET));
    bot.assert_shows(task, FAILED, NOT_YET);
}
