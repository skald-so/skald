//! A member agent reads its Commits and Welcomes from its mailbox, so the creation and token
//! commands refuse its token without the receive permission.

use crate::fixtures::fronted;
use crate::kit::latest;
use crate::member::{Bot, MEMBER};
use crate::service_kit::{
    Acme, BOT, CHANNEL, CREATE_SERVICE_TOKEN, PERMISSION, RECEIVE, SEND, Snapshot, assert_record,
    assert_refused_saying, create_agent, listed_ids, own_record, secrets_of, service_of, token_of,
};
use crate::world::{ACME, run};

/// The queues permission of a token.
const QUEUES: &str = "queues";

/// Ada, the admin of `acme`, runs `skald create-service-agent acme ci/deploy-bot --channel
/// <default> --member` with `--permission send`, then with `--permission send --permission
/// queues`: each is refused, saying `receive`, with nothing on stdout and nothing registered,
/// signed or stored. With `--permission send --permission receive` it then creates the member
/// agent: the latest revision lists its device in the default channel's `services`, and its token
/// has both permissions.
#[test]
fn creating_a_member_agent_without_the_receive_permission_is_refused() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(
        3_135_720_001,
        "ada-member-refused-standin",
        "ada-workstation",
    );
    let acme = Acme::create(cloud, &ada);
    let before = Snapshot::take(cloud, &ada, &ada, acme.id);

    for permissions in [
        &[PERMISSION, SEND][..],
        &[PERMISSION, SEND, PERMISSION, QUEUES],
    ] {
        let mut extra = vec![MEMBER];
        extra.extend_from_slice(permissions);
        let refused = create_agent(&ada.machine, ACME, BOT, &[acme.default], &extra);

        assert_refused_saying(&refused, RECEIVE);
        assert_eq!(
            Snapshot::take(cloud, &ada, &ada, acme.id),
            before,
            "a refused creation changed the server's records: {refused:?}"
        );
    }

    let extra = [MEMBER, PERMISSION, SEND, PERMISSION, RECEIVE];
    let created = create_agent(&ada.machine, ACME, BOT, &[acme.default], &extra);
    let secrets = secrets_of(&created);
    let (device, _) = service_of(cloud, &ada);
    let revision = latest(cloud, &ada.token, acme.id);
    let default = &revision.body().channels[0];
    assert_eq!(default.id, acme.default, "{revision:?}");
    assert_eq!(default.services, [device], "{revision:?}");
    let record = own_record(cloud, &secrets.token).expect("the server refuses the new token");
    assert_record(&record, device, &[SEND, RECEIVE], &[acme.default]);
}

/// With `ci/deploy-bot` a member agent of the talk's channel, Ada's `skald create-service-token acme
/// ci/deploy-bot --channel <channel>` with `--permission send`, then with `--permission send
/// --permission queues`, is refused, saying `receive`, with nothing on stdout: the space's tokens
/// stay as they were and the agent's token still reads its record. With `--permission send
/// --permission receive` it gives a new token.
#[test]
fn a_member_agent_s_new_token_without_the_receive_permission_is_refused() {
    let bot = Bot::created(
        [3_135_721_001, 3_135_721_002],
        ["ada-member-token-standin", "bob-member-token-standin"],
    );
    let (cloud, ada) = (bot.talk.cloud(), &bot.talk.ada);
    let channel = bot.channel().to_string();
    let before = listed_ids(cloud, ada, bot.talk.space.id);

    for permissions in [
        &[PERMISSION, SEND][..],
        &[PERMISSION, SEND, PERMISSION, QUEUES],
    ] {
        let mut args = vec![CREATE_SERVICE_TOKEN, ACME, BOT, CHANNEL, channel.as_str()];
        args.extend_from_slice(permissions);
        let refused = run(&ada.machine, &args);

        assert_refused_saying(&refused, RECEIVE);
        assert_eq!(
            listed_ids(cloud, ada, bot.talk.space.id),
            before,
            "a refused token's run changed the tokens: {refused:?}"
        );
        assert!(
            own_record(cloud, &bot.secrets.token).is_some(),
            "a refused token's run revoked the agent's token: {refused:?}"
        );
    }

    let args = [
        CREATE_SERVICE_TOKEN,
        ACME,
        BOT,
        CHANNEL,
        channel.as_str(),
        PERMISSION,
        SEND,
        PERMISSION,
        RECEIVE,
    ];
    let token = token_of(&run(&ada.machine, &args));
    assert!(
        own_record(cloud, &token).is_some(),
        "the server refuses the new token"
    );
}
