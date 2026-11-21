//! Results on a stateless run of a service agent: it keeps no tasks, so it checks a result against
//! the server.

use std::num::NonZeroU64;

use serde_json::json;
use skald_crypto::{SigningKey, seal_message};
use skald_proto::{
    Cbor as _, ClaimId, Envelope, MessageId, Outcome, QueuedTask, RawEnvelope, Scheme,
};

use crate::claiming::{Craft, address, sealed_by, tampers, uploads};
use crate::played::mls_envelope;
use crate::queuing::default_queue;
use crate::results::{Named, assert_dropped, named, result, result_objects};
use crate::scene::Scene;
use crate::service_kit::key_of;
use crate::states;
use crate::talk::stored;

/// What the tests write; no text is anywhere else.
const DEPLOY: &str = "stateless result test: deploy-bot asks for a deploy of build 912";
const DEPLOYED: &str = "stateless result test: bob's agent deployed build 912";
const MOVED: &str = "stateless result test: deploy-bot asks for a task the server moved";
const MOVED_FAILED: &str = "stateless result test: bob failed the moved task";
const FOR_ADA: &str = "stateless result test: deploy-bot asks ada's device alone";
const FOR_ADA_FAILED: &str = "stateless result test: bob failed the task for ada's device";

/// Bob's agent, whose address Bob's device writes results as.
const BOB_AGENT: &str = "claude-r5t2mz";

/// The scene of the tests of stateless agents, Bob's device having taken in, and the counter of
/// its last line to the agent's device.
struct Bot {
    scene: Scene,
    bob_said: u64,
}

impl Bot {
    #[track_caller]
    fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let mut scene = Scene::new(ids, logins);
        let talk = &mut scene.talk;
        talk.played[0].take_in(&talk.world.cloud);
        Bot { scene, bob_said: 0 }
    }

    /// A task of `text` from the agent in the channel's queue `default`, with `craft` changing
    /// it: signed with the agent's key as its device, sealed to the channel's queue key, and
    /// uploaded with the proof of Ada's device. Fails unless the server stores it.
    #[track_caller]
    fn enqueues_as_the_agent(&self, text: &str, craft: impl FnOnce(&mut Craft)) -> QueuedTask {
        let talk = &self.scene.talk;
        let (cloud, at) = (talk.cloud(), talk.at());
        let queue = default_queue(cloud, &talk.ada, at);
        let mut crafted = Craft::new(&self.scene.address, queue, text);
        craft(&mut crafted);
        let key = SigningKey::from_bytes(&key_of(&self.scene.secrets.key).signing);
        let record = sealed_by(&talk.played[0], at, (&key, self.scene.device), &crafted);
        uploads(cloud, &talk.ada, at, &record);
        record
    }

    /// Bob's device writes to the agent's device under `hpke`, as its next line to it, a result
    /// of `task` from Bob's agent to the agent, naming the claim `named`, with `outcome` and
    /// `text`; fails unless the server stores it. Gives its `msg_id`.
    #[track_caller]
    fn bob_results(
        &mut self,
        (task, named): (skald_proto::TaskId, Named),
        (outcome, text): (Outcome, &str),
    ) -> MessageId {
        self.bob_said += 1;
        let talk = &self.scene.talk;
        let bob = &talk.played[0];
        let from = address(&format!(
            "{}/{}/{BOB_AGENT}@{}",
            crate::world::ACME,
            bob.person.handle,
            crate::world::HOST
        ));
        let to = address(&self.scene.address);
        let mut header = mls_envelope(talk.at(), &[], &[self.scene.device])
            .envelope()
            .header
            .clone();
        header.scheme = Scheme::Hpke;
        let msg_id = header.msg_id;
        let counter = NonZeroU64::new(self.bob_said).expect("a counter");
        let content = result(task, named, (outcome, text), msg_id).content((from, to), counter, 1);
        let payload = seal_message(
            &header,
            &content,
            &bob.person.key,
            &self.scene.cert.body().hpke,
        )
        .expect("a sealed payload");
        let envelope =
            RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope");
        stored(talk.cloud(), bob, &envelope);
        msg_id
    }
}

/// A task the agent enqueued, which Bob's device claims and completes; Bob's device writes a
/// `done` result of it to the agent, naming that claim. The agent's `skald recv --json` gives it
/// once, as an object of the type `result` with the task's ID, `done` and Bob's agent's text.
#[test]
fn stateless_run_shows_the_result_of_a_task_its_agent_enqueued() {
    let mut bot = Bot::new(
        [4_268_121_001, 4_268_121_002],
        ["ada-runs-a-stateless-deployer", "bob-deploys-for-the-bot"],
    );
    let record = bot.enqueues_as_the_agent(DEPLOY, |_| {});
    let talk = &bot.scene.talk;
    let (cloud, bob) = (talk.cloud(), &talk.played[0].person);
    let held = states::claims(cloud, bob, talk.at(), record.queue);
    assert_eq!(held.task, record.task, "Bob claimed another task");
    states::taken(cloud, bob, &held, ("complete", json!({})));
    bot.bob_results((record.task, named(&held)), (Outcome::Done, DEPLOYED));

    let got = bot.scene.agent.received();

    let objects = result_objects(&got.stdout);
    let [object] = objects.as_slice() else {
        panic!("not one result: {got:?}");
    };
    assert_eq!(object["task"], record.task.to_string(), "{object}");
    assert_eq!(object["outcome"], "done", "{object}");
    assert_eq!(object["text"], DEPLOYED, "{object}");
}

/// Two tasks the agent enqueued: one the server then keeps in the queue `default` of the space's
/// default channel, and one whose `sched` names Ada's device. Bob's device writes a `failed`
/// result of each to the agent. The agent's run drops both with a warning each.
#[test]
fn stateless_run_drops_a_result_of_a_task_of_another_channel_or_for_another_device() {
    let mut bot = Bot::new(
        [4_268_122_001, 4_268_122_002],
        ["ada-runs-a-careful-deployer", "bob-answers-the-wrong-tasks"],
    );
    let moved = bot.enqueues_as_the_agent(MOVED, |_| {});
    let talk = &bot.scene.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let elsewhere = default_queue(cloud, ada, (talk.space.id, talk.space.default));
    tampers(
        cloud,
        moved.task,
        &format!("queue_id = '{elsewhere}'::uuid"),
    );
    let ada_device = ada.device;
    let for_ada = bot.enqueues_as_the_agent(FOR_ADA, |craft| craft.sched.device = Some(ada_device));
    let moved_failed = bot.bob_results(
        (moved.task, (ClaimId::generate(), 1)),
        (Outcome::Failed, MOVED_FAILED),
    );
    let for_ada_failed = bot.bob_results(
        (for_ada.task, (ClaimId::generate(), 1)),
        (Outcome::Failed, FOR_ADA_FAILED),
    );

    let got = bot.scene.agent.received();

    assert_dropped(&got, moved_failed, MOVED_FAILED);
    assert_dropped(&got, for_ada_failed, FOR_ADA_FAILED);
}
