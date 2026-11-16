//! What the tests share: spaces Ada's machine creates and their revisions on the server, fixture
//! revisions, a channel's group as the server gives its GroupInfo, and what `skald channels` and
//! the warnings show.

use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use skald_crypto::{ChannelView, accept_next};
use skald_proto::{
    AccountId, Channel, ChannelId, ChannelMember, DeviceId, Role, Roster, RosterBody, SpaceId,
    Unknown,
};

use crate::cloud::{Cloud, unb64};
use crate::machine::{Finished, Machine};
use crate::world::{
    ADD_RULE, AGENT, CHANNELS, CLIENT_A, CREATE_SPACE, Person, dir_of, lines_naming, next_with,
    path, post_as, run, space_certs, space_id, store, warnings, words,
};

/// The command of a run that connects and goes through the mailbox.
pub const RECV: &str = "recv";

/// What `skald channels` says on the line of a channel the device is admitted to but not in.
pub const NOT_IN: &str = "not in";

/// The problem of a Commit of an epoch that is not the channel's current one.
pub const EPOCH_MISMATCH: &str = "epoch-mismatch";

/// Runs `skald recv` on `machine`; fails unless it succeeds.
#[track_caller]
pub fn received(machine: &Machine) -> Finished {
    let finished = run(machine, &[RECV]);
    assert!(finished.success, "{finished:?}");
    finished
}

/// Binds `~/work/clientA` on `machine` to `clienta`; fails unless the binding succeeds. Gives the
/// folder.
#[track_caller]
pub fn bind_client_a(machine: &Machine) -> PathBuf {
    let folder = dir_of(machine, "work/clientA");
    let bound = run(machine, &[ADD_RULE, path(&folder), CLIENT_A]);
    assert!(bound.success, "not bound: {bound:?}");
    folder
}

/// Runs `skald --agent <agent> recv` on `machine` in `folder`; fails unless it succeeds.
#[track_caller]
pub fn received_as_agent(machine: &Machine, folder: &Path, agent: &str) -> Finished {
    let finished = machine.start_in(folder, &[AGENT, agent, RECV]).finish();
    assert!(finished.success, "{finished:?}");
    finished
}

/// Fails if the output of `agent`, a run as an agent, names one of `names`, in any case.
#[track_caller]
pub fn assert_names_none(agent: &Finished, names: &[String]) {
    let shown = agent.shown().to_lowercase();
    let named: Vec<&String> = (names.iter())
        .filter(|name| shown.contains(&name.to_lowercase()))
        .collect();
    assert!(
        named.is_empty(),
        "the agent's output names {named:?}: {agent:?}"
    );
}

/// Runs `skald channels <space>` on `machine`; fails unless it succeeds.
#[track_caller]
pub fn channels(machine: &Machine, space: &str) -> Finished {
    let listed = run(machine, &[CHANNELS, space]);
    assert!(listed.success, "{listed:?}");
    listed
}

/// The revisions of `space` the server holds, for the session `token`.
pub fn rosters(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<Roster> {
    let held = cloud.revisions(token, space);
    held.into_iter().map(|pair| pair.1).collect()
}

/// The last revision of `space` the server holds, for the session `token`.
pub fn latest(cloud: &Cloud, token: &str, space: SpaceId) -> Roster {
    rosters(cloud, token, space)
        .pop()
        .expect("a revision of the space")
}

/// A space of Ada's: its ID, name and default channel.
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    pub default: ChannelId,
}

/// `person`'s machine creates the space `name`, which then has revision 1 and revision 2 with its
/// default channel; gives the space.
#[track_caller]
pub fn create_space(cloud: &Cloud, person: &Person, name: &str) -> Space {
    let created = run(&person.machine, &[CREATE_SPACE, name]);
    assert!(created.success, "{name}: {created:?}");
    let id = space_id(cloud, &person.token, name);
    let held = rosters(cloud, &person.token, id);
    assert_eq!(
        held.len(),
        2,
        "{name}: not revision 1 and the one with the default channel: {held:?}"
    );
    let default = assert_adds_default(cloud, &held[0], &held[1], person);
    Space {
        id,
        name: name.to_owned(),
        default,
    }
}

