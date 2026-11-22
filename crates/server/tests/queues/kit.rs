//! What the tests share: the world of the tests, a device's proved requests for a channel's queues
//! and tasks, tasks as the server sees them, the queues and tasks it gives, and a database as a
//! server before queues left it.

use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};
use skald_proto::{
    Cbor, Cert, ChannelId, DeviceKind, PROTOCOL_VERSION, QueueId, QueuedTask, Sched, SpaceId,
    TaskId, Unknown, VERSION_HEADER,
};
use sqlx::{AssertSqlSafe, Connection};

use crate::api::{Person, template, the_spec};
use crate::bot::token;
use crate::common::agent::agent;
use crate::common::device_flow::{json as json_of, text};
use crate::common::devices::{Device, random, unb64};
use crate::common::spec::operation_violations;
use crate::common::{Response, Server, TestDb};
use crate::fixture::Fixture;
use crate::https::{CBOR, Https, PROOF};
use crate::presence::{assert_refused_with, held, random_bytes};
use crate::rosters::service_agent;
use crate::scene::{Peer, empty};
use crate::tokens::{DAY, Stage, grant};
use crate::wire::now;

/// The queue every channel has.
pub const DEFAULT: &str = "default";
/// The modes of a queue.
pub const AT_LEAST_ONCE: &str = "at-least-once";
pub const AT_MOST_ONCE: &str = "at-most-once";
/// A queue's default lease, in seconds, and its default `max_attempts`.
pub const LEASE: u64 = 300;
pub const MAX_ATTEMPTS: u64 = 5;

/// The problems of queue requests.
pub const NOT_A_MEMBER: &str = "not-a-member";
pub const NOT_ALLOWED: &str = "not-allowed";
pub const UNKNOWN_QUEUE: &str = "unknown-queue";
pub const UNKNOWN_TASK: &str = "unknown-task";
pub const DUPLICATE: &str = "duplicate";
pub const EPOCH_MISMATCH: &str = "epoch-mismatch";
pub const TOO_LARGE: &str = "too-large";

/// The server setting of the message text limit, in bytes.
pub const SIZE_LIMIT: &str = "--message-size-limit";

/// The state of a task waiting for a claim.
pub const QUEUED: &str = "queued";

/// The length of the `ct` of the tests' tasks.
const CT: usize = 300;
/// How long a test waits for the server to delete what it deletes.
const GONE_WITHIN: Duration = Duration::from_secs(10);
/// The latest migration of the server before queues.
const BEFORE_QUEUES: i64 = 20270112000000;

/// The space of the tests of storing sent envelopes, with `general`'s group formed by alice's
/// laptop with bob's and carol's, and alice's `service` device, the bot, listed as a service agent
/// but in no group.
pub struct World {
    pub stage: Stage,
    pub space: SpaceId,
    pub channel: ChannelId,
    pub alice: Peer,
    pub bob: Peer,
    pub carol: Peer,
}

impl World {
    /// The world; GitHub user IDs from `github` on.
    pub fn new(github: u64) -> Self {
        Self::with(github, &[])
    }

