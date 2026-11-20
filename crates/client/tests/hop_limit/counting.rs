//! How an agent's send counts its `hops`: one more than the highest `hops` of the messages handed to
//! the agent since the last message the device sent as it, or 0 when none was; a reply one more
//! than the original's; over 10, nothing is sent.

use serde_json::json;
use skald_proto::{ChannelId, MessageId};

use crate::agents::{Agents, CLAUDE};
use crate::client::{CHANNEL_FIELD, CLIENT, SEND_TOOL, Session, TEXT_FIELD, TO_FIELD, session_in};
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::lines::says_sent;
use crate::loops::{Said, assert_person_warned_once, assert_stopped, person_warned, reference_of};
use crate::machine::Finished;
use crate::replying::{
    BOB_CLAUDE, REPLY, Writes, agent_of, assert_bob_never_reads, bob_reads, bob_writes, handles,
    run_in, the_one,
};
use crate::world::{ACME, AGENT};

/// Ada's machine with claude in `~/work/acme`, and Bob's played device, whose agent claude sends
/// to and which reads each message's `hops`.
struct Counting {
    agents: Agents,
    /// Bob's claude in `acme`, in full and as `--to` takes it.
    bob_claude: String,
    to_bob: String,
    /// Ada's claude in `acme`, in full.
    ada_claude: String,
    /// The counter of Bob's last message in the channel.
    bob_said: u64,
}

impl Counting {
    fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let agents = Agents::new(ids, logins);
        agents.run_as(CLAUDE);
        let (bob, ada) = handles(&agents.talk);
        Counting {
            bob_claude: agent_of(&bob, ACME, BOB_CLAUDE),
            to_bob: format!("{bob}/{BOB_CLAUDE}"),
            ada_claude: agent_of(&ada, ACME, CLAUDE),
            agents,
            bob_said: 0,
        }
    }

    fn channel(&self) -> ChannelId {
        self.agents.talk.channel
    }

    /// Bob's device writes `text` in the channel as Bob's claude with `hops`, to Ada's claude if
    /// `to_claude`, to everyone otherwise. Gives its `msg_id`.
    #[track_caller]
    fn bob_writes(&mut self, text: &'static str, hops: u64, to_claude: bool) -> MessageId {
        self.bob_said += 1;
        let to = to_claude.then_some(self.ada_claude.as_str());
        let writes = Writes::new(&self.bob_claude, to, self.bob_said, text).hops(hops);
        let at = self.agents.talk.at();
        bob_writes(&mut self.agents.talk, at, &writes)
    }

    /// Runs claude's `skald recv`; gives what it showed.
    #[track_caller]
    fn claude_reads(&self) -> String {
        self.agents.run_as(CLAUDE).stdout
    }

    /// Runs claude's `skald send --to <Bob's claude> --channel <the channel> <text>`.
    fn claude_sends(&self, text: &str) -> Finished {
        let channel = self.channel().to_string();
        let args = [TO, self.to_bob.as_str(), CHANNEL, &channel, text];
        run_in(
            &self.agents.talk.ada.machine,
            (&self.agents.folder, Some(CLAUDE)),
            SEND,
            &args,
        )
    }

    /// The `hops` of the one message Bob's device read in the channel with `text`.
    #[track_caller]
    fn hops_of(&mut self, text: &str) -> u64 {
        let channel = self.channel();
        let read = bob_reads(&mut self.agents.talk, channel);
        the_one(&read, text).1.hops
    }

    /// As [`Counting::claude_sends`]; fails unless it is sent. Gives the `hops` Bob's device reads
    /// of it.
    #[track_caller]
    fn claude_sends_hops(&mut self, text: &str) -> u64 {
        let sent = self.claude_sends(text);
        assert!(
            sent.success && says_sent(&sent.stdout),
            "{text:?}: {sent:?}"
        );
        self.hops_of(text)
    }

    /// Starts claude's MCP session in `~/work/acme`.
    fn session(&self) -> Session {
        let machine = &self.agents.talk.ada.machine;
        session_in(machine, &self.agents.folder, &[AGENT, CLAUDE], CLIENT)
    }

    /// `session` calls the send tool `to` Bob's claude in the channel with `text`; fails unless it
    /// is sent. Gives the `hops` Bob's device reads of it.
    #[track_caller]
    fn tool_sends_hops(&mut self, session: &mut Session, text: &str) -> u64 {
        let arguments = json!({
            TO_FIELD: self.to_bob,
            CHANNEL_FIELD: self.channel().to_string(),
            TEXT_FIELD: text,
        });
        let called = session.call(SEND_TOOL, arguments);
        assert!(
            !called.error && says_sent(&called.text),
            "{text:?}: {called:?}"
        );
        self.hops_of(text)
    }
}

