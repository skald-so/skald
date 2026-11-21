//! Claiming, running and completing tasks in the CLI (`docs/protocol/queues.md`, "Claims and
//! leases", "Fencing", "Claim checks" and "Results"; `docs/dev/architecture.md`, "Queue").
//!
//! - `skald claim`, as an agent or the person, first accepts the space's latest revisions and goes
//!   through the mailbox as `skald send` does, then claims one task from a queue of the channel,
//!   by default `default`, stating the trust level `own` and that it takes tasks without a
//!   template. With no task it says so in one line.
//! - The device runs a task only once the claim checks 1–6 pass, the trust level of `sender` as a
//!   message's ([`recv`]). A task that fails check 1 for a key the device lacks, or check 6, a
//!   `client` device releases and a member `service` device fails with retry; one that fails any
//!   other check the device fails without retry, warning its user. The agent gets none of them.
//! - A task that passes is extended at once with `started`, then handed to the agent as one line
//!   with its `#<ref>`, made from its ID as a message's from its `msg_id`, labelled with `from`
//!   and the sender's trust level as a message is; `--json` has every field. It counts as a
//!   message handed to the agent for the `hops` of its next send.
//! - The device keeps each claim it holds, and when it sent the last answered request that started
//!   its lease. `skald extend`, `skald complete` and `skald fail` name a claim by the task's
//!   `#<ref>` or ID: an agent's only its own in its folder's space. On `claim-lost`, or once the
//!   lease passed on the device's clock without an answered extend, the command says the claim is
//!   lost, the device drops it and sends no result.
//! - A complete or fail waits in the local state before its request, so a run killed after it
//!   repeats it. Once the server takes it, the device sends the `task-result` ([`groups`]): from
//!   the claiming agent, to the task's `from`, `hops` the task's plus one, under `mls` when the
//!   task's `sender` is a member device of the current epoch, otherwise under `hpke` to `sender`,
//!   with a history entry; also over the limit on `hops`, as nothing answers a result. The claim
//!   ends with that result in the outbox. Each `skald recv` repeats the completes and fails that
//!   wait, and sends the results that wait, first.

use serde::{Deserialize, Serialize};
use serde_json::json;
use skald_crypto::{HistoryKey, StateKey, check_grant, trust_level};
use skald_proto::{
    Address, Cbor as _, Cert, ChannelId, Claim, ClaimId, Content, DeviceId, DeviceKind, MessageId,
    Name, Outcome, QueueId, QueueMode, Roster, ServerName, SpaceId, TaskId, TaskResult, TrustLevel,
    Unknown,
};

use crate::connection::{self, Method, printable};
use crate::devices::text;
use crate::follow::SpaceDevices;
use crate::forms::{self, Form, References};
use crate::groups::{self, Link, Rosters};
use crate::history;
use crate::key_packages;
use crate::queues::{self, Run};
use crate::recv;
use crate::reply;
use crate::send;
use crate::service;
use crate::spaces::{self, Follower, Scope, Synced};
use crate::state::{self, State, TaskClaim};

/// What a claim states it takes: the trust level `own` and tasks without a template. Other levels
/// and templates come with the device's queue settings.
const TAKES: &str = r#"{"trust_levels":["own"],"templates":[],"without_template":true}"#;

/// A claim's progress, as an extend states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Progress {
    /// The task runs.
    Started,
    /// It waits for its user's yes.
    AwaitingApproval,
}

impl Progress {
    /// Its name, as the server and the CLI write it.
    fn name(self) -> &'static str {
        match self {
            Progress::Started => "started",
            Progress::AwaitingApproval => "awaiting-approval",
        }
    }
}

/// How an agent or the person ends a claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Complete,
    /// A fail, with retry if true.
    Fail(bool),
}

/// What the device keeps of a claim besides its task, place, agent and time, sealed under the
/// state key: each ID in its text form.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    claim_id: String,
    generation: u64,
    /// In seconds.
    lease: u64,
    /// The task's sending device, its `from` and its `hops`.
    sender: String,
    from: String,
    hops: u64,
    /// Its complete or fail, once given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<End>,
}

/// A complete or fail of a claim, with its text, and whether the server took it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct End {
    done: bool,
    retry: bool,
    text: String,
    taken: bool,
}

impl End {
    /// The request's last path segment.
    fn act(&self) -> &'static str {
        if self.done { "complete" } else { "fail" }
    }
}

/// A claim the device holds, opened.
struct Held {
    row: TaskClaim,
    record: Record,
}

/// What the claim checks make of a claimed task.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// It passes.
    Runs(Box<Passed>),
    /// It fails check 1 for a key the device lacks, or check 6: the device hands it back; why.
    Back(String),
    /// It fails another check: the device fails it without retry and tells its user; why.
    Refused(String),
}

/// A task that passed the claim checks, as the device hands it over and keeps its claim.
#[derive(Debug, PartialEq)]
struct Passed {
    sender: DeviceId,
    /// Its sending device's certificate `name` and trust level.
    device_name: String,
    level: TrustLevel,
    from: Address,
    to: Option<Address>,
    hops: u64,
    text: String,
}

/// What the server answered to an extend, complete, fail or release.
#[derive(Debug, PartialEq)]
enum Fenced {
    Taken,
    /// `claim-lost`.
    Lost,
    /// Refused for another lasting reason: its problem and detail.
    Refused(String),
    /// Not answered, for this reason.
    Unanswered(String),
}

