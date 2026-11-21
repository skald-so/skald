//! What the tests of claims share besides the kit of the tests of queues: claims and what a device
//! does with them, restarts and cancels, tasks as reads give them with their last claims, time
//! passing for a task, and devices joining the channel's group.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};
use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::{Cbor, Cert, Claim, ClaimId, DeviceId, DeviceKind, QueueId, QueuedTask, TaskId};
use sqlx::AssertSqlSafe;

use crate::api::Person;
use crate::common::device_flow::{json as json_of, text};
use crate::common::devices::{Device, unb64};
use crate::common::{Response, Server, TestDb, hex};
use crate::kit::{Body, Client, DEFAULT, World, assert_problem};
use crate::scene::removing;
use crate::wire::now;

/// The trust levels a claim states.
pub const OWN: &str = "own";
pub const PEER: &str = "peer";
pub const SERVICE: &str = "service";
/// Every trust level.
pub const ALL: [&str; 3] = [OWN, PEER, SERVICE];

/// The states of a task besides `queued`.
pub const CLAIMED: &str = "claimed";
pub const DONE: &str = "done";
pub const DEAD: &str = "dead";
pub const CANCELLED: &str = "cancelled";

/// The progress of a claim.
pub const STARTED: &str = "started";
pub const AWAITING_APPROVAL: &str = "awaiting-approval";

/// The reasons of a `dead` task.
pub const LEASE_EXPIRED: &str = "lease-expired";
pub const FAILED: &str = "failed";
pub const RELEASED: &str = "released";
pub const DEADLINE_PASSED: &str = "deadline-passed";

/// The problems of claims, restarts and cancels besides those of the kit.
pub const CLAIM_LOST: &str = "claim-lost";
pub const NOT_DEAD: &str = "not-dead";
pub const NOT_CANCELLABLE: &str = "not-cancellable";

/// How soon reads show a lease's end or a deadline.
pub const READS_WITHIN: Duration = Duration::from_secs(10);
/// How often a test reads again.
const LOOK: Duration = Duration::from_millis(200);

/// A claim stating `levels`, no template and that it takes tasks without one.
pub fn takes(levels: &[&str]) -> Json {
    takes_templates(levels, &[], true)
}

/// A claim stating `levels` and `templates`, and whether it takes tasks without a template.
pub fn takes_templates(levels: &[&str], templates: &[&str], without_template: bool) -> Json {
    json!({
        "trust_levels": levels,
        "templates": templates,
        "without_template": without_template,
    })
}

/// A task's last claim as reads give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Last {
    pub claim_id: ClaimId,
    pub generation: u64,
    pub device: DeviceId,
}

impl Last {
    /// The last claim `claim` makes, held by `device`.
    pub fn of(claim: &Claim, device: DeviceId) -> Self {
        Last {
            claim_id: claim.claim_id,
            generation: claim.generation,
            device,
        }
    }
}

/// A task as reads give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    /// The `queued-task`'s bytes.
    pub bytes: Vec<u8>,
    pub state: String,
    pub progress: Option<String>,
    pub reason: Option<String>,
    pub attempts: u64,
    pub claim: Option<Last>,
}

impl Read {
    #[track_caller]
    pub fn of(value: &Json) -> Self {
        let string = |name: &str| value[name].as_str().map(str::to_owned);
        let claim = (!value["claim"].is_null()).then(|| {
            let claim = &value["claim"];
            let id = |name: &str| {
                (claim[name].as_str()).unwrap_or_else(|| panic!("no claim {name} in {value}"))
            };
            Last {
                claim_id: id("claim_id").parse().expect("a claim ID"),
                generation: (claim["generation"].as_u64())
                    .unwrap_or_else(|| panic!("no claim generation in {value}")),
                device: id("device").parse().expect("a device ID"),
            }
        });
        Read {
            bytes: unb64(
                &string("queued_task").unwrap_or_else(|| panic!("no queued_task in {value}")),
            ),
            state: string("state").unwrap_or_else(|| panic!("no state in {value}")),
            progress: string("progress"),
            reason: string("reason"),
            attempts: (value["attempts"].as_u64())
                .unwrap_or_else(|| panic!("no attempts in {value}")),
            claim,
        }
    }