    /// As [`World::new`], on a server with the extra arguments `args`.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let mut f = Fixture::with(github, args);
        let owner = &f.team.alice;
        let bot = owner.certified_by(f.db(), &owner.laptop, DeviceKind::Service, "Queue bot");
        let entry = service_agent(&bot);
        f.revise(|body| body.service_agents.push(entry));
        let (space, channel) = (f.team.space, f.general);
        let t = &f.team;
        let [mut alice, mut bob, mut carol] = [&t.alice, &t.bob, &t.carol].map(|p| Peer::of(&f, p));
        let packages = [bob.key_package(), carol.key_package()];
        let certs = [&t.alice.cert, &t.bob.cert, &t.carol.cert];
        let admitted = certs.map(|bytes| Cert::from_cbor(bytes).expect("a certificate"));
        let commit = alice.creates(&f, &packages, &admitted);
        alice.stores(&f, &commit, &[]);
        let welcome = (alice.merges_and_uploads(&f).0).expect("the Welcome");
        alice.stores(&f, &welcome, &[bob.id, carol.id]);
        bob.joins(&f, &welcome);
        carol.joins(&f, &welcome);
        World {
            stage: Stage { f, bot },
            space,
            channel,
            alice,
            bob,
            carol,
        }
    }

    pub fn fixture(&self) -> &Fixture {
        &self.stage.f
    }

    pub fn db(&self) -> &TestDb {
        self.stage.f.db()
    }

    /// The server of the world.
    pub fn server(&self) -> &Server {
        &self.stage.f.team.flow.stand.server
    }

    /// The channel's current epoch, as alice's laptop holds it.
    #[track_caller]
    pub fn epoch(&self) -> u64 {
        (self.alice.mls.epoch(self.channel)).expect("alice's epoch")
    }

    /// alice's laptop commits an empty Commit, which bob's and carol's process; gives the new
    /// epoch.
    #[track_caller]
    pub fn rotate(&mut self) -> u64 {
        let f = &self.stage.f;
        let commit = self.alice.commits(f, empty);
        for peer in [&mut self.bob, &mut self.carol] {
            peer.process(f, &commit);
        }
        self.epoch()
    }

    /// The requests of `person`'s laptop on `person`'s session to `server`.
    pub fn laptop_on(&self, server: &Server, person: &Person) -> Client {
        let who = format!("{}'s laptop", person.handle);
        let token = person.session.token();
        self.client(server, token, &person.laptop, who)
    }

    /// The requests of the laptops of alice, bob, carol, dave and eve.
    pub fn laptops(&self) -> [Client; 5] {
        let f = self.fixture();
        let t = &f.team;
        [&t.alice, &t.bob, &t.carol, &t.dave, &f.eve]
            .map(|person| self.laptop_on(self.server(), person))
    }

    /// The bot's requests with a new token of `permissions` for `general`.
    #[track_caller]
    pub fn bot(&self, permissions: &[&str]) -> Client {
        let created = token(&self.stage, permissions);
        self.bot_on(&created.token, &format!("the bot with {permissions:?}"))
    }

    /// The bot's requests, named `who`, with a new token of `permissions` for `channel` alone, which
    /// alice's laptop grants.
    pub fn bot_in(&self, channel: ChannelId, permissions: &[&str], who: &str) -> Client {
        let (owner, bot) = (&self.stage.f.team.alice, self.stage.bot.id);
        let granted = grant(bot, &[channel], &owner.laptop, now() + DAY);
        let created = self.stage.create_as(owner, bot, &granted, permissions);
        self.bot_on(&created.token, who)
    }

    /// The bot's requests with the token `token`.
    pub fn bot_on(&self, token: &str, who: &str) -> Client {
        self.client(self.server(), token, &self.stage.bot, who.to_owned())
    }

    pub fn client(&self, server: &Server, token: &str, device: &Device, who: String) -> Client {
        let flow = &self.stage.f.team.flow;
        Client {
            who,
            base: server.base().to_owned(),
            token: token.to_owned(),
            https: Https::on_session(server, flow, token, device),
            space: self.space,
            channel: self.channel,
        }
    }

    /// A new task in `queue` under the channel's current epoch.
    #[track_caller]
    pub fn task(&self, queue: QueueId) -> QueuedTask {
        task(queue, self.epoch())
    }
}

/// A task of `queue` sealed under `epoch` as the server sees one: it cannot open `ct`, random bytes
/// here; with a new ID and no scheduling fields.
pub fn task(queue: QueueId, epoch: u64) -> QueuedTask {
    QueuedTask {
        task: TaskId::generate(),
        queue,
        sched: Sched::default(),
        epoch,
        enc: random(),
        ct: random_bytes(CT),
        unknown: Unknown::default(),
    }
}

/// `task` with the `idempotency_key` `key`.
pub fn keyed(mut task: QueuedTask, key: &[u8]) -> QueuedTask {
    task.sched.idempotency_key = Some(key.to_vec());
    task
}