/// How ending a claim went.
enum Finish {
    /// The result is in the outbox, as the message `msg_id`.
    Sent(MessageId),
    /// The server took the complete or fail; the result waits for a later run, for this reason.
    Waits(String),
    Lost,
    Refused(String),
    Unanswered(String),
}

/// `skald claim`: claims one task from the queue `queue` of the run's channel, as `agent` if
/// given, whose run holds its warnings for the person; shows it as `json` says.
pub fn claim(run: &Run, queue: &str, agent: Option<&Name>, json: bool) -> Result<(), String> {
    recv::holding_warnings(run.dir, agent.is_some(), || {
        claimed(run, queue, agent, json)
    })
}

/// [`claim`].
fn claimed(run: &Run, queue: &str, agent: Option<&Name>, json: bool) -> Result<(), String> {
    let name = queues::queue_name(queue)?;
    let (synced, channel, scope) = run.synced()?;
    let Synced {
        mut follower,
        space,
        roster,
        devices,
    } = synced;
    if follower.stateless() {
        return Err(
            "a service agent that keeps no state claims no task: it is no member of the \
             channel's group"
                .to_owned(),
        );
    }
    let server = send::server_of(&mut follower)?;
    if let Some(agent) = agent {
        (follower.state)
            .add_agent(space.id, agent.as_str())
            .map_err(text)?;
    }
    let (roster, devices) =
        send::through_mailbox(&mut follower, scope, space.id, (roster, devices))?;
    let at = (space.id, channel);
    let queue = queues::named(&mut follower, at, &name)?;
    let queue_id = queue.id()?;
    let form = Form::of(agent.is_some(), json);
    let since = state::unix_now();
    let path = format!("{}/{queue_id}/claims", queues::queues_path(at));
    let body = serde_json::from_str(TAKES).expect("a claim's body is JSON");
    let answer = queues::proved_json(&mut follower, Method::Post, &path, &body)
        .map_err(|err| queues::refused("no task is claimed", err))?;
    if answer.is_empty() {
        match form {
            Form::Short => println!("no task"),
            Form::Full => println!("No task waits for this device in the queue {name}."),
            Form::Json => println!("{}", forms::object("claim", json!({ "task": null }))),
        }
        return Ok(());
    }
    let claim = Claim::from_cbor(&answer)
        .map_err(|err| format!("unexpected answer from the server: a claim: {err}"))?;
    let task = claim.task.task;
    let reference = reference(&follower.state, task);
    let verdict = verdict(
        &follower,
        (&roster, &devices),
        &server,
        (channel, queue_id),
        &claim,
    )?;
    let fence = |follower: &mut Follower, act: &str, extra: serde_json::Value| {
        let path = task_path(at, queue_id, task, act);
        let mut body = json!({
            "claim_id": claim.claim_id.to_string(),
            "generation": claim.generation,
        });
        if let (Some(body), serde_json::Value::Object(extra)) = (body.as_object_mut(), extra) {
            body.extend(extra);
        }
        fenced(queues::proved_json(follower, Method::Post, &path, &body))
    };
    let passed = match verdict {
        Verdict::Runs(passed) => passed,
        Verdict::Back(why) => {
            // A `service` device releases no task; it fails it with retry.
            let (act, extra, done) = match follower.service {
                Some(_) => ("fail", json!({ "retry": true }), "failed with retry"),
                None => ("release", json!({}), "released"),
            };
            let answer = fence(&mut follower, act, extra);
            if answer != Fenced::Taken {
                eprintln!(
                    "skald: warning: the task #{reference} is not {done}: {}",
                    answer.why()
                );
            }
            match form {
                Form::Short => println!("no task: {done} #{reference}, as {why}"),
                Form::Full => println!(
                    "The task {task} (#{reference}) of the queue {name} is {done}, as {why}; no \
                     task runs."
                ),
                Form::Json => println!(
                    "{}",
                    forms::object(
                        "claim",
                        json!({
                            "task": null,
                            "handed_back": task.to_string(),
                            "ref": reference,
                            "why": why,
                        })
                    )
                ),
            }
            return Ok(());
        }
        Verdict::Refused(why) => {
            let answer = fence(&mut follower, "fail", json!({ "retry": false }));
            let failed = match answer {
                Fenced::Taken => "is failed without retry".to_owned(),
                other => format!(
                    "is to be failed without retry, which failed: {}",
                    other.why()
                ),
            };
            let warning = format!(
                "the task {task} (#{reference}) of the queue {name} fails the claim checks, as \
                 {why}, so it does not run and {failed}"
            );
            // The person's run warns in its output; an agent's tells the agent and holds the
            // warning for the person.
            match form {
                Form::Short => {
                    println!("warning: #{reference} does not run and {failed}: {why}");
                    eprintln!("skald: warning: {warning}");
                }
                Form::Full => println!("Warning: {warning}."),
                Form::Json => println!(
                    "{}",
                    forms::object(
                        "claim",
                        json!({
                            "task": null,
                            "refused": task.to_string(),
                            "ref": reference,
                            "warning": warning,
                        })
                    )
                ),
            }
            return Ok(());
        }
    };
    let counted = send::counted(&follower, agent);
    let record = Record {
        claim_id: claim.claim_id.to_string(),
        generation: claim.generation,
        lease: claim.lease,
        sender: passed.sender.to_string(),
        from: passed.from.to_string(),
        hops: passed.hops,
        end: None,
    };
    let mut held = Held {
        row: TaskClaim {
            task,
            space: space.id,
            channel,
            queue: queue_id,
            agent: agent.map(|agent| agent.as_str().to_owned()),
            since,
            sealed: Vec::new(),
        },
        record,
    };
    keep(&follower, &mut held, None)?;
    let extended = state::unix_now();
    match fence(&mut follower, "extend", json!({ "progress": "started" })) {
        Fenced::Taken => held.row.since = extended,
        Fenced::Lost => {
            drop_claim(&follower.state, task);
            return Err(format!(
                "the claim of the task #{reference} is lost: the server took no extend of it; \
                 the task does not run"
            ));
        }
        other => eprintln!(
            "skald: warning: the server took no extend of the claim of the task #{reference} \
             with started ({}); skald extend {reference} extends it",
            other.why()
        ),
    }
    keep(
        &follower,
        &mut held,
        counted.map(|agent| (agent, passed.hops)),
    )?;
    show(
        form,
        (&claim, &passed),
        (&space.id, channel, &name),
        &reference,
    );
    Ok(())
}