/// From now on, the server's database refuses every revision but revision 1, as a failing
/// database would.
pub fn refuse_later_revisions(cloud: &Cloud) {
    cloud.execute(
        "CREATE FUNCTION skald_test_first_only() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN
                IF NEW.revision > 1 THEN RAISE EXCEPTION 'the database is down'; END IF;
                RETURN NEW;
              END $$;
         CREATE TRIGGER skald_test_first_only BEFORE INSERT ON roster_revisions
           FOR EACH ROW EXECUTE FUNCTION skald_test_first_only();",
    );
}

/// From now on, the server's database stores every revision again.
pub fn store_later_revisions(cloud: &Cloud) {
    cloud.execute(
        "DROP TRIGGER skald_test_first_only ON roster_revisions;
         DROP FUNCTION skald_test_first_only();",
    );
}

/// The local state of `machine` keeps no revision of `space` waiting for the server, as a client
/// from before default channels left a space it created.
pub fn forget_waiting(machine: &Machine, space: SpaceId) {
    let state = rusqlite::Connection::open(machine.state_dir().join("state.sqlite3"))
        .expect("the machine's local state");
    state
        .execute(
            "UPDATE space SET pending = NULL WHERE space = ?1",
            rusqlite::params![space.to_string()],
        )
        .expect("forget the waiting revision");
}

/// `person`'s machine creates the space `name` as one made before default channels: the server
/// holds its revision 1 alone, and the machine waits to send no other. Gives the space's ID and
/// revision 1.
#[track_caller]
pub fn create_older_space(cloud: &Cloud, person: &Person, name: &str) -> (SpaceId, Roster) {
    refuse_later_revisions(cloud);
    // The run may fail, or warn, as the server refuses the revision with the default channel.
    let _ = run(&person.machine, &[CREATE_SPACE, name]);
    store_later_revisions(cloud);
    let id = space_id(cloud, &person.token, name);
    forget_waiting(&person.machine, id);
    let mut held = rosters(cloud, &person.token, id);
    assert_eq!(held.len(), 1, "{name}: not revision 1 alone: {held:?}");
    (id, held.remove(0))
}

/// Fails unless `next`, a revision the server holds, is the one after `previous` that `signer`'s
/// device signed to add a default channel: the same members, service agents and policy, a new
/// channel first with every member and `history` true and no services, and every channel of
/// `previous` after it as it was. The signer accepts it. Gives the default channel's ID.
#[track_caller]
pub fn assert_adds_default(
    cloud: &Cloud,
    previous: &Roster,
    next: &Roster,
    signer: &Person,
) -> ChannelId {
    let (before, after) = (previous.body(), next.body());
    assert_eq!(
        (after.revision, after.prev, after.signer),
        (before.revision + 1, Some(previous.hash()), signer.device),
        "not the next revision, signed by the signer's device"
    );
    assert!(
        (&after.members, &after.service_agents, &after.policy)
            == (&before.members, &before.service_agents, &before.policy),
        "the members, service agents or policy changed: {after:?}"
    );
    let accepted = accept_next(
        previous,
        next,
        &space_certs(cloud, &signer.token, after.space),
        signer.account,
        &signer.master,
    );
    assert!(
        accepted.is_ok(),
        "the signer does not accept it: {accepted:?}"
    );
    assert_eq!(
        after.channels.len(),
        before.channels.len() + 1,
        "not one channel added: {:?}",
        after.channels
    );
    assert_eq!(
        after.channels[1..],
        before.channels[..],
        "the channels before are not kept after the new one"
    );
    let default = &after.channels[0];
    assert_with_every_member(after, default);
    default.id
}