/// What claude sends; none is anywhere else.
const FIRST: &str = "hop limit test: claude's first message, with nothing handed to it";
const AFTER_THREE: &str = "hop limit test: claude answers after a message with 3 hops";
const NEXT: &str = "hop limit test: claude's next message, with nothing new handed to it";
const AFTER_MANY: &str = "hop limit test: claude answers after messages with 2, 7 and 5 hops";
const NOT_TAKEN: &str = "hop limit test: claude sends before it takes the message with 6 hops";
const AFTER_SIX: &str = "hop limit test: claude answers after taking the message with 6 hops";

/// What Bob's claude says; none is anywhere else.
const THREE: &str = "hop limit test: bob's claude says something with 3 hops";
const TWO: &str = "hop limit test: bob's claude says something with 2 hops";
const SEVEN: &str = "hop limit test: bob's claude tells everyone something with 7 hops";
const FIVE: &str = "hop limit test: bob's claude says something with 5 hops";
const SIX: &str = "hop limit test: bob's claude says something with 6 hops";

/// claude's `skald send --to` Bob's claude, as Bob's played device reads the `hops` of each:
/// - with nothing handed to claude, 0;
/// - after claude's `skald recv` takes a message to it with `hops` 3, 4; the next send, 0;
/// - after it takes messages with `hops` 2 to it, 7 to everyone and 5 to it, 8;
/// - before it takes a message to it with `hops` 6, 0; after, 7.
#[test]
fn skald_send_carries_one_more_than_the_highest_hops_handed_over_since_the_agent_s_last_message() {
    let mut counting = Counting::new(
        [4_811_005_001, 4_811_005_002],
        ["ada-counts-the-hops", "bob-sends-claude-hops"],
    );

    assert_eq!(counting.claude_sends_hops(FIRST), 0, "{FIRST:?}");
    counting.bob_writes(THREE, 3, true);
    assert!(
        counting.claude_reads().contains(THREE),
        "{THREE:?} not handed over"
    );
    assert_eq!(
        counting.claude_sends_hops(AFTER_THREE),
        4,
        "{AFTER_THREE:?}"
    );
    assert_eq!(counting.claude_sends_hops(NEXT), 0, "{NEXT:?}");
    counting.bob_writes(TWO, 2, true);
    counting.bob_writes(SEVEN, 7, false);
    counting.bob_writes(FIVE, 5, true);
    let read = counting.claude_reads();
    for text in [TWO, SEVEN, FIVE] {
        assert!(read.contains(text), "{text:?} not handed over: {read}");
    }
    assert_eq!(counting.claude_sends_hops(AFTER_MANY), 8, "{AFTER_MANY:?}");
    counting.bob_writes(SIX, 6, true);
    assert_eq!(counting.claude_sends_hops(NOT_TAKEN), 0, "{NOT_TAKEN:?}");
    assert!(
        counting.claude_reads().contains(SIX),
        "{SIX:?} not handed over"
    );
    assert_eq!(counting.claude_sends_hops(AFTER_SIX), 7, "{AFTER_SIX:?}");
}

/// What claude sends with the send tool; none is anywhere else.
const TOOL_FIRST: &str = "hop limit test: the tool's first message, with nothing handed over";
const TOOL_AFTER: &str = "hop limit test: the tool answers after a message with 3 hops";
const TOOL_NEXT: &str = "hop limit test: the tool's next message, with nothing new handed over";
const TOOL_THREE: &str = "hop limit test: bob's claude says something with 3 hops to the session";

