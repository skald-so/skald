//! Local rules over synthetic message counts, with findings read from the private test database.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::{AccountId, DeviceId, SpaceId};

use crate::api::{Person, Session};
use crate::common::device_flow::{json as body, text};
use crate::common::{SERVER, Server, TestDb, hex};
use crate::erasure::ERASURE;
use crate::fixture::Fixture;
use crate::listener::second_device;
use crate::wire::{Answer, Socket, envelope, key_of, query};

const RULES_ENV: &str = "SKALD_ABUSE_RULES_FILE";
const VERSION: &str = "synthetic-count-v1";
const WINDOW: i64 = 120;
const RETENTION: i64 = 60 * 86_400;
const COUNTING: &[&str] = &["--monthly-messages-limit", "100"];
const LOOK: Duration = Duration::from_millis(250);
const SWEEP_INTERVAL: Duration = Duration::from_secs(6);

struct RuleFile(PathBuf);

impl RuleFile {
    fn new(value: &Value) -> Self {
        let path = env::temp_dir().join(format!("skald-test-rules-{}.json", AccountId::generate()));
        fs::write(&path, value.to_string()).expect("write the synthetic rules");
        Self(path)
    }

    fn start(&self, fixture: &Fixture) -> Server {
        fixture.team.flow.stand.instance(
            COUNTING,
            &[(RULES_ENV, self.0.to_str().expect("a UTF-8 rules path"))],
        )
    }

    /// Give invalid rules an evaluation interval even if the process rejects them at startup.
    fn try_invalid(&self, db: &TestDb) {
        let mut child = Command::new(SERVER)
            .args(["--listen", "127.0.0.1:0", "--database-url", &db.url()])
            .args(COUNTING)
            .env(RULES_ENV, &self.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start skald-server with invalid rules");
        thread::sleep(SWEEP_INTERVAL);
        let _ = child.kill();
        child.wait().expect("reap skald-server");
    }
}

impl Drop for RuleFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn rule(window: i64, minimum: i64) -> Value {
    json!({
        "signal": "counted_messages",
        "window_seconds": window,
        "minimum_count": minimum
    })
}

fn rules(retention: i64, rules: &[Value]) -> RuleFile {
    RuleFile::new(&json!({
        "version": VERSION,
        "retention_seconds": retention,
        "rules": rules
    }))
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd, sqlx::FromRow)]
struct Finding {
    account: String,
    rule_version: String,
    count: i64,
}

impl Finding {
    fn of(person: &Person, count: i64) -> Self {
        Self {
            account: person.account.to_string(),
            rule_version: VERSION.to_owned(),
            count,
        }
    }
}

fn findings(db: &TestDb) -> Vec<Finding> {
    let mut found = query(db, async |conn| {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass('abuse_findings') IS NOT NULL")
            .fetch_one(&mut *conn)
            .await?;
        if !exists {
            return Ok(Vec::new());
        }
        sqlx::query_as(
            "SELECT account_id::text AS account, rule_version, (evidence->>'count')::bigint AS count
             FROM abuse_findings",
        )
        .fetch_all(conn)
        .await
    });
    found.sort();
    found
}

#[track_caller]
fn await_findings(db: &TestDb, mut expected: Vec<Finding>) {
    expected.sort();
    let deadline = Instant::now() + ERASURE;
    loop {
        let found = findings(db);
        if found == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected:?}, found {found:?}"
        );
        thread::sleep(LOOK);
    }
}

/// Ordinary sends populate the existing server counter.
fn send(fixture: &Fixture, person: &Person, to: DeviceId, count: usize) {
    let mut socket = Socket::of(&fixture.team.flow, person);
    for _ in 0..count {
        let header = fixture.to(to);
        socket.send(&envelope(&header));
        match socket.answer() {
            Answer::Stored(stored) => assert_eq!(stored.send_key, key_of(&header)),
            other => panic!("the message should be stored: {other:?}"),
        }
    }
}

/// Add timestamped counter data without waiting for the fixture's time window to elapse.
fn seed(fixture: &Fixture, device: DeviceId, count: i64, seconds_ago: i64) {
    let device = device.to_string();
    let space = fixture.team.space.to_string();
    query(fixture.db(), async |conn| {
        sqlx::query(
            "INSERT INTO counted_messages (sender_id, space_id, msg_id, counted_at)
             SELECT $1::uuid, $2::uuid, gen_random_uuid(), now() - make_interval(secs => $4)
             FROM generate_series(1, $3::bigint)",
        )
        .bind(device)
        .bind(space)
        .bind(count)
        .bind(seconds_ago as f64)
        .execute(conn)
        .await
    });
}

fn customer_view(session: &Session, space: SpaceId) -> Vec<Value> {
    [
        "/session",
        "/account/usage",
        &format!("/spaces/{space}/usage"),
    ]
    .into_iter()
    .map(|path| {
        let mut response = session.call("get", path, None);
        assert_eq!(response.status, 200, "{path}: {}", text(&response));
        response.headers.retain(|(name, _)| name != "date");
        response.headers.sort();
        json!({
            "content_type": response.content_type,
            "headers": response.headers,
            "body": body(&response)
        })
    })
    .collect()
}