/// Fails unless `channel` of the revision `body` holds every member of the space with `history`
/// true, and no services.
#[track_caller]
pub fn assert_with_every_member(body: &RosterBody, channel: &Channel) {
    let held: BTreeSet<(AccountId, bool)> = channel
        .members
        .iter()
        .map(|entry| (entry.account, entry.history))
        .collect();
    let every: BTreeSet<(AccountId, bool)> = body
        .members
        .iter()
        .map(|member| (member.account, true))
        .collect();
    assert_eq!(held, every, "not every member, with its history");
    assert_eq!(channel.members.len(), every.len(), "a member twice");
    assert!(channel.services.is_empty(), "services: {channel:?}");
}

/// A channel ID the server issues for `space` to the session `token`, through the API.
#[track_caller]
pub fn issue_channel(cloud: &Cloud, token: &str, space: SpaceId) -> ChannelId {
    let (status, body) = post_as(cloud, token, &format!("/spaces/{space}/channels"), None);
    match (
        status,
        body["channel"].as_str().map(str::parse::<ChannelId>),
    ) {
        (201, Some(Ok(id))) => id,
        _ => panic!("no channel ID issued for {space}: {status} {body}"),
    }
}

/// The revision after the latest of `space`, signed by `signer`'s device and stored as a fixture,
/// that adds `added` with their roles to the space and its default channel, and a new channel with
/// `signer` and them; gives the new channel's ID.
pub fn with_channel(
    cloud: &Cloud,
    space: SpaceId,
    signer: &Person,
    added: &[(&Person, Role)],
) -> ChannelId {
    let channel = issue_channel(cloud, &signer.token, space);
    let mut members = vec![signer];
    members.extend(added.iter().map(|(person, _)| *person));
    let previous = latest(cloud, &signer.token, space);
    let change = Change {
        members: added.to_vec(),
        channels: vec![(channel, members)],
        ..Change::default()
    };
    revise(cloud, &previous, signer, &change);
    channel
}

