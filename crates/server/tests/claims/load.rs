//! A queue load test: 1,000 tasks of both modes, and 20 member devices claiming, completing,
//! failing and letting leases end through two instances. No task is held by two claims at once, no
//! `at-most-once` task is claimed twice, and every task ends `done` or `dead`.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use skald_proto::{DeviceKind, QueueId, QueueMode, QueuedTask, TaskId};

use crate::common::device_flow::text;
use crate::holder::{
    ALL, CLAIM_LOST, CLAIMED, DEAD, DONE, FAILED, Holder, LEASE_EXPIRED, READS_WITHIN, STARTED,
    client_of, device_of, join, passes_for, takes,
};
use crate::kit::{AT_LEAST_ONCE, AT_MOST_ONCE, Client, World, named, task};

/// Tasks in each of the two queues.
const EACH: usize = 500;
/// The member devices claiming.
const DEVICES: usize = 20;
/// The attempts of the `at-least-once` queue.
const MAX_ATTEMPTS: u64 = 3;
/// The first 4 bytes of the IDs of the test's tasks, so that time passes for them alone.
const PREFIX: [u8; 4] = [0x10, 0xad, 0x7e, 0x57];
/// The same, as the text of an ID begins.
const PREFIX_TEXT: &str = "10ad7e57";
/// Seconds that pass between rounds: past any lease and backoff of the test's queues.
const BETWEEN: u64 = 4_000;
/// The most rounds the tasks may need: one per attempt, and some to spare.
const ROUNDS: usize = 10;

/// What a holder did with a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Completed,
    Failed {
        retry: bool,
    },
    /// Let its lease end.
    Abandoned,
    /// Got `claim-lost`.
    Lost,
}

/// A claim a device got, and what came of it.
#[derive(Debug, Clone)]
struct Record {
    task: TaskId,
    mode: QueueMode,
    generation: u64,
    attempt: u64,
    round: usize,
    outcome: Outcome,
}

/// What a holder does with the claim of generation `generation` of `task`: spread over the outcomes
/// by the task's ID.
fn choose(task: TaskId, generation: u64) -> Outcome {
    let bytes = task.as_bytes();
    let spread = (u64::from(bytes[15]) + u64::from(bytes[14]) * 7 + generation * 13) % 20;
    match spread {
        0..=10 => Outcome::Completed,
        11..=13 => Outcome::Failed { retry: true },
        14 => Outcome::Failed { retry: false },
        _ => Outcome::Abandoned,
    }
}

/// A task of `queue` under `epoch` whose ID begins with [`PREFIX`].
fn load_task(queue: QueueId, epoch: u64) -> QueuedTask {
    let mut new = task(queue, epoch);
    let mut bytes = *new.task.as_bytes();
    bytes[..4].copy_from_slice(&PREFIX);
    new.task = TaskId::from_bytes(bytes);
    new
}

/// `client` claims from `queues` in turn until neither has a task, and does with each claim what
/// [`choose`] says; adds what it did to `records`.
fn work(client: &Client, queues: [QueueId; 2], round: usize, records: &Mutex<Vec<Record>>) {
    let mut empty = 0;
    let mut turn = 0;
    while empty < queues.len() {
        let queue = queues[turn % queues.len()];
        turn += 1;
        let Some(claim) = client.claim_of(queue, &takes(&ALL)) else {
            empty += 1;
            continue;
        };
        empty = 0;
        let mut outcome = choose(claim.task.task, claim.generation);
        let answer = match outcome {
            Outcome::Completed => {
                if claim.generation % 2 == 0 {
                    let extend = client.extend(&claim, STARTED);
                    assert!(
                        (200..300).contains(&extend.status),
                        "{}: an extend: {}",
                        client.who,
                        text(&extend)
                    );
                }
                Some(client.complete(&claim))
            }
            Outcome::Failed { retry } => Some(client.fail(&claim, retry)),
            Outcome::Abandoned | Outcome::Lost => None,
        };
        if let Some(answer) = answer
            && !(200..300).contains(&answer.status)
        {
            let lost = crate::https::problem(CLAIM_LOST);
            let body: serde_json::Value = serde_json::from_slice(&answer.body).unwrap_or_default();
            assert_eq!(
                body["type"],
                json!(lost),
                "{}: {}",
                client.who,
                text(&answer)
            );
            outcome = Outcome::Lost;
        }
        (records.lock().expect("the records")).push(Record {
            task: claim.task.task,
            mode: claim.mode,
            generation: claim.generation,
            attempt: claim.attempt,
            round,
            outcome,
        });
    }
}

