//! What the tests share: A, the agent of Ada's second machine, and B, claude on her first machine,
//! answering each other in the channel, by the reply tool, the send tool or `skald send`; the
//! `hops` Bob's played device reads of each message; what a refusal of the limit says, and the
//! person's warning of it.

use serde_json::json;
use skald_proto::{ChannelId, Content, MessageId};

use crate::agents::CLAUDE;
use crate::client::{
    CHANNEL_FIELD, CLIENT, MESSAGE_FIELD, REPLY_TOOL, SEND_TOOL, Session, TEXT_FIELD, TO_FIELD,
    session_in,
};
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::kit::received;
use crate::lines::says_sent;
use crate::machine::{Finished, Machine};
use crate::receipts::Roles;
use crate::replying::{assert_bob_never_reads, bob_reads, the_one};
use crate::senders::{LAPTOP_AGENT, holds_word, labelled_of};
use crate::world::AGENT;

/// The highest `hops` an agent's message carries.
pub const LIMIT: u64 = 10;

/// What a refusal of the limit names: the limit, and that a person's message starts the count
/// again.
pub const PERSON: &str = "person";

/// The word of the person's warning.
pub const WARNING_WORD: &str = "warning";

/// How the agents answer each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// The MCP reply tool, after a first message by the send tool.
    ReplyTool,
    /// The MCP send tool, `to` the other.
    SendTool,
    /// `skald --agent <agent> send --to <the other>`, after the agent's `skald recv`.
    Cli,
}

/// One of the two agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The agent of Ada's second machine.
    A,
    /// claude, on Ada's first machine.
    B,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }

    fn index(self) -> usize {
        match self {
            Side::A => 0,
            Side::B => 1,
        }
    }
}

/// What an agent's send or reply gave: whether the command succeeded or the tool's result is no
/// error, and its output or result.
#[derive(Debug)]
pub struct Said {
    pub ok: bool,
    pub shown: String,
}

impl From<Finished> for Said {
    fn from(finished: Finished) -> Self {
        Said {
            ok: finished.success,
            shown: finished.shown(),
        }
    }
}

/// The roles of the tests of `read` receipts, with A's and B's MCP sessions for the tools.
pub struct Chain {
    pub roles: Roles,
    pub way: Way,
    /// A's session, then B's, for the tools.
    sessions: Vec<Session>,
}

impl Chain {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, as [`Roles::new`] sets
    /// them up; A's agent has run once on the second machine, so the messages for it are kept for
    /// it, and, for the tools, A's and B's sessions run.
    #[track_caller]
    pub fn new(way: Way, ids: [u64; 2], logins: [&str; 2]) -> Self {
        let roles = Roles::new(ids, logins);
        roles.a_agent_reads(&[]);
        let mut chain = Chain {
            roles,
            way,
            sessions: Vec::new(),
        };
        if way != Way::Cli {
            let a = session_in(
                chain.machine(Side::A),
                &chain.folder(Side::A),
                &chain.args(Side::A),
                CLIENT,
            );
            let b = session_in(
                chain.machine(Side::B),
                &chain.folder(Side::B),
                &chain.args(Side::B),
                CLIENT,
            );
            chain.sessions = vec![a, b];
        }
        chain
    }

    /// `side`'s machine.
    pub fn machine(&self, side: Side) -> &Machine {
        match side {
            Side::A => self.roles.a(),
            Side::B => self.roles.b(),
        }
    }

    /// `side`'s folder, `~/work/acme` on its machine.
    pub fn folder(&self, side: Side) -> std::path::PathBuf {
        match side {
            Side::A => self.roles.senders.laptop_folder.clone(),
            Side::B => self.roles.senders.agents.folder.clone(),
        }
    }