/// What a request carries: no body, JSON or CBOR.
pub enum Body<'a> {
    None,
    Json(&'a Json),
    Cbor(&'a [u8]),
}

/// A device's requests for the queues and tasks of `general`, with a bearer token: a device
/// session's or a service token's.
pub struct Client {
    /// Who makes them, as the checks name it.
    pub who: String,
    base: String,
    token: String,
    https: Https,
    space: SpaceId,
    channel: ChannelId,
}

impl Client {
    /// `<method> <path>` with `body`, stating the version, with the bearer token and, if `proved`,
    /// the device's proof on a new challenge; checked against the spec.
    pub fn call(&self, method: &str, path: &str, body: Body, proved: bool) -> Response {
        let version = PROTOCOL_VERSION.to_string();
        let mut headers = vec![
            (VERSION_HEADER.to_owned(), version),
            ("authorization".to_owned(), format!("Bearer {}", self.token)),
        ];
        if proved {
            headers.push((PROOF.to_owned(), self.https.proof()));
        }
        let content = match body {
            Body::None => None,
            Body::Json(value) => Some(("application/json", value.to_string().into_bytes())),
            Body::Cbor(bytes) => Some((CBOR, bytes.to_vec())),
        };
        if let Some((media_type, _)) = &content {
            headers.push(("content-type".to_owned(), (*media_type).to_owned()));
        }
        let url = format!("{}{path}", self.base);
        let response = http(method, &url, &headers, content.map(|(_, bytes)| bytes));
        let violations = operation_violations(the_spec(), method, &template(path), &response);
        assert!(
            violations.is_empty(),
            "{} {method} {path}: {violations:?}: {}",
            self.who,
            text(&response)
        );
        response
    }

    /// The path of `general`'s queues.
    pub fn queues_path(&self) -> String {
        format!("/spaces/{}/channels/{}/queues", self.space, self.channel)
    }

    /// The path of `queue` of `general`.
    pub fn queue_path(&self, queue: QueueId) -> String {
        format!("{}/{queue}", self.queues_path())
    }

    /// The path of `queue`'s tasks.
    pub fn tasks_path(&self, queue: QueueId) -> String {
        format!("{}/tasks", self.queue_path(queue))
    }

    /// `GET` of `general`'s queues, proved.
    pub fn list(&self) -> Response {
        self.call("get", &self.queues_path(), Body::None, true)
    }

    /// `general`'s queues, which the device gets, sorted by name.
    #[track_caller]
    pub fn queues(&self) -> Vec<Queue> {
        let response = self.list();
        assert_eq!(response.status, 200, "{}: {}", self.who, text(&response));
        let body = json_of(&response);
        let items = body["queues"]
            .as_array()
            .unwrap_or_else(|| panic!("no queues in {body}"));
        let mut queues: Vec<Queue> = items.iter().map(Queue::of).collect();
        queues.sort_by(|a, b| a.name.cmp(&b.name));
        queues
    }

    /// The queue `name` of `general`, which the device gets.
    #[track_caller]
    pub fn queue(&self, name: &str) -> Queue {
        (self.queues().into_iter())
            .find(|queue| queue.name == name)
            .unwrap_or_else(|| panic!("{}: no queue {name}", self.who))
    }

    /// `POST` of a new queue of `settings`, proved if `proved`.
    pub fn create_with(&self, settings: &Json, proved: bool) -> Response {
        self.call("post", &self.queues_path(), Body::Json(settings), proved)
    }

    /// `POST` of a new queue of `settings`, proved.
    pub fn create(&self, settings: &Json) -> Response {
        self.create_with(settings, true)
    }

    /// Creates a queue of `settings`, which the server must take; gives it.
    #[track_caller]
    pub fn creates(&self, settings: &Json) -> Queue {
        let response = self.create(settings);
        assert_eq!(response.status, 201, "{}: {}", self.who, text(&response));
        Queue::of(&json_of(&response))
    }

    /// `PATCH` of `queue` with `settings`, proved if `proved`.
    pub fn change_with(&self, queue: QueueId, settings: &Json, proved: bool) -> Response {
        self.call(
            "patch",
            &self.queue_path(queue),
            Body::Json(settings),
            proved,
        )
    }

    /// `PATCH` of `queue` with `settings`, proved.
    pub fn change(&self, queue: QueueId, settings: &Json) -> Response {
        self.change_with(queue, settings, true)
    }

    /// Changes `queue` with `settings`, which the server must take; gives the queue it answers.
    #[track_caller]
    pub fn changes(&self, queue: QueueId, settings: &Json) -> Queue {
        let response = self.change(queue, settings);
        assert_eq!(response.status, 200, "{}: {}", self.who, text(&response));
        Queue::of(&json_of(&response))
    }

    /// `DELETE` of `queue`, proved if `proved`.
    pub fn delete_with(&self, queue: QueueId, proved: bool) -> Response {
        self.call("delete", &self.queue_path(queue), Body::None, proved)
    }

    /// `DELETE` of `queue`, proved.
    pub fn delete(&self, queue: QueueId) -> Response {
        self.delete_with(queue, true)
    }

    /// `POST` of `bytes` to `queue`'s tasks, proved if `proved`.
    pub fn upload(&self, queue: QueueId, bytes: &[u8], proved: bool) -> Response {
        self.call("post", &self.tasks_path(queue), Body::Cbor(bytes), proved)
    }

    /// Enqueues `task` in its queue, proved.
    pub fn enqueue(&self, task: &QueuedTask) -> Response {
        self.upload(task.queue, &task.to_cbor(), true)
    }

    /// Enqueues `task`, which the server must store; gives the answer.
    #[track_caller]
    pub fn stores(&self, task: &QueuedTask) -> Response {
        let response = self.enqueue(task);
        assert_stored(&response, &self.who);
        response
    }

    /// `GET` of `queue`'s tasks, proved if `proved`.
    pub fn list_tasks(&self, queue: QueueId, proved: bool) -> Response {
        self.call("get", &self.tasks_path(queue), Body::None, proved)
    }

    /// `queue`'s tasks, which the device gets, sorted by their bytes.
    #[track_caller]
    pub fn tasks(&self, queue: QueueId) -> Vec<Given> {
        let response = self.list_tasks(queue, true);
        assert_eq!(response.status, 200, "{}: {}", self.who, text(&response));
        let body = json_of(&response);
        let items = body["tasks"]
            .as_array()
            .unwrap_or_else(|| panic!("no tasks in {body}"));
        let mut tasks: Vec<Given> = items.iter().map(Given::of).collect();
        tasks.sort_by(|a, b| a.bytes.cmp(&b.bytes));
        tasks
    }

    /// `GET` of the task `task` of `queue`, proved if `proved`.
    pub fn read(&self, queue: QueueId, task: TaskId, proved: bool) -> Response {
        let path = format!("{}/{task}", self.tasks_path(queue));
        self.call("get", &path, Body::None, proved)
    }

    /// The task `task` of `queue`, which the device gets.
    #[track_caller]
    pub fn task(&self, queue: QueueId, task: TaskId) -> Given {
        let response = self.read(queue, task, true);
        assert_eq!(response.status, 200, "{}: {}", self.who, text(&response));
        Given::of(&json_of(&response))
    }
}

/// `<method> <url>` with `headers` and `body`.
fn http(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>) -> Response {
    let mut request = ureq::http::Request::builder()
        .method(method.to_ascii_uppercase().as_str())
        .uri(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let sent = match body {
        Some(bytes) => agent().run(request.body(bytes).expect("a request")),
        None => agent().run(request.body(()).expect("a request")),
    };
    let mut answer = sent.unwrap_or_else(|err| panic!("{method} {url}: {err}"));
    let status = answer.status().as_u16();
    let mut content_type = String::new();
    let mut kept = Vec::new();
    for (name, value) in answer.headers() {
        let value = value.to_str().expect("an ASCII header").to_owned();
        match name.as_str() {
            "content-type" => content_type = value,
            other => kept.push((other.to_owned(), value)),
        }
    }
    let body = answer.body_mut().read_to_vec().expect("the body");
    Response {
        status,
        content_type,
        headers: kept,
        body,
    }
}

/// A queue as the server gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub id: QueueId,
    pub name: String,
    pub mode: String,
    pub lease: u64,
    pub max_attempts: u64,
}

impl Queue {
    #[track_caller]
    pub fn of(value: &Json) -> Self {
        let field = |name: &str| &value[name];
        let string = |name: &str| {
            (field(name).as_str())
                .unwrap_or_else(|| panic!("no {name} in {value}"))
                .to_owned()
        };
        let number =
            |name: &str| (field(name).as_u64()).unwrap_or_else(|| panic!("no {name} in {value}"));
        Queue {
            id: string("id").parse().expect("a queue ID"),
            name: string("name"),
            mode: string("mode"),
            lease: number("lease"),
            max_attempts: number("max_attempts"),
        }
    }

    /// Checks that its settings are `mode`, `lease` and `max_attempts`.
    #[track_caller]
    pub fn assert_settings(&self, mode: &str, lease: u64, max_attempts: u64) {
        let found = (self.mode.as_str(), self.lease, self.max_attempts);
        assert_eq!(
            found,
            (mode, lease, max_attempts),
            "{}'s settings",
            self.name
        );
    }
}

/// A task as the server gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    /// The `queued-task`'s bytes.
    pub bytes: Vec<u8>,
    pub state: String,
    pub attempts: u64,
}

