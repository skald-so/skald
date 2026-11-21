//! A channel's tasks with their states in the CLI (`docs/protocol/queues.md`, "Task states";
//! `docs/dev/architecture.md`, "Queue").
//!
//! - `skald tasks` lists the tasks the server gives the device in the run's channel
//!   ([`queues::Run`]), or in one of its queues: each with its `#<ref>`, queue, state, its holder's
//!   last progress while `claimed`, its reason while `dead`, and its attempts. The person's lines
//!   hold the task's ID; an agent's, only the tasks it enqueued ([`crate::state`]), name no ID.
//!   References count the kept messages and the listed tasks ([`crate::forms`]).
//! - `skald restart-task` and `skald cancel-task` name a task of the channel by `#<ref>`, `<ref>`
//!   or its ID and have the server restart or cancel it. They are the person's: an agent's run
//!   makes no request. A refusal of the server fails the run, naming its problem.

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;
use skald_proto::{Cbor as _, ChannelId, MessageId, Name, QueuedTask, SpaceId, TaskId};

use crate::connection::{Method, printable};
use crate::devices::text;
use crate::forms::References;
use crate::queues::{self, Queue, Run};
use crate::recv;
use crate::reply;
use crate::spaces::Follower;

/// `Task` in the OpenAPI spec.
#[derive(Deserialize)]
struct Answered {
    queued_task: String,
    state: String,
    progress: Option<String>,
    reason: Option<String>,
    attempts: u64,
    claim: Option<Claim>,
}

/// A task's last claim, as the server gives it.
#[derive(Deserialize)]
struct Claim {
    device: String,
}

/// `Tasks` in the OpenAPI spec.
#[derive(Deserialize)]
struct Answer {
    tasks: Vec<Answered>,
}

/// A task of a queue of the channel, as the server gives it.
struct Listed<'q> {
    queued: QueuedTask,
    queue: &'q Queue,
    state: String,
    progress: Option<String>,
    reason: Option<String>,
    attempts: u64,
    /// The device of its last claim, if it had one.
    claim_device: Option<String>,
}

impl Listed<'_> {
    /// Its task ID.
    fn task(&self) -> TaskId {
        self.queued.task
    }

    /// Its state, followed by its progress or its reason, if any.
    fn state(&self) -> String {
        match self.progress.as_deref().or(self.reason.as_deref()) {
            Some(detail) => format!("{}  {}", printable(&self.state), printable(detail)),
            None => printable(&self.state),
        }
    }

    /// Its line in `skald tasks`: with its ID for the person, without for an agent.
    fn line(&self, reference: &str, agent: bool) -> String {
        let id = if agent {
            String::new()
        } else {
            format!("  {}", self.task())
        };
        format!(
            "#{reference}{id}  {}  {}  attempts {}",
            printable(&self.queue.name),
            self.state(),
            self.attempts
        )
    }

    /// Its JSON object, with every field, in `channel`.
    fn json(&self, reference: &str, channel: ChannelId) -> Result<serde_json::Value, String> {
        let sched = &self.queued.sched;
        Ok(json!({
            "task": self.task().to_string(),
            "ref": reference,
            "channel": channel.to_string(),
            "queue": self.queue.id()?.to_string(),
            "queue_name": self.queue.name,
            "state": self.state,
            "progress": self.progress,
            "reason": self.reason,
            "attempts": self.attempts,
            "claim_device": self.claim_device,
            "deadline": sched.deadline,
            "device": sched.device.map(|device| device.to_string()),
        }))
    }
}