    /// `side`'s agent's name.
    pub fn agent(side: Side) -> &'static str {
        match side {
            Side::A => LAPTOP_AGENT,
            Side::B => CLAUDE,
        }
    }

    fn args(&self, side: Side) -> [&'static str; 2] {
        [AGENT, Self::agent(side)]
    }

    /// `side`'s agent as `--to` and `to` take it: `<ada>/<agent>`.
    pub fn short(&self, side: Side) -> String {
        format!("{}/{}", self.roles.senders.ada(), Self::agent(side))
    }

    /// The channel.
    pub fn channel(&self) -> ChannelId {
        self.roles.channel()
    }

    /// `side`'s agent takes what is handed over to it: `inbox` for the tools, its `skald recv` for
    /// the CLI. Gives what it showed.
    #[track_caller]
    pub fn take(&mut self, side: Side) -> String {
        match self.way {
            Way::Cli => {
                let args = [AGENT, Self::agent(side), crate::kit::RECV];
                let read = self
                    .machine(side)
                    .start_in(&self.folder(side), &args)
                    .finish();
                assert!(read.success, "{side:?}'s agent: {read:?}");
                read.stdout
            }
            _ => self.sessions[side.index()].inbox(),
        }
    }

    /// `side`'s agent sends `text` to the other's in the channel: with the send tool for the
    /// tools, with `skald send --to` for the CLI.
    pub fn send(&mut self, side: Side, text: &str) -> Said {
        let to = self.short(side.other());
        let channel = self.channel().to_string();
        match self.way {
            Way::Cli => {
                let args = [
                    AGENT,
                    Self::agent(side),
                    SEND,
                    TO,
                    &to,
                    CHANNEL,
                    &channel,
                    text,
                ];
                self.machine(side)
                    .start_in(&self.folder(side), &args)
                    .finish()
                    .into()
            }
            _ => {
                let arguments = json!({ TO_FIELD: to, CHANNEL_FIELD: channel, TEXT_FIELD: text });
                let session = &mut self.sessions[side.index()];
                let called = session.call(SEND_TOOL, arguments);
                Said {
                    ok: !called.error,
                    shown: called.text,
                }
            }
        }
    }

    /// `side`'s agent answers the message whose line in `taken`, what it took, holds `answering`,
    /// with `text`: by the reply tool for [`Way::ReplyTool`], by a send to the other otherwise.
    pub fn answer(&mut self, side: Side, (taken, answering): (&str, &str), text: &str) -> Said {
        match self.way {
            Way::ReplyTool => {
                let message = format!("#{}", reference_of(taken, answering));
                let session = &mut self.sessions[side.index()];
                let called = session.call(
                    REPLY_TOOL,
                    json!({ MESSAGE_FIELD: message, TEXT_FIELD: text }),
                );
                Said {
                    ok: !called.error,
                    shown: called.text,
                }
            }
            _ => self.send(side, text),
        }
    }

    /// B sends `texts[0]` to A; then A and B, in turn, each take the other's last message and
    /// answer it with the next text, up to `texts[LIMIT]`, the last one B sends. Fails unless each
    /// is sent. Gives what A took last, which shows `texts[LIMIT]`.
    #[track_caller]
    pub fn answer_each_other(&mut self, texts: &[String]) -> String {
        assert_eq!(texts.len() as u64, LIMIT + 1, "one text per hop");
        let first = self.send(Side::B, &texts[0]);
        assert_sent(&first, &texts[0]);
        let mut side = Side::B;
        for pair in texts.windows(2) {
            side = side.other();
            let taken = self.take(side);
            let said = self.answer(side, (&taken, &pair[0]), &pair[1]);
            assert_sent(&said, &pair[1]);
        }
        assert_eq!(side, Side::B, "B says the last text");
        let last = self.take(Side::A);
        let text = &texts[texts.len() - 1];
        assert!(last.contains(text), "A got no {text:?}: {last}");
        last
    }

    /// Bob's played device takes in its mailbox; gives each content it read in the channel so far,
    /// with its `msg_id`.
    pub fn c_reads(&mut self) -> Vec<(MessageId, Content)> {
        let channel = self.channel();
        bob_reads(&mut self.roles.senders.agents.talk, channel)
    }

    /// Fails unless Bob's played device read each of `texts` once in the channel, the `n`th with
    /// `hops` `n`.
    #[track_caller]
    pub fn assert_hops_rise(&mut self, texts: &[String]) {
        let read = self.c_reads();
        for (hops, text) in texts.iter().enumerate() {
            let (_, content) = the_one(&read, text);
            assert_eq!(content.hops, hops as u64, "hops of {text:?}: {content:?}");
        }
    }

    /// Fails if Bob's played device reads one of `texts` in the channel or the default one.
    #[track_caller]
    pub fn assert_never_sent(&mut self, texts: &[&str]) {
        let talk = &mut self.roles.senders.agents.talk;
        let channels = [talk.channel, talk.space.default];
        assert_bob_never_reads(talk, &channels, texts);
    }
}

