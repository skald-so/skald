//! Tokens of a service agent in the CLI: a new token revokes the agent's earlier ones unless they
//! are kept; kept tokens are listed and work until revoked.

use std::path::PathBuf;

use crate::cloud::Cloud;
use crate::fixtures::{fronted, last_channel};
use crate::kit::{
    Acme, BOT, CREATE_SERVICE_TOKEN, EXPIRES, KEEP_EARLIER, PERMISSION, RECEIVE,
    REVOKE_SERVICE_TOKEN, SEND, assert_grant, assert_record, cli_lines_of, create_agent, grant_of,
    id_of, listed_ids, own_record, run_for_channels, secrets_of, service_of, token_of,
};
use crate::world::{ACME, CREATE_CHANNEL, Person, World, run};

/// Ada's `acme`, with a second channel, and `ci/deploy-bot`, which she created for the default
/// channel: its first token and that token's record.
struct Bot {
    acme: Acme,
    second: skald_proto::ChannelId,
    token: String,
    record: serde_json::Value,
}

impl Bot {
    fn create(world: &World, ada: &Person) -> Self {
        let cloud = &world.cloud;
        let acme = Acme::create(cloud, ada);
        let added = run(&ada.machine, &[CREATE_CHANNEL, ACME]);
        assert!(added.success, "{added:?}");
        let (_, second) = last_channel(cloud, ada, acme.id);
        assert_ne!(second, acme.default, "no second channel");
        let secrets = secrets_of(&create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]));
        let record = own_record(cloud, &secrets.token).expect("the server refuses the first token");
        Bot {
            acme,
            second,
            token: secrets.token,
            record,
        }
    }

    /// Runs `skald create-service-token acme ci/deploy-bot` on `ada`'s machine for both channels,
    /// expiring in 30 days, with only `send`, and `extra`; gives the new token.
    fn new_token(&self, ada: &Person, extra: &[&str]) -> String {
        let command = (CREATE_SERVICE_TOKEN, ACME, BOT);
        let channels = [self.acme.default, self.second];
        let options = [&[EXPIRES, "30d", PERMISSION, SEND][..], extra].concat();
        let token = token_of(&run_for_channels(
            &ada.machine,
            command,
            &channels,
            &options,
        ));
        assert_eq!(
            ada.machine.files_holding(&token),
            Vec::<PathBuf>::new(),
            "files that hold the new token"
        );
        token
    }

    /// Fails unless the server and the CLI list the token `new` of `acme` to `ada`, and not
    /// `earlier`.
    #[track_caller]
    fn assert_only_listed(&self, cloud: &Cloud, ada: &Person, new: &str, earlier: &str) {
        assert_eq!(
            listed_ids(cloud, ada, self.acme.id),
            [new],
            "the server's list"
        );
        let lines = cli_lines_of(&ada.machine, &[new, earlier]);
        assert!(
            lines[0].is_some() && lines[1].is_none(),
            "the CLI's list: {lines:?}"
        );
    }
}

/// A new token of `ci/deploy-bot`, for both channels, for 30 days, with only `send`, works with a
/// new grant as asked, and revokes the first: the server refuses it, and the CLI and the server
/// list only the new one.
#[test]
fn new_token_revokes_the_earlier_one() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_312_001, "ada-renew-standin", "ada-workstation");
    let bot = Bot::create(&world, &ada);
    let (device, _) = service_of(cloud, &ada);
    let channels = [bot.acme.default, bot.second];

    let token = bot.new_token(&ada, &[]);

    let record = own_record(cloud, &token).expect("the server refuses the new token");
    assert_record(&record, device, &[SEND], &channels);
    let grant = grant_of(&record);
    assert_grant(&grant, device, &channels, &ada, 30);
    assert_ne!(grant, grant_of(&bot.record), "the first token's grant");
    assert_eq!(
        own_record(cloud, &bot.token),
        None,
        "the first token still works"
    );
    let (first, new) = (id_of(&bot.record), id_of(&record));
    bot.assert_only_listed(cloud, &ada, &new, &first);
}

/// With `--keep-earlier`, both tokens of `ci/deploy-bot` work and both are listed; once the CLI
/// revokes the first, the server refuses it, and only the new one is listed.
#[test]
fn kept_token_is_listed_until_revoked() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_312_101, "ada-keep-standin", "ada-workstation");
    let bot = Bot::create(&world, &ada);
    let (device, _) = service_of(cloud, &ada);

    let token = bot.new_token(&ada, &[KEEP_EARLIER]);

    let record = own_record(cloud, &token).expect("the server refuses the new token");
    let kept = own_record(cloud, &bot.token).expect("the kept token is refused");
    assert_record(&kept, device, &[SEND, RECEIVE], &[bot.acme.default]);
    let (first, new) = (id_of(&bot.record), id_of(&record));
    let mut both = vec![first.clone(), new.clone()];
    both.sort();
    assert_eq!(listed_ids(cloud, &ada, bot.acme.id), both);
    let lines = cli_lines_of(&ada.machine, &[&new, &first]);
    assert!(
        lines.iter().all(Option::is_some),
        "the CLI's list: {lines:?}"
    );

    let revoked = run(&ada.machine, &[REVOKE_SERVICE_TOKEN, ACME, &first]);

    assert!(revoked.success, "{revoked:?}");
    assert_eq!(
        own_record(cloud, &bot.token),
        None,
        "the revoked token still works"
    );
    assert!(
        own_record(cloud, &token).is_some(),
        "the new token is refused"
    );
    bot.assert_only_listed(cloud, &ada, &new, &first);
}