/// The tasks of `queues`, queues of the channel `at`, that the server gives `follower`'s device, in
/// the order of `queues`, each queue's in the order the server stored them.
fn tasks_of<'q>(
    follower: &mut Follower,
    at: (SpaceId, ChannelId),
    queues: &'q [Queue],
) -> Result<Vec<Listed<'q>>, String> {
    let mut found = Vec::new();
    for queue in queues {
        let path = format!("{}/{}/tasks", queues::queues_path(at), queue.id()?);
        let proof = queues::proof(follower)?;
        let answer = (follower.connection)
            .get_proved(&path, (&follower.token, &proof))
            .map_err(|err| {
                let what = format!("the tasks of the queue {} are not listed", queue.name);
                queues::refused(&what, err)
            })?;
        let unexpected = || "unexpected answer from the server: a queue's tasks".to_owned();
        let Answer { tasks } = serde_json::from_slice(&answer).map_err(|_| unexpected())?;
        for task in tasks {
            let bytes = (URL_SAFE_NO_PAD.decode(&task.queued_task)).map_err(|_| unexpected())?;
            let queued = QueuedTask::from_cbor(&bytes).map_err(|_| unexpected())?;
            found.push(Listed {
                queued,
                queue,
                state: task.state,
                progress: task.progress,
                reason: task.reason,
                attempts: task.attempts,
                claim_device: task.claim.map(|claim| claim.device),
            });
        }
    }
    Ok(found)
}

/// The references of `tasks` among the messages `follower`'s device keeps.
fn references(follower: &Follower, tasks: &[Listed]) -> References {
    let kept = follower.state.kept_ids().unwrap_or_default();
    let listed = tasks.iter().map(|one| as_message(one.task()));
    References::new(kept.into_iter().chain(listed))
}

/// `task` as a `msg_id`, whose reference is the task's.
fn as_message(task: TaskId) -> MessageId {
    MessageId::from_bytes(*task.as_bytes())
}

/// `skald tasks`: lists the tasks of the run's channel, or of its queue `queue`, as JSON objects,
/// one per line, if `json`; as `agent`, only those it enqueued.
pub fn list(
    run: &Run,
    queue: Option<&str>,
    json: bool,
    agent: Option<&Name>,
) -> Result<(), String> {
    recv::holding_warnings(run.dir, agent.is_some(), || {
        let (mut synced, channel, _) = run.synced()?;
        let at = (synced.space.id, channel);
        let follower = &mut synced.follower;
        let queues = match queue {
            Some(name) => vec![queues::named(follower, at, &queues::queue_name(name)?)?],
            None => queues::listed(follower, at)?,
        };
        let mut tasks = tasks_of(follower, at, &queues)?;
        if let Some(agent) = agent {
            let own: BTreeSet<TaskId> = (follower.state)
                .enqueued_by(at.0, agent.as_str())
                .map_err(text)?;
            tasks.retain(|one| own.contains(&one.task()));
        }
        let references = references(follower, &tasks);
        let reference = |one: &Listed| references.of(as_message(one.task()));
        let place = match queue {
            Some(name) => format!("the queue {name} of the channel {channel}"),
            None => format!("the channel {channel}"),
        };
        if json {
            for one in &tasks {
                println!("{}", one.json(&reference(one), channel)?);
            }
        } else if agent.is_some() {
            if tasks.is_empty() {
                println!("no tasks");
            }
            for one in &tasks {
                println!("{}", one.line(&reference(one), true));
            }
        } else if tasks.is_empty() {
            println!("No tasks in {place}.");
        } else {
            println!("The tasks of {place}:");
            for one in &tasks {
                println!("  {}", one.line(&reference(one), false));
            }
        }
        Ok(())
    })
}

/// A change the person makes to a task.
#[derive(Clone, Copy)]
pub enum Change {
    Restart,
    Cancel,
}

impl Change {
    /// The last segment of its request's path.
    fn segment(self) -> &'static str {
        match self {
            Change::Restart => "restart",
            Change::Cancel => "cancel",
        }
    }

    /// What it did to a task, as a run says it.
    fn done(self) -> &'static str {
        match self {
            Change::Restart => "restarted",
            Change::Cancel => "cancelled",
        }
    }
}