/// The texts of a chain of `way`, one per hop from 0 to [`LIMIT`] and one past it; none is a part
/// of another or anywhere else.
pub fn texts(way: Way) -> Vec<String> {
    (0..=LIMIT + 1)
        .map(|hop| format!("hop limit test ({way:?}): the loop's message at hop {hop:02}"))
        .collect()
}

/// The reference of the message whose line in `shown`, an agent's output, has the text `text`,
/// without its `#`; a reply's line names the message it answers after it.
#[track_caller]
pub fn reference_of(shown: &str, text: &str) -> String {
    let got = labelled_of(shown, text, text);
    got.reference
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Fails unless `said` succeeded and says `sent #<ref>`.
#[track_caller]
pub fn assert_sent(said: &Said, text: &str) {
    assert!(
        said.ok && says_sent(&said.shown),
        "{text:?} not sent: {said:?}"
    );
}

/// Fails unless `said` is a refusal of the limit: it failed, says no `sent #<ref>`, and holds the
/// number 10 and the word `person`.
#[track_caller]
pub fn assert_stopped(said: &Said, what: &str) {
    assert!(!said.ok, "{what}: not refused: {said:?}");
    assert!(!says_sent(&said.shown), "{what}: says sent: {said:?}");
    let limit = LIMIT.to_string();
    let holds = |word: &str| said.shown.lines().any(|line| holds_word(line, word));
    assert!(
        holds(&limit),
        "{what}: names not the limit {limit}: {said:?}"
    );
    assert!(
        holds(PERSON),
        "{what}: names no {PERSON}'s message: {said:?}"
    );
}

/// The lines of `shown`, the person's output, that warn of a refusal of the limit in `channel` to
/// `addressee`: they hold the word [`WARNING_WORD`], the channel's ID and `addressee`.
pub fn limit_warnings(shown: &str, channel: ChannelId, addressee: &str) -> Vec<String> {
    let channel = channel.to_string();
    (shown.lines())
        .filter(|line| holds_word(line, WARNING_WORD))
        .filter(|line| line.contains(&channel) && line.contains(addressee))
        .map(str::to_owned)
        .collect()
}

/// Runs the person's `skald recv` on `machine`; gives how many of its lines warn of a refusal of
/// the limit in `channel` to `addressee`.
#[track_caller]
pub fn person_warned(machine: &Machine, channel: ChannelId, addressee: &str) -> usize {
    let shown = received(machine).shown();
    limit_warnings(&shown, channel, addressee).len()
}

/// Fails unless the person's next run on `machine` warns once of a refusal of the limit in
/// `channel` to `addressee`, and the run after it does not.
#[track_caller]
pub fn assert_person_warned_once(machine: &Machine, channel: ChannelId, addressee: &str) {
    let shown = received(machine).shown();
    let warned = limit_warnings(&shown, channel, addressee);
    assert_eq!(
        warned.len(),
        1,
        "not warned once of {addressee} in {channel}: {shown}"
    );
    let next = received(machine).shown();
    let again = limit_warnings(&next, channel, addressee);
    assert!(again.is_empty(), "warned again: {next}");
}