#[test]
fn configured_boundaries_produce_findings_without_changing_customer_responses() {
    let fixture = Fixture::with(3_618_400_001, COUNTING);
    let t = &fixture.team;
    let phone = second_device(fixture.db(), &t.bob, "Phone");
    send(&fixture, &t.alice, t.bob.laptop.id, 3);
    send(&fixture, &t.bob, t.carol.laptop.id, 1);
    send(&fixture, &t.carol, t.bob.laptop.id, 1);
    seed(&fixture, phone.id, 1, WINDOW / 2);
    seed(&fixture, t.carol.laptop.id, 2, WINDOW + 1);
    let views = [&t.alice, &t.carol].map(|person| customer_view(&person.session, t.space));

    let file = rules(RETENTION, &[rule(WINDOW, 2)]);
    let first = file.start(&fixture);
    await_findings(
        fixture.db(),
        vec![Finding::of(&t.alice, 3), Finding::of(&t.bob, 2)],
    );
    for (person, before) in [&t.alice, &t.carol].into_iter().zip(views) {
        assert_eq!(
            customer_view(&person.session.on(&first), t.space),
            before,
            "{}",
            person.handle
        );
    }
    drop(first);

    // A new finding proves the second instance evaluated the counters again.
    send(&fixture, &t.carol, t.bob.laptop.id, 1);
    let _second = file.start(&fixture);
    await_findings(
        fixture.db(),
        vec![
            Finding::of(&t.alice, 3),
            Finding::of(&t.bob, 2),
            Finding::of(&t.carol, 2),
        ],
    );
}

fn account_rows(db: &TestDb) -> Vec<String> {
    query(db, async |conn| {
        sqlx::query_scalar("SELECT row_to_json(a)::text FROM accounts a ORDER BY id")
            .fetch_all(conn)
            .await
    })
}

#[track_caller]
fn stays_empty(db: &TestDb) {
    let deadline = Instant::now() + SWEEP_INTERVAL;
    loop {
        let found = findings(db);
        assert!(found.is_empty(), "unexpected findings: {found:?}");
        if Instant::now() >= deadline {
            return;
        }
        thread::sleep(LOOK);
    }
}

#[test]
fn missing_or_invalid_rules_leave_accounts_and_findings_alone() {
    let fixture = Fixture::with(3_618_400_011, COUNTING);
    let t = &fixture.team;
    seed(&fixture, t.alice.laptop.id, 2, 0);
    let db = fixture.db();
    let accounts = account_rows(db);
    let customer = customer_view(&t.alice.session, t.space);

    stays_empty(db);
    assert_eq!(account_rows(db), accounts, "without rules");
    assert_eq!(
        customer_view(&t.alice.session, t.space),
        customer,
        "without rules"
    );

    // One invalid rule makes the file invalid, including its otherwise matching rule.
    rules(RETENTION, &[rule(WINDOW, 2), rule(0, 2)]).try_invalid(db);
    assert!(findings(db).is_empty(), "with invalid rules");
    assert_eq!(account_rows(db), accounts, "with invalid rules");
    assert_eq!(
        customer_view(&t.alice.session, t.space),
        customer,
        "with invalid rules"
    );

    // The same evidence qualifies when the rules are valid.
    let valid = rules(RETENTION, &[rule(WINDOW, 2)]);
    let _instance = valid.start(&fixture);
    await_findings(db, vec![Finding::of(&t.alice, 2)]);
    assert_eq!(account_rows(db), accounts, "with valid rules");
}

#[test]
fn the_configured_retention_removes_expired_findings() {
    const SHORT_RETENTION: i64 = 1200;
    const SHORT_WINDOW: i64 = 60;
    let fixture = Fixture::with(3_618_400_021, COUNTING);
    let t = &fixture.team;
    seed(&fixture, t.alice.laptop.id, 3, 0);
    seed(&fixture, t.bob.laptop.id, 3, 0);
    seed(&fixture, t.carol.laptop.id, 2, 0);
    seed(&fixture, t.carol.laptop.id, 1, SHORT_WINDOW + 1);
    let db = fixture.db();
    let file = rules(SHORT_RETENTION, &[rule(SHORT_WINDOW, 3)]);
    let _instance = file.start(&fixture);
    await_findings(db, vec![Finding::of(&t.alice, 3), Finding::of(&t.bob, 3)]);

    db.execute(&format!(
        "BEGIN;
         UPDATE counted_messages SET counted_at = now() - make_interval(secs => {SHORT_WINDOW} + 1)
         WHERE sender_id = '{}'::uuid;
         UPDATE abuse_findings SET created_at = now() - make_interval(secs => {SHORT_RETENTION} + 1)
         WHERE account_id = '{}'::uuid;
         COMMIT",
        t.alice.laptop.id, t.alice.account
    ));
    await_findings(db, vec![Finding::of(&t.bob, 3)]);
}

/// The existing every-table test's alice has already sent one counted application message.
pub fn fill_for_erasure(fixture: &Fixture) {
    let t = &fixture.team;
    seed(fixture, t.alice.laptop.id, 1, 0);
    seed(fixture, t.bob.laptop.id, 2, 0);
    let file = rules(RETENTION, &[rule(WINDOW, 2)]);
    let _instance = file.start(fixture);
    await_findings(
        fixture.db(),
        vec![Finding::of(&t.alice, 2), Finding::of(&t.bob, 2)],
    );
}

/// The finding is still recent according to its separate retention setting.
#[track_caller]
pub fn assert_finding_erased(fixture: &Fixture) {
    let t = &fixture.team;
    let id = t.alice.account.to_string();
    let id_hex = hex(t.alice.account.as_bytes());
    let bob = t.bob.account.to_string();
    let mut kept_bob = false;
    for row in fixture.db().rows() {
        if row.starts_with("abuse_findings: ") {
            assert!(
                !row.contains(&id) && !row.contains(&id_hex),
                "deleted account in {row}"
            );
            kept_bob |= row.contains(&bob);
        }
    }
    assert!(kept_bob, "the other account's finding should remain");
}
