//! A task's results on the device that enqueued it (`docs/protocol/queues.md`, "Results").
//!
//! - A result that passes the checks of a message ([`crate::recv`]) is dropped, with a warning,
//!   unless it names a task this device enqueued in the result's channel, its `to` is that task's
//!   `from` and its `from` that task's `to`, if any; and, for `done`, the server gives the task as
//!   `done` with the result's `claim_id` and `generation`, claimed by the result's sending device.
//! - The first `done` result that passes is the task's, kept with the task ([`crate::state`]); a
//!   later one is dropped with a warning. Each `failed` one that passes is a failed attempt.
//! - A stateless run keeps no tasks, so it asks the server: the task is in a queue of the result's
//!   channel, `to` is the run's agent, and the device in the task's `sched`, if any, sent it.
//! - A result that passes is kept and handed over as a message is, to its `to` agent with its
//!   `read`; its line names the task's reference and the outcome ([`crate::forms`]).
//! - A result the device sends of a task it enqueued itself is kept for it as its own message is
//!   ([`recv::own_message`]), and, if `done`, taken as the task's.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_proto::{
    Address, Cbor as _, ChannelId, ClaimId, Content, DeviceId, Header, MessageId, Outcome, QueueId,
    QueuedTask, SpaceId, TaskId, TaskResult,
};

use crate::connection::{self, printable};
use crate::devices::text;
use crate::forms::ResultOf;
use crate::queues::{self, Queue};
use crate::spaces::Follower;
use crate::state::State;

/// What a kept result's line names of `result`.
pub fn result_of(result: &TaskResult) -> ResultOf {
    ResultOf {
        task: result.task.to_string(),
        outcome: outcome(result.outcome).to_owned(),
    }
}

/// `outcome` as the CLI writes it.
fn outcome(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "done",
        Outcome::Failed => "failed",
    }
}

/// Checks 2 and 3 of "Results" of `result`, the task result of the message with `header` from the
/// device `sender`, its content `from` and `to`, and `ours`, the agent of a stateless run; for a
/// `done` result that passes, takes it as the task's. Gives why it is dropped, if it is; fails if
/// the server does not answer, so the run acks nothing it has not judged.
pub fn judge(
    follower: &mut Follower,
    (header, sender): (&Header, DeviceId),
    (from, to, ours): (&Address, Option<&Address>, Option<&Address>),
    result: &TaskResult,
) -> Result<Result<(), String>, String> {
    let at = (header.space, header.channel);
    let task = result.task;
    let found = if follower.stateless() {
        on_server(follower, at, sender, (to, ours), task)?
    } else {
        enqueued(&follower.state, at, (from, to), task)
            .map_err(text)?
            .map(Found::Queue)
    };
    let found = match found {
        Ok(found) => found,
        Err(why) => return Ok(Err(why)),
    };
    if result.outcome == Outcome::Failed {
        return Ok(Ok(()));
    }
    let given = match found {
        Found::Given(given) => given,
        Found::Queue(queue) => match given(follower, at, queue, task)? {
            Ok(given) => given,
            Err(why) => return Ok(Err(why)),
        },
    };
    if let Err(why) = completed(&given, result, sender) {
        return Ok(Err(why));
    }
    if !follower.stateless()
        && !follower
            .state
            .take_result(task, header.msg_id)
            .map_err(text)?
    {
        return Ok(Err(format!(
            "its task {task} has a done result already, which this device took"
        )));
    }
    Ok(Ok(()))
}

/// Where a result's task is: its queue, or the task as the server gives it.
enum Found {
    Queue(QueueId),
    Given(Given),
}

/// Check 2 of "Results" by `state`, the device's: the task `task` is one the device enqueued in
/// the channel `at`, `to` is its `from` and `from` its `to`, if any. Gives its queue, or why not.
fn enqueued(
    state: &State,
    at: (SpaceId, ChannelId),
    (from, to): (&Address, Option<&Address>),
    task: TaskId,
) -> Result<Result<QueueId, String>, crate::state::Error> {
    let Some(enqueued) = state.enqueued(task)? else {
        return Ok(Err(format!("its task {task} is none this device enqueued")));
    };
    let (Some(channel), Some(queue)) = (enqueued.channel, enqueued.queue) else {
        return Ok(Err(format!(
            "its task {task} was enqueued by an older skald, which kept not where"
        )));
    };
    if (enqueued.space, channel) != at {
        return Ok(Err(format!(
            "its task {task} is not one this device enqueued in the channel {}",
            at.1
        )));
    }
    if to.map(ToString::to_string) != enqueued.author {
        return Ok(Err(format!("its to is not the from of its task {task}")));
    }
    if (enqueued.addressee).is_some_and(|addressee| addressee != from.to_string()) {
        return Ok(Err(format!("its from is not the to of its task {task}")));
    }
    Ok(Ok(queue))
}