    /// Its state, attempts, progress and reason.
    pub fn shows(&self) -> (&str, u64, Option<&str>, Option<&str>) {
        (
            self.state.as_str(),
            self.attempts,
            self.progress.as_deref(),
            self.reason.as_deref(),
        )
    }

    /// The task ID of its bytes.
    #[track_caller]
    pub fn task(&self) -> TaskId {
        QueuedTask::from_cbor(&self.bytes)
            .expect("the bytes of a task")
            .task
    }
}

/// What a device does with claims and tasks, through the requests of the kit's [`Client`].
pub trait Holder {
    /// `POST` of a claim of `queue` stating `takes`, proved if `proved`.
    fn claim_with(&self, queue: QueueId, takes: &Json, proved: bool) -> Response;
    /// `POST` of a claim of `queue` stating `takes`, proved.
    fn claim(&self, queue: QueueId, takes: &Json) -> Response;
    /// Claims from `queue` stating `takes`; the claim it gets, or none.
    fn claim_of(&self, queue: QueueId, takes: &Json) -> Option<Claim>;
    /// Claims from `queue` stating `takes`, which must get a task; gives the claim.
    fn claims(&self, queue: QueueId, takes: &Json) -> Claim;
    /// Claims from `queue` stating `takes`, which must get no task.
    fn gets_none(&self, queue: QueueId, takes: &Json);
    /// `POST` of `action` on `claim`'s task naming `claim_id` and `generation`, with `extra`
    /// members, proved if `proved`.
    fn act_with(&self, claim: &Claim, action: &str, extra: &Json, proved: bool) -> Response;
    /// An extend of `claim` stating `progress`.
    fn extend(&self, claim: &Claim, progress: &str) -> Response;
    /// A complete of `claim`.
    fn complete(&self, claim: &Claim) -> Response;
    /// A fail of `claim`, with retry if `retry`.
    fn fail(&self, claim: &Claim, retry: bool) -> Response;
    /// A release of `claim`.
    fn release(&self, claim: &Claim) -> Response;
    /// `POST` of `action`, `restart` or `cancel`, on `task` of `queue`, proved if `proved`.
    fn on_task(&self, queue: QueueId, task: TaskId, action: &str, proved: bool) -> Response;
    /// A restart of `task` of `queue`.
    fn restart(&self, queue: QueueId, task: TaskId) -> Response;
    /// A cancel of `task` of `queue`.
    fn cancel(&self, queue: QueueId, task: TaskId) -> Response;
    /// The task `task` of `queue` as the device reads it.
    fn state_of(&self, queue: QueueId, task: TaskId) -> Read;
    /// The tasks of `queue` as the device reads them, in any order.
    fn states(&self, queue: QueueId) -> Vec<Read>;
    /// Reads `task` of `queue` until it is `state`, within [`READS_WITHIN`]; gives the read.
    fn awaits(&self, queue: QueueId, task: TaskId, state: &str) -> Read;
}

impl Holder for Client {
    fn claim_with(&self, queue: QueueId, takes: &Json, proved: bool) -> Response {
        let path = format!("{}/claims", self.queue_path(queue));
        self.call("post", &path, Body::Json(takes), proved)
    }

    fn claim(&self, queue: QueueId, takes: &Json) -> Response {
        self.claim_with(queue, takes, true)
    }

    #[track_caller]
    fn claim_of(&self, queue: QueueId, takes: &Json) -> Option<Claim> {
        let response = self.claim(queue, takes);
        match response.status {
            204 => {
                assert!(response.body.is_empty(), "{}: a body with 204", self.who);
                None
            }
            200 => {
                assert!(
                    response.content_type.starts_with("application/cbor"),
                    "{}: a claim of {:?}",
                    self.who,
                    response.content_type
                );
                Some(Claim::from_cbor(&response.body).expect("a claim"))
            }
            _ => panic!("{}: a claim or none, not {}", self.who, text(&response)),
        }
    }

