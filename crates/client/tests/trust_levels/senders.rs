//! What the tests share: Ada's agents in `~/work/acme` with Bob's played device and Ada's second
//! machine as senders, and what an agent's output shows of each message's sender and level.

use std::path::PathBuf;

use skald_proto::MessageId;

use crate::agents::{ACME_DIR, Agents, CLAUDE};
use crate::devices::second_machine;
use crate::fixtures::{SEND, strs};
use crate::kit::{RECV, received};
use crate::lines::{JSON_FLAG, json_objects};
use crate::machine::Finished;
use crate::talk::Line;
use crate::world::{ACME, ADD_RULE, AGENT, HOST, Person, dir_of, path, run};

/// The levels as the CLI shows them.
pub const OWN: &str = "own";
pub const PEER: &str = "peer";

/// The word of the warning line before `peer` and `service` lines.
pub const DATA: &str = "data";

/// The host name of Ada's second machine, its certificate's `name`.
pub const LAPTOP: &str = "ada-laptop";

/// The agent of Ada's second machine that sends, in its `~/work/acme`, and Bob's agent whose
/// address Bob's device sends as.
pub const LAPTOP_AGENT: &str = "codex-v5d8tm";
pub const BOB_AGENT: &str = "claude-q3x6np";

/// The flag of `skald recv` that waits for what is to come.
pub const WAIT_FLAG: &str = "--wait";

/// An agent's message line with its sender's level: `[<from> · <device> · <level>] #<ref>: <text>`,
/// with ` (after missing)` after the reference when it follows missing ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labelled {
    pub from: String,
    /// The sending device's certificate `name`, as the line shows it.
    pub device: String,
    pub level: String,
    /// The reference, without its `#`.
    pub reference: String,
    pub text: String,
}

/// `line` read as a message line with a level, if it is one: the brackets end at the line's first
/// `]`, and hold three parts between ` · `.
pub fn labelled(line: &str) -> Option<Labelled> {
    let (who, rest) = line.strip_prefix('[')?.split_once(']')?;
    let parts: Vec<&str> = who.split(" · ").collect();
    let [from, device, level] = parts.as_slice() else {
        return None;
    };
    let (reference, text) = rest.strip_prefix(" #")?.split_once(": ")?;
    let reference = reference
        .strip_suffix(" (after missing)")
        .unwrap_or(reference);
    Some(Labelled {
        from: (*from).to_owned(),
        device: (*device).to_owned(),
        level: (*level).to_owned(),
        reference: reference.to_owned(),
        text: text.to_owned(),
    })
}

/// The message lines of `shown` with a level, each with its line's index.
pub fn labelled_lines(shown: &str) -> Vec<(usize, Labelled)> {
    (shown.lines().enumerate())
        .filter_map(|(at, line)| labelled(line).map(|got| (at, got)))
        .collect()
}

/// The one line of `shown`, an agent's output, that holds `needle`, read as a message line with a
/// level whose text is `text`; fails unless there is one and no other line holds `needle`.
#[track_caller]
pub fn labelled_of(shown: &str, needle: &str, text: &str) -> Labelled {
    let holding: Vec<&str> = shown.lines().filter(|line| line.contains(needle)).collect();
    let [line] = holding.as_slice() else {
        panic!("not one line holds {needle:?}: {shown}");
    };
    match labelled(line) {
        Some(got) if got.text == text => got,
        _ => panic!("{line:?} is not [<from> · <device> · <level>] #<ref>: {text:?}"),
    }
}

/// As [`labelled_of`], the line of `text`; fails unless it is from `from`, by the device shown as
/// `device`, at `level`.
#[track_caller]
pub fn assert_labelled(shown: &str, text: &str, (from, device, level): (&str, &str, &str)) {
    let got = labelled_of(shown, text, text);
    assert_eq!(
        (got.from.as_str(), got.device.as_str(), got.level.as_str()),
        (from, device, level),
        "{got:?} in {shown}"
    );
}

/// Whether `line` holds `word` as a word, in any case.
pub fn holds_word(line: &str, word: &str) -> bool {
    (line.split(|c: char| !c.is_alphanumeric())).any(|one| one.eq_ignore_ascii_case(word))
}

/// The indexes of the warning lines of `shown`, an agent's output: lines that are no message and
/// hold the word [`DATA`].
pub fn data_warnings(shown: &str) -> Vec<usize> {
    (shown.lines().enumerate())
        .filter(|(_, line)| !line.starts_with('[') && holds_word(line, DATA))
        .map(|(at, _)| at)
        .collect()
}

/// Fails unless `read`, an agent's run, shows one warning line, on its standard output before each
/// of its message lines at a level other than `own`, and has such a line.
#[track_caller]
pub fn assert_warned_before_others(read: &Finished) {
    let shown = read.shown();
    assert_eq!(
        data_warnings(&shown).len(),
        1,
        "not one warning line: {shown}"
    );
    let warnings = data_warnings(&read.stdout);
    let [warning] = warnings.as_slice() else {
        panic!("no warning line on the standard output: {shown}");
    };
    let others: Vec<usize> = (labelled_lines(&read.stdout).into_iter())
        .filter(|(_, got)| got.level != OWN)
        .map(|(at, _)| at)
        .collect();
    assert!(!others.is_empty(), "no line of another level: {shown}");
    assert!(
        others.iter().all(|at| at > warning),
        "a line of another level before the warning line: {shown}"
    );
}

