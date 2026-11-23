//! What the tests share: Bob's space, whose channels hold Ada's two devices and Carol's, Ada's
//! deletion of her account, what the server and a compromised one report of it, and the lines the
//! CLI shows of it.

use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use skald_crypto::{MlsFraming, sign_anchors};
use skald_proto::{
    AccountId, AnchorsBody, Cbor as _, ChannelId, DeviceId, MessageId, Role, SpaceId, Unknown,
};

use crate::cloud::{Cloud, WAIT, agent};
use crate::fixtures::fronted;
use crate::front::Front;
use crate::kit::{Space, assert_members, create_space, group, latest, received, with_channel};
use crate::linking::{NewDevice, link};
use crate::machine::{Finished, Machine};
use crate::played::Played;
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST, is_commit, messages_sent, sent_in};
use crate::world::{ACME, Person, World, devices_request, post_as, words};

/// The host name of Ada's second machine, its certificate's `name`.
pub const LAPTOP: &str = "ada-laptop";

/// What the CLI says of a member the server reports deleted.
pub const DELETED: &str = "deleted";

/// The request that deletes the session's account.
pub const DELETE_ACCOUNT: &str = "/account";

/// How many rounds of runs the setup gives the devices to get into the channels.
const ROUNDS: usize = 6;

/// Bob, whose machine runs the CLI and created `acme`; Ada, with her first device and her second,
/// linked with a link code; Carol; and a channel of `acme` with all of them, whom the revision that
/// adds it adds to the default channel too. Each device is in both groups.
pub struct Scene {
    pub world: World,
    pub front: Front,
    pub bob: Person,
    /// Ada's first device, then her second.
    pub ada: Vec<Played>,
    pub carol: Played,
    pub space: Space,
    pub channel: ChannelId,
}