/// Check 2 of "Results" of a stateless run, whose agent is `ours`, against the server: `to` is
/// `ours`, the task `task` is in a queue of the channel `at`, and the device in its `sched`, if
/// any, is `sender`. Gives the task as the server gives it, or why not.
fn on_server(
    follower: &mut Follower,
    at: (SpaceId, ChannelId),
    sender: DeviceId,
    (to, ours): (Option<&Address>, Option<&Address>),
    task: TaskId,
) -> Result<Result<Found, String>, String> {
    if ours.is_none() || to != ours {
        return Ok(Err("its to is not this run's agent".to_owned()));
    }
    #[derive(Deserialize)]
    struct Queues {
        queues: Vec<Queue>,
    }
    let listed = match get(follower, &queues::queues_path(at))? {
        Ok(answer) => serde_json::from_slice::<Queues>(&answer)
            .map_err(|_| "unexpected answer from the server: a channel's queues".to_owned())?,
        Err(why) => {
            return Ok(Err(format!(
                "the server lists no queues of its channel: {why}"
            )));
        }
    };
    for queue in listed.queues {
        let Ok(given) = given(follower, at, queue.id()?, task)? else {
            continue;
        };
        let queued = (URL_SAFE_NO_PAD.decode(&given.queued_task).ok())
            .and_then(|bytes| QueuedTask::from_cbor(&bytes).ok())
            .ok_or_else(|| "unexpected answer from the server: a task".to_owned())?;
        if queued.sched.device.is_some_and(|device| device != sender) {
            return Ok(Err(format!(
                "its task {task} names another device than its sending device in sched"
            )));
        }
        return Ok(Ok(Found::Given(given)));
    }
    Ok(Err(format!(
        "its task {task} is in no queue of its channel"
    )))
}

/// A task as the server gives it: of `Task` in the OpenAPI spec, what the checks read.
#[derive(Deserialize)]
struct Given {
    queued_task: String,
    state: String,
    claim: Option<GivenClaim>,
}

/// A task's last claim as the server gives it.
#[derive(Deserialize)]
struct GivenClaim {
    claim_id: String,
    generation: u64,
    device: String,
}

/// The task `task` of the queue `queue` of the channel `at` as the server gives it, or the problem
/// of its refusal.
fn given(
    follower: &mut Follower,
    at: (SpaceId, ChannelId),
    queue: QueueId,
    task: TaskId,
) -> Result<Result<Given, String>, String> {
    let path = format!("{}/{queue}/tasks/{task}", queues::queues_path(at));
    Ok(match get(follower, &path)? {
        Ok(answer) => Ok(serde_json::from_slice(&answer)
            .map_err(|_| "unexpected answer from the server: a task".to_owned())?),
        Err(why) => Err(format!("the server gives no task {task}: {why}")),
    })
}

/// `GET <path>` as `follower`'s device: the answer, or the problem of the server's refusal; fails
/// if the server does not answer.
fn get(follower: &mut Follower, path: &str) -> Result<Result<Vec<u8>, String>, String> {
    let proof = queues::proof(follower)?;
    match (follower.connection).get_proved(path, (&follower.token, &proof)) {
        Ok(answer) => Ok(Ok(answer)),
        Err(connection::Error::Refused { kind, .. }) => Ok(Err(queues::problem(&kind))),
        Err(err) => Err(err.to_string()),
    }
}

/// Check 3 of "Results": the server gives the task as `done` with `result`'s `claim_id` and
/// `generation`, claimed by `sender`.
fn completed(given: &Given, result: &TaskResult, sender: DeviceId) -> Result<(), String> {
    let task = result.task;
    if given.state != "done" {
        return Err(format!(
            "the server gives its task {task} as {}, not done",
            printable(&given.state)
        ));
    }
    let claim = (given.claim.as_ref()).ok_or_else(|| format!("its task {task} had no claim"))?;
    if claim.claim_id.parse::<ClaimId>().ok() != Some(result.claim_id)
        || claim.generation != result.generation
    {
        return Err(format!("its task {task} was done by another claim"));
    }
    if claim.device.parse::<DeviceId>().ok() != Some(sender) {
        return Err(format!("its task {task} was done by another device"));
    }
    Ok(())
}

/// Whether the device keeps for itself its own result `content`, sent as `msg_id` in `channel`, of
/// the task `task`: it enqueued the task in that channel and, for `done`, takes it as the task's.
pub fn kept_here(
    follower: &Follower,
    (channel, task): (ChannelId, TaskId),
    msg_id: MessageId,
    content: &Content,
) -> Result<bool, String> {
    let Some(enqueued) = follower.state.enqueued(task).map_err(text)? else {
        return Ok(false);
    };
    if enqueued.channel != Some(channel) {
        return Ok(false);
    }
    match content.task_result() {
        Some(Ok(result)) if result.outcome == Outcome::Done => {
            follower.state.take_result(task, msg_id).map_err(text)
        }
        Some(Ok(_)) => Ok(true),
        _ => Ok(false),
    }
}
