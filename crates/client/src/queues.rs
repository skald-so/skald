//! A channel's queues and its tasks in the CLI (`docs/protocol/queues.md`, "Queues", "Tasks" and
//! "Queue key"; `docs/dev/architecture.md`, "Queue").
//!
//! - `skald queues` lists the queues of a channel, by default the default channel of the working
//!   directory's space, with their settings; as an agent too, only in the space of its folder.
//!   `skald create-queue`, `skald change-queue` and `skald delete-queue` are the person's. Each
//!   request proves the device; a refusal of the server fails the run, naming its problem.
//! - `skald enqueue` first accepts the space's latest revisions and goes through the mailbox as
//!   `skald send` does, then seals one task to the `pk` of the channel's current epoch, which the
//!   device must have judged clean, signed by the device: content `from` the agent or the person,
//!   with the `task` body, no `counter` and `hops` as a send has them, its request an A2A message
//!   whose `messageId` is the task ID. `--to` names the addressee: the agent in `to`, and in
//!   `sched` the one device whose valid presence value in the channel lists it. With no
//!   `--deadline`, the device sets one 7 days after enqueue if a device of another account may
//!   claim the task. Nothing is enqueued, saying why, under an unclean epoch, over the limit on
//!   `hops`, for a queue the channel lacks, or an addressee no device or several list.
//! - The task waits in the local state before its upload; every upload carries its same bytes.
//!   Each `skald enqueue` and `skald recv` uploads the tasks that wait first, in order, until the
//!   server answers each: stored, or refused, which is final.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use skald_crypto::{seal_task, sign_task};
use skald_proto::{
    AccountId, Address, Cbor as _, ChannelId, DeviceId, IDEMPOTENCY_KEY_MAX, MessageId, Name,
    QueueId, QueueMode, Roster, Sched, SpaceId, Task, TaskBody, TaskId, Unknown, VersionRange,
};

use crate::connection::{self, Method, printable};
use crate::devices::text;
use crate::follow::SpaceDevices;
use crate::forms::References;
use crate::groups;
use crate::presence;
use crate::recv;
use crate::secrets;
use crate::send::{self, MOST_HOPS};
use crate::spaces::{self, Follower, Scope, Synced};
use crate::state::{self, State, TaskOutgoing};

/// The queue every channel has.
pub const DEFAULT: &str = "default";

/// How long after enqueue the `deadline` a device sets is, in seconds.
const WEEK: u64 = 7 * 24 * 60 * 60;

/// A queue's mode (`docs/protocol/queues.md`, "Modes and retries").
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    AtLeastOnce,
    AtMostOnce,
}

impl Mode {
    /// Its name, as the server and the CLI write it.
    fn name(self) -> &'static str {
        match self {
            Mode::AtLeastOnce => "at-least-once",
            Mode::AtMostOnce => "at-most-once",
        }
    }
}

/// A queue as the server gives it: `Queue` in the OpenAPI spec.
#[derive(Debug, Clone, Deserialize)]
pub struct Queue {
    id: String,
    name: String,
    mode: Mode,
    lease: u64,
    max_attempts: u64,
}

impl Queue {
    /// Its line in `skald queues`.
    fn line(&self) -> String {
        format!(
            "{}  {}  lease {} s  max_attempts {}",
            printable(&self.name),
            self.mode.name(),
            self.lease,
            self.max_attempts
        )
    }

    /// Its JSON object, with every field, in `channel`.
    fn json(&self, channel: ChannelId) -> serde_json::Value {
        json!({
            "id": self.id,
            "name": self.name,
            "channel": channel.to_string(),
            "mode": self.mode.name(),
            "lease": self.lease,
            "max_attempts": self.max_attempts,
        })
    }

    /// Its ID; fails for one that is no queue ID.
    pub fn id(&self) -> Result<QueueId, String> {
        self.id
            .parse()
            .map_err(|_| "unexpected answer from the server: a queue ID".to_owned())
    }
}

/// `Queues` in the OpenAPI spec.
#[derive(Deserialize)]
struct Queues {
    queues: Vec<Queue>,
}

/// The settings a command gives a queue; those left out keep their value, or the server's default
/// at creation.
#[derive(Default, Serialize)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u64>,
}

/// What a queue command runs with: the state directory, where its secrets are, its protocol
/// versions and timeout, the channel it names, if any, and whether it runs as an agent.
pub struct Run<'a> {
    pub dir: &'a Path,
    pub storage: secrets::Storage,
    pub settings: (VersionRange, Duration),
    pub channel: Option<&'a str>,
    pub agent: bool,
}