impl Given {
    #[track_caller]
    pub fn of(value: &Json) -> Self {
        let bytes =
            (value["queued_task"].as_str()).unwrap_or_else(|| panic!("no queued_task in {value}"));
        Given {
            bytes: unb64(bytes),
            state: (value["state"].as_str())
                .unwrap_or_else(|| panic!("no state in {value}"))
                .to_owned(),
            attempts: (value["attempts"].as_u64())
                .unwrap_or_else(|| panic!("no attempts in {value}")),
        }
    }

    /// `task` as the server gives it once stored: `queued` with attempts 0.
    pub fn queued(task: &QueuedTask) -> Self {
        Self::queued_bytes(&task.to_cbor())
    }

    /// The task of `bytes` once stored.
    pub fn queued_bytes(bytes: &[u8]) -> Self {
        Given {
            bytes: bytes.to_vec(),
            state: QUEUED.to_owned(),
            attempts: 0,
        }
    }
}

/// `tasks` as the server gives them once stored, sorted as [`Client::tasks`] sorts them.
pub fn queued(tasks: &[&QueuedTask]) -> Vec<Given> {
    let mut given: Vec<Given> = tasks.iter().map(|task| Given::queued(task)).collect();
    given.sort_by(|a, b| a.bytes.cmp(&b.bytes));
    given
}

