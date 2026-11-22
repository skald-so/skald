//! What the tests share: the scene of the tests of claiming tasks with a short lease, sessions of
//! codex on Ada's second machine, the heartbeat as the relay sees it, the server's reads of a held
//! task, and the second machine's presence value.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{AgentState, PresenceBody, TaskId};

use crate::claiming::{CLAIMED, Claims, Held, LOST, Read, says};
use crate::client::{CLIENT, MCP, Session, session_in};
use crate::fixtures::{CHANNEL, cert_of};
use crate::helped::Helped;
use crate::machine::{Finished, Run};
use crate::queuing::run_in;
use crate::senders::LAPTOP_AGENT;
use crate::states::passes;
use crate::values::{status_in, values_in};
use crate::world::{AGENT, now};

/// The lease the tests give the queue `default`, in seconds, and a third of it.
pub const LEASE: u64 = 15;
pub const THIRD: Duration = Duration::from_secs(LEASE / 3);

/// How late a heartbeat may come past a third of the lease on a loaded machine.
pub const LATE: Duration = Duration::from_secs(3);

/// How long the tests hold a task: three leases.
pub const THREE_LEASES: Duration = Duration::from_secs(3 * LEASE);

/// How long a lease that is no longer extended takes to end in the server's reads at most: the
/// lease, the server's look for ended leases every 5 seconds, and room for a loaded machine.
pub const ENDS_WITHIN: Duration = Duration::from_secs(LEASE + 5 + 25);

/// The command of `skald recv` and its flag that waits.
pub const RECV: &str = "recv";
pub const WAIT_FLAG: &str = "--wait";

/// The scene of the tests of claiming tasks, the queue `default` handing out claims with a lease
/// of [`LEASE`] seconds.
pub struct Scene {
    pub claims: Claims,
}

