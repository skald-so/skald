//! What the tests share: Ada's machine alone in `clienta`'s groups, `~/work/clientA` bound to it,
//! the MCP sessions of her agents there, and what the front saw her machine send.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use serde_json::json;
use skald_proto::{ChannelId, MessageId, Role};

pub use crate::client::{
    CLIENT, EVERYONE, MESSAGE_FIELD, REPLY_TOOL, SEND_TOOL, TEXT_FIELD, TO_FIELD,
};
use crate::client::{Called, Session, session_in as session_of};
use crate::fixtures::{SEND, TO};
use crate::kit::{Change, bind_client_a, create_space, latest, received, revise};
use crate::lines::{SENT, agent_line_of, assert_no_uuid, assert_says_sent, refers_to, says_sent};
use crate::machine::{Finished, Machine};
use crate::senders::{assert_labelled, labelled_of};
use crate::talk::{BOB_HOST, Talk, messages_sent, one_message_sent};
use crate::world::{ADD_RULE, AGENT, CLIENT_A, CLIENT_B, Person, dir_of, path, run};

/// The agents of Ada's device, each in its folder under `~/work/clientA`.
pub const API: &str = "api";
pub const WEB: &str = "web";

/// The local state's database in a machine's state directory.
const STATE_FILE: &str = "state.sqlite3";

/// Ada's machine, alone in the groups of `clienta`, the talk's space, and `~/work/clientA` bound to
/// it.
pub struct Solo {
    pub talk: Talk,
    /// `~/work/clientA` on Ada's machine.
    pub folder: PathBuf,
}

impl Solo {
    /// Ada, GitHub user `id` with `login`; no other account is in `clienta`.
    pub fn new(id: u64, login: &str) -> Self {
        let talk = Talk::in_space(CLIENT_A, &[id], &[login], true);
        let folder = bind_client_a(&talk.ada.machine);
        Solo { talk, folder }
    }

    pub fn machine(&self) -> &Machine {
        &self.talk.ada.machine
    }

    /// `clienta`'s default channel.
    pub fn default_channel(&self) -> ChannelId {
        self.talk.space.default
    }

    /// The folder of Ada's agent `agent`, `~/work/clientA/<agent>`.
    pub fn dir(&self, agent: &str) -> PathBuf {
        dir_of(self.machine(), &format!("work/clientA/{agent}"))
    }

    /// Starts `skald --agent <agent> mcp` in the agent's folder, initialized as [`CLIENT`].
    pub fn session(&self, agent: &str) -> Session {
        session_in(self.machine(), &self.dir(agent), agent)
    }

    /// Ada's agent `agent` as `--to` and `to` take it: `owner/agent`.
    pub fn short(&self, agent: &str) -> String {
        format!("{}/{agent}", self.talk.ada.handle)
    }

    /// Ada's person address in `clienta`.
    pub fn person(&self) -> String {
        self.talk.address(&self.talk.ada)
    }

    /// How many `send` frames the front saw so far.
    pub fn mark(&self) -> usize {
        self.talk.front.sent().len()
    }

    /// The `msg_id`s of the messages the front saw sent in `clienta`'s default channel after
    /// `mark`, in order.
    pub fn sent_since(&self, mark: usize) -> Vec<MessageId> {
        (messages_sent(&self.talk.front, self.default_channel(), mark).iter())
            .map(|sent| sent.envelope.envelope().header.msg_id)
            .collect()
    }

    /// `session` calls the send tool `to` `to` with `text` and no channel; fails unless the result
    /// is no error, the front saw one message in `clienta`'s default channel after the call's
    /// start, and the result is `sent #<ref>` of it. Gives its `msg_id`.
    #[track_caller]
    pub fn sends(&self, session: &mut Session, to: &str, text: &str) -> MessageId {
        let mark = self.mark();
        let called = session.call(SEND_TOOL, json!({ TO_FIELD: to, TEXT_FIELD: text }));
        assert!(!called.error, "{to}: {called:?}: {}", session.errors());
        let msg_id = one_message_sent(&self.talk.front, self.default_channel(), mark);
        assert_says_sent(&called.text, msg_id);
        msg_id
    }

    /// Runs the person's `skald recv` on Ada's machine; fails unless it succeeds. Gives what it
    /// showed.
    #[track_caller]
    pub fn person_reads(&self) -> String {
        let read = received(self.machine());
        assert!(read.success, "{read:?}");
        read.shown()
    }

    /// Runs api's `skald send --to ada/web <text>` in its folder; fails unless it succeeds, the
    /// front saw one message in `clienta`'s default channel meanwhile, and it says `sent #<ref>`
    /// of it. Gives its `msg_id`.
    #[track_caller]
    pub fn api_sends_web(&self, text: &str) -> MessageId {
        let mark = self.mark();
        let args = [AGENT, API, SEND, TO, &self.short(WEB), text];
        let sent = self.machine().start_in(&self.dir(API), &args).finish();
        assert!(sent.success, "{sent:?}");
        let msg_id = one_message_sent(&self.talk.front, self.default_channel(), mark);
        assert_says_sent(&sent.stdout, msg_id);
        msg_id
    }

    /// Runs `skald <args>` as the person on Ada's machine in `~/work/clientA`.
    pub fn person_runs(&self, args: &[&str]) -> Finished {
        self.machine().start_in(&self.folder, args).finish()
    }

