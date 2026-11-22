//! The lease heartbeat (`docs/protocol/queues.md`, "Claims and leases"): the device extends each
//! claim of an agent while a session of that agent runs, so a live but slow agent keeps its task
//! and a stopped one lets its lease end.
//!
//! - A claim's session is any session of its agent in its space on the device: an MCP server's or
//!   a `skald recv --wait`. The helper extends the claims of the agents whose sessions run through
//!   it ([`Beats`]); without the helper, a one-shot session extends its own agent's
//!   ([`beat_agent`]). A claim as the person, or of an agent with no session running, gets none:
//!   `skald extend` keeps it.
//! - Each extend comes once a third of the lease has passed since the last answered request that
//!   started it, and states the progress of the last answered extend, `started` until `skald
//!   extend` states another, which extends at once.
//! - The heartbeat stops when the claim ends and when the session stops. A claim whose extend gets
//!   `claim-lost` or a lasting refusal, or whose lease passed on the device's clock, it marks lost
//!   and stops extending; the agent's next output says so, and so does the next command on it
//!   ([`crate::claims`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use serde_json::json;
use skald_proto::{Name, SpaceId, VersionRange};

use crate::claims::{self, Fenced, Held, Progress};
use crate::devices::text;
use crate::spaces::Follower;
use crate::state::{self, State, TaskClaim};
use crate::uploads::{self, Running, earliest};
use crate::{folders, lock, secrets};

/// How long after an extend the server did not answer the heartbeat tries again, in seconds.
const AGAIN: u64 = 1;

/// How often a one-shot session looks whether an extend of its agent's claims is due.
pub const TICK: Duration = Duration::from_secs(1);

/// What a heartbeat made of the claims it looked at.
#[derive(Debug, Default, PartialEq)]
pub struct Beaten {
    /// When the next extend of one of them is due, in Unix seconds.
    pub due: Option<u64>,
    /// Whether it found one of them lost.
    pub lost: bool,
}

/// The helper's heartbeat: what it last looked with, and when it looks again.
#[derive(Default)]
pub struct Beats {
    running: Option<BTreeSet<Running>>,
    due: Option<u64>,
    /// Whether a command ran since: it may have changed the claims.
    ran: bool,
}

impl Beats {
    /// Notes that a command ran.
    pub fn ran(&mut self) {
        self.ran = true;
    }

    /// Extends what is due of the claims of the agents of `running` by the state in `dir`
    /// ([`beat`]), once a command ran, the agents that run changed or an extend is due; gives
    /// whether it found a claim lost.
    pub fn check(&mut self, dir: &Path, running: &BTreeSet<Running>) -> bool {
        let now = state::unix_now();
        let same = self.running.as_ref() == Some(running);
        if !self.ran && same && self.due.is_none_or(|due| now < due) {
            return false;
        }
        let Some(settings) = uploads::remembered() else {
            return false;
        };
        self.ran = false;
        self.running = Some(running.clone());
        let runs = |space: SpaceId, agent: &str| {
            running.contains(&Running {
                space,
                agent: agent.to_owned(),
            })
        };
        let beaten = beat(
            dir,
            settings.storage,
            (settings.speaks, settings.timeout),
            runs,
        );
        match beaten {
            Ok(beaten) => {
                self.due = beaten.due;
                beaten.lost
            }
            Err(_) => {
                self.due = Some(now + AGAIN);
                false
            }
        }
    }
}

/// A one-shot session's heartbeat: extends what is due of `agent`'s claims in the working
/// directory's space, by the state in `dir`, once it holds the lock, giving way to a helper that
/// starts meanwhile. What fails is tried at the next look.
pub fn beat_agent(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    agent: &Name,
) -> Result<(), String> {
    let Ok(space) = folders::working_space(dir) else {
        return Ok(());
    };
    let runs = |at: SpaceId, held_by: &str| at == space.id && held_by == agent.as_str();
    beat(dir, storage, settings, runs).map(drop)
}