impl Scene {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones.
    #[track_caller]
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let claims = Claims::new(ids, logins);
        claims.cloud().execute(&format!(
            "ALTER TABLE queues DROP CONSTRAINT IF EXISTS queues_lease_check;
             UPDATE queues SET lease = {LEASE} WHERE id = '{}'::uuid",
            claims.queue
        ));
        Scene { claims }
    }

    /// Ada's second machine's own copy of `skald`, whose runs may go through a helper.
    pub fn helped(&self) -> Helped {
        Helped::new(&self.claims.laptop().machine)
    }

    /// Starts `skald --agent <agent> mcp` of `helped`'s copy in the second machine's `~/work/acme`,
    /// a session through the helper, and initializes it as `claude-code`.
    pub fn helped_session_of(&self, helped: &Helped, agent: &str) -> Session {
        let dir = self.claims.senders.laptop_folder.as_path();
        let mut command = helped.command_in(dir, &[], &[AGENT, agent, MCP]);
        Session::start(&mut command, CLIENT)
    }

    /// As [`Scene::helped_session_of`], of codex.
    pub fn helped_session(&self, helped: &Helped) -> Session {
        self.helped_session_of(helped, LAPTOP_AGENT)
    }

    /// Starts `skald --agent codex-v5d8tm mcp` one-shot in the second machine's `~/work/acme`, and
    /// initializes it as `claude-code`.
    pub fn one_shot_session(&self) -> Session {
        let laptop = &self.claims.laptop().machine;
        let dir = self.claims.senders.laptop_folder.as_path();
        session_in(laptop, dir, &[AGENT, LAPTOP_AGENT], CLIENT)
    }

    /// Starts `skald --agent codex-v5d8tm recv --wait` of `helped`'s copy in the second machine's
    /// `~/work/acme`, a session through the helper.
    pub fn helped_wait(&self, helped: &Helped) -> Run {
        let dir = self.claims.senders.laptop_folder.as_path();
        helped.start_in(dir, &[], &[AGENT, LAPTOP_AGENT, RECV, WAIT_FLAG])
    }

    /// As claude on Ada's first machine, enqueues `text` in the queue `default`; gives the task.
    #[track_caller]
    pub fn claude_enqueues(&self, text: &str) -> TaskId {
        self.claims.claude_enqueues(text).task
    }

    /// Codex claims a task of the queue `default`, which must pass the claim checks: the server's
    /// reads show `task` claimed by the second machine's device. Gives its claim, and when the
    /// claim was asked for.
    #[track_caller]
    pub fn codex_holds(&self, task: TaskId) -> (Held, Instant) {
        let since = self.claims.cloud().relay.mark();
        self.claims.codex_claimed(&[]);
        (self.held_by_laptop(task), since)
    }

    /// The claim of `task` the server's reads show the second machine's device holding; fails
    /// unless they show one.
    #[track_caller]
    pub fn held_by_laptop(&self, task: TaskId) -> Held {
        let read = self.claims.read(task);
        assert_eq!(read.state, CLAIMED, "{task} not claimed: {read:?}");
        let held = read.held().clone();
        assert_eq!(
            held.device,
            self.claims.laptop().device,
            "{task} held by another device: {read:?}"
        );
        held
    }

    /// Codex's `skald <command> <args>`, which must fail and say the claim is lost.
    #[track_caller]
    pub fn assert_codex_lost(&self, command: &str, args: &[&str]) -> Finished {
        let ran = self.claims.codex_runs(command, args);
        assert!(!ran.success, "codex's {command} of a lost claim: {ran:?}");
        assert!(says(&ran.shown(), LOST), "{ran:?}");
        ran
    }

    /// The times, in order, of the extends of `task` the machines sent from `since` on.
    pub fn extends_since(&self, since: Instant, task: TaskId) -> Vec<Instant> {
        let named = format!("/tasks/{task}/extend ");
        (self.claims.cloud().relay.sent_since(since).into_iter())
            .filter(|sent| {
                let line = sent.head.lines().next().unwrap_or_default();
                line.starts_with("post ") && line.contains(&named)
            })
            .map(|sent| sent.at)
            .collect()
    }

    /// For `span` from now, fails as soon as the server's reads show `task` other than `claimed`
    /// by `held`, the same claim and generation, with `progress`. Looks every second.
    #[track_caller]
    pub fn assert_held_for(&self, task: TaskId, held: &Held, progress: &str, span: Duration) {
        let until = Instant::now() + span;
        loop {
            let read = self.claims.read(task);
            assert_held(&read, held, progress);
            if Instant::now() >= until {
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
    }

    /// Fails unless the extends of `task` the machines sent from `since` until now came each third
    /// of the lease: the first of them, the claim's own, then one within a third, and [`LATE`], of
    /// each other and of now, and not many more than one a third.
    #[track_caller]
    pub fn assert_heartbeat(&self, since: Instant, task: TaskId) {
        let extends = self.extends_since(since, task);
        let end = Instant::now();
        let span = end.duration_since(since);
        assert!(!extends.is_empty(), "no extend of {task} in {span:?}");
        let mut times = extends.clone();
        times.push(end);
        let gaps: Vec<Duration> = times.windows(2).map(|two| two[1] - two[0]).collect();
        let longest = gaps.iter().max().copied().unwrap_or_default();
        assert!(
            longest <= THIRD + LATE,
            "no extend of {task} for {longest:?}, longer than a third of the lease, \
             in {span:?}: {gaps:?}"
        );
        let most = (span.as_secs() / THIRD.as_secs()) as usize + 4;
        assert!(
            extends.len() <= most,
            "{} extends of {task} in {span:?}, more than one a third of the lease: {gaps:?}",
            extends.len()
        );
    }

    /// Fails if the machines sent an extend of `task` from `since` on.
    #[track_caller]
    pub fn assert_no_extend(&self, since: Instant, task: TaskId, what: &str) {
        let extends = self.extends_since(since, task);
        assert!(
            extends.is_empty(),
            "{what}: {} extends of {task} sent",
            extends.len()
        );
    }

    /// Waits for up to [`ENDS_WITHIN`] until the server's reads show `task` no longer held by
    /// `held`: its lease ended. Gives the read.
    #[track_caller]
    pub fn await_lease_end(&self, task: TaskId, held: &Held) -> Read {
        let until = Instant::now() + ENDS_WITHIN;
        loop {
            let read = self.claims.read(task);
            if read.state != CLAIMED || read.claim.as_ref() != Some(held) {
                return read;
            }
            assert!(
                Instant::now() < until,
                "the lease of {task} has not ended within {ENDS_WITHIN:?}: {read:?}"
            );
            thread::sleep(Duration::from_millis(500));
        }
    }

    /// The backoff of `task` after its lease ended passes: the server's times of the task move back
    /// an hour.
    pub fn backoff_passes(&self, task: TaskId) {
        passes(self.claims.cloud(), task, 3600);
    }

    /// As the person on Ada's first machine, `skald claim --channel <channel>`; fails unless it
    /// succeeds.
    #[track_caller]
    pub fn ada_claims(&self) -> Finished {
        let ada = &self.claims.ada().machine;
        let dir = self.claims.senders.agents.folder.as_path();
        let channel = self.claims.channel.as_str();
        let ran = run_in(ada, (dir, None), "claim", &[CHANNEL, channel]);
        assert!(ran.success, "Ada's claim on her first machine: {ran:?}");
        ran
    }

    /// The body of the second machine's presence value in the talk's channel, as Bob's device opens
    /// it now, if the server holds one Bob's device opens.
    pub fn laptop_body(&mut self) -> Option<PresenceBody> {
        let at = self.claims.at;
        let device = self.claims.laptop().device;
        let value = values_in(self.claims.talk(), at)
            .into_iter()
            .find(|value| value.device == device)?;
        let talk = &mut self.claims.senders.agents.talk;
        if (talk.played[0].mls)
            .history_key(value.channel, value.epoch)
            .is_none()
        {
            talk.played[0].take_in(&talk.world.cloud);
        }
        let key = (talk.played[0].mls).history_key(value.channel, value.epoch)?;
        let signing = cert_of(talk.cloud(), &self.claims.senders.laptop)
            .body()
            .signing;
        key.presence_key().open(&value, &signing, now()).ok()
    }

    /// Waits for up to a minute until the second machine's presence value in the talk's channel
    /// lists codex in `state`; fails unless it does. Gives the body.
    #[track_caller]
    pub fn await_codex_shown(&mut self, state: AgentState) -> PresenceBody {
        let codex = self.claims.codex();
        let until = Instant::now() + Duration::from_secs(60);
        let mut last = None;
        loop {
            if let Some(body) = self.laptop_body() {
                if status_in(&body, &codex).is_some_and(|status| status.state == state) {
                    return body;
                }
                last = Some(body);
            }
            assert!(
                Instant::now() < until,
                "codex not shown {state:?} on the second machine; the last value: {last:?}"
            );
            thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Fails unless `read` shows its task `claimed` by `held`, the same claim and generation, with
/// `progress`.
#[track_caller]
pub fn assert_held(read: &Read, held: &Held, progress: &str) {
    assert_eq!(read.state, CLAIMED, "the claim was not kept: {read:?}");
    assert_eq!(
        read.claim.as_ref(),
        Some(held),
        "another claim holds the task: {read:?}"
    );
    assert_eq!(read.progress.as_deref(), Some(progress), "{read:?}");
}

/// Waits for `span`.
pub fn waits(span: Duration) {
    thread::sleep(span);
}
