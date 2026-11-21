//! A member `service` device fails with retry where a `client` releases ("Claim checks"): a task
//! naming a template, one whose `to` names no agent of the device, one under an epoch whose key the
//! device lacks, and one of a trust level its claim did not state.

use skald_proto::{ChannelId, SpaceId};

use crate::claiming::{
    CLAIM, Craft, QUEUED, gives_bytes, hands_out, read, requests_naming, sealed_by, sent_act,
    tampers, uploads,
};
use crate::fixtures::CHANNEL;
use crate::member::{Bot, MEMBER, Runner, talk_with_bot};
use crate::queuing::{QUEUES, default_queue};
use crate::service_kit::{PERMISSION, RECEIVE, SEND, bot_address, key_of, service_of};
use crate::world::{ACME, HOST, run};

/// What the tests write; no text is anywhere else.
const TEMPLATED: &str = "service test: ada asks the bot for a review through a template";
const FOR_GEMINI: &str = "service test: ada asks her gemini on the bot's device";
const NEXT_EPOCH: &str = "service test: ada's task under an epoch the bot never had";
const BOB_S: &str = "service test: bob asks the bot to rotate the deploy keys";

/// An agent of Ada's that is no agent of the bot's device.
const GEMINI: &str = "gemini-p4r8vd";

/// The template the task names, and an epoch the channel never had.
const REVIEW: &str = "review";
const NO_SUCH_EPOCH: u64 = 99;

/// The talk of Ada and Bob with `ci/deploy-bot`, which Ada's machine created as a member of the
/// talk's channel with the permissions `send`, `receive` and `queues`, in the channel's group.
fn bot_with_queues(ids: [u64; 2], logins: [&str; 2]) -> Bot {
    let permissions = [
        MEMBER, PERMISSION, SEND, PERMISSION, RECEIVE, PERMISSION, QUEUES,
    ];
    let (talk, secrets) = talk_with_bot(ids, logins, &permissions);
    let (device, _) = service_of(talk.cloud(), &talk.ada);
    let runner = Runner::of(&secrets);
    let mut bot = Bot {
        key: key_of(&secrets.key),
        talk,
        secrets,
        device,
        address: bot_address(),
        runner,
    };
    bot.brought_in();
    bot
}

/// Runs the bot's `skald claim --channel <channel>`.
fn bot_claims(bot: &Bot, at: (SpaceId, ChannelId)) -> String {
    let channel = at.1.to_string();
    run(&bot.runner.machine, &[CLAIM, CHANNEL, &channel]).shown()
}

/// Ada's first device writes three tasks to the queue `default`: one naming the template `review`,
/// which a compromised server gives as naming none; one for the bot's device whose `to` is Ada's
/// `gemini-p4r8vd`; and one the server gives under epoch 99. The bot's claims show none of their
/// texts; the server gives each `queued` with attempts 1, its last claim the bot's device's.
#[test]
fn member_service_device_fails_with_retry_the_tasks_a_client_releases() {
    let mut bot = bot_with_queues(
        [4_267_091_001, 4_267_091_002],
        ["ada-runs-a-member-bot", "bob-watches-the-bot-fail"],
    );
    bot.bob_takes_in();
    let talk = &bot.talk;
    let (cloud, ada, at) = (talk.cloud(), &talk.ada, talk.at());
    let queue = default_queue(cloud, ada, at);
    let person = talk.address(ada);
    let signer = (&ada.key, ada.device);
    let bob = &talk.played[0];
    let mut templated = Craft::new(&person, queue, TEMPLATED);
    templated.sched.template = Some(REVIEW.parse().expect("a name"));
    let mut for_gemini = Craft::new(&person, queue, FOR_GEMINI);
    for_gemini.sched.device = Some(bot.device);
    for_gemini.to = Some(format!("{ACME}/{}/{GEMINI}@{HOST}", ada.handle));
    let crafts = [
        templated,
        for_gemini,
        Craft::new(&person, queue, NEXT_EPOCH),
    ];
    let records: Vec<_> = (crafts.iter())
        .map(|craft| {
            let record = sealed_by(bob, at, signer, craft);
            uploads(cloud, ada, at, &record);
            record
        })
        .collect();
    tampers(cloud, records[0].task, "template = NULL");
    let mut next_epoch = records[2].clone();
    next_epoch.epoch = NO_SUCH_EPOCH;
    gives_bytes(cloud, &next_epoch);

    let shown: String = (0..crafts.len())
        .map(|_| bot_claims(&bot, at))
        .collect::<Vec<_>>()
        .join("\n");

    for (record, craft) in records.iter().zip(&crafts) {
        let text = &craft.text;
        assert!(!shown.contains(text), "{text:?} handed to the bot: {shown}");
        let read = read(cloud, ada, at, (queue, record.task));
        assert_eq!(read.summary(), (QUEUED, None, 1), "{text:?}: {read:?}");
        assert_eq!(
            read.held().device,
            bot.device,
            "{text:?} not claimed: {read:?}"
        );
    }
}

/// Bob's device writes a task, `peer` for the bot's device, which the server stores. The relay
/// answers the bot's next claim itself with a claim of that task, as a compromised server would
/// hand it to a claim stating only `own`. The bot's run shows none of its text; the bot fails it
/// and sends no release of it.
#[test]
fn member_service_device_fails_a_peer_task_handed_to_an_own_claim() {
    let mut bot = bot_with_queues(
        [4_267_092_001, 4_267_092_002],
        ["ada-runs-the-bot-for-bob", "bob-writes-to-the-bot"],
    );
    bot.bob_takes_in();
    let talk = &bot.talk;
    let (cloud, at) = (talk.cloud(), talk.at());
    let queue = default_queue(cloud, &talk.ada, at);
    let bob = &talk.played[0];
    let craft = Craft::new(&talk.address(&bob.person), queue, BOB_S);
    let record = sealed_by(bob, at, (&bob.person.key, bob.device()), &craft);
    uploads(cloud, &bob.person, at, &record);
    let since = hands_out(cloud, at, bob.device(), &record);

    let shown = bot_claims(&bot, at);

    cloud.relay.stop_refusing();
    assert!(!shown.contains(BOB_S), "handed to the bot: {shown}");
    let requests = requests_naming(cloud, since, record.task);
    assert!(sent_act(&requests, "fail"), "not failed: {requests:?}");
    assert!(!sent_act(&requests, "release"), "released: {requests:?}");
}