/// A fixture revision: what it changes of the previous one.
#[derive(Default)]
pub struct Change<'a> {
    /// People it adds as members, with their roles; each joins the default channel, the first one,
    /// with `history` true, unless `outside_default`.
    pub members: Vec<(&'a Person, Role)>,
    /// Whether the people it adds stay out of the default channel.
    pub outside_default: bool,
    /// Channels it adds after the others, each with its members.
    pub channels: Vec<(ChannelId, Vec<&'a Person>)>,
    /// People it adds to existing channels.
    pub join: Vec<(ChannelId, &'a Person)>,
    /// People it removes from existing channels.
    pub leave: Vec<(ChannelId, &'a Person)>,
}

/// The revision after `previous` that `signer`'s device signs with `change`, stored on the server
/// as `signer`'s session.
pub fn revise(cloud: &Cloud, previous: &Roster, signer: &Person, change: &Change) -> Roster {
    let next = next_with(previous, (signer.device, &signer.key), |body| {
        let mut members = body.members.clone().into_vec();
        for (person, role) in &change.members {
            members.push(person.member(*role));
            if !change.outside_default {
                let default = body.channels.first_mut().expect("a default channel");
                default.members.push(entry(person, true));
            }
        }
        body.members = skald_proto::NonEmpty::from_vec(members).expect("a member");
        for (id, people) in &change.channels {
            body.channels.push(Channel {
                id: *id,
                members: people.iter().map(|person| entry(person, false)).collect(),
                services: Vec::new(),
                unknown: Unknown::default(),
            });
        }
        let find = |channels: &mut Vec<Channel>, id: ChannelId| -> usize {
            channels
                .iter()
                .position(|channel| channel.id == id)
                .expect("a channel of the space")
        };
        for (id, person) in &change.join {
            let at = find(&mut body.channels, *id);
            body.channels[at].members.push(entry(person, false));
        }
        for (id, person) in &change.leave {
            let at = find(&mut body.channels, *id);
            body.channels[at]
                .members
                .retain(|member| member.account != person.account);
        }
    });
    store(cloud, &signer.token, &next);
    next
}

fn entry(person: &Person, history: bool) -> ChannelMember {
    ChannelMember {
        account: person.account,
        history,
    }
}

/// A channel's group as the server's GroupInfo of its current epoch shows it.
#[derive(Debug, Clone)]
pub struct Group {
    pub epoch: u64,
    /// The member devices, in leaf order.
    pub members: Vec<DeviceId>,
    /// The device that signed the GroupInfo.
    pub signer: DeviceId,
}

impl Group {
    /// Fails unless the group is at `epoch` with each of `devices` once and no other.
    #[track_caller]
    pub fn assert_is(&self, epoch: u64, devices: &[DeviceId]) {
        assert_eq!(self.epoch, epoch, "the group's epoch: {self:?}");
        assert_members(&self.members, devices);
    }
}

/// Fails unless `members` holds each of `devices` once and no other.
#[track_caller]
pub fn assert_members(members: &[DeviceId], devices: &[DeviceId]) {
    let mut held = members.to_vec();
    held.sort();
    let mut expected = devices.to_vec();
    expected.sort();
    assert_eq!(held, expected, "the group's member devices");
}

/// `bytes` in base64url without padding.
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The path of `channel`'s GroupInfo in `space`.
pub fn group_info_path(space: SpaceId, channel: ChannelId) -> String {
    format!("/spaces/{space}/channels/{channel}/group-info")
}

/// What the server gives the session `token` of `at`'s channel's group: whether the channel has
/// one, and the bytes of its GroupInfo of the current epoch, if the server has one.
#[track_caller]
pub fn group_info_on_server(
    cloud: &Cloud,
    token: &str,
    (space, channel): (SpaceId, ChannelId),
) -> (bool, Option<Vec<u8>>) {
    let path = group_info_path(space, channel);
    let (status, body) = cloud.get_as(&path, token);
    assert_eq!(status, 200, "GET {path}: {body}");
    let has = body["group"]
        .as_bool()
        .unwrap_or_else(|| panic!("GET {path}: {body}"));
    (has, body["group_info"].as_str().map(unb64))
}

/// What the server gives the session `token` of `channel`'s group: whether the channel has one, and
/// its GroupInfo of the current epoch, if the server has one, as the group it shows.
#[track_caller]
pub fn group_on_server(
    cloud: &Cloud,
    token: &str,
    space: SpaceId,
    channel: ChannelId,
) -> (bool, Option<Group>) {
    let (has, group_info) = group_info_on_server(cloud, token, (space, channel));
    let group = group_info.map(|bytes| {
        let view = ChannelView::observe(channel, &bytes)
            .unwrap_or_else(|err| panic!("a GroupInfo of the channel: {err:?}"));
        Group {
            epoch: view.epoch(),
            members: view.members().iter().map(|member| member.device).collect(),
            signer: view.signer().device,
        }
    });
    (has, group)
}

/// `channel`'s group as the server's GroupInfo shows it to the session `token`; fails unless the
/// server has one.
#[track_caller]
pub fn group(cloud: &Cloud, token: &str, space: SpaceId, channel: ChannelId) -> Group {
    match group_on_server(cloud, token, space, channel) {
        (true, Some(group)) => group,
        other => panic!("no GroupInfo of {channel}: {other:?}"),
    }
}

/// `PUT` of `group_info` as `channel`'s GroupInfo, as the session `token`; gives the status.
pub fn upload_group_info(
    cloud: &Cloud,
    token: &str,
    (space, channel): (SpaceId, ChannelId),
    group_info: &[u8],
) -> u16 {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .build();
    let url = format!("{}{}", cloud.url, group_info_path(space, channel));
    let body = json!({ "group_info": b64(group_info) }).to_string();
    ureq::Agent::new_with_config(config)
        .put(&url)
        .header("authorization", format!("Bearer {token}"))
        .header(
            skald_proto::VERSION_HEADER,
            skald_proto::PROTOCOL_VERSION.to_string(),
        )
        .header("content-type", "application/json")
        .send(body.as_bytes())
        .unwrap_or_else(|err| panic!("PUT {url}: {err}"))
        .status()
        .as_u16()
}

/// The lines of `listed` but its warnings that hold `channel`'s ID as a word.
pub fn lines_of(listed: &Finished, channel: ChannelId) -> Vec<String> {
    lines_naming(listed, &channel.to_string())
}

/// `channel`'s one line in `listed`.
#[track_caller]
pub fn line_of(listed: &Finished, channel: ChannelId) -> String {
    let lines = lines_of(listed, channel);
    assert_eq!(lines.len(), 1, "{channel} is not listed once: {listed:?}");
    lines[0].clone()
}

/// Fails unless `listed` shows `channel` on one line with the device names `with`, as the devices
/// in its group, and none of `without`.
#[track_caller]
pub fn assert_listed_with(listed: &Finished, channel: ChannelId, with: &[&str], without: &[&str]) {
    let line = line_of(listed, channel);
    let held = words(&line);
    for name in with {
        assert!(held.contains(name), "{channel} without {name}: {listed:?}");
    }
    for name in without {
        assert!(!held.contains(name), "{channel} with {name}: {listed:?}");
    }
    assert!(
        !line.to_lowercase().contains(NOT_IN),
        "{channel} listed as not in: {listed:?}"
    );
}

/// Fails unless `listed` shows `channel` on one line as a channel the device is admitted to but not
/// in, with none of the device names `without`.
#[track_caller]
pub fn assert_listed_not_in(listed: &Finished, channel: ChannelId, without: &[&str]) {
    let line = line_of(listed, channel);
    assert!(
        line.to_lowercase().contains(NOT_IN),
        "{channel} not listed as not in: {listed:?}"
    );
    let held = words(&line);
    for name in without {
        assert!(!held.contains(name), "{channel} with {name}: {listed:?}");
    }
}

/// The warning lines of `finished` that hold `id`, such as a device's or a channel's ID.
pub fn warnings_naming(finished: &Finished, id: impl Display) -> Vec<String> {
    let id = id.to_string();
    warnings(finished)
        .into_iter()
        .filter(|line| line.contains(&id))
        .collect()
}

/// From now on, the relay says `channel` has no group yet to every request for its GroupInfo, as a
/// device that asked before another device's first Commit was stored sees it; until
/// [`tell_groups`].
pub fn hide_group(cloud: &Cloud, space: SpaceId, channel: ChannelId) {
    cloud
        .relay
        .rewrite(&group_info_request(space, channel), |_| {
            json!({ "group": false, "group_info": null }).to_string()
        });
}

/// From now on, the relay gives `channel`'s GroupInfo as the server answers.
pub fn tell_groups(cloud: &Cloud, space: SpaceId, channel: ChannelId) {
    cloud
        .relay
        .rewrite(&group_info_request(space, channel), str::to_owned);
}

/// The request for `channel`'s GroupInfo, as its line starts.
pub fn group_info_request(space: SpaceId, channel: ChannelId) -> String {
    format!("GET {}", group_info_path(space, channel))
}

/// The request that hands out KeyPackages, as its line starts.
pub const HAND_OUT: &str = "POST /key-packages ";

/// From now on, the relay gives `swapped` in place of each KeyPackage of `device` the server hands
/// out, as a compromised server would.
pub fn swap_key_packages(cloud: &Cloud, device: DeviceId, swapped: &[u8]) {
    let (device, swapped) = (device.to_string(), b64(swapped));
    cloud.relay.rewrite(HAND_OUT, move |answer| {
        let mut held: serde_json::Value =
            serde_json::from_str(answer).expect("the KeyPackages handed out");
        for entry in held["key_packages"].as_array_mut().into_iter().flatten() {
            if entry["device"] == json!(device) {
                entry["key_package"] = json!(swapped);
            }
        }
        held.to_string()
    });
}