impl Run<'_> {
    /// The space of the channel the run names, or of the working directory's space's default
    /// channel, synced, with that channel and the run's scope.
    pub fn synced(&self) -> Result<(Synced, ChannelId, Scope), String> {
        let (speaks, timeout) = self.settings;
        let scope = Scope::of_run(self.dir, self.agent)?;
        match self.channel.map(send::channel_id).transpose()? {
            Some(channel) => {
                let synced =
                    spaces::of_channel(self.dir, self.storage, speaks, timeout, channel, scope)?;
                Ok((synced, channel, scope))
            }
            None => {
                let (synced, channel) =
                    spaces::of_working_dir(self.dir, self.storage, speaks, timeout, scope)?;
                Ok((synced, channel, scope))
            }
        }
    }
}

/// `skald queues`: lists the queues of the run's channel with their settings; as JSON objects, one
/// per line, if `json`.
pub fn list(run: &Run, json: bool) -> Result<(), String> {
    recv::holding_warnings(run.dir, run.agent, || {
        let (mut synced, channel, _) = run.synced()?;
        let queues = listed(&mut synced.follower, (synced.space.id, channel))?;
        if json {
            for queue in &queues {
                println!("{}", queue.json(channel));
            }
        } else if run.agent {
            for queue in &queues {
                println!("{}", queue.line());
            }
        } else {
            println!("The queues of the channel {channel}:");
            for queue in &queues {
                println!("  {}", queue.line());
            }
        }
        Ok(())
    })
}

/// `skald create-queue`: creates the queue `name` with `settings` in the run's channel.
pub fn create(run: &Run, name: &str, settings: &Settings) -> Result<(), String> {
    let name = queue_name(name)?;
    let (mut synced, channel, _) = run.synced()?;
    let path = queues_path((synced.space.id, channel));
    let mut body = serde_json::to_value(settings).expect("settings are JSON");
    body["name"] = json!(name.as_str());
    let answer = proved_json(&mut synced.follower, Method::Post, &path, &body)
        .map_err(|err| refused(&format!("the queue {name} is not created"), err))?;
    let queue: Queue = serde_json::from_slice(&answer)
        .map_err(|_| "unexpected answer from the server: a created queue".to_owned())?;
    println!("Created in the channel {channel}: {}", queue.line());
    Ok(())
}

/// `skald change-queue`: changes the settings `settings` gives of the queue `name` of the run's
/// channel.
pub fn change(run: &Run, name: &str, settings: &Settings) -> Result<(), String> {
    if settings.lease.is_none() && settings.max_attempts.is_none() {
        return Err("give --lease or --max-attempts; nothing is changed".to_owned());
    }
    let (name, mut synced, channel, path) = existing(run, name)?;
    let body = serde_json::to_value(settings).expect("settings are JSON");
    let answer = proved_json(&mut synced.follower, Method::Patch, &path, &body)
        .map_err(|err| refused(&format!("the queue {name} is not changed"), err))?;
    let queue: Queue = serde_json::from_slice(&answer)
        .map_err(|_| "unexpected answer from the server: a changed queue".to_owned())?;
    println!("Changed in the channel {channel}: {}", queue.line());
    Ok(())
}

/// `skald delete-queue`: deletes the queue `name` of the run's channel, with its tasks.
pub fn delete(run: &Run, name: &str) -> Result<(), String> {
    let (name, mut synced, channel, path) = existing(run, name)?;
    let follower = &mut synced.follower;
    let proof = proof(follower)?;
    (follower.connection)
        .delete_proved(&path, (&follower.token, &proof))
        .map_err(|err| refused(&format!("the queue {name} is not deleted"), err))?;
    println!("Deleted the queue {name} of the channel {channel}, with its tasks.");
    Ok(())
}

/// The queue `given` names in the run's channel: its name, the channel's space synced, the
/// channel, and the queue's path; fails, naming it, if the channel has none.
fn existing(run: &Run, given: &str) -> Result<(Name, Synced, ChannelId, String), String> {
    let name = queue_name(given)?;
    let (mut synced, channel, _) = run.synced()?;
    let at = (synced.space.id, channel);
    let queue = named(&mut synced.follower, at, &name)?;
    let path = format!("{}/{}", queues_path(at), queue.id()?);
    Ok((name, synced, channel, path))
}