/// The names of `queues`, in their order.
pub fn names(queues: &[Queue]) -> Vec<&str> {
    queues.iter().map(|queue| queue.name.as_str()).collect()
}

/// The settings of a new queue `name`, with `extra` settings.
pub fn named(name: &str, extra: Json) -> Json {
    let mut settings = json!({ "name": name });
    if let (Some(all), Some(more)) = (settings.as_object_mut(), extra.as_object()) {
        all.extend(more.clone());
    }
    settings
}

/// Checks that `response` takes an upload: a `2xx`.
#[track_caller]
pub fn assert_stored(response: &Response, what: &str) {
    assert!(
        (200..300).contains(&response.status),
        "{what}: stored, not {}",
        text(response)
    );
}

/// Checks that `response` refuses with `problem`.
#[track_caller]
pub fn assert_problem(response: &Response, problem: &str, what: &str) {
    assert_refused_with(response, Some(problem), what);
}

/// Checks that `response` refuses with `duplicate`, naming `task` in its member `task`.
#[track_caller]
pub fn assert_duplicate(response: &Response, task: TaskId, what: &str) {
    assert_problem(response, DUPLICATE, what);
    let body = json_of(response);
    assert_eq!(
        body["task"],
        json!(task.to_string()),
        "{what}: the task named"
    );
}

/// Checks that the database holds nothing of `task`: no row has its `ct`.
#[track_caller]
pub fn assert_not_held(db: &TestDb, task: &QueuedTask, what: &str) {
    assert!(!held(db, &task.ct), "{what}: the task is kept");
}

/// Checks that the database holds `task`'s `ct` in no row within [`GONE_WITHIN`].
#[track_caller]
pub fn assert_gone(db: &TestDb, task: &QueuedTask, what: &str) {
    let deadline = Instant::now() + GONE_WITHIN;
    while held(db, &task.ct) {
        assert!(
            Instant::now() < deadline,
            "{what}: still kept after {GONE_WITHIN:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A new database with the schema of the server before queues, holding the rows `db` has in the
/// tables that schema has: what a server before queues would have kept.
pub fn before_queues(db: &TestDb) -> TestDb {
    let old = TestDb::create();
    old.migrate_to(BEFORE_QUEUES);
    let tables: Vec<String> = (old.query(async |conn| {
        sqlx::query_scalar(
            "SELECT table_name::text FROM information_schema.tables
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
               AND table_name <> '_sqlx_migrations'",
        )
        .fetch_all(conn)
        .await
    }))
    .expect("the tables before queues");
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    for table in tables {
        let select = format!("SELECT row_to_json(t)::text FROM \"{table}\" t");
        let found: Vec<String> = (db.query(async |conn| {
            sqlx::query_scalar(AssertSqlSafe(select))
                .fetch_all(conn)
                .await
        }))
        .unwrap_or_else(|err| panic!("the rows of {table}: {err}"));
        rows.push((table, found));
    }
    // Foreign keys are checked by triggers, which the replica role skips: the rows go in any order.
    (old.query(async |conn| {
        let mut tx = conn.begin().await?;
        sqlx::raw_sql("SET LOCAL session_replication_role = replica")
            .execute(&mut *tx)
            .await?;
        for (table, found) in &rows {
            let insert = format!(
                "INSERT INTO \"{table}\" OVERRIDING SYSTEM VALUE
                 SELECT * FROM json_populate_record(NULL::\"{table}\", $1::json)"
            );
            for row in found {
                sqlx::query(AssertSqlSafe(insert.clone()))
                    .bind(row)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await
    }))
    .expect("copy the rows");
    // The sequences of identity and serial columns go on past the copied rows.
    old.execute(
        "DO $$ DECLARE c record; BEGIN
           FOR c IN SELECT table_name, column_name,
                           pg_get_serial_sequence(quote_ident(table_name::text), column_name::text) AS seq
                    FROM information_schema.columns WHERE table_schema = 'public' LOOP
             IF c.seq IS NOT NULL THEN
               EXECUTE format('SELECT setval(%L, COALESCE((SELECT max(%I) FROM %I), 0) + 1, false)',
                              c.seq, c.column_name, c.table_name);
             END IF;
           END LOOP;
         END $$;",
    );
    old
}