/// Fails if `shown`, an agent's output, has a warning line.
#[track_caller]
pub fn assert_unwarned(shown: &str) {
    assert_eq!(data_warnings(shown), Vec::<usize>::new(), "{shown}");
}

/// Fails if a message line of `shown` is at the level `own`.
#[track_caller]
pub fn assert_none_own(shown: &str) {
    let own: Vec<Labelled> = (labelled_lines(shown).into_iter())
        .map(|(_, got)| got)
        .filter(|got| got.level == OWN)
        .collect();
    assert!(own.is_empty(), "{own:?} in {shown}");
}

/// Ada's agents, with Bob's played device and Ada's second machine, linked by hand, in the
/// channel; the second machine's `~/work/acme` is bound to `acme`.
pub struct Senders {
    pub agents: Agents,
    pub laptop: Person,
    pub laptop_folder: PathBuf,
    /// The counter of Bob's last line in the channel.
    bob_said: u64,
}

impl Senders {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones. Ada's device has run as
    /// claude in `~/work/acme`; her second machine has joined the channels, and Bob's device has
    /// taken in the Commits.
    #[track_caller]
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let mut agents = Agents::new(ids, logins);
        agents.run_as(CLAUDE);
        let laptop = second_machine(&agents.talk.world, &agents.talk.ada, LAPTOP);
        received(&laptop.machine);
        agents.talk.played[0].take_in(&agents.talk.world.cloud);
        let laptop_folder = dir_of(&laptop.machine, ACME_DIR);
        let bound = run(&laptop.machine, &[ADD_RULE, path(&laptop_folder), ACME]);
        assert!(bound.success, "not bound: {bound:?}");
        Senders {
            agents,
            laptop,
            laptop_folder,
            bob_said: 0,
        }
    }

    /// Ada's handle.
    pub fn ada(&self) -> String {
        self.agents.talk.ada.handle.to_string()
    }

    /// Bob's handle.
    pub fn bob(&self) -> String {
        self.agents.talk.played[0].person.handle.to_string()
    }

    /// The full address of Bob's agent [`BOB_AGENT`] in `acme`.
    pub fn bob_agent(&self) -> String {
        format!("{ACME}/{}/{BOB_AGENT}@{HOST}", self.bob())
    }

    /// Bob's device says `text` in the channel as `from`; fails unless the server stores it. Gives
    /// its `msg_id`.
    #[track_caller]
    pub fn bob_says_as(&mut self, from: &str, text: &str) -> MessageId {
        self.bob_said += 1;
        self.agents.bob_sends(&Line::new(from, self.bob_said, text))
    }

    /// Runs `skald --agent <agent> send --all --channel <channel> <text>` to the channel on Ada's
    /// second machine in its `~/work/acme`; fails unless it succeeds.
    #[track_caller]
    pub fn laptop_agent_says(&self, agent: &str, text: &str) {
        let channel = self.agents.talk.channel.to_string();
        let args = [
            AGENT,
            agent,
            SEND,
            "--all",
            "--channel",
            channel.as_str(),
            text,
        ];
        let sent = (self.laptop.machine)
            .start_in(&self.laptop_folder, &args)
            .finish();
        assert!(sent.success, "{agent} on the laptop: {sent:?}");
    }

    /// Runs `skald send --json --all --channel <channel> <text>` to the channel on Ada's second
    /// machine, as the person; fails unless it succeeds. Gives the message's `msg_id`.
    #[track_caller]
    pub fn laptop_person_says(&self, text: &str) -> MessageId {
        let channel = self.agents.talk.channel.to_string();
        let args = [SEND, JSON_FLAG, "--all", "--channel", &channel, text].map(str::to_owned);
        let sent = self.laptop.machine.start(&strs(&args)).finish();
        assert!(sent.success, "the person on the laptop: {sent:?}");
        let ids: Vec<MessageId> = (json_objects(&sent.stdout).iter())
            .filter_map(|object| object["msg_id"].as_str()?.parse().ok())
            .collect();
        match ids.as_slice() {
            [id, ..] if ids.iter().all(|one| one == id) => *id,
            _ => panic!("not one msg_id: {sent:?}"),
        }
    }

    /// Runs `skald --agent claude recv`, with `--wait` if `wait`, in `~/work/acme` on Ada's
    /// machine; fails unless it succeeds.
    #[track_caller]
    pub fn claude_reads(&self, wait: bool) -> Finished {
        let mut args = vec![AGENT, CLAUDE, RECV];
        if wait {
            args.push(WAIT_FLAG);
        }
        let read = (self.agents.talk.ada.machine)
            .start_in(&self.agents.folder, &args)
            .finish();
        assert!(read.success, "claude: {read:?}");
        read
    }
}