    #[track_caller]
    fn claims(&self, queue: QueueId, takes: &Json) -> Claim {
        (self.claim_of(queue, takes)).unwrap_or_else(|| panic!("{}: no task claimed", self.who))
    }

    #[track_caller]
    fn gets_none(&self, queue: QueueId, takes: &Json) {
        if let Some(claim) = self.claim_of(queue, takes) {
            panic!(
                "{}: no task, not {} of generation {}",
                self.who, claim.task.task, claim.generation
            );
        }
    }

    fn act_with(&self, claim: &Claim, action: &str, extra: &Json, proved: bool) -> Response {
        let mut body = json!({
            "claim_id": claim.claim_id.to_string(),
            "generation": claim.generation,
        });
        if let (Some(all), Some(more)) = (body.as_object_mut(), extra.as_object()) {
            all.extend(more.clone());
        }
        let path = format!(
            "{}/{}/{action}",
            self.tasks_path(claim.task.queue),
            claim.task.task
        );
        self.call("post", &path, Body::Json(&body), proved)
    }

    fn extend(&self, claim: &Claim, progress: &str) -> Response {
        self.act_with(claim, "extend", &json!({ "progress": progress }), true)
    }

    fn complete(&self, claim: &Claim) -> Response {
        self.act_with(claim, "complete", &json!({}), true)
    }

    fn fail(&self, claim: &Claim, retry: bool) -> Response {
        self.act_with(claim, "fail", &json!({ "retry": retry }), true)
    }

    fn release(&self, claim: &Claim) -> Response {
        self.act_with(claim, "release", &json!({}), true)
    }

    fn on_task(&self, queue: QueueId, task: TaskId, action: &str, proved: bool) -> Response {
        let path = format!("{}/{task}/{action}", self.tasks_path(queue));
        self.call("post", &path, Body::None, proved)
    }

    fn restart(&self, queue: QueueId, task: TaskId) -> Response {
        self.on_task(queue, task, "restart", true)
    }

    fn cancel(&self, queue: QueueId, task: TaskId) -> Response {
        self.on_task(queue, task, "cancel", true)
    }

    #[track_caller]
    fn state_of(&self, queue: QueueId, task: TaskId) -> Read {
        Read::of(&answered(&self.read(queue, task, true), &self.who))
    }

    #[track_caller]
    fn states(&self, queue: QueueId) -> Vec<Read> {
        let listed = answered(&self.list_tasks(queue, true), &self.who)["tasks"].clone();
        let Json::Array(tasks) = listed else {
            panic!("{}: no list of tasks: {listed}", self.who);
        };
        tasks.iter().map(Read::of).collect()
    }

    #[track_caller]
    fn awaits(&self, queue: QueueId, task: TaskId, state: &str) -> Read {
        let deadline = Instant::now() + READS_WITHIN;
        loop {
            let read = self.state_of(queue, task);
            if read.state == state {
                return read;
            }
            assert!(
                Instant::now() < deadline,
                "{}: {task} still {:?} after {READS_WITHIN:?}, not {state}",
                self.who,
                read.shows()
            );
            thread::sleep(LOOK);
        }
    }
}

/// Checks that `response` takes the request: a `2xx`.
#[track_caller]
pub fn assert_taken(response: &Response, what: &str) {
    assert!(
        (200..300).contains(&response.status),
        "{what}: taken, not {}",
        text(response)
    );
}

/// Checks that `response` refuses with `claim-lost`.
#[track_caller]
pub fn assert_claim_lost(response: &Response, what: &str) {
    assert_problem(response, CLAIM_LOST, what);
}

/// Checks that `again` is the answer `first` was: its status, media type and body.
#[track_caller]
pub fn assert_same_answer(again: &Response, first: &Response, what: &str) {
    let found = (again.status, &again.content_type, &again.body);
    let expected = (first.status, &first.content_type, &first.body);
    assert_eq!(found, expected, "{what}: the first answer");
}