/// claude's MCP session sends `to` Bob's claude with the send tool: with nothing handed to it,
/// `hops` 0; after its `inbox` returns a message to it with `hops` 3, 4; the next send, 0.
#[test]
fn send_tool_carries_one_more_than_the_highest_hops_handed_over_since_the_agent_s_last_message() {
    let mut counting = Counting::new(
        [4_811_006_001, 4_811_006_002],
        ["ada-session-counts-hops", "bob-sends-the-session-hops"],
    );
    let mut session = counting.session();

    assert_eq!(counting.tool_sends_hops(&mut session, TOOL_FIRST), 0);
    counting.bob_writes(TOOL_THREE, 3, true);
    let got = session.inbox();
    assert!(
        got.contains(TOOL_THREE),
        "{TOOL_THREE:?} not handed over: {got}"
    );
    assert_eq!(counting.tool_sends_hops(&mut session, TOOL_AFTER), 4);
    assert_eq!(counting.tool_sends_hops(&mut session, TOOL_NEXT), 0);
}

/// What Bob's claude says, and what claude answers; none is anywhere else.
const NINE: &str = "hop limit test: bob's claude asks with 9 hops";
const AT_TEN: &str = "hop limit test: claude's reply at hop 10";
const TEN: &str = "hop limit test: bob's claude asks again with 10 hops";
const OVER_REPLY: &str = "hop limit test: claude's reply that would carry 11 hops";
const OVER_SEND: &str = "hop limit test: claude's send that would carry 11 hops";

/// Bob's claude says something to claude with `hops` 9; claude's `skald reply` to it is sent with
/// `hops` 10. Ada's person's run warns of nothing. Bob's claude says something to claude with
/// `hops` 10; claude's `skald reply` to it fails, says why, and is not sent. Ada's person's next
/// run warns once, with the channel and Bob's claude, and the run after it does not. claude's
/// `skald send --to` Bob's claude then fails too, says why, and is not sent.
#[test]
fn reply_or_send_after_a_message_with_ten_hops_sends_nothing() {
    let mut counting = Counting::new(
        [4_811_007_001, 4_811_007_002],
        ["ada-agent-hits-the-limit", "bob-pushes-to-ten-hops"],
    );
    counting.bob_writes(NINE, 9, true);
    let read = counting.claude_reads();
    let replied = claude_replies(&counting, &read, NINE, AT_TEN);
    assert!(replied.ok && says_sent(&replied.shown), "{replied:?}");
    assert_eq!(counting.hops_of(AT_TEN), 10, "{AT_TEN:?}");
    let (machine, channel) = (&counting.agents.talk.ada.machine, counting.channel());
    assert_eq!(
        person_warned(machine, channel, &counting.to_bob),
        0,
        "warned too soon"
    );
    counting.bob_writes(TEN, 10, true);
    let read = counting.claude_reads();

    let refused = claude_replies(&counting, &read, TEN, OVER_REPLY);

    assert_stopped(&refused, "the reply at hop 11");
    let machine = &counting.agents.talk.ada.machine;
    assert_person_warned_once(machine, channel, &counting.to_bob);

    let refused: Said = counting.claude_sends(OVER_SEND).into();

    assert_stopped(&refused, "the send at hop 11");
    let talk = &mut counting.agents.talk;
    let channels = [channel, talk.space.default];
    assert_bob_never_reads(talk, &channels, &[OVER_REPLY, OVER_SEND]);
}

/// Runs claude's `skald reply #<ref> <text>` to the message whose line in `read`, its output,
/// holds `asked`.
fn claude_replies(counting: &Counting, read: &str, asked: &str, text: &str) -> Said {
    let reference = format!("#{}", reference_of(read, asked));
    let machine = &counting.agents.talk.ada.machine;
    run_in(
        machine,
        (&counting.agents.folder, Some(CLAUDE)),
        REPLY,
        &[&reference, text],
    )
    .into()
}