/// `given` as a queue's name; fails for one that is no `name`.
pub fn queue_name(given: &str) -> Result<Name, String> {
    given.parse().map_err(|err| {
        format!(
            "\"{}\" is no queue name: {err}; give a-z, 0-9 and '-' inside, 1 to 63 characters",
            printable(given)
        )
    })
}

/// The path of the queues of the channel `at`.
pub fn queues_path((space, channel): (SpaceId, ChannelId)) -> String {
    format!("/spaces/{space}/channels/{channel}/queues")
}

/// The proof of a request of `follower`'s device.
pub fn proof(follower: &mut Follower) -> Result<String, String> {
    let keys = follower.keys()?;
    (follower.connection)
        .request_proof(&follower.token, follower.this, keys.signing())
        .map_err(text)
}

/// `<method> <path>` of the JSON `body` as `follower`'s device.
pub fn proved_json(
    follower: &mut Follower,
    method: Method,
    path: &str,
    body: &serde_json::Value,
) -> Result<Vec<u8>, connection::Error> {
    let keys = follower.keys().map_err(connection::Error::Unexpected)?;
    let proof =
        (follower.connection).request_proof(&follower.token, follower.this, keys.signing())?;
    let json = body.to_string();
    (follower.connection).send_json_proved(method, path, (&follower.token, &proof), json.as_bytes())
}

/// The queues of the channel `at` the server gives `follower`'s device, by name.
fn listed(follower: &mut Follower, at: (SpaceId, ChannelId)) -> Result<Vec<Queue>, String> {
    let proof = proof(follower)?;
    let answer = (follower.connection)
        .get_proved(&queues_path(at), (&follower.token, &proof))
        .map_err(|err| refused("the channel's queues are not listed", err))?;
    let Queues { mut queues } = serde_json::from_slice(&answer)
        .map_err(|_| "unexpected answer from the server: a channel's queues".to_owned())?;
    queues.sort_by(|one, other| one.name.cmp(&other.name));
    Ok(queues)
}

/// Records that `agent`, if given, ran in `space`, then goes through the mailbox within `scope` as
/// `skald send` does ([`send::through_mailbox`]), by `synced`, the space's last accepted revision
/// and devices; gives them as they are after it.
pub fn through_mailbox_as(
    follower: &mut Follower,
    (scope, space): (Scope, SpaceId),
    agent: Option<&Name>,
    synced: (Roster, SpaceDevices),
) -> Result<(Roster, SpaceDevices), String> {
    if let Some(agent) = agent {
        (follower.state)
            .add_agent(space, agent.as_str())
            .map_err(text)?;
    }
    send::through_mailbox(follower, scope, space, synced)
}

/// The queue `name` of the channel `at`; fails, naming it, if the channel has none.
pub fn named(
    follower: &mut Follower,
    at: (SpaceId, ChannelId),
    name: &Name,
) -> Result<Queue, String> {
    (listed(follower, at)?.into_iter())
        .find(|queue| queue.name == name.as_str())
        .ok_or_else(|| format!("the channel {} has no queue {name}", at.1))
}

/// The problem of the type `kind`, as a word: the last segment of its URI, such as `not-allowed`.
pub fn problem(kind: &str) -> String {
    printable(kind.rsplit('/').next().unwrap_or(kind))
}

/// Why `what` happened: `err`, a refusal of the server naming its problem.
pub fn refused(what: &str, err: connection::Error) -> String {
    match err {
        connection::Error::Refused { kind, detail, .. } => format!(
            "{what}: the server refused it: {} ({})",
            problem(&kind),
            printable(&detail)
        ),
        other => format!("{what}: {other}"),
    }
}

/// What a `skald enqueue` names.
pub struct Enqueue<'a> {
    pub queue: &'a str,
    pub to: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
    pub deadline: Option<&'a str>,
    pub text: &'a str,
    /// Whether it says the task as a JSON object.
    pub json: bool,
}

/// `skald enqueue`: enqueues the task `given` names in the run's channel, as `agent` if given,
/// whose run holds its warnings for the person.
pub fn enqueue(run: &Run, given: &Enqueue, agent: Option<&Name>) -> Result<(), String> {
    recv::holding_warnings(run.dir, agent.is_some(), || enqueued(run, given, agent))
}