/// Shows the task of `claim` that passed the checks as `passed`, in `space`, `channel` and the
/// queue `name`, by its `reference`, in `form`.
fn show(
    form: Form,
    (claim, passed): (&Claim, &Passed),
    (space, channel, name): (&SpaceId, ChannelId, &Name),
    reference: &str,
) {
    let task = claim.task.task;
    match form {
        Form::Short => {
            let message = forms::Message {
                msg_id: task.to_string(),
                space: space.to_string(),
                channel: channel.to_string(),
                from: passed.from.to_string(),
                device: passed.sender.to_string(),
                device_name: passed.device_name.clone(),
                level: passed.level,
                text: passed.text.clone(),
                after_missing: false,
                reply_to: None,
                refused: Vec::new(),
            };
            println!("{}", message.shown(Form::Short, reference, None));
        }
        Form::Full => println!(
            "The task {task} (#{reference}) of the queue {name} in the channel {channel}, attempt \
             {}, from {} ({}), by the device {} ({}); its lease is {} s:\n  {}",
            claim.attempt,
            passed.from,
            passed.level,
            printable(&passed.device_name),
            passed.sender,
            claim.lease,
            printable(&passed.text)
        ),
        Form::Json => println!(
            "{}",
            forms::object(
                "claim",
                json!({
                    "task": task.to_string(),
                    "ref": reference,
                    "space": space.to_string(),
                    "channel": channel.to_string(),
                    "queue": claim.task.queue.to_string(),
                    "queue_name": name.as_str(),
                    "from": passed.from.to_string(),
                    "to": passed.to.as_ref().map(ToString::to_string),
                    "device": passed.sender.to_string(),
                    "device_name": passed.device_name,
                    "level": passed.level.name(),
                    "text": passed.text,
                    "hops": passed.hops,
                    "claim_id": claim.claim_id.to_string(),
                    "generation": claim.generation,
                    "attempt": claim.attempt,
                    "lease": claim.lease,
                })
            )
        ),
    }
}

/// The claim checks of `claim`, from the queue `queue` of `channel`, by `roster`, the space's last
/// accepted revision with `devices`, its accounts' devices, on `server`, as `follower`'s device
/// makes them now; fails only if the device does.
fn verdict(
    follower: &Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    server: &ServerName,
    (channel, queue): (ChannelId, QueueId),
    claim: &Claim,
) -> Result<Verdict, String> {
    let keys = follower.keys()?;
    let epoch = claim.task.epoch;
    let key = groups::read(follower, &keys, |device| {
        device
            .history_key(channel, epoch)
            .map(HistoryKey::queue_key)
    })?;
    let Some(key) = key else {
        return Ok(Verdict::Back(format!(
            "this device holds no key of its epoch {epoch}"
        )));
    };
    let refused = |why: &str| Ok(Verdict::Refused(why.to_owned()));
    let plaintext = match key.open_task(&claim.task, channel, queue) {
        Ok(plaintext) => plaintext,
        Err(err) => return refused(&err.to_string()),
    };
    let sender = plaintext.body().sender;
    let Some(cert) = recv::member_cert(&follower.own, (roster, devices), sender) else {
        return refused(
            "its sender is no device with a valid certificate in the space's last accepted \
             revision",
        );
    };
    if sender != claim.uploader {
        return refused("its sender is not the device that uploaded it");
    }
    let body = match key.check_claim(claim, channel, queue, &cert.body().signing) {
        Ok(body) => body,
        Err(err) => return refused(&err.to_string()),
    };
    let content = &body.content;
    if let Err(why) = from_sender(
        follower,
        (roster, devices),
        server,
        (cert, channel),
        content,
    ) {
        return refused(&why);
    }
    let request = match content.task() {
        Some(Ok(task)) => task.request,
        Some(Err(_)) => return refused("its task body does not decode"),
        None => return refused("its content has no task body"),
    };
    if content.counter.is_some() {
        return refused("its content has a counter, which a task's has not");
    }
    if content.to.is_some() != body.sched.device.is_some() {
        return refused("its content has a to exactly when its sched names no addressee's device");
    }
    let Some(text) = request_text(&request, body.task) else {
        return refused("its request is no A2A message whose messageId is its task ID");
    };
    if body
        .sched
        .deadline
        .is_some_and(|deadline| deadline <= state::unix_now())
    {
        return refused("its deadline passed");
    }
    if claim.mode == QueueMode::AtMostOnce && claim.attempt != 1 {
        return refused(&format!(
            "it is an at-most-once task handed out for attempt {}",
            claim.attempt
        ));
    }
    let back = |why: String| Ok(Verdict::Back(why));
    if body
        .sched
        .device
        .is_some_and(|device| device != follower.this)
    {
        return back("it is for another device".to_owned());
    }
    let level = trust_level(follower.own.account, cert, roster.body());
    if level != Some(TrustLevel::Own) {
        let level = level.map_or("none", TrustLevel::name);
        return back(format!(
            "its sender's trust level is {level}, which the claim did not state"
        ));
    }
    if let Some(template) = &body.sched.template {
        return back(format!(
            "it names the template {template}, which the claim did not state"
        ));
    }
    if let Some(to) = &content.to
        && !of_this_device(follower, roster, server, to)?
    {
        return back(format!("its to, {to}, names no agent of this device"));
    }
    Ok(Verdict::Runs(Box::new(Passed {
        sender,
        device_name: cert.body().name.clone(),
        level: TrustLevel::Own,
        from: content.from.clone(),
        to: content.to.clone(),
        hops: content.hops,
        text,
    })))
}

