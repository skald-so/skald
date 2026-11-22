//! Creations the CLI refuses, saying why, before it registers, signs or stores anything: by an
//! account that is no admin, by an agent, for an address the space has, for an expiry in the past,
//! and in a space with as many service agents as the server allows.

use skald_proto::Role;

use crate::fixtures::{fronted, fronted_with};
use crate::kit::{
    Acme, BOT, CHANNEL, CREATE_SERVICE_AGENT, CREATE_SERVICE_TOKEN, EXPIRES, REVOKE_SERVICE_TOKEN,
    Snapshot, assert_refused_saying, create_agent, rosters, secrets_of, writes_since,
};
use crate::machine::Finished;
use crate::world::{
    ACME, ADMIN, AGENT, Person, World, create, dir_of, next_by, path, set_role, store,
};

/// Runs `create` on `person`'s machine, as it says, and fails unless it is refused saying `why`,
/// with no write to the server and nothing changed of `admin`'s view of `acme`.
#[track_caller]
fn assert_refused_creation(
    world: &World,
    person: &Person,
    admin: &Person,
    acme: &Acme,
    why: &str,
    create: impl FnOnce() -> Finished,
) {
    let cloud = &world.cloud;
    let before = Snapshot::take(cloud, person, admin, acme.id);
    let since = cloud.relay.mark();

    let refused = create();

    assert_refused_saying(&refused, why);
    assert_eq!(
        writes_since(cloud, since, acme.id),
        0,
        "the refused run wrote to the server: {:?}",
        cloud.relay.sent_since(since)
    );
    assert_eq!(
        Snapshot::take(cloud, person, admin, acme.id),
        before,
        "{refused:?}"
    );
}

/// `acme`, which `ada`'s machine creates, with `ci/deploy-bot` that it creates next.
fn with_bot(world: &World, ada: &Person) -> Acme {
    let cloud = &world.cloud;
    let acme = Acme::create(cloud, ada);
    secrets_of(&create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]));
    Acme {
        revisions: rosters(cloud, &ada.token, acme.id),
        ..acme
    }
}

/// Bob created `acme` and made Ada an admin; Ada then made Bob a member. Bob's creation of
/// `ci/deploy-bot` is refused as he is no admin.
#[test]
fn account_that_is_no_admin_cannot_create_a_service_agent() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_311_001, "ada-noadmin-standin", "ada-workstation");
    let bob = world.sign_up(3_135_311_002, "bob-noadmin-standin", "bob-laptop");
    let second = create(cloud, &bob, ACME);
    let third = next_by(&second, &bob, |members| {
        members.push(ada.member(Role::Admin))
    });
    store(cloud, &bob.token, &third);
    let fourth = next_by(&third, &ada, |members| {
        set_role(members, bob.account, Role::Member);
    });
    store(cloud, &ada.token, &fourth);
    let acme = Acme {
        id: second.body().space,
        default: second.body().channels[0].id,
        revisions: vec![second.clone(), third, fourth],
    };

    assert_refused_creation(&world, &bob, &ada, &acme, ADMIN, || {
        create_agent(&bob.machine, ACME, BOT, &[acme.default], &[])
    });
}

/// As an agent, Ada's machine refuses to create `ci/deploy-bot`, a token of it, or to revoke one:
/// each is the person's command, which the refusal names.
#[test]
fn agent_cannot_create_a_service_agent_or_its_tokens() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_311_101, "ada-asagent-standin", "ada-workstation");
    let acme = Acme::create(cloud, &ada);
    let default = acme.default.to_string();
    let folder = dir_of(&ada.machine, "work");
    let token_id = "00000000-0000-4000-8000-000000000000";
    let commands: [&[&str]; 3] = [
        &[CREATE_SERVICE_AGENT, ACME, BOT, CHANNEL, &default],
        &[CREATE_SERVICE_TOKEN, ACME, BOT, CHANNEL, &default],
        &[REVOKE_SERVICE_TOKEN, ACME, token_id],
    ];
    for args in commands {
        let mut all = vec![AGENT, "claude-s3rv1ce"];
        all.extend_from_slice(args);
        let named = format!("skald {}", args[0]);
        assert_refused_creation(&world, &ada, &ada, &acme, "person's command", || {
            let refused = ada.machine.start_in(&folder, &all).finish();
            let said = refused.stderr.to_lowercase();
            assert!(
                said.contains(&named) && said.contains("an agent cannot run"),
                "{args:?} in {}: {refused:?}",
                path(&folder)
            );
            refused
        });
    }
}

/// After Ada creates `ci/deploy-bot`, creating it again is refused, as is `<Ada's handle>/deploy-bot`,
/// an address of her own agents: the space's latest revision has both.
#[test]
fn address_the_space_has_is_refused() {
    let (world, _front) = fronted();
    let ada = world.sign_up(3_135_311_201, "ada-taken-standin", "ada-workstation");
    let acme = with_bot(&world, &ada);
    let handles = format!("{}/deploy-bot", ada.handle);

    for address in [BOT, handles.as_str()] {
        assert_refused_creation(&world, &ada, &ada, &acme, "deploy-bot", || {
            create_agent(&ada.machine, ACME, address, &[acme.default], &[])
        });
    }
}

/// A creation whose grant would expire in the past is refused, saying so.
#[test]
fn expiry_in_the_past_is_refused() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_311_301, "ada-expired-standin", "ada-workstation");
    let acme = Acme::create(cloud, &ada);

    assert_refused_creation(&world, &ada, &ada, &acme, "expir", || {
        let past = [EXPIRES, "2020-01-01T00:00:00Z"];
        create_agent(&ada.machine, ACME, BOT, &[acme.default], &past)
    });
}

/// On a server that allows a space one service agent, Ada creates `ci/deploy-bot`; creating
/// `ci/release-bot` then is refused, saying the space has as many service agents as it may.
#[test]
fn creation_in_a_space_at_the_service_agents_limit_is_refused() {
    let (world, _front) = fronted_with(|settings| settings.service_agents_limit = Some(1));
    let ada = world.sign_up(3_135_311_401, "ada-limit-standin", "ada-workstation");
    let acme = with_bot(&world, &ada);

    assert_refused_creation(
        &world,
        &ada,
        &ada,
        &acme,
        "as many service agents as it may",
        || create_agent(&ada.machine, ACME, "ci/release-bot", &[acme.default], &[]),
    );
}
