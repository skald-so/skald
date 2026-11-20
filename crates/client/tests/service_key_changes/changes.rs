//! What the tests share: the commands that replace a service agent's key and remove an agent, a
//! service agent of Bob's account set up by hand, the service devices of an account and its
//! revocations, and what a revision lists of a service device.

use skald_proto::{ChannelId, DeviceId, DeviceKind, Roster, ServiceAgent, SpaceId, Unknown};

use crate::cloud::Cloud;
use crate::fixtures::{Issuer, add_device_of_kind};
use crate::kit::latest;
use crate::machine::Finished;
use crate::revocations::account_revocations;
use crate::service_kit::{AGENT_NAME, BOT, OWNER, run_for_channels};
use crate::world::{ACME, Person, next_with, run, store};

/// The commands: replace a service agent's key; remove a service agent.
pub const REPLACE_SERVICE_KEY: &str = "replace-service-key";
pub const REMOVE_SERVICE_AGENT: &str = "remove-service-agent";

/// The requests that store a revocation, as their lines start.
pub const STORE_REVOCATION: &str = "POST /account/revocations";

/// How many rounds of runs the agents and the members' devices get to bring a group in line.
pub const ROUNDS: usize = 4;

/// Runs `skald replace-service-key acme ci/deploy-bot --channel <channel>...` on `person`'s machine.
pub fn replace_key(person: &Person, channels: &[ChannelId]) -> Finished {
    let command = (REPLACE_SERVICE_KEY, ACME, BOT);
    run_for_channels(&person.machine, command, channels, &[])
}

/// Runs `skald remove-service-agent acme ci/deploy-bot` on `person`'s machine.
pub fn remove_agent(person: &Person) -> Finished {
    run(&person.machine, &[REMOVE_SERVICE_AGENT, ACME, BOT])
}

/// A `service` device of `owner`'s account, certified by its device and set up in the server's
/// database, and the next revision of `space`, which `admin`'s device signs and the server stores,
/// with its entry `ci/deploy-bot` and the device in the `services` of `channel`. Gives the device.
#[track_caller]
pub fn agent_of(
    cloud: &Cloud,
    (owner, admin): (&Person, &Person),
    space: SpaceId,
    channel: ChannelId,
) -> DeviceId {
    let device = add_device_of_kind(
        cloud,
        (owner.account, DeviceKind::Service),
        "bob-deploy-bot",
        &Issuer::Device(owner),
    )
    .id;
    let name = |text: &str| text.parse().expect("a name");
    let added = next_with(
        &latest(cloud, &admin.token, space),
        (admin.device, &admin.key),
        |body| {
            body.service_agents.push(ServiceAgent {
                device,
                owner: name(OWNER),
                agent: name(AGENT_NAME),
                unknown: Unknown::default(),
            });
            let entry = body.channels.iter_mut().find(|it| it.id == channel);
            entry.expect("the channel").services.push(device);
        },
    );
    store(cloud, &admin.token, &added);
    device
}

/// The devices of `person`'s account beside its first one, as the server gives them.
pub fn other_devices(cloud: &Cloud, person: &Person) -> Vec<DeviceId> {
    (cloud.devices(&person.token).into_iter())
        .map(|device| device.id)
        .filter(|id| *id != person.device)
        .collect()
}

/// The one device of `person`'s account beside its first one and `old`; fails unless there is
/// exactly one.
#[track_caller]
pub fn new_device(cloud: &Cloud, person: &Person, old: DeviceId) -> DeviceId {
    let others: Vec<DeviceId> = other_devices(cloud, person)
        .into_iter()
        .filter(|id| *id != old)
        .collect();
    match others.as_slice() {
        [one] => *one,
        other => panic!("not one new device of {}: {other:?}", person.handle),
    }
}

/// Whether the server holds a revocation of `device` on `person`'s account.
pub fn revoked(cloud: &Cloud, person: &Person, device: DeviceId) -> bool {
    account_revocations(cloud, &person.token)
        .iter()
        .any(|revocation| revocation.body().device == device)
}

/// Whether a channel of `roster` lists `device` in its `services`.
pub fn in_services(roster: &Roster, device: DeviceId) -> bool {
    (roster.body().channels.iter()).any(|channel| channel.services.contains(&device))
}

/// The `services` of the channel `id` in `roster`.
#[track_caller]
pub fn services_of(roster: &Roster, id: ChannelId) -> Vec<DeviceId> {
    (roster.body().channels.iter())
        .find(|channel| channel.id == id)
        .unwrap_or_else(|| panic!("no channel {id} in {roster:?}"))
        .services
        .clone()
}

/// Whether `roster` lists `device` anywhere a service device goes: in a service agent entry or in
/// a channel's `services`.
pub fn lists(roster: &Roster, device: DeviceId) -> bool {
    (roster.body().service_agents.iter()).any(|entry| entry.device == device)
        || in_services(roster, device)
}