impl Scene {
    /// Bob, Ada and Carol, GitHub users `ids` with `logins`, Ada a member with `role`; the devices
    /// of Ada and Carol played.
    pub fn new(ids: [u64; 3], logins: [&str; 3], role: Role) -> Self {
        let (world, front) = fronted();
        let cloud = &world.cloud;
        let bob = world.sign_up(ids[0], logins[0], BOB_HOST);
        let ada = world.sign_up(ids[1], logins[1], ADA_HOST);
        let laptop = linked(&world, &ada, LAPTOP);
        let carol = world.sign_up(ids[2], logins[2], CAROL_HOST);
        let space = create_space(cloud, &bob, ACME);
        let channel = with_channel(
            cloud,
            space.id,
            &bob,
            &[(&ada, role), (&carol, Role::Member)],
        );
        let mut ada = vec![Played::new(cloud, ada), Played::new(cloud, laptop)];
        let mut carol = Played::new(cloud, carol);
        received(&bob.machine);
        let devices = [bob.device, ada[0].device(), ada[1].device(), carol.device()];
        for one in ada.iter_mut().chain([&mut carol]) {
            one.take_in(cloud);
            for of in [space.default, channel] {
                assert_members(&one.members(of), &devices);
            }
        }
        Scene {
            world,
            front,
            bob,
            ada,
            carol,
            space,
            channel,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// The space's channels: the default one, then the other.
    pub fn channels(&self) -> [ChannelId; 2] {
        [self.space.default, self.channel]
    }

    /// Ada's devices.
    pub fn ada_devices(&self) -> Vec<DeviceId> {
        self.ada.iter().map(Played::device).collect()
    }

    /// Ada's first device's person.
    pub fn ada(&self) -> &Person {
        &self.ada[0].person
    }

    /// Fails unless the server's GroupInfo of each channel holds `devices` alone.
    #[track_caller]
    pub fn assert_groups(&self, devices: &[DeviceId]) {
        for channel in self.channels() {
            let held = group(self.cloud(), &self.bob.token, self.space.id, channel);
            assert_members(&held.members, devices);
        }
    }
}

/// A second machine of `first`'s account, with the host name `host`, linked with a link code as a
/// person links one: it signs in at a terminal and shows its code, which the person types into
/// `skald link` on `first`'s machine.
#[track_caller]
pub fn linked(world: &World, first: &Person, host: &'static str) -> Person {
    let mut new = NewDevice::start(world, first, host);
    link(&world.cloud, first, &mut new);
    new.into_person(&world.cloud, first)
}

/// `person`'s session deletes its account through the API; fails unless the server deletes it.
#[track_caller]
pub fn delete_account(cloud: &Cloud, person: &Person) {
    let mut response = agent()
        .delete(format!("{}{DELETE_ACCOUNT}", cloud.url))
        .header(
            skald_proto::VERSION_HEADER,
            skald_proto::PROTOCOL_VERSION.to_string(),
        )
        .header("authorization", format!("Bearer {}", person.token))
        .call()
        .unwrap_or_else(|err| panic!("DELETE {DELETE_ACCOUNT}: {err}"));
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    assert_eq!(status, 204, "the account is not deleted: {body}");
}

/// The accounts the server reports deleted among those of `space`, to the session `token`.
#[track_caller]
pub fn reported_deleted(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<AccountId> {
    let path = format!("/spaces/{space}/devices");
    let (status, body) = cloud.get_as(&path, token);
    assert_eq!(status, 200, "GET {path}: {body}");
    (body["deleted"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .map(|id| id.parse().expect("an account ID"))
        .collect()
}

/// From now on, the relay reports `account` deleted among the accounts of `space`, as a
/// compromised server would; until [`stop_reporting`].
pub fn report_deleted(cloud: &Cloud, space: SpaceId, account: AccountId) {
    cloud.relay.rewrite(&devices_request(space), move |body| {
        let mut list: Value = serde_json::from_str(body).expect("the server's device list");
        let deleted = list
            .as_object_mut()
            .expect("a device list")
            .entry("deleted")
            .or_insert_with(|| json!([]));
        if let Some(ids) = deleted.as_array_mut() {
            ids.push(Value::String(account.to_string()));
        }
        list.to_string()
    });
}

/// From now on, the relay gives the devices of `space`'s accounts as the server gives them.
pub fn stop_reporting(cloud: &Cloud, space: SpaceId) {
    cloud.relay.rewrite(&devices_request(space), str::to_owned);
}

/// The lines of `shown` that tell that the member `handle` is deleted: holding, as words, the
/// handle and `deleted`.
pub fn told_deleted(shown: &Finished, handle: &str) -> Vec<String> {
    shown
        .shown()
        .lines()
        .filter(|line| {
            let held = words(line);
            held.contains(&handle) && line.to_lowercase().contains(DELETED)
        })
        .map(str::to_owned)
        .collect()
}

/// Fails unless the first of `runs`, runs of `skald recv` or `skald send` on one device, tells that
/// the member `handle` is deleted on one line, and no other run tells it again.
#[track_caller]
pub fn assert_told_once(runs: &[&Finished], handle: &str) {
    let told: Vec<usize> = runs
        .iter()
        .map(|run| told_deleted(run, handle).len())
        .collect();
    let mut expected = vec![0; runs.len()];
    expected[0] = 1;
    assert_eq!(
        told, expected,
        "the lines telling that {handle} is deleted, run by run: {runs:?}"
    );
}

/// Fails unless `shown`, a run of `skald members`, lists `handle` marked deleted, and none of
/// `others` so.
#[track_caller]
pub fn assert_marked_deleted(shown: &Finished, handle: &str, others: &[&str]) {
    assert!(shown.success, "{shown:?}");
    let marked = |name: &str| {
        let lines: Vec<&str> = (shown.stdout.lines())
            .filter(|line| words(line).contains(&name))
            .collect();
        assert!(!lines.is_empty(), "{name} is not listed: {shown:?}");
        lines
            .iter()
            .any(|line| line.to_lowercase().contains(DELETED))
    };
    assert!(marked(handle), "{handle} is not marked deleted: {shown:?}");
    for other in others {
        assert!(!marked(other), "{other} is marked deleted: {shown:?}");
    }
}

/// The Commits of `channel` the front saw sent after its first `from` sends, each by the MLS epoch
/// it was sent in.
pub fn commits_sent(front: &Front, channel: ChannelId, from: usize) -> Vec<u64> {
    (sent_in(front, channel, from).iter())
        .map(|sent| sent.envelope.envelope().clone())
        .filter(is_commit)
        .map(|envelope| epoch_of(&envelope.payload))
        .collect()
}

/// The MLS epoch of an `mls` payload; 0 if it has none.
pub fn epoch_of(payload: &[u8]) -> u64 {
    match MlsFraming::read(payload) {
        Some(MlsFraming::Message { epoch, .. }) => epoch,
        _ => 0,
    }
}

/// An anchors object of `person`'s account by its device, anchoring `space` at its latest revision.
/// `person`'s session stores it; fails unless the server takes it.
#[track_caller]
pub fn anchors_space(cloud: &Cloud, person: &Person, space: SpaceId) {
    let revision = latest(cloud, &person.token, space);
    let body = AnchorsBody {
        account: person.account,
        device: person.device,
        anchors: [(space, revision.hash())].into_iter().collect(),
        personal: None,
        unknown: Unknown::default(),
    };
    let object = sign_anchors(&body, &person.key).expect("an anchors object");
    let request = json!({ "anchors": URL_SAFE_NO_PAD.encode(object.to_cbor()) });
    let (status, answer) = post_as(cloud, &person.token, "/account/anchors", Some(&request));
    assert!(
        (200..300).contains(&status),
        "the anchors object is not stored: {status} {answer}"
    );
}

/// Runs `skald recv` on each of `machines` and has each of `played` take in its mailbox, round by
/// round, until the server's GroupInfo of each of `channels` of `space` holds `devices` alone;
/// fails after [`ROUNDS`] rounds or the tests' timeout.
#[track_caller]
pub fn settle(
    cloud: &Cloud,
    (space, channels): (SpaceId, &[ChannelId]),
    (machines, played): (&[&Machine], &mut [&mut Played]),
    devices: &[DeviceId],
) {
    let deadline = Instant::now() + WAIT;
    let token = played[0].person.token.clone();
    let mut wanted = devices.to_vec();
    wanted.sort();
    let mut last = Vec::new();
    for _ in 0..ROUNDS {
        for machine in machines {
            last.push(received(machine));
        }
        for one in played.iter_mut() {
            one.take_in(cloud);
        }
        let done = channels.iter().all(|channel| {
            let mut held = group(cloud, &token, space, *channel).members;
            held.sort();
            held == wanted
        });
        if done {
            return;
        }
        assert!(Instant::now() < deadline, "not settled: {last:?}");
        thread::sleep(Duration::from_millis(200));
    }
    panic!("the channels never held {devices:?}: {last:?}");
}

/// How long a run takes to act on a frame the front pushed, at most.
pub const PUSHED: Duration = Duration::from_millis(500);

impl Scene {
    /// Fails if the front saw Bob's machine send a Commit in either channel after its first `from`
    /// sends; `runs` are Bob's runs since.
    #[track_caller]
    pub fn assert_no_commit(&self, from: usize, runs: &[&Finished]) {
        for channel in self.channels() {
            let committed = commits_sent(&self.front, channel, from);
            assert!(committed.is_empty(), "Bob committed in {channel}: {runs:?}");
        }
    }
}

/// Fails unless `played` read the line of the envelope `msg_id` in `channel`, from `sender`.
#[track_caller]
pub fn assert_read_from(played: &Played, channel: ChannelId, msg_id: MessageId, sender: DeviceId) {
    let read = played.read_in(channel);
    let found = (read.iter()).find(|(id, _)| *id == msg_id);
    assert_eq!(
        found.map(|(_, decrypted)| decrypted.sender),
        Some(sender),
        "{} did not read {msg_id} from {sender}: {read:?}",
        played.device()
    );
}

/// The ID of the one message the front saw sent in `channel` after its first `from` sends; fails
/// unless there is one.
#[track_caller]
pub fn one_message_sent(front: &Front, channel: ChannelId, from: usize) -> MessageId {
    let said = messages_sent(front, channel, from);
    assert_eq!(said.len(), 1, "not one message sent in {channel}: {said:?}");
    said[0].envelope.envelope().header.msg_id
}