/// Checks the claim's `generation`, `attempt` and `uploader`, and that it carries `task`'s bytes.
#[track_caller]
pub fn assert_claim(claim: &Claim, task: &QueuedTask, generation: u64, attempt: u64, what: &str) {
    assert_eq!(
        claim.task.to_cbor(),
        task.to_cbor(),
        "{what}: the task claimed"
    );
    assert_eq!(
        (claim.generation, claim.attempt),
        (generation, attempt),
        "{what}: the generation and attempt"
    );
}

/// `claim` with another `claim_id`, as a device that holds no such claim names it.
pub fn other_claim_id(claim: &Claim) -> Claim {
    let mut other = claim.clone();
    other.claim_id = ClaimId::generate();
    other
}

/// `claim` with the next generation.
pub fn next_generation(claim: &Claim) -> Claim {
    let mut other = claim.clone();
    other.generation += 1;
    other
}

/// One statement that moves back by `seconds` the `timestamptz` columns of every row whose text
/// holds `needle` or `bytes`, as if that much time had passed for them; it fails when no such row
/// has one.
fn moved_back(needle: &str, bytes: &str, seconds: u64) -> String {
    format!(
        "DO $$
         DECLARE
           rel regclass;
           sets text;
           hit bigint;
           total bigint := 0;
         BEGIN
           FOR rel, sets IN
             SELECT a.attrelid::regclass,
                    string_agg(format('%1$I = %1$I - make_interval(secs => {seconds})', a.attname),
                               ', ')
             FROM pg_attribute a JOIN pg_class k ON k.oid = a.attrelid
             WHERE k.relkind = 'r' AND k.relnamespace = current_schema()::regnamespace
               AND a.atttypid = 'timestamptz'::regtype AND a.attnum > 0 AND NOT a.attisdropped
             GROUP BY a.attrelid
           LOOP
             EXECUTE format('UPDATE %s AS r SET %s
                             WHERE position(%L IN r::text) > 0 OR position(%L IN r::text) > 0',
                            rel, sets, '{needle}', '{bytes}');
             GET DIAGNOSTICS hit = ROW_COUNT;
             total := total + hit;
           END LOOP;
           IF total = 0 THEN
             RAISE EXCEPTION 'no stored time holds %', '{needle}';
           END IF;
         END $$;"
    )
}

/// `seconds` pass for the task `task`: its stored times move back.
#[track_caller]
pub fn passes(db: &TestDb, task: TaskId, seconds: u64) {
    db.execute(&moved_back(
        &task.to_string(),
        &hex(task.as_bytes()),
        seconds,
    ));
}

/// `seconds` pass for every task whose ID's text begins with `prefix`, 8 hex digits.
#[track_caller]
pub fn passes_for(db: &TestDb, prefix: &str, seconds: u64) {
    db.execute(&moved_back(&format!("{prefix}-"), prefix, seconds));
}

/// Waits until the clock of seconds since the Unix epoch is past `timestamp`.
pub fn wait_past(timestamp: u64) {
    while now() <= timestamp {
        thread::sleep(LOOK);
    }
}

/// The certificate the server holds for `device`.
#[track_caller]
pub fn cert_held(db: &TestDb, device: DeviceId) -> Vec<u8> {
    let select = format!("SELECT cert FROM devices WHERE id = '{device}'");
    (db.query(async |conn| {
        sqlx::query_scalar(AssertSqlSafe(select))
            .fetch_one(conn)
            .await
    }))
    .unwrap_or_else(|err| panic!("the certificate of {device}: {err}"))
}

/// A new certified device of `kind` on `person`'s account, issued by `person`'s laptop.
pub fn device_of(w: &World, person: &Person, kind: DeviceKind, name: &str) -> Device {
    person.certified_by(w.db(), &person.laptop, kind, name)
}