    /// Ada creates `clientb`, whose groups hold her device alone, and binds `~/work/clientB` to
    /// it. Gives the folder.
    pub fn with_client_b(&self) -> PathBuf {
        create_space(self.talk.cloud(), &self.talk.ada, CLIENT_B);
        received(self.machine());
        let folder = dir_of(self.machine(), "work/clientB");
        let bound = run(self.machine(), &[ADD_RULE, path(&folder), CLIENT_B]);
        assert!(bound.success, "not bound: {bound:?}");
        folder
    }

    /// Bob, GitHub user `id` with `login`, joins `clienta` and its default channel by a revision
    /// Ada's device signs, which Ada's machine accepts. As joining by an invite will, Bob's machine
    /// starts from that revision: the test writes `clienta` with it into his machine's local state.
    /// Ada's machine and Bob's take in their mailboxes, and Bob's `~/work/clientA` is bound to
    /// `clienta`. Gives Bob and that folder.
    pub fn with_bob(&self, id: u64, login: &str) -> (Person, PathBuf) {
        let cloud = self.talk.cloud();
        let bob = self.talk.world.sign_up(id, login, BOB_HOST);
        let previous = latest(cloud, &self.talk.ada.token, self.talk.space.id);
        let change = Change {
            members: vec![(&bob, Role::Member)],
            ..Change::default()
        };
        revise(cloud, &previous, &self.talk.ada, &change);
        received(self.machine());
        follows_as(&bob.machine, self.machine(), CLIENT_A);
        for machine in [&bob.machine, self.machine(), &bob.machine] {
            let taken = received(machine);
            assert!(taken.success, "{taken:?}");
        }
        let folder = dir_of(&bob.machine, "work/clientA");
        let bound = run(&bob.machine, &[ADD_RULE, path(&folder), CLIENT_A]);
        assert!(bound.success, "not bound: {bound:?}");
        (bob, folder)
    }
}

/// Writes into the local state of `machine` the space `name` with the latest revision `other`'s
/// machine accepted of it, not as its personal space.
fn follows_as(machine: &Machine, other: &Machine, name: &str) {
    let state = Connection::open(machine.state_dir().join(STATE_FILE)).expect("open a local state");
    let accepted = other.state_dir().join(STATE_FILE);
    state
        .execute(
            "ATTACH DATABASE ?1 AS other",
            params![accepted.to_string_lossy()],
        )
        .expect("open the other machine's local state");
    let written = state
        .execute(
            "INSERT INTO space (space, name, anchor, accepted, personal)
                 SELECT space, name, anchor, accepted, 0 FROM other.space WHERE name = ?1",
            params![name],
        )
        .expect("write the space");
    assert_eq!(
        written, 1,
        "{name} is not in the other machine's local state"
    );
    state
        .execute_batch("DETACH DATABASE other")
        .expect("close the other machine's local state");
}

/// Starts `skald --agent <agent> mcp` on `machine` in `dir`, initialized as [`CLIENT`].
pub fn session_in(machine: &Machine, dir: &Path, agent: &str) -> Session {
    session_of(machine, dir, &[AGENT, agent], CLIENT)
}

/// Fails unless `shown`, an agent's output, shows `text` once as the message `msg_id`, from
/// `sender`'s `(from, device, level)`: `from` by the device named `device` at `level`. A reply's
/// line names the message it answers after its reference: `#<ref> (reply to #<ref>): <text>`.
#[track_caller]
pub fn assert_got(shown: &str, text: &str, msg_id: MessageId, sender: (&str, &str, &str)) {
    let got = labelled_of(shown, text, text);
    let reference = got.reference.split(' ').next().unwrap_or_default();
    assert!(
        refers_to(reference, msg_id),
        "#{reference} does not refer to {msg_id}: {shown}"
    );
    assert_no_uuid(shown);
    assert_labelled(shown, text, sender);
}

/// Fails if `shown`, an agent's output or the person's, holds `text`.
#[track_caller]
pub fn assert_not_shown(shown: &str, text: &str, who: &str) {
    assert!(!shown.contains(text), "{who} shows {text:?}: {shown}");
}

/// Fails unless `called`, a send tool call, sent nothing by what its result says: no line of it is
/// `sent #<ref>`, and it says something.
#[track_caller]
pub fn assert_not_sent(called: &Called, what: &str) {
    assert!(!called.text.trim().is_empty(), "{what}: says nothing");
    assert!(!says_sent(&called.text), "{what}: says sent: {called:?}");
}

/// `session` replies with the reply tool to the message whose line in `shown`, its `inbox`, holds
/// `asked`, by `#<ref>`, with `text`; fails unless the result is no error and says `sent #`.
#[track_caller]
pub fn replies(session: &mut Session, shown: &str, asked: &str, text: &str) -> Called {
    let reference = format!("#{}", agent_line_of(shown, asked).reference);
    let called = session.call(
        REPLY_TOOL,
        json!({ MESSAGE_FIELD: reference, TEXT_FIELD: text }),
    );
    assert!(!called.error, "{called:?}: {}", session.errors());
    assert!(
        called.text.trim().starts_with(&format!("{SENT} #")),
        "not sent: {called:?}"
    );
    called
}