/// `skald restart-task` and `skald cancel-task`: makes `change` to the task of the run's channel
/// that `given` names, by `#<ref>`, `<ref>` or its ID.
pub fn change(run: &Run, given: &str, change: Change) -> Result<(), String> {
    let shown = printable(given.trim());
    let named = reply::named(given).ok_or_else(|| {
        format!(
            "\"{shown}\" names no task: give its #<ref>, as skald tasks shows it, or its ID; \
             nothing is {}",
            change.done()
        )
    })?;
    let (mut synced, channel, _) = run.synced()?;
    let at = (synced.space.id, channel);
    let follower = &mut synced.follower;
    let queues = queues::listed(follower, at)?;
    let tasks = tasks_of(follower, at, &queues)?;
    let references = references(follower, &tasks);
    let found = reply::pick(&named, tasks, |one| as_message(one.task()), &[])
        .map_err(|shared| unpicked((&shown, channel), shared, change))?;
    let (task, queue) = (found.task(), found.queue.id()?);
    let reference = references.of(as_message(task));
    let path = format!(
        "{}/{queue}/tasks/{task}/{}",
        queues::queues_path(at),
        change.segment()
    );
    queues::proved_json(follower, Method::Post, &path, &json!({})).map_err(|err| {
        queues::refused(
            &format!("the task {task} (#{reference}) is not {}", change.done()),
            err,
        )
    })?;
    let now = match change {
        Change::Restart => "queued, with attempts 0",
        Change::Cancel => "cancelled",
    };
    println!(
        "The task {task} (#{reference}) of the queue {} is {now}.",
        printable(&found.queue.name)
    );
    Ok(())
}

/// Why a run makes `change` to no task of `channel` by `shown`, as it names it: a reference more
/// than one of its tasks shares if `shared`, otherwise none.
fn unpicked((shown, channel): (&str, ChannelId), shared: bool, change: Change) -> String {
    let why = if shared {
        format!(
            "{shown} is the reference of more than one task of the channel {channel}: give more \
             of it, or the ID"
        )
    } else {
        format!("the channel {channel} has no task {shown}")
    };
    format!("{why}; nothing is {}", change.done())
}

#[cfg(test)]
mod tests {
    use skald_proto::{QueueId, Sched, Unknown};

    use super::*;

    /// A task of `queue` in `state` with its progress or reason, `detail`, and 2 attempts.
    fn listed<'q>(
        queue: &'q Queue,
        state: &str,
        detail: (Option<&str>, Option<&str>),
    ) -> Listed<'q> {
        Listed {
            queued: QueuedTask {
                task: TaskId::generate(),
                queue: QueueId::generate(),
                sched: Sched::default(),
                epoch: 1,
                enc: [0; 32],
                ct: Vec::new(),
                unknown: Unknown::default(),
            },
            queue,
            state: state.to_owned(),
            progress: detail.0.map(str::to_owned),
            reason: detail.1.map(str::to_owned),
            attempts: 2,
            claim_device: None,
        }
    }

    /// A task's line holds its reference, its queue, its state with its progress or reason, and
    /// its attempts; the person's also its ID.
    #[test]
    fn a_task_s_line_holds_its_state_and_the_person_s_its_id() {
        let queue: Queue = serde_json::from_value(json!({
            "id": QueueId::generate().to_string(),
            "name": "deploys",
            "mode": "at-least-once",
            "lease": 300,
            "max_attempts": 5,
        }))
        .unwrap();
        let claimed = listed(&queue, "claimed", (Some("started"), None));
        assert_eq!(
            claimed.line("abcd", true),
            "#abcd  deploys  claimed  started  attempts 2"
        );
        assert_eq!(
            claimed.line("abcd", false),
            format!(
                "#abcd  {}  deploys  claimed  started  attempts 2",
                claimed.task()
            )
        );
        let dead = listed(&queue, "dead", (None, Some("failed")));
        assert_eq!(
            dead.line("efgh", true),
            "#efgh  deploys  dead  failed  attempts 2"
        );
        let queued = listed(&queue, "queued", (None, None));
        assert_eq!(
            queued.line("ijkl", true),
            "#ijkl  deploys  queued  attempts 2"
        );
    }
}