/// [`enqueue`].
fn enqueued(run: &Run, given: &Enqueue, agent: Option<&Name>) -> Result<(), String> {
    let to = given.to.map(send::addressee).transpose()?;
    let idempotency_key = given.idempotency_key.map(idempotency_key).transpose()?;
    let now = state::unix_now();
    let set = given.deadline.map(|text| deadline(text, now)).transpose()?;
    let name = queue_name(given.queue)?;
    let (synced, channel, scope) = run.synced()?;
    let Synced {
        mut follower,
        space,
        roster,
        devices,
    } = synced;
    let server = send::server_of(&mut follower)?;
    let to = (to.map(|to| to.in_space(&roster.body().name, &server))).transpose()?;
    let hops = match send::counted(&follower, agent) {
        Some(counted) => (follower.state)
            .handed_hops(space.id, counted)
            .map_err(text)?
            .map_or(0, |highest| highest.saturating_add(1)),
        None => 0,
    };
    if agent.is_some() && hops > MOST_HOPS {
        return Err(send::over_the_limit(
            agent,
            channel,
            to.as_ref(),
            "enqueued",
        ));
    }
    let (roster, devices) =
        through_mailbox_as(&mut follower, (scope, space.id), agent, (roster, devices))?;
    let at = (space.id, channel);
    let queue = named(&mut follower, at, &name).map_err(|err| format!("{err}; {NOTHING}"))?;
    let addressee = match &to {
        Some(to) => Some(addressee_device(
            &mut follower,
            (&roster, &devices),
            (at, &server),
            to,
        )?),
        None => None,
    };
    let (epoch, pk) = clean_epoch(&follower, channel)?;
    let deadline = set.or_else(|| {
        let claimers = Claimers::of((&roster, &devices), channel, addressee);
        claimers.others(follower.own.account).then_some(now + WEEK)
    });
    let from = send::sender(&follower, &roster, server, agent)?;
    let task = TaskId::generate();
    let request = json!({
        "messageId": task.to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": given.text }],
    })
    .to_string();
    let sched = Sched {
        idempotency_key,
        template: None,
        deadline,
        device: addressee,
        unknown: Unknown::default(),
    };
    let body = TaskBody {
        sender: follower.this,
        channel,
        queue: queue.id()?,
        task,
        sched,
        mode: match queue.mode {
            Mode::AtLeastOnce => QueueMode::AtLeastOnce,
            Mode::AtMostOnce => QueueMode::AtMostOnce,
        },
        enqueued: now,
        content: Task {
            request,
            unknown: Unknown::default(),
        }
        .content(from, to, hops),
        unknown: Unknown::default(),
    };
    let keys = follower.keys()?;
    let plaintext = sign_task(&body, keys.signing()).map_err(|err| err.to_string())?;
    let record = seal_task(&plaintext, epoch, &pk).map_err(|err| err.to_string())?;
    let outgoing = TaskOutgoing {
        task,
        space: space.id,
        channel,
        queue: body.queue,
        record: record.to_cbor(),
    };
    (follower.state)
        .keep_task(&outgoing, agent.map(Name::as_str))
        .map_err(text)?;
    let answered = upload_waiting(&mut follower, scope);
    let reference = reference(&follower.state, task);
    match answered.into_iter().find(|(one, _)| one.task == task) {
        Some((_, Answer::Stored)) => {
            match (given.json, agent) {
                (true, _) => println!(
                    "{}",
                    json!({
                        "task": task.to_string(),
                        "ref": reference,
                        "channel": channel.to_string(),
                        "queue": body.queue.to_string(),
                        "queue_name": name.as_str(),
                        "deadline": deadline,
                        "device": addressee.map(|device| device.to_string()),
                    })
                ),
                (false, Some(_)) => println!("enqueued #{reference}"),
                (false, None) => println!(
                    "The task {task} (#{reference}) is in the queue {name} of the channel \
                     {channel}."
                ),
            }
            Ok(())
        }
        Some((_, refusal @ Answer::Refused { .. })) => Err(format!(
            "the server refused the task #{reference}: {}; nothing is enqueued",
            refusal.why(&follower.state)
        )),
        Some((_, Answer::Waiting(why))) => Err(format!(
            "the server has not answered the task #{reference} ({why}); {KEPT}"
        )),
        None => Err(format!(
            "an earlier task waits for the server, so the task #{reference} is not uploaded yet; \
             {KEPT}"
        )),
    }
}

/// What a refused enqueue says of the task.
const NOTHING: &str = "nothing is enqueued";

