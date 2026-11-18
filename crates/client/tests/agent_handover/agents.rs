//! What the tests of handing messages over share: the talk of the tests of channel messages, with
//! Ada's agents in `~/work/acme`, bound to `acme`, and what a run as an agent shows of a message.

use std::path::{Path, PathBuf};

use skald_proto::{MessageId, RawEnvelope};

use crate::gaps::sends;
use crate::kit::RECV;
use crate::lines::assert_handed_line;
use crate::machine::{Finished, Machine};
use crate::talk::{Line, Talk, encrypted};
use crate::world::{ACME, ADD_RULE, AGENT, HOST, dir_of, path, run};

/// The agents of Ada's device the tests run.
pub const CLAUDE: &str = "claude-h7w2kq";
pub const CODEX: &str = "codex-m3t9xa";

/// The folder of Ada's machine where her agents run, bound to `acme`.
pub const ACME_DIR: &str = "work/acme";

/// The talk of the tests of channel messages, Ada's machine with `~/work/acme` bound to `acme`.
pub struct Agents {
    pub talk: Talk,
    pub folder: PathBuf,
}

impl Agents {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, in `acme`'s channel;
    /// `~/work/acme` bound to `acme`.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Self::bound(Talk::new(&ids, &logins, false))
    }

    /// `talk`, a talk in `acme`, with `~/work/acme` bound to `acme`.
    pub fn bound(talk: Talk) -> Self {
        let folder = dir_of(&talk.ada.machine, ACME_DIR);
        let bound = run(&talk.ada.machine, &[ADD_RULE, path(&folder), ACME]);
        assert!(bound.success, "not bound: {bound:?}");
        Agents { talk, folder }
    }

    /// Runs `skald --agent <agent> recv` in `~/work/acme`; fails unless it succeeds.
    #[track_caller]
    pub fn run_as(&self, agent: &str) -> Finished {
        run_as_in(&self.talk.ada.machine, &self.folder, agent)
    }

    /// The address of Ada's agent `agent` in `acme`.
    pub fn address(&self, agent: &str) -> String {
        format!("{ACME}/{}/{agent}@{HOST}", self.talk.ada.handle)
    }

    /// Bob's person address in `acme`.
    pub fn bob(&self) -> String {
        self.talk.address(&self.talk.played[0].person)
    }

    /// The `mls` envelope in which Bob's device says `line` in the channel; not sent.
    pub fn bob_says(&mut self, line: &Line) -> RawEnvelope {
        let at = self.talk.at();
        encrypted(&mut self.talk.played[0], at, at.1, line)
    }

    /// Bob's device sends `line` in the channel; fails unless the server stores it. Gives its
    /// `msg_id`.
    #[track_caller]
    pub fn bob_sends(&mut self, line: &Line) -> MessageId {
        let envelope = self.bob_says(line);
        sends(&self.talk, &envelope)
    }
}

/// Runs `skald --agent <agent> recv` on `machine` in `dir`; fails unless it succeeds.
#[track_caller]
pub fn run_as_in(machine: &Machine, dir: &Path, agent: &str) -> Finished {
    let finished = machine.start_in(dir, &[AGENT, agent, RECV]).finish();
    assert!(
        finished.success,
        "{agent} in {}: {finished:?}",
        dir.display()
    );
    finished
}

/// Fails unless `shown`, an agent's output, holds `text` once, on one line whose reference refers
/// to the `msg_id` `id`, and holds no UUID.
#[track_caller]
pub fn assert_handed_once(shown: &str, text: &str, id: MessageId) {
    assert_handed_line(shown, text, id);
}

/// Fails if `shown` holds `text`.
#[track_caller]
pub fn assert_not_handed(shown: &str, text: &str) {
    assert!(!shown.contains(text), "{text:?} handed over: {shown}");
}