/// The requests of `device`, of `person`'s account, on `person`'s session to `server`.
pub fn client_of(w: &World, server: &Server, person: &Person, device: &Device) -> Client {
    let who = format!("{}'s {}", person.handle, device.name);
    w.client(server, person.session.token(), device, who)
}

/// alice's laptop adds `devices` to `general`'s group by a Commit the server stores; bob's and
/// carol's laptops process it.
#[track_caller]
pub fn join(w: &mut World, devices: &[&Device]) {
    let keys: Vec<_> = (devices.iter())
        .map(|device| (device.id, device.signing.secret))
        .collect();
    join_keys(w, &keys);
}

/// As [`join`], for the devices of these IDs and signing keys' secrets.
#[track_caller]
fn join_keys(w: &mut World, devices: &[(DeviceId, [u8; 32])]) {
    let mut packages = Vec::new();
    let mut admitted = Vec::new();
    for (id, secret) in devices {
        let signing = SigningKey::from_bytes(secret);
        let mut mls = MlsDevice::new(&signing, &StateKey::generate());
        let package = mls.key_package(now()).expect("a KeyPackage");
        packages.push(package.as_ref().to_vec());
        let cert = cert_held(w.db(), *id);
        admitted.push(Cert::from_cbor(&cert).expect("a certificate"));
    }
    let f = &w.stage.f;
    let commit = w.alice.commits(f, |device, channel| {
        device.add_devices(channel, &packages, &admitted, now())
    });
    w.bob.process(f, &commit);
    w.carol.process(f, &commit);
}

/// Adds the bot to `general`'s group, and to the channel's `services` in a new revision.
#[track_caller]
pub fn admit_bot(w: &mut World) {
    let bot = (w.stage.bot.id, w.stage.bot.signing.secret);
    join_keys(w, &[bot]);
    let channel = w.channel;
    w.stage.f.revise(|body| {
        for listed in &mut body.channels {
            if listed.id == channel {
                listed.services.push(bot.0);
            }
        }
    });
}

/// alice's laptop removes bob's laptop from `general`'s group by a Commit the server stores;
/// carol's laptop processes it. bob stays in the channel in the roster.
#[track_caller]
pub fn remove_bob(w: &mut World) {
    let f = &w.stage.f;
    let commit = w.alice.commits(f, removing(&[w.bob.id]));
    w.carol.process(f, &commit);
}

/// `client` enqueues `N` new tasks to `queue`, in that order; gives them.
#[track_caller]
pub fn enqueued<const N: usize>(w: &World, client: &Client, queue: QueueId) -> [QueuedTask; N] {
    std::array::from_fn(|_| {
        let task = w.task(queue);
        client.stores(&task);
        task
    })
}

/// The world, once `prepare` has changed it, with a task alice's laptop enqueued to `default`: the
/// laptops, as [`World::laptops`] gives them, `default`'s ID and the task.
#[track_caller]
pub fn scene_after(
    github: u64,
    prepare: impl FnOnce(&mut World),
) -> (World, [Client; 5], QueueId, QueuedTask) {
    let mut w = World::new(github);
    prepare(&mut w);
    let laptops = w.laptops();
    let default = laptops[0].queue(DEFAULT).id;
    let [task] = enqueued(&w, &laptops[0], default);
    (w, laptops, default, task)
}

/// As [`scene_after`], with the world as it is.
#[track_caller]
pub fn scene(github: u64) -> (World, [Client; 5], QueueId, QueuedTask) {
    scene_after(github, |_| {})
}

/// As [`scene`], once carol's laptop has claimed the task: also its claim.
#[track_caller]
pub fn held(github: u64) -> (World, [Client; 5], QueueId, QueuedTask, Claim) {
    let (w, laptops, default, task) = scene(github);
    let claim = laptops[2].claims(default, &takes(&ALL));
    (w, laptops, default, task, claim)
}

/// The JSON body of `response`, a `200` answer to `who`.
#[track_caller]
fn answered(response: &Response, who: &str) -> Json {
    assert_eq!(response.status, 200, "{who}: {}", text(response));
    json_of(response)
}