/// Checks 2 and 3 of "Envelope" of `content`, a task's, from the device of `cert` in `channel`:
/// `from` is an address of its account in the space, or, for a `service` device, its service
/// agent entry, with a grant that passes the checks of "Service grant"; why it fails otherwise.
fn from_sender(
    follower: &Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    server: &ServerName,
    (cert, channel): (&Cert, ChannelId),
    content: &Content,
) -> Result<(), String> {
    let body = roster.body();
    let sender = cert.body().device;
    if cert.body().kind != DeviceKind::Service {
        if content.grant.is_some() {
            return Err("its content carries a grant, which only a service device's may".into());
        }
        if !recv::of_account(&content.from, body, cert.body().account, Some(server)) {
            return Err("its from is no address of its sender's account in the space".into());
        }
        return Ok(());
    }
    let entry = recv::service_entry(roster, sender)
        .ok_or("the space's last accepted revision lists its service sender as no agent")?;
    let listed = Address::new(
        body.name.clone(),
        entry.owner.clone(),
        entry.agent.clone(),
        server.clone(),
    );
    if content.from != listed {
        return Err("its from is not its service sender's agent".into());
    }
    let grant = (content.grant.as_ref()).ok_or("its content from a service device has no grant")?;
    check_grant(
        grant,
        (sender, channel),
        roster,
        &devices.certs,
        &devices.revocations,
        (follower.own.account, &follower.own.master),
        state::unix_now(),
    )
    .map_err(|err| err.to_string())
}

/// Whether `to` names an agent of `follower`'s device in the space of `roster` on `server`: the
/// service agent of a `service` device, otherwise an agent of its account it has run as there.
fn of_this_device(
    follower: &Follower,
    roster: &Roster,
    server: &ServerName,
    to: &Address,
) -> Result<bool, String> {
    let body = roster.body();
    if follower.service.is_some() {
        return Ok(
            recv::service_entry(roster, follower.this).is_some_and(|entry| {
                *to == Address::new(
                    body.name.clone(),
                    entry.owner.clone(),
                    entry.agent.clone(),
                    server.clone(),
                )
            }),
        );
    }
    let handle = body
        .member(follower.own.account)
        .map(|member| &member.handle);
    if to.is_person()
        || to.space() != &body.name
        || Some(to.owner()) != handle
        || to.server() != server
    {
        return Ok(false);
    }
    let agents = (follower.state.agent_statuses(body.space)).map_err(text)?;
    Ok(agents.iter().any(|agent| agent.name == to.agent().as_str()))
}