/// As [`beat_agent`] in a one-shot `skald recv --wait`, which holds no lock between its runs: takes
/// the lock first, giving way to a helper that starts meanwhile.
pub fn beat_here(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    agent: &Name,
) {
    let taken = lock::take_unless(dir, false, settings.1, || crate::helper::running(dir));
    if let Ok(lock::Taken::Lock(Some(_lock))) = taken {
        let _ = beat_agent(dir, storage, settings, agent);
    }
}

/// Extends each claim the device holds by the state in `dir` whose agent, in the claim's space,
/// `runs` gives, once a third of its lease passed, as [`self`] says; marks lost those it finds
/// lost. A claim that does not open it leaves out.
pub fn beat(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    runs: impl Fn(SpaceId, &str) -> bool,
) -> Result<Beaten, String> {
    if !dir.join(state::FILE).is_file() {
        return Ok(Beaten::default());
    }
    let rows: Vec<TaskClaim> = (State::open(dir).map_err(text)?)
        .claims(None)
        .map_err(text)?
        .into_iter()
        .filter(|row| (row.agent.as_deref()).is_some_and(|agent| runs(row.space, agent)))
        .collect();
    if rows.is_empty() {
        return Ok(Beaten::default());
    }
    let Some(mut follower) = Follower::unchecked(dir, storage, speaks, timeout)? else {
        return Ok(Beaten::default());
    };
    let mut beaten = Beaten::default();
    for mut held in claims::opened_each(&follower, rows)? {
        if held.record.end.is_some() || held.record.lost.is_some() {
            continue;
        }
        let lease = held.record.lease;
        let third = (lease / 3).max(1);
        let now = state::unix_now();
        if let Some(why) = claims::lease_passed(&held) {
            lose(&follower, &mut held, why)?;
            beaten.lost = true;
            continue;
        }
        let due = held.row.since.saturating_add(third);
        if now < due {
            beaten.due = earliest(beaten.due, due);
            continue;
        }
        let progress = held.record.progress.unwrap_or(Progress::Started);
        let body = json!({ "progress": progress.name() });
        match claims::fence(&mut follower, &held.fencing(), "extend", body) {
            Fenced::Taken => {
                held.row.since = now;
                claims::keep(&follower, &mut held, None)?;
                beaten.due = earliest(beaten.due, now + third);
            }
            Fenced::Lost => {
                lose(&follower, &mut held, claims::NO_EXTEND.to_owned())?;
                beaten.lost = true;
            }
            Fenced::Refused(why) => {
                let why = format!("the server refused its extend: {why}");
                lose(&follower, &mut held, why)?;
                beaten.lost = true;
            }
            Fenced::Unanswered(_) => beaten.due = earliest(beaten.due, now + AGAIN),
        }
    }
    Ok(beaten)
}

/// The progress of the claims each agent holds, in its space, by `follower`'s state: none for
/// those the heartbeat found lost; `awaiting-approval` over `started` for an agent that holds
/// both. A claim that does not open counts as none.
pub fn progress_of_agents(follower: &Follower) -> BTreeMap<Running, Progress> {
    let rows: Vec<TaskClaim> = (follower.state.claims(None).unwrap_or_default().into_iter())
        .filter(|row| row.agent.is_some())
        .collect();
    let mut held = BTreeMap::new();
    if rows.is_empty() {
        return held;
    }
    for claim in claims::opened_each(follower, rows).unwrap_or_default() {
        if claim.record.lost.is_some() {
            continue;
        }
        let Some(agent) = claim.row.agent else {
            continue;
        };
        let progress = claim.record.progress.unwrap_or(Progress::Started);
        let running = Running {
            space: claim.row.space,
            agent,
        };
        let kept = held.entry(running).or_insert(progress);
        if progress == Progress::AwaitingApproval {
            *kept = progress;
        }
    }
    held
}

/// Marks `held` lost as `why` says, and keeps for its agent's next output a line that says so.
fn lose(follower: &Follower, held: &mut Held, why: String) -> Result<(), String> {
    let reference = claims::reference(&follower.state, held.row.task);
    let line = format!("skald: warning: {}", claims::lost(&reference, &why));
    held.record.lost = Some(why);
    claims::keep(follower, held, None)?;
    if let Some(agent) = &held.row.agent {
        (follower.state)
            .keep_notice(held.row.space, agent, &line)
            .map_err(text)?;
    }
    Ok(())
}