/// The tasks of `queues` as `client` lists them: each task's state, attempts and reason.
fn listed(
    client: &Client,
    queues: [QueueId; 2],
) -> BTreeMap<String, (String, u64, Option<String>)> {
    let mut all = BTreeMap::new();
    for queue in queues {
        for read in client.states(queue) {
            let shown = (read.state.clone(), read.attempts, read.reason.clone());
            all.insert(read.task().to_string(), shown);
        }
    }
    all
}

/// Reads through `client` until no task of `queues` is `claimed`, within [`READS_WITHIN`].
#[track_caller]
fn until_none_claimed(client: &Client, queues: [QueueId; 2]) {
    let deadline = Instant::now() + READS_WITHIN;
    loop {
        let claimed = (listed(client, queues).values())
            .filter(|(state, ..)| state == CLAIMED)
            .count();
        if claimed == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{claimed} tasks still claimed after {READS_WITHIN:?}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// 17 more client devices of alice, bob and carol join the group; with the three laptops, 10 of
/// them work through the first instance and 10 through a second. They enqueue 500 tasks to an
/// `at-least-once` queue of 3 attempts and 500 to an `at-most-once` one, then claim in rounds,
/// completing, failing with and without retry, and abandoning claims; between rounds, time passes
/// past every lease and backoff. Every task ends `done` or `dead`, with the attempts and reason of
/// its claims, and each task's claims, by generation 1, 2, ..., each began after the one before
/// ended: an abandoned claim's lease ends only between rounds, a completed claim is the last, and
/// no claim is lost. Each `at-most-once` task has one claim, of `attempt` 1; no `at-least-once`
/// task has more claims than its attempts.
#[test]
fn a_thousand_tasks_through_two_instances_are_never_lost_or_doubled() {
    let mut w = World::new(3_926_900_001);
    let team = &w.fixture().team;
    let people = [&team.alice, &team.bob, &team.carol];
    let workers: Vec<_> = (0..DEVICES - 3)
        .map(|n| {
            let person = people[n % people.len()];
            (
                n % people.len(),
                device_of(&w, person, DeviceKind::Client, &format!("Worker {n}")),
            )
        })
        .collect();
    let devices: Vec<_> = workers.iter().map(|(_, device)| device).collect();
    join(&mut w, &devices);

    let other = w.fixture().team.flow.stand.instance(&[], &[]);
    let team = &w.fixture().team;
    let people = [&team.alice, &team.bob, &team.carol];
    let mut clients: Vec<Client> = Vec::new();
    for (n, person) in people.iter().enumerate() {
        let server = if n % 2 == 0 { w.server() } else { &other };
        clients.push(w.laptop_on(server, person));
    }
    for (n, (owner, device)) in workers.iter().enumerate() {
        let server = if n % 2 == 0 { &other } else { w.server() };
        clients.push(client_of(&w, server, people[*owner], device));
    }
    assert_eq!(clients.len(), DEVICES, "the devices claiming");

    let steady = clients[0]
        .creates(&named(
            "steady",
            json!({ "mode": AT_LEAST_ONCE, "max_attempts": MAX_ATTEMPTS }),
        ))
        .id;
    let deploys = clients[0]
        .creates(&named("deploys", json!({ "mode": AT_MOST_ONCE })))
        .id;
    let queues = [steady, deploys];
    let epoch = w.epoch();
    thread::scope(|scope| {
        for (n, client) in clients.iter().enumerate() {
            scope.spawn(move || {
                for k in (n..2 * EACH).step_by(DEVICES) {
                    client.stores(&load_task(queues[k % 2], epoch));
                }
            });
        }
    });
    assert_eq!(
        listed(&clients[1], queues).len(),
        2 * EACH,
        "the tasks enqueued"
    );

    let records = Mutex::new(Vec::new());
    let mut finished = false;
    for round in 0..ROUNDS {
        thread::scope(|scope| {
            for client in &clients {
                let records = &records;
                scope.spawn(move || work(client, queues, round, records));
            }
        });
        // The abandoned claims' leases end, and every backoff, also one counted from when the
        // server saw a lease end.
        passes_for(w.db(), PREFIX_TEXT, BETWEEN);
        until_none_claimed(&clients[1], queues);
        passes_for(w.db(), PREFIX_TEXT, BETWEEN);
        let states = listed(&clients[1], queues);
        if states
            .values()
            .all(|(state, ..)| state == DONE || state == DEAD)
        {
            finished = true;
            break;
        }
    }
    let states = listed(&clients[0], queues);
    assert!(
        finished,
        "tasks not done or dead after {ROUNDS} rounds: {:?}",
        (states.iter())
            .filter(|(_, (state, ..))| state != DONE && state != DEAD)
            .take(10)
            .collect::<Vec<_>>()
    );
    assert_eq!(states.len(), 2 * EACH, "every task listed");

    let records = records.into_inner().expect("the records");
    let mut by_task: BTreeMap<String, Vec<Record>> = BTreeMap::new();
    for record in records {
        by_task
            .entry(record.task.to_string())
            .or_default()
            .push(record);
    }
    for (task, (state, attempts, reason)) in &states {
        let claims = by_task
            .get_mut(task)
            .unwrap_or_else(|| panic!("{task}: never claimed"));
        claims.sort_by_key(|record| record.generation);
        let generations: Vec<u64> = claims.iter().map(|record| record.generation).collect();
        let expected: Vec<u64> = (1..=claims.len() as u64).collect();
        assert_eq!(
            generations, expected,
            "{task}: the generations of its claims"
        );
        for pair in claims.windows(2) {
            let (before, after) = (&pair[0], &pair[1]);
            match before.outcome {
                Outcome::Abandoned => assert!(
                    after.round > before.round,
                    "{task}: generation {} while {} held it",
                    after.generation,
                    before.generation
                ),
                Outcome::Failed { retry: true } => {}
                other => panic!(
                    "{task}: generation {} after generation {} ended {other:?}",
                    after.generation, before.generation
                ),
            }
        }
        let last = claims.last().expect("a claim");
        assert!(
            claims.iter().all(|record| record.outcome != Outcome::Lost),
            "{task}: a claim lost: {claims:?}"
        );
        match last.mode {
            QueueMode::AtMostOnce => {
                assert_eq!(claims.len(), 1, "{task}: at-most-once claimed again");
                assert_eq!(last.attempt, 1, "{task}: at-most-once attempt");
            }
            QueueMode::AtLeastOnce => {
                assert!(
                    claims.len() as u64 <= MAX_ATTEMPTS,
                    "{task}: {} claims",
                    claims.len()
                );
                for record in claims.iter() {
                    assert_eq!(record.attempt, record.generation, "{task}: the attempt");
                }
            }
        }
        let expected = match last.outcome {
            Outcome::Completed => (DONE, None),
            Outcome::Failed { .. } => (DEAD, Some(FAILED)),
            Outcome::Abandoned => (DEAD, Some(LEASE_EXPIRED)),
            Outcome::Lost => unreachable!("no claim is lost"),
        };
        assert_eq!(
            (state.as_str(), reason.as_deref()),
            expected,
            "{task}: its end after {claims:?}"
        );
        assert_eq!(*attempts, claims.len() as u64, "{task}: its attempts");
    }
}