/// The text of `request`, an A2A message in JSON, its text parts each on a line, once its
/// `messageId` is `task` in text form.
fn request_text(request: &str, task: TaskId) -> Option<String> {
    let message: serde_json::Value = serde_json::from_str(request).ok()?;
    (message["messageId"].as_str() == Some(task.to_string().as_str())).then(|| {
        (message["parts"].as_array().into_iter().flatten())
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// `skald extend`: extends the claim of the task `given` names, by `#<ref>` or its ID, with
/// `progress`, as `agent` if given, whose run holds its warnings for the person.
pub fn extend(
    run: &Run,
    given: &str,
    progress: Progress,
    agent: Option<&Name>,
) -> Result<(), String> {
    recv::holding_warnings(run.dir, agent.is_some(), || {
        let (mut held, synced, _) = located(run, given, agent)?;
        let mut follower = synced.follower;
        let reference = reference(&follower.state, held.row.task);
        if held.record.end.is_some() {
            return Err(format!(
                "the task #{reference} waits for the server to take its complete or fail; skald \
                 recv repeats it, and nothing extends its claim"
            ));
        }
        lapsed(&follower.state, &held, &reference)?;
        let sent = state::unix_now();
        let body = json!({ "progress": progress.name() });
        match fence(&mut follower, &held, "extend", body) {
            Fenced::Taken => {
                held.row.since = sent;
                keep(&follower, &mut held, None)?;
                match agent {
                    Some(_) => println!("extended #{reference} ({})", progress.name()),
                    None => println!(
                        "The claim of the task {} (#{reference}) holds for {} s more, {}.",
                        held.row.task,
                        held.record.lease,
                        progress.name()
                    ),
                }
                Ok(())
            }
            Fenced::Lost => {
                drop_claim(&follower.state, held.row.task);
                Err(lost(&reference, "the server took no extend of it"))
            }
            Fenced::Refused(why) => {
                drop_claim(&follower.state, held.row.task);
                Err(format!(
                    "the server refused the extend of the task #{reference}: {why}; the claim \
                     ends, and no result is sent"
                ))
            }
            Fenced::Unanswered(why) => Err(format!(
                "the server has not answered the extend of the task #{reference} ({why}); its \
                 lease runs from its last answered extend"
            )),
        }
    })
}

/// `skald complete` and `skald fail`: ends the claim of the task `given` names, by `#<ref>` or
/// its ID, as `ending` says, with `text`, as `agent` if given, whose run holds its warnings for
/// the person; then sends its result.
pub fn end(
    run: &Run,
    (given, text): (&str, &str),
    ending: Ending,
    agent: Option<&Name>,
) -> Result<(), String> {
    recv::holding_warnings(run.dir, agent.is_some(), || {
        let (mut held, synced, scope) = located(run, given, agent)?;
        let Synced {
            mut follower,
            space,
            roster,
            devices,
        } = synced;
        let task = held.row.task;
        let reference = reference(&follower.state, task);
        let earlier = held.record.end.is_some();
        if !earlier {
            lapsed(&follower.state, &held, &reference)?;
            held.record.end = Some(End {
                done: ending == Ending::Complete,
                retry: ending == Ending::Fail(true),
                text: text.to_owned(),
                taken: false,
            });
            keep(&follower, &mut held, None)?;
        }
        let server = send::server_of(&mut follower)?;
        let (roster, devices) =
            send::through_mailbox(&mut follower, scope, space.id, (roster, devices))?;
        let finish = finish(&mut follower, (&roster, &devices), &server, &mut held)?;
        let delivered = match &finish {
            Finish::Sent(msg_id) => Some(deliver(&mut follower, scope, *msg_id, held.row.channel)),
            _ => None,
        };
        let end = held.record.end.as_ref().expect("an end");
        let what = match (end.done, end.retry) {
            (true, _) => "complete",
            (false, true) => "fail with retry",
            (false, false) => "fail",
        };
        if earlier {
            eprintln!(
                "skald: warning: the task #{reference} had a {what} the server had not answered; \
                 it is repeated in place of this one"
            );
        }
        let result = match (finish, delivered) {
            (Finish::Sent(_), Some(Ok(()))) => "its result is sent".to_owned(),
            (Finish::Sent(_), Some(Err(why))) => why,
            (Finish::Waits(why), _) => {
                format!("its result is not sent yet ({why}); the next skald recv sends it")
            }
            (Finish::Lost, _) => {
                return Err(lost(
                    &reference,
                    &format!("the server took no {what} of it"),
                ));
            }
            (Finish::Refused(why), _) => {
                return Err(format!(
                    "the server refused the {what} of the task #{reference}: {why}; the claim \
                     ends, and no result is sent"
                ));
            }
            (Finish::Unanswered(why), _) => {
                return Err(format!(
                    "the server has not answered the {what} of the task #{reference} ({why}); the \
                     local state keeps it, and the next skald recv repeats it"
                ));
            }
            (Finish::Sent(_), None) => unreachable!("a sent result is delivered"),
        };
        let done = match (end.done, end.retry) {
            (true, _) => "completed",
            (false, true) => "failed with retry",
            (false, false) => "failed",
        };
        match agent {
            Some(_) => println!("{done} #{reference}; {result}"),
            None => println!("The task {task} (#{reference}) is {done}; {result}."),
        }
        Ok(())
    })
}

/// Repeats the completes and fails that wait within `scope` and sends the results that wait, of
/// the spaces of `synced`, the last accepted revisions and devices a run holds; warns of each
/// claim that does not end so.
pub fn finish_waiting(follower: &mut Follower, scope: Scope, synced: &Rosters) {
    if follower.stateless() {
        return;
    }
    let rows = match follower.state.claims(None) {
        Ok(rows) => rows,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    let rows: Vec<TaskClaim> = (rows.into_iter())
        .filter(|row| !scope.excludes(row.space) && synced.contains_key(&row.space))
        .collect();
    if rows.is_empty() {
        return;
    }
    let waiting: Vec<Held> = match opened_all(follower, rows) {
        Ok(held) => held
            .into_iter()
            .filter(|held| held.record.end.is_some())
            .collect(),
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    if waiting.is_empty() {
        return;
    }
    let server = match send::server_of(follower) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("skald: warning: no complete or fail that waits is repeated: {err}");
            return;
        }
    };
    let mut sent = false;
    for mut held in waiting {
        let reference = reference(&follower.state, held.row.task);
        let (roster, devices) = &synced[&held.row.space];
        let act = held.record.end.as_ref().map_or("", End::act);
        match finish(follower, (roster, devices), &server, &mut held) {
            Ok(Finish::Sent(_)) => sent = true,
            Ok(Finish::Waits(why)) => eprintln!(
                "skald: warning: the result of the task #{reference} is not sent yet: {why}"
            ),
            Ok(Finish::Lost) => eprintln!(
                "skald: warning: {}",
                lost(&reference, &format!("the server took no {act} of it"))
            ),
            Ok(Finish::Refused(why)) => eprintln!(
                "skald: warning: the server refused the {act} of the task #{reference}: {why}; \
                 the claim ends, and no result is sent"
            ),
            Ok(Finish::Unanswered(why)) => eprintln!(
                "skald: warning: the server has not answered the {act} of the task #{reference} \
                 ({why}); a later skald recv repeats it"
            ),
            Err(err) => eprintln!("skald: warning: {err}"),
        }
    }
    if sent {
        let queued = match send::queued(&follower.state, scope) {
            Ok(queued) => queued,
            Err(err) => {
                eprintln!("skald: warning: {err}");
                return;
            }
        };
        let timeout = follower.timeout;
        let mut delivered = send::deliver(follower, &queued, timeout, None);
        upload_entries(follower, &mut delivered, scope);
    }
}

/// Repeats the complete or fail of `held` unless the server took it, then puts its result in the
/// outbox, which ends the claim, by `roster`, the space's last accepted revision with `devices`,
/// on `server`; fails only if the device does.
fn finish(
    follower: &mut Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    server: &ServerName,
    held: &mut Held,
) -> Result<Finish, String> {
    let end = held.record.end.clone().expect("a claim that ends");
    if !end.taken {
        let body = match end.done {
            true => json!({}),
            false => json!({ "retry": end.retry }),
        };
        match fence(follower, held, end.act(), body) {
            Fenced::Taken => {
                if let Some(end) = held.record.end.as_mut() {
                    end.taken = true;
                }
                keep(follower, held, None)?;
            }
            Fenced::Lost => {
                drop_claim(&follower.state, held.row.task);
                return Ok(Finish::Lost);
            }
            Fenced::Refused(why) => {
                drop_claim(&follower.state, held.row.task);
                return Ok(Finish::Refused(why));
            }
            Fenced::Unanswered(why) => return Ok(Finish::Unanswered(why)),
        }
    }
    let agent = (held.row.agent.as_deref())
        .map(|agent| agent.parse::<Name>())
        .transpose()
        .map_err(|_| "the local state is malformed: a claim's agent".to_owned())?;
    let (from, grant) = match follower.service {
        Some(_) => match service::sender(
            follower,
            (roster, devices),
            (held.row.channel, server.clone()),
        ) {
            Ok((from, grant)) => (from, Some(grant)),
            Err(why) => return Ok(Finish::Waits(why)),
        },
        None => match send::sender(follower, roster, server.clone(), agent.as_ref()) {
            Ok(from) => (from, None),
            Err(why) => return Ok(Finish::Waits(why)),
        },
    };
    let malformed = |what: &str| format!("the local state is malformed: a claim's {what}");
    let record = &held.record;
    let to: Address = record.from.parse().map_err(|_| malformed("from"))?;
    let sender: DeviceId = record.sender.parse().map_err(|_| malformed("sender"))?;
    let claim_id: ClaimId = record.claim_id.parse().map_err(|_| malformed("claim ID"))?;
    let cert = recv::member_cert(&follower.own, (roster, devices), sender);
    let result = TaskResult {
        task: held.row.task,
        claim_id,
        generation: record.generation,
        outcome: if end.done {
            Outcome::Done
        } else {
            Outcome::Failed
        },
        response: String::new(),
        unknown: Unknown::default(),
    };
    let hops = record.hops.saturating_add(1);
    let sent = groups::encrypt_result(
        follower,
        (roster, devices),
        (held.row.channel, held.row.agent.as_deref(), held.row.task),
        (sender, cert),
        |msg_id, counter| {
            let response = json!({
                "messageId": msg_id.to_string(),
                "role": "ROLE_AGENT",
                "parts": [{ "text": end.text }],
            })
            .to_string();
            let result = TaskResult { response, ..result };
            let mut content = result.content((from, to), counter, hops);
            content.grant = grant;
            content
        },
    )?;
    Ok(match sent {
        Ok(msg_id) => Finish::Sent(msg_id),
        Err(why) => Finish::Waits(why),
    })
}

/// Sends the outbox within `scope` and uploads the history entries of what the server stored; the
/// server's answer to the result `msg_id` in `channel`: why it is not stored, if it is not.
fn deliver(
    follower: &mut Follower,
    scope: Scope,
    msg_id: MessageId,
    channel: ChannelId,
) -> Result<(), String> {
    let queued = send::queued(&follower.state, scope)?;
    let timeout = follower.timeout;
    let mut delivered = send::deliver(follower, &queued, timeout, None);
    upload_entries(follower, &mut delivered, scope);
    let answer = (queued.iter())
        .map(send::Wire::key)
        .filter(|(id, _)| *id == msg_id)
        .find_map(|key| delivered.answer(&key));
    match answer {
        Some(send::Answer::Stored) => Ok(()),
        Some(send::Answer::Refused(problem)) => Err(format!(
            "the server refused its result in the channel {channel}: {}",
            printable(problem)
        )),
        None => Err(
            "its result waits in the outbox, and the next skald send or skald recv sends it"
                .to_owned(),
        ),
    }
}

/// Uploads the history entries within `scope` of what the server stored, on the connection
/// `delivered` left open, if any.
fn upload_entries(follower: &mut Follower, delivered: &mut send::Delivered, scope: Scope) {
    match Link::new(follower) {
        Ok(mut link) => {
            link.mailbox = delivered.mailbox.take();
            history::upload(follower, &mut link, scope);
        }
        Err(err) => eprintln!("skald: warning: {err}"),
    }
}

/// The claim the task `given` names, by `#<ref>` or its ID, among those the person, or else
/// `agent` in its folder's space, holds by the run's state, with the space of its channel synced
/// and the run's scope.
fn located(run: &Run, given: &str, agent: Option<&Name>) -> Result<(Held, Synced, Scope), String> {
    let shown = printable(given.trim());
    let named = reply::given(given).map_err(|_| {
        format!("\"{shown}\" names no task: give its #<ref>, as skald claim shows it, or its ID")
    })?;
    let scope = Scope::of_run(run.dir, agent.is_some())?;
    let unknown = || match agent {
        Some(_) => format!("{shown} is no task this agent holds a claim of in its folder's space"),
        None => format!("{shown} is no task this device holds a claim of"),
    };
    if !run.dir.join(state::FILE).is_file() {
        return Err(unknown());
    }
    let row = {
        let state = State::open(run.dir).map_err(text)?;
        let rows: Vec<TaskClaim> = (state.claims(None).map_err(text)?.into_iter())
            .filter(|row| !scope.excludes(row.space))
            .filter(|row| agent.is_none_or(|agent| row.agent.as_deref() == Some(agent.as_str())))
            .collect();
        let kept = references_of(&state);
        reply::pick(&named, rows, |row| as_message(row.task), &kept).map_err(|shared| {
            if shared {
                format!("{}; nothing is sent", reply::shared(&shown))
            } else {
                unknown()
            }
        })?
    };
    let (speaks, timeout) = run.settings;
    let synced = spaces::of_channel(run.dir, run.storage, speaks, timeout, row.channel, scope)?;
    let mut held = opened_all(&synced.follower, vec![row])?;
    Ok((held.remove(0), synced, scope))
}

/// Each of `rows` with what the device sealed of it opened.
fn opened_all(follower: &Follower, rows: Vec<TaskClaim>) -> Result<Vec<Held>, String> {
    let key = key_packages::local_state_key(&follower.state, &follower.secrets)?;
    rows.into_iter()
        .map(|row| {
            let record = open(&key, follower.this, &row)?;
            Ok(Held { row, record })
        })
        .collect()
}

/// What the device sealed of `row` with `key` for the device `this`.
fn open(key: &StateKey, this: DeviceId, row: &TaskClaim) -> Result<Record, String> {
    key.open(this, &item(row.task), &row.sealed)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| {
            format!(
                "what this device kept of its claim of the task {} does not open",
                row.task
            )
        })
}

/// Keeps `held`, sealing its record, with the task's `hops` for the agent `hops` names, if any
/// ([`State::keep_claim`]).
fn keep(follower: &Follower, held: &mut Held, hops: Option<(&str, u64)>) -> Result<(), String> {
    let key = key_packages::local_state_key(&follower.state, &follower.secrets)?;
    let record = serde_json::to_vec(&held.record).expect("a claim encodes");
    held.row.sealed = key.seal(follower.this, &item(held.row.task), &record);
    (follower.state).keep_claim(&held.row, hops).map_err(text)
}

/// Drops the claim of `task` from `state`, warning if it cannot.
fn drop_claim(state: &State, task: TaskId) {
    if let Err(err) = state.remove_claim(task) {
        eprintln!("skald: warning: the local state keeps the lost claim of the task {task}: {err}");
    }
}

/// Fails, dropping it, if the lease of `held` passed on the device's clock since the last answered
/// request that started it.
fn lapsed(state: &State, held: &Held, reference: &str) -> Result<(), String> {
    if state::unix_now() < held.row.since.saturating_add(held.record.lease) {
        return Ok(());
    }
    drop_claim(state, held.row.task);
    Err(lost(
        reference,
        &format!(
            "its lease of {} s passed on this device's clock without an answered extend",
            held.record.lease
        ),
    ))
}

/// What a command says of the claim of the task `reference` names, lost as `why` says.
fn lost(reference: &str, why: &str) -> String {
    format!(
        "the claim of the task #{reference} is lost: {why}; another device may hold the task, so \
         stop its work: no result is sent"
    )
}

/// The `act` request, such as `extend`, of the claim `held`, with the fields of `extra` besides
/// its claim.
fn fence(follower: &mut Follower, held: &Held, act: &str, extra: serde_json::Value) -> Fenced {
    let row = &held.row;
    let path = task_path((row.space, row.channel), row.queue, row.task, act);
    let mut body = json!({
        "claim_id": held.record.claim_id,
        "generation": held.record.generation,
    });
    if let (Some(body), serde_json::Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    fenced(queues::proved_json(follower, Method::Post, &path, &body))
}

/// The path of the `act` request of the task `task` of the queue `queue` in the channel `at`.
fn task_path(at: (SpaceId, ChannelId), queue: QueueId, task: TaskId, act: &str) -> String {
    format!("{}/{queue}/tasks/{task}/{act}", queues::queues_path(at))
}

/// The answer `sent`, the result of an extend, complete, fail or release, gives.
fn fenced(sent: Result<Vec<u8>, connection::Error>) -> Fenced {
    match sent {
        Ok(_) => Fenced::Taken,
        Err(connection::Error::Refused { kind, .. }) if queues::problem(&kind) == "claim-lost" => {
            Fenced::Lost
        }
        // A refused proof, a timeout, a rate limit and the server's failure change with time.
        Err(connection::Error::Refused {
            status,
            kind,
            detail,
            ..
        }) if (400..500).contains(&status) && ![401, 408, 429].contains(&status) => {
            Fenced::Refused(format!(
                "{} ({})",
                queues::problem(&kind),
                printable(&detail)
            ))
        }
        Err(err) => Fenced::Unanswered(err.to_string()),
    }
}

impl Fenced {
    /// Why it was not taken.
    fn why(&self) -> String {
        match self {
            Fenced::Taken => String::new(),
            Fenced::Lost => "the server answered claim-lost".to_owned(),
            Fenced::Refused(why) => format!("the server refused it: {why}"),
            Fenced::Unanswered(why) => format!("the server has not answered: {why}"),
        }
    }
}

/// The name the state key seals what the device keeps of its claim of `task` under.
fn item(task: TaskId) -> Vec<u8> {
    [b"claim:".as_slice(), task.as_bytes()].concat()
}

/// `task` as references count it: as a message ID of the same bytes.
fn as_message(task: TaskId) -> MessageId {
    MessageId::from_bytes(*task.as_bytes())
}

/// What references count by the device of `state`: its kept messages and the tasks it holds
/// claims of.
fn references_of(state: &State) -> Vec<MessageId> {
    let claims = state.claims(None).unwrap_or_default();
    (state.kept_ids().unwrap_or_default().into_iter())
        .chain(claims.into_iter().map(|row| as_message(row.task)))
        .collect()
}

/// The reference, `#<ref>` without its `#`, of the task `task` by the device of `state`.
fn reference(state: &State, task: TaskId) -> String {
    let task = as_message(task);
    References::new(references_of(state).into_iter().chain([task])).of(task)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A request's text is its text parts, each on a line, once its `messageId` is the task's ID.
    #[test]
    fn request_text_is_its_text_parts_once_its_message_id_is_the_task_s() {
        let task = TaskId::generate();
        let request = json!({
            "messageId": task.to_string(),
            "role": "ROLE_USER",
            "parts": [{ "text": "one" }, { "data": {} }, { "text": "two" }],
        })
        .to_string();
        assert_eq!(request_text(&request, task), Some("one\ntwo".to_owned()));
        assert_eq!(request_text(&request, TaskId::generate()), None);
        assert_eq!(request_text("not json", task), None);
    }

    /// `claim-lost` loses a claim; a lasting refusal is final; a refused proof, a timeout, a rate
    /// limit and the server's failure go unanswered.
    #[test]
    fn claim_lost_loses_a_claim_and_only_lasting_refusals_are_final() {
        let refused = |status, problem: &str| {
            fenced(Err(connection::Error::Refused {
                status,
                kind: format!("https://skald.so/problems/{problem}"),
                detail: "no".to_owned(),
                task: None,
            }))
        };
        assert_eq!(fenced(Ok(Vec::new())), Fenced::Taken);
        assert_eq!(refused(409, "claim-lost"), Fenced::Lost);
        assert!(matches!(refused(404, "unknown-task"), Fenced::Refused(_)));
        assert!(matches!(refused(403, "not-a-member"), Fenced::Refused(_)));
        for status in [401, 408, 429, 500, 503] {
            let answer = refused(status, "other");
            assert!(
                matches!(answer, Fenced::Unanswered(_)),
                "{status}: {answer:?}"
            );
        }
        let timeout = fenced(Err(connection::Error::Timeout(Duration::from_secs(1))));
        assert!(matches!(timeout, Fenced::Unanswered(_)), "{timeout:?}");
    }

    /// A claim is lost once its lease passed on the device's clock since its last answered
    /// request, and dropped from the local state.
    #[test]
    fn lease_that_passed_on_the_device_s_clock_loses_the_claim() {
        let dir = crate::test_dir::TestDir::new("claims-lapsed");
        let state = State::open(&dir).unwrap();
        let now = state::unix_now();
        let held = |since: u64| Held {
            row: TaskClaim {
                task: TaskId::generate(),
                space: SpaceId::generate(),
                channel: ChannelId::generate(),
                queue: QueueId::generate(),
                agent: None,
                since,
                sealed: vec![1],
            },
            record: Record {
                claim_id: ClaimId::generate().to_string(),
                generation: 1,
                lease: 30,
                sender: DeviceId::from_signing_key(&[1; 32]).to_string(),
                from: String::new(),
                hops: 0,
                end: None,
            },
        };
        let live = held(now - 29);
        state.keep_claim(&live.row, None).unwrap();
        assert_eq!(lapsed(&state, &live, "abcd"), Ok(()));
        assert_eq!(state.claims(None).unwrap(), std::slice::from_ref(&live.row));
        let gone = held(now - 30);
        state.keep_claim(&gone.row, None).unwrap();
        let err = lapsed(&state, &gone, "abcd").unwrap_err();
        assert!(err.contains("lost"), "{err}");
        assert_eq!(state.claims(None).unwrap(), [live.row]);
    }
}