/// What a run says of a task the server has not answered.
const KEPT: &str =
    "the local state keeps it, and the next skald enqueue or skald recv uploads it first";

/// `given` as an `idempotency_key`: its UTF-8 bytes, 1 to [`IDEMPOTENCY_KEY_MAX`] of them.
fn idempotency_key(given: &str) -> Result<Vec<u8>, String> {
    if given.is_empty() || given.len() > IDEMPOTENCY_KEY_MAX {
        return Err(format!(
            "the idempotency key is {} bytes in UTF-8: give 1 to {IDEMPOTENCY_KEY_MAX}; {NOTHING}",
            given.len()
        ));
    }
    Ok(given.as_bytes().to_vec())
}

/// The `deadline` of the RFC 3339 time `given`, in seconds since the Unix epoch; fails for a time
/// that is not after `now`.
fn deadline(given: &str, now: u64) -> Result<u64, String> {
    let time = given
        .parse::<jiff::Timestamp>()
        .ok()
        .and_then(|time| u64::try_from(time.as_second()).ok())
        .ok_or_else(|| {
            format!(
                "the deadline \"{}\" is no RFC 3339 date-time, such as 2027-01-31T00:00:00Z; \
                 {NOTHING}",
                printable(given)
            )
        })?;
    if time <= now {
        return Err(format!("the deadline {given} has passed; {NOTHING}"));
    }
    Ok(time)
}

/// The one device whose valid presence value in the channel `at` lists `to`; fails, naming the
/// agent or the devices, for none or several.
fn addressee_device(
    follower: &mut Follower,
    space: (&Roster, &SpaceDevices),
    (at, server): ((SpaceId, ChannelId), &skald_proto::ServerName),
    to: &Address,
) -> Result<DeviceId, String> {
    let listing = presence::listing(follower, space, at, server, to)?;
    match listing.as_slice() {
        [(device, _)] => Ok(*device),
        [] => Err(format!(
            "no device's presence value in the channel lists the agent {to}, so no device would \
             claim its task; {NOTHING}"
        )),
        several => {
            let names: Vec<String> = (several.iter())
                .map(|(device, name)| format!("{} ({device})", printable(name)))
                .collect();
            Err(format!(
                "the presence values of several devices list the agent {to}: {}; {NOTHING}",
                names.join(", ")
            ))
        }
    }
}

/// The current epoch of `channel` in the device's group and the `pk` of its queue key; fails
/// unless the device holds the epoch's key and judged it clean.
fn clean_epoch(follower: &Follower, channel: ChannelId) -> Result<(u64, [u8; 32]), String> {
    let keys = follower.keys()?;
    let held = groups::read(follower, &keys, |device| {
        let epoch = device.epoch(channel).ok()?;
        let key = device.history_key(channel, epoch)?;
        Some((epoch, key.queue_key().public_key()))
    })?;
    let (epoch, pk) = held.ok_or_else(|| {
        format!(
            "this device holds no key of the current epoch of the channel {channel}: it is not in \
             the channel's group yet; {NOTHING}"
        )
    })?;
    let clean = follower.state.judgments(channel).map_err(text)?;
    if clean.get(&epoch) != Some(&true) {
        return Err(format!(
            "this device judged the current epoch {epoch} of the channel {channel} unclean: a \
             device the channel does not admit may hold its key; {NOTHING} until a Commit starts \
             a clean epoch"
        ));
    }
    Ok((epoch, pk))
}

/// The accounts that may claim a task, each `None` for a device whose certificate the device
/// does not know.
#[derive(Debug)]
enum Claimers {
    /// Those of its addressee's device.
    Addressee(Option<AccountId>),
    /// With no addressee, the channel's members and the devices in its `services`.
    Channel {
        members: Vec<AccountId>,
        services: Vec<Option<AccountId>>,
    },
}

impl Claimers {
    /// Who may claim a task of `channel` to the device `addressee`, if any, by `roster`, the
    /// latest revision the device accepted, and `devices`, its accounts' devices.
    fn of(
        (roster, devices): (&Roster, &SpaceDevices),
        channel: ChannelId,
        addressee: Option<DeviceId>,
    ) -> Self {
        let account_of = |device: DeviceId| {
            (devices.certs.iter())
                .map(|cert| cert.body())
                .find(|body| body.device == device)
                .map(|body| body.account)
        };
        if let Some(device) = addressee {
            return Claimers::Addressee(account_of(device));
        }
        let body = roster.body();
        let listed = body.channels.iter().find(|it| it.id == channel);
        Claimers::Channel {
            members: (listed.iter())
                .flat_map(|it| it.members.iter().map(|member| member.account))
                .collect(),
            services: (listed.iter())
                .flat_map(|it| it.services.iter().map(|&device| account_of(device)))
                .collect(),
        }
    }

