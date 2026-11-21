//! What the tests share: the talk with Ada's agents and the channel's `default`, the task commands
//! as the person or an agent runs them, the lines and JSON of `skald tasks`, a task as the server's
//! reads give it, the requests of a test device holding a claim, and time passing for a task.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::{ChannelId, QueueId, SpaceId, TaskId};

use crate::agents::{Agents, CLAUDE};
use crate::cloud::Cloud;
use crate::lines::json_objects;
use crate::machine::{Finished, Machine};
use crate::queuing::{
    Sending, proved, queues_path, references, refers_to_task, run_in, sending, tasks, tasks_path,
};
use crate::world::{Person, words};

/// Ada's agents in the talk of the tests of channel messages, claude having run once, with the
/// channel's ID and its queue `default`; Bob's device, the talk's first played one, claims.
pub struct Scene {
    pub agents: Agents,
    pub at: (SpaceId, ChannelId),
    /// The channel's ID, as a command line names it.
    pub channel: String,
    /// The channel's queue `default`.
    pub queue: QueueId,
}

impl Scene {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let agents = Agents::new(ids, logins);
        agents.run_as(CLAUDE);
        let Sending {
            at, channel, queue, ..
        } = sending(&agents.talk);
        Scene {
            agents,
            at,
            channel,
            queue,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        self.agents.talk.cloud()
    }

    /// The cloud, Ada, Bob, the channel and its `default`.
    pub fn parts(&self) -> (&Cloud, &Person, &Person, (SpaceId, ChannelId), QueueId) {
        (self.cloud(), self.ada(), self.bob(), self.at, self.queue)
    }

    pub fn ada(&self) -> &Person {
        &self.agents.talk.ada
    }

    pub fn bob(&self) -> &Person {
        &self.agents.talk.played[0].person
    }

    /// Ada's machine.
    pub fn machine(&self) -> &Machine {
        &self.agents.talk.ada.machine
    }

    /// Where and as whom a run as `agent` goes: `~/work/acme`, as `agent`.
    pub fn as_agent<'a>(&'a self, agent: &'a str) -> (&'a Path, Option<&'a str>) {
        (self.agents.folder.as_path(), Some(agent))
    }

    /// Where and as whom a run as the person goes: `~/work/acme`, as the person.
    pub fn as_person(&self) -> (&Path, Option<&str>) {
        (self.agents.folder.as_path(), None)
    }

    /// `--channel <channel>`, followed by `args`.
    pub fn in_channel<'a>(&'a self, args: &[&'a str]) -> Vec<&'a str> {
        let mut all = vec![crate::fixtures::CHANNEL, self.channel.as_str()];
        all.extend_from_slice(args);
        all
    }
}

/// The task commands.
pub const TASKS: &str = "tasks";
pub const RESTART_TASK: &str = "restart-task";
pub const CANCEL_TASK: &str = "cancel-task";

/// The states, progresses and reasons of "Task states".
pub const QUEUED: &str = "queued";
pub const CLAIMED: &str = "claimed";
pub const DONE: &str = "done";
pub const DEAD: &str = "dead";
pub const CANCELLED: &str = "cancelled";
pub const STARTED: &str = "started";
pub const AWAITING_APPROVAL: &str = "awaiting-approval";
pub const LEASE_EXPIRED: &str = "lease-expired";
pub const FAILED: &str = "failed";
pub const RELEASED: &str = "released";
pub const DEADLINE_PASSED: &str = "deadline-passed";

/// The problems the tests meet.
pub const NOT_DEAD: &str = "not-dead";
pub const NOT_CANCELLABLE: &str = "not-cancellable";
pub const CLAIM_LOST: &str = "claim-lost";

/// How long the tests wait for reads to show a lease's end or a deadline: the server shows them
/// within 10 seconds.
const READS_WITHIN: Duration = Duration::from_secs(30);

/// Runs `[--agent <agent>] tasks <args>` on `machine` in `dir`; fails unless it succeeds.
#[track_caller]
pub fn listed(machine: &Machine, at: (&Path, Option<&str>), args: &[&str]) -> Finished {
    let ran = run_in(machine, at, TASKS, args);
    assert!(ran.success, "tasks {args:?}: {ran:?}");
    ran
}

/// The lines of `finished` that refer to `task` by its `#<ref>` or name its ID, in lowercase.
pub fn lines_of(finished: &Finished, task: TaskId) -> Vec<String> {
    let id = task.to_string().to_lowercase();
    (finished.shown().to_lowercase().lines())
        .filter(|line| refers_to_task(line, task) || line.contains(&id))
        .map(str::to_owned)
        .collect()
}

