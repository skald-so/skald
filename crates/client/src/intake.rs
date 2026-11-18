//! What the helper takes in for the `skald recv --wait` sessions that wait through it
//! ([`crate::wait`]): on each arrival it does the work once, not a run of `skald recv` per session.
//!
//! - A session's run of `skald recv --wait` leaves its follower, what it checks messages with and
//!   its link ([`leave`]); the helper takes in with them what comes next.
//! - Taking in catches up, syncs each space a `space-notice` names and brings its groups in line,
//!   decrypts the application messages, then checks each message once, keeping it for the person
//!   and the agents it is for, and takes out each status, as `skald recv` does ([`crate::recv`]).
//!   Nothing else on the way: the KeyPackages, the GroupInfo of the groups the device holds, the
//!   check that the device is not revoked and the server's name come with the session's first run,
//!   a run of another command or a roster's change.
//! - Each waiting session is then handed only its own: the person's what the person's `skald recv`
//!   shows; an agent's the messages kept for it, and, with no person's session waiting, the
//!   statuses and warnings of its space. A message is marked handed over once the session has
//!   written it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::SystemTime;

use skald_proto::{Name, SpaceId};

use crate::groups::{self, Link};
use crate::mailbox::{Transport, Watched};
use crate::output::{self, Stream};
use crate::recv::{self, Checks};
use crate::spaces::{Follower, Scope};
use crate::{key_chain, wraps};

/// What a run wrote to its outputs, in order.
pub type Said = Vec<(Stream, Vec<u8>)>;

/// When each step of an intake ended, by its name, for a session that traces it.
pub type Marks = Vec<(&'static str, SystemTime)>;

/// What takes in what comes: the follower of a session's run, what it checks messages with, and
/// its link.
pub struct Intake {
    follower: Follower,
    checks: Checks,
    link: Link,
}

thread_local! {
    /// What the last run of `skald recv --wait` on this thread left to take in with.
    static LEFT: RefCell<Option<Intake>> = const { RefCell::new(None) };
}

/// Leaves `intake` for the next to take in with.
pub fn leave(intake: Intake) {
    LEFT.with(|left| *left.borrow_mut() = Some(intake));
}

/// What was left to take in with, if anything.
pub fn take() -> Option<Intake> {
    LEFT.with(|left| left.borrow_mut().take())
}

/// The transport of the connection of what was left to take in with, if anything.
pub fn left_transport() -> Option<Transport> {
    LEFT.with(|left| {
        left.borrow()
            .as_ref()
            .map(|intake| intake.follower.transport)
    })
}

/// What watches for what comes with the session of what was left to take in with, if anything
/// ([`Watched::of`]).
pub fn left_watched() -> Option<Watched> {
    LEFT.with(|left| (left.borrow().as_ref()).and_then(|intake| Watched::of(&intake.follower)))
}

/// What taking in said: to the person, and to the agents of each space.
#[derive(Default)]
pub struct Told {
    pub person: Said,
    pub spaces: BTreeMap<SpaceId, Said>,
}

impl Intake {
    pub fn new(follower: Follower, checks: Checks, link: Link) -> Self {
        Intake {
            follower,
            checks,
            link,
        }
    }

    /// Takes in what came for the sessions of `scopes`: the person's, `None`, or the agents' of a
    /// space; `noticed`, the spaces of `space-notice`s that came, are synced too. After a retry,
    /// the connection tries WebSocket first again. Gives what it said, and
    /// for whom: to the person if the person's session is among them; otherwise, of each space, to
    /// its agents.
    pub fn take_in(
        &mut self,
        scopes: &BTreeSet<Option<SpaceId>>,
        noticed: &BTreeSet<SpaceId>,
        (retry, marks): (bool, &mut Marks),
    ) -> Result<Told, String> {
        if retry {
            self.follower.transport = Transport::WebSocket;
        }
        let person = scopes.contains(&None);
        let scopes: Vec<Scope> = if person {
            vec![Scope::All]
        } else {
            scopes
                .iter()
                .flatten()
                .map(|space| Scope::Only(*space))
                .collect()
        };
        let (taken, mut said) = captured(|| self.receive(&scopes, noticed.clone(), marks));
        taken?;
        let mut told = Told::default();
        for scope in scopes {
            let (shown, mut by_scope) =
                captured(|| recv::show(&mut self.follower, scope, &mut self.checks));
            shown?;
            marks.push(("shown", SystemTime::now()));
            match scope {
                Scope::All => said.append(&mut by_scope),
                Scope::Only(space) => {
                    told.spaces.insert(space, by_scope);
                }
            }
        }
        if person {
            told.person = said;
        }
        Ok(told)
    }

    /// Catches up, syncs the spaces of `due` and of each `space-notice` that came and brings their
    /// groups in line, and decrypts the application messages within `scopes`.
    fn receive(
        &mut self,
        scopes: &[Scope],
        mut due: BTreeSet<SpaceId>,
        marks: &mut Marks,
    ) -> Result<(), String> {
        let (follower, checks, link) = (&mut self.follower, &mut self.checks, &mut self.link);
        let timeout = follower.timeout;
        let mut mailbox = recv::catch_up_noting(follower, None, timeout, &mut due)?;
        marks.push(("caught up", SystemTime::now()));
        let roster_changed = !due.is_empty();
        while !due.is_empty() {
            recv::sync_due(follower, Scope::All, &mut due, &mut checks.synced)?;
            due.extend(mailbox.waiting_notices(follower, &link.keys));
        }
        drop(mailbox);
        marks.push(("synced", SystemTime::now()));
        checks.follow(follower)?;
        for scope in scopes {
            groups::take_in(follower, link, *scope, &mut checks.synced)?;
        }
        marks.push(("decrypted", SystemTime::now()));
        if roster_changed {
            groups::align_with(follower, link);
            key_chain::upload_missing(follower, link, Scope::All);
            wraps::upload_missing(follower, link, Scope::All);
        }
        Ok(())
    }

    /// Hands the session of `agent` in `space`, or the person's if `None`, what is its own: `said`,
    /// then each message kept for it, marked once written. Whether that is what the session waits
    /// for: as an agent, a message; as the person, anything.
    pub fn hand(&self, said: &Said, agent: Option<(SpaceId, &Name)>) -> Result<bool, String> {
        for (stream, bytes) in said {
            output::write(*stream, bytes)
                .map_err(|err| format!("cannot write to the output: {err}"))?;
        }
        Ok(match agent {
            Some((space, agent)) => recv::hand_over(&self.follower, space, agent)? > 0,
            None => recv::show_to_person(&self.follower)? > 0 || !said.is_empty(),
        })
    }
}

/// Runs `run` with its output kept rather than written; gives what it gave, and the output.
fn captured<R>(run: impl FnOnce() -> R) -> (R, Said) {
    let said = Rc::new(RefCell::new(Said::new()));
    let kept = Rc::clone(&said);
    let ran = output::into(
        move |stream, bytes| {
            kept.borrow_mut().push((stream, bytes.to_vec()));
            Ok(())
        },
        run,
    );
    let said = said.take();
    (ran, said)
}