    /// Whether a device of another account than `own` may claim the task.
    fn others(&self, own: AccountId) -> bool {
        let other = |account: &Option<AccountId>| *account != Some(own);
        match self {
            Claimers::Addressee(account) => other(account),
            Claimers::Channel { members, services } => {
                members.iter().any(|&member| member != own) || services.iter().any(other)
            }
        }
    }
}

/// The reference, `#<ref>` without its `#`, of the task `task` among the messages the device of
/// `state` keeps.
fn reference(state: &State, task: TaskId) -> String {
    let as_message = MessageId::from_bytes(*task.as_bytes());
    let kept = state.kept_ids().unwrap_or_default();
    References::new(kept.into_iter().chain([as_message])).of(as_message)
}

/// The server's answer to an upload of a task.
#[derive(Debug)]
pub enum Answer {
    Stored,
    /// Refused for `problem`, as a word, with its `detail`; for `duplicate`, the task stored
    /// before, if the server names it.
    Refused {
        problem: String,
        detail: String,
        duplicate: Option<TaskId>,
    },
    /// Not answered, for this reason: it waits for a later run.
    Waiting(String),
}

impl Answer {
    /// Of a refusal: its problem and detail, and the task stored before.
    fn why(&self, state: &State) -> String {
        let Answer::Refused {
            problem,
            detail,
            duplicate,
        } = self
        else {
            return String::new();
        };
        let before = duplicate.map_or_else(String::new, |task| {
            format!(
                ", as the task #{} holds its ID or idempotency key",
                reference(state, task)
            )
        });
        format!("{problem} ({}){before}", printable(detail))
    }
}

/// Uploads the tasks of the outbox of `follower`'s device within `scope`, in order, until one gets
/// no answer; takes each answered one out of the outbox. Gives each task it tried with its answer.
pub fn upload_waiting(follower: &mut Follower, scope: Scope) -> Vec<(TaskOutgoing, Answer)> {
    let waiting = match follower.state.task_outbox() {
        Ok(waiting) => waiting,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return Vec::new();
        }
    };
    let mut answered = Vec::new();
    for task in waiting
        .into_iter()
        .filter(|task| !scope.excludes(task.space))
    {
        let answer = upload(follower, &task);
        if !matches!(answer, Answer::Waiting(_))
            && let Err(err) = follower.state.remove_task(task.task)
        {
            eprintln!(
                "skald: warning: the local state keeps the task {}, which the server answered, \
                 so a later run uploads it again: {err}",
                task.task
            );
        }
        let stop = matches!(answer, Answer::Waiting(_));
        answered.push((task, answer));
        if stop {
            break;
        }
    }
    answered
}

/// [`upload_waiting`] for a run that receives: warns of each task the server refused and of those
/// that still wait.
pub fn upload_received(follower: &mut Follower, scope: Scope) {
    for (task, answer) in upload_waiting(follower, scope) {
        let reference = reference(&follower.state, task.task);
        match answer {
            Answer::Stored => {}
            Answer::Refused { .. } => eprintln!(
                "skald: warning: the server refused the task #{reference}: {}; it is not \
                 uploaded again",
                answer.why(&follower.state)
            ),
            Answer::Waiting(why) => eprintln!(
                "skald: warning: the server has not answered the task #{reference} ({why}); {KEPT}"
            ),
        }
    }
}

/// Uploads `task` as `follower`'s device: the server's answer.
fn upload(follower: &mut Follower, task: &TaskOutgoing) -> Answer {
    let path = format!(
        "{}/{}/tasks",
        queues_path((task.space, task.channel)),
        task.queue
    );
    let proof = match proof(follower) {
        Ok(proof) => proof,
        Err(err) => return Answer::Waiting(err),
    };
    let sent = (follower.connection).send_cbor_proved(
        Method::Post,
        &path,
        (&follower.token, &proof),
        &task.record,
    );
    answer(sent)
}