/// The one line of `finished`, a run of `skald tasks`, that refers to `task`; fails unless there
/// is one.
#[track_caller]
pub fn line_of(finished: &Finished, task: TaskId) -> String {
    let found = lines_of(finished, task);
    match found.as_slice() {
        [line] => line.clone(),
        _ => panic!("not one line of {task}: {found:?} in {finished:?}"),
    }
}

/// Fails unless `finished`, a run of `skald tasks`, lists `task` on one line holding each of
/// `parts` as a word, and none of `absent`.
#[track_caller]
pub fn assert_listed(finished: &Finished, task: TaskId, parts: &[&str], absent: &[&str]) {
    let line = line_of(finished, task);
    let found = words(&line);
    for part in parts {
        assert!(found.contains(part), "{task}: no {part} in {line:?}");
    }
    for word in absent {
        assert!(!found.contains(word), "{task}: {word} in {line:?}");
    }
}

/// Fails if `finished` refers to `task`.
#[track_caller]
pub fn assert_not_listed(finished: &Finished, task: TaskId) {
    let found = lines_of(finished, task);
    assert!(found.is_empty(), "{task} listed: {found:?}");
}

/// The reference `#<ref>` that `finished`, a run of `skald enqueue`, says for `task`.
#[track_caller]
pub fn reference_of(finished: &Finished, task: TaskId) -> String {
    let shown = finished.shown();
    let found =
        (references(&shown).into_iter()).find(|short| refers_to_task(&format!("#{short}"), task));
    let short = found.unwrap_or_else(|| panic!("no #<ref> of {task}: {finished:?}"));
    format!("#{short}")
}

/// The one JSON object of `finished`, a run of `skald tasks --json`, whose `task` is `task`; fails
/// unless there is one.
#[track_caller]
pub fn object_of(finished: &Finished, task: TaskId) -> Value {
    let id = task.to_string();
    let found: Vec<Value> = (json_objects(&finished.stdout).into_iter())
        .filter(|object| object["task"].as_str() == Some(id.as_str()))
        .collect();
    match found.as_slice() {
        [object] => object.clone(),
        _ => panic!("not one object of {task}: {finished:?}"),
    }
}

/// A task as the server's reads give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub task: TaskId,
    pub state: String,
    pub progress: Option<String>,
    pub reason: Option<String>,
    pub attempts: u64,
    /// Its last claim's `claim_id`, generation and device, if it had one.
    pub claim: Option<(String, u64, String)>,
}

impl Read {
    /// Its state, attempts, progress and reason.
    pub fn shows(&self) -> (&str, u64, Option<&str>, Option<&str>) {
        let (progress, reason) = (self.progress.as_deref(), self.reason.as_deref());
        (self.state.as_str(), self.attempts, progress, reason)
    }
}

/// `item`, the server's answer of the task `task`, read.
#[track_caller]
fn read_of(task: TaskId, item: &Value) -> Read {
    let text = |name: &str| item[name].as_str().map(str::to_owned);
    let claim = item
        .get("claim")
        .filter(|claim| !claim.is_null())
        .map(|claim| {
            let id = claim["claim_id"].as_str().unwrap_or_default().to_owned();
            let device = claim["device"].as_str().unwrap_or_default().to_owned();
            (id, claim["generation"].as_u64().unwrap_or_default(), device)
        });
    Read {
        task,
        state: text("state").unwrap_or_else(|| panic!("no state in {item}")),
        progress: text("progress"),
        reason: text("reason"),
        attempts: (item["attempts"].as_u64()).unwrap_or_else(|| panic!("no attempts in {item}")),
        claim,
    }
}

/// The task `task` of `at`'s channel's queue `queue`, as the server gives it to `person`'s device.
#[track_caller]
pub fn read(
    cloud: &Cloud,
    person: &Person,
    (at, queue): ((SpaceId, ChannelId), QueueId),
    task: TaskId,
) -> Read {
    let path = format!("{}/{task}", tasks_path(at, queue));
    let (status, body) = proved(cloud, person, ("GET", &path), None);
    assert_eq!(status, 200, "GET {path} as {}: {body}", person.host);
    read_of(task, &body)
}

/// As [`read`], once reads show `task` in `state`; fails unless they do within a while.
#[track_caller]
pub fn awaits(
    cloud: &Cloud,
    person: &Person,
    (at, queue): ((SpaceId, ChannelId), QueueId),
    (task, state): (TaskId, &str),
) -> Read {
    let until = Instant::now() + READS_WITHIN;
    loop {
        let now = read(cloud, person, (at, queue), task);
        if now.state == state {
            return now;
        }
        assert!(
            Instant::now() < until,
            "{task} still {:?}, not {state}",
            now.shows()
        );
        thread::sleep(Duration::from_millis(250));
    }
}