/// The answer `sent`, the result of an upload, gives.
fn answer(sent: Result<Vec<u8>, connection::Error>) -> Answer {
    match sent {
        Ok(_) => Answer::Stored,
        // A refused proof, a timeout, a rate limit and the server's failure change with time.
        Err(connection::Error::Refused {
            status,
            kind,
            detail,
            task,
        }) if (400..500).contains(&status) && ![401, 408, 429].contains(&status) => {
            Answer::Refused {
                problem: problem(&kind),
                detail,
                duplicate: task.and_then(|task| task.parse().ok()),
            }
        }
        Err(err) => Answer::Waiting(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A problem is named by the last segment of its type.
    #[test]
    fn problems_are_named_by_their_type_s_last_segment() {
        assert_eq!(
            problem("https://skald.so/problems/not-allowed"),
            "not-allowed"
        );
        assert_eq!(problem("duplicate"), "duplicate");
    }

    /// An idempotency key is 1 to 32 bytes of UTF-8.
    #[test]
    fn idempotency_keys_are_1_to_32_bytes() {
        assert_eq!(idempotency_key("a"), Ok(b"a".to_vec()));
        assert_eq!(
            idempotency_key(&"é".repeat(16)).map(|key| key.len()),
            Ok(32)
        );
        for refused in ["", &"k".repeat(33), &"é".repeat(17)] {
            assert!(idempotency_key(refused).is_err(), "{refused:?}");
        }
    }

    /// A deadline is an RFC 3339 time after now.
    #[test]
    fn deadlines_are_rfc_3339_times_after_now() {
        let now = 1_799_999_999;
        assert_eq!(deadline("2027-01-15T08:00:00Z", now), Ok(1_800_000_000));
        let err = deadline("2027-01-15T08:00:00Z", 1_800_000_000).unwrap_err();
        assert!(err.contains("has passed"), "{err}");
        assert!(deadline("tomorrow", now).is_err());
    }

    /// A task stays waiting for a refused proof, a timeout, a rate limit, the server's failure or
    /// no answer; any other refusal is final.
    #[test]
    fn only_lasting_refusals_are_final() {
        let refused = |status| {
            answer(Err(connection::Error::Refused {
                status,
                kind: "https://skald.so/problems/duplicate".to_owned(),
                detail: "taken".to_owned(),
                task: Some(TaskId::generate().to_string()),
            }))
        };
        for status in [401, 408, 429, 500, 503] {
            let answer = refused(status);
            assert!(matches!(answer, Answer::Waiting(_)), "{status}: {answer:?}");
        }
        for status in [400, 403, 404, 409, 413] {
            let answer = refused(status);
            assert!(
                matches!(answer, Answer::Refused { .. }),
                "{status}: {answer:?}"
            );
        }
        let unanswered = answer(Err(connection::Error::Timeout(Duration::from_secs(1))));
        assert!(matches!(unanswered, Answer::Waiting(_)), "{unanswered:?}");
        assert!(matches!(answer(Ok(Vec::new())), Answer::Stored));
    }

    /// Another account may claim a task to its device, or to one whose certificate is unknown;
    /// with no addressee, when the channel lists another account as a member, or another
    /// account's or an unknown device in its `services`.
    #[test]
    fn another_account_may_claim_by_the_addressee_or_the_channel() {
        let (own, other) = (AccountId::generate(), AccountId::generate());
        let channel = |members: &[AccountId], services: &[Option<AccountId>]| Claimers::Channel {
            members: members.to_vec(),
            services: services.to_vec(),
        };
        for (claimers, others) in [
            (Claimers::Addressee(Some(own)), false),
            (Claimers::Addressee(Some(other)), true),
            (Claimers::Addressee(None), true),
            (channel(&[own], &[]), false),
            (channel(&[own], &[Some(own)]), false),
            (channel(&[own, other], &[]), true),
            (channel(&[own], &[Some(other)]), true),
            (channel(&[own], &[Some(own), None]), true),
        ] {
            assert_eq!(claimers.others(own), others, "{claimers:?}");
        }
    }

    /// A queue's line holds its name, its mode, its lease and its `max_attempts`.
    #[test]
    fn a_queue_s_line_holds_its_settings() {
        let queue = Queue {
            id: QueueId::generate().to_string(),
            name: "deploys".to_owned(),
            mode: Mode::AtMostOnce,
            lease: 120,
            max_attempts: 3,
        };
        assert_eq!(
            queue.line(),
            "deploys  at-most-once  lease 120 s  max_attempts 3"
        );
    }
}