/// A claim a test device holds: its task, `claim_id` and generation, in its queue of its channel.
#[derive(Debug, Clone)]
pub struct Held {
    pub at: (SpaceId, ChannelId),
    pub queue: QueueId,
    pub task: TaskId,
    pub claim_id: String,
    pub generation: u64,
}

/// `holder`'s device claims a task of `at`'s channel's queue `queue`, stating the trust level
/// `peer` and taking tasks without a template; fails unless it gets one. Gives the claim, as the
/// server's reads show it held by that device.
#[track_caller]
pub fn claims(cloud: &Cloud, holder: &Person, at: (SpaceId, ChannelId), queue: QueueId) -> Held {
    let path = format!("{}/{queue}/claims", queues_path(at));
    let takes = json!({ "trust_levels": ["peer"], "templates": [], "without_template": true });
    let (status, body) = proved(cloud, holder, ("POST", &path), Some(&takes));
    assert_eq!(
        status, 200,
        "POST {path} as {}: no claim: {body}",
        holder.host
    );
    let device = holder.device.to_string();
    let found: Vec<Read> = (tasks(cloud, holder, at, queue).iter())
        .map(|stored| read(cloud, holder, (at, queue), stored.task.task))
        .filter(|one| one.state == CLAIMED)
        .filter(|one| one.claim.as_ref().is_some_and(|claim| claim.2 == device))
        .collect();
    let [one] = found.as_slice() else {
        panic!(
            "not one task of {queue} claimed by {}: {found:?}",
            holder.host
        );
    };
    let (claim_id, generation, _) = one.claim.clone().expect("a claim");
    Held {
        at,
        queue,
        task: one.task,
        claim_id,
        generation,
    }
}

/// `holder`'s device sends `act` of `held`, `extend`, `complete`, `fail` or `release`, with the
/// fields of `more` beside the claim's: the status and the body of the answer.
pub fn acts(
    cloud: &Cloud,
    holder: &Person,
    held: &Held,
    (act, more): (&str, Value),
) -> (u16, Value) {
    let path = format!("{}/{}/{act}", tasks_path(held.at, held.queue), held.task);
    let mut body = json!({ "claim_id": held.claim_id, "generation": held.generation });
    if let (Some(fields), Value::Object(extra)) = (body.as_object_mut(), more) {
        fields.extend(extra);
    }
    proved(cloud, holder, ("POST", &path), Some(&body))
}

/// As [`acts`]; fails unless the server takes it.
#[track_caller]
pub fn taken(cloud: &Cloud, holder: &Person, held: &Held, (act, more): (&str, Value)) {
    let (status, body) = acts(cloud, holder, held, (act, more));
    assert!(
        (200..300).contains(&status),
        "{act} of {}: {status} {body}",
        held.task
    );
}

/// `holder`'s device extends `held` with `progress`; fails unless the server takes it.
#[track_caller]
pub fn extends(cloud: &Cloud, holder: &Person, held: &Held, progress: &str) {
    taken(
        cloud,
        holder,
        held,
        ("extend", json!({ "progress": progress })),
    );
}

/// `holder`'s device fails `held`, with retry if `retry`; fails unless the server takes it.
#[track_caller]
pub fn fails(cloud: &Cloud, holder: &Person, held: &Held, retry: bool) {
    taken(cloud, holder, held, ("fail", json!({ "retry": retry })));
}

/// The problem a refusal names: the last segment of its `type`.
pub fn problem(body: &Value) -> &str {
    let kind = body["type"].as_str().unwrap_or_default();
    kind.rsplit('/').next().unwrap_or_default()
}

/// `seconds` pass for `task`: every time the server keeps in a row that names it moves back by that
/// much.
pub fn passes(cloud: &Cloud, task: TaskId, seconds: u64) {
    cloud.execute(&format!(
        "DO $$ DECLARE tab regclass; moves text; BEGIN
           FOR tab, moves IN
             SELECT col.attrelid::regclass,
                    string_agg(format('%1$I = %1$I - interval ''{seconds} seconds''',
                                      col.attname), ', ')
             FROM pg_attribute col JOIN pg_class cls ON cls.oid = col.attrelid
             WHERE cls.relkind = 'r' AND cls.relnamespace = current_schema()::regnamespace
               AND col.atttypid = 'timestamptz'::regtype AND col.attnum > 0 AND NOT col.attisdropped
             GROUP BY col.attrelid
           LOOP
             EXECUTE format('UPDATE %s AS one SET %s WHERE strpos(one::text, %L) > 0',
                            tab, moves, '{task}');
           END LOOP;
         END $$;"
    ));
}
