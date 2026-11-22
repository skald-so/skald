//! `skald hook claude`: the hook handler of Claude Code's `PostToolUse` and `Stop`
//! (`docs/dev/architecture.md`, "Components"; `docs/protocol/delivery.md`, "Device to agent").
//! Codex's hooks: [`crate::codex`].
//!
//! - It reads Claude Code's hook input on standard input ([`input`]) and runs in its `cwd`, the
//!   session's folder, as the agent `--agent` names or, without it, the one the session's
//!   `skald mcp` registers by default: of the client name Claude Code gives, [`CLAUDE_CODE`].
//! - It gives the agent's waiting statuses and `own` messages as their lines, and the signal,
//!   [`SIGNAL`], once for each message below `own`, which waits for `inbox`; under `inline`, such a
//!   message's line after the warning line ([`compose`]), as the delivery settings say
//!   ([`crate::delivery`]). What it prints stays within Claude Code's hook output limit,
//!   [`LIMIT`]: what does not fit waits for the next run; a message longer than the limit is cut
//!   and ends with how to read it whole.
//! - `Stop` continues the turn with them; once Claude Code took its cap of continuations in a row
//!   with no tool call between them, [`CAP`] unless `CLAUDE_CODE_STOP_HOOK_BLOCK_CAP` sets it,
//!   whose next output it drops, the run leaves them waiting.
//! - It prints nothing while nothing waits, in a folder in no space, on another event, and on a
//!   failure, which exits with 1, never with 2, which would keep a turn going.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use skald_proto::{Name, SpaceId};

use crate::devices::text;
use crate::folders;
use crate::state::State;

use crate::forms::DATA_WARNING;

/// The agent CLIs whose hooks `skald hook` handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
pub enum Cli {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
}

impl Cli {
    /// Whether its hooks run the handler on `event`.
    pub fn handles(self, event: Event) -> bool {
        match self {
            Cli::Claude => event != Event::UserPromptSubmit,
            Cli::Codex => event != Event::SessionEnd,
        }
    }
}

/// The client name Claude Code gives an MCP server at initialization.
pub const CLAUDE_CODE: &str = "claude-code";

/// The client name Codex gives an MCP server at initialization.
pub const CODEX: &str = "codex-mcp-client";

/// The setting that names a Codex session's config home, which its hooks inherit.
pub const CODEX_HOME: &str = "CODEX_HOME";

/// Claude Code's hook output limit, in characters.
pub const LIMIT: usize = 10_000;

/// How many `Stop` continuations in a row Claude Code takes by default.
pub const CAP: u64 = 8;

/// The setting of Claude Code that changes [`CAP`], which its hooks inherit.
pub const CAP_SETTING: &str = "CLAUDE_CODE_STOP_HOOK_BLOCK_CAP";

/// The most bytes of hook input a run reads.
const INPUT_MOST: u64 = 1 << 20;

/// The signal (`docs/protocol/delivery.md`, "Device to agent"): a fixed line, without the senders,
/// texts, count or references of the messages that wait, that tells the agent to call `inbox`.
pub const SIGNAL: &str =
    "skald: messages wait for you: call the inbox tool, or skald recv, to read them";

/// The settings Claude Code gives its hooks but `SessionEnd`'s: the session's cross-session
/// socket, and the token that makes a line on it the session's own child's.
pub const SOCKET_SETTING: &str = "CLAUDE_CODE_MESSAGING_SOCKET";
pub const TOKEN_SETTING: &str = "CLAUDE_CODE_MESSAGING_TOKEN";

/// A hook event the handler acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// `SessionStart`, which registers the session ([`crate::claude_socket`]).
    SessionStart,
    /// `SessionEnd`, which ends its registration.
    SessionEnd,
    PostToolUse,
    /// `UserPromptSubmit`, Codex's only.
    UserPromptSubmit,
    /// `Stop`; `continued` in a turn a `Stop` hook kept going.
    Stop {
        continued: bool,
    },
}

impl Event {
    /// Its `hook_event_name`.
    pub fn name(self) -> &'static str {
        match self {
            Event::SessionStart => "SessionStart",
            Event::SessionEnd => "SessionEnd",
            Event::PostToolUse => "PostToolUse",
            Event::UserPromptSubmit => "UserPromptSubmit",
            Event::Stop { .. } => "Stop",
        }
    }
}

/// What a run takes from its hook input and its environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// The event, if the handler acts on it.
    pub event: Option<Event>,
    /// The session's folder.
    pub cwd: PathBuf,
    /// How many `Stop` continuations in a row Claude Code takes.
    pub cap: u64,
    /// The session, as the input and the settings name it.
    #[serde(default)]
    pub session: Session,
}

/// A session, as a hook's input and settings name it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Its ID, Codex's thread, and its session file.
    pub id: Option<String>,
    pub file: Option<PathBuf>,
    /// Claude Code's cross-session socket and token, from [`SOCKET_SETTING`] and
    /// [`TOKEN_SETTING`].
    pub socket: Option<PathBuf>,
    pub token: Option<String>,
    /// Codex's config home, from [`CODEX_HOME`].
    #[serde(default)]
    pub home: Option<PathBuf>,
    /// The ID of the process that ran the hook: Codex's own, which runs the session; and its
    /// executable, that session's `codex`.
    #[serde(default)]
    pub process: Option<u32>,
    #[serde(default)]
    pub codex: Option<PathBuf>,
}

/// The hook input on `from`, its first JSON object, with the cap `cap` sets, if any; fails on input
/// that is not one.
pub fn input(from: impl Read, cap: Option<&str>) -> Result<Input, String> {
    #[derive(Deserialize)]
    struct Given {
        hook_event_name: String,
        cwd: PathBuf,
        #[serde(default)]
        stop_hook_active: bool,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        transcript_path: Option<PathBuf>,
    }
    // Read up to the end of the object only: a shell between the CLI and the hook may never end
    // the input.
    let given: Given = serde_json::Deserializer::from_reader(from.take(INPUT_MOST))
        .into_iter()
        .next()
        .ok_or("the hook input is empty")?
        .map_err(|err| format!("the hook input is malformed: {err}"))?;
    let event = match given.hook_event_name.as_str() {
        "SessionStart" => Some(Event::SessionStart),
        "SessionEnd" => Some(Event::SessionEnd),
        "PostToolUse" => Some(Event::PostToolUse),
        "UserPromptSubmit" => Some(Event::UserPromptSubmit),
        "Stop" => Some(Event::Stop {
            continued: given.stop_hook_active,
        }),
        _ => None,
    };
    let cap = cap.and_then(|cap| cap.trim().parse().ok()).unwrap_or(CAP);
    Ok(Input {
        event,
        cwd: given.cwd,
        cap,
        session: Session {
            id: given.session_id.filter(|id| !id.is_empty()),
            file: given.transcript_path,
            socket: None,
            token: None,
            home: None,
            process: None,
            codex: None,
        },
    })
}

/// What registers a session of `agent` whose folder is the working directory, by the state in
/// `dir`: the state, with the agent added to the folder's space, the space and the folder; `None`
/// in a folder in no space.
pub fn registering(dir: &Path, agent: &Name) -> Result<Option<(State, SpaceId, PathBuf)>, String> {
    let Ok(space) = folders::working_space(dir) else {
        return Ok(None);
    };
    let folder = std::env::current_dir()
        .map_err(|err| format!("cannot read the working directory: {err}"))?;
    let state = State::open(dir).map_err(text)?;
    state.add_agent(space.id, agent.as_str()).map_err(text)?;
    Ok(Some((state, space.id, folder)))
}

/// `line`, the line of the message `#<reference>`, as Skald puts it into a session at once: whole
/// within [`LIMIT`] characters; longer, cut so that it ends within it with how to read it whole.
pub fn fitted(line: &str, reference: &str) -> String {
    if line.chars().count() <= LIMIT {
        return line.to_owned();
    }
    let tail = cut_tail(reference);
    let head: String = line
        .chars()
        .take(LIMIT - 1 - tail.chars().count())
        .collect();
    format!("{head}\n{tail}")
}

/// The line that ends a message cut at `#<reference>`.
fn cut_tail(reference: &str) -> String {
    format!(
        "skald: #{reference} is cut here: inbox with message #{reference}, or skald recv \
         #{reference}, gives it whole"
    )
}

/// What waits for the agent, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// A status of a message it sent, at `position`, and its line, if the agent sees it.
    Status { position: u64, line: Option<String> },
    /// An `own` message of the entry `seq`, its line and reference.
    Own {
        seq: u64,
        line: String,
        reference: String,
    },
    /// A message below `own` of the entry `seq` that goes in as its text, `inline`, its line and
    /// reference.
    Inline {
        seq: u64,
        line: String,
        reference: String,
    },
    /// A message below `own` of the entry `seq`, and whether its signal went.
    Below { seq: u64, signalled: bool },
}

/// What a run gives: its output, and what it hands over.
#[derive(Debug, PartialEq, Eq)]
pub struct Given {
    /// The JSON object the run prints.
    pub output: String,
    /// The statuses it gives, by position.
    pub statuses: Vec<u64>,
    /// The messages it hands over, by `seq`.
    pub handed: Vec<u64>,
    /// The messages whose signal it gives, by `seq`.
    pub signalled: Vec<u64>,
}

/// What a run on `event` gives of `waiting`, if anything: the signal first if a message below
/// `own` waits whose signal did not go; then each status, `own` and `inline` line, in order, the
/// first `inline` one after [`DATA_WARNING`], while they fit within [`LIMIT`], the first cut if it
/// alone does not fit. Its output is one JSON object of at most [`LIMIT`] characters.
pub fn compose(event: Event, waiting: &[Waiting]) -> Option<Given> {
    let budget = LIMIT.saturating_sub(escaped(&output(event, "")).chars().count());
    let mut context = Context::default();
    let signalled: Vec<u64> = (waiting.iter())
        .filter_map(|waiting| match waiting {
            Waiting::Below {
                seq,
                signalled: false,
            } => Some(*seq),
            _ => None,
        })
        .collect();
    if !signalled.is_empty() {
        context.push(SIGNAL);
    }
    let mut statuses = Vec::new();
    let mut handed = Vec::new();
    let mut lines = 0;
    let mut warned = false;
    for waiting in waiting {
        match waiting {
            Waiting::Below { .. } => {}
            Waiting::Status { position, line } => {
                if let Some(line) = line {
                    if !context.fits(line, budget) {
                        break;
                    }
                    context.push(line);
                    lines += 1;
                }
                statuses.push(*position);
            }
            Waiting::Own {
                seq,
                line,
                reference,
            }
            | Waiting::Inline {
                seq,
                line,
                reference,
            } => {
                let warning =
                    (matches!(waiting, Waiting::Inline { .. }) && !warned).then_some(DATA_WARNING);
                let given: Vec<&str> = warning.into_iter().chain([line.as_str()]).collect();
                if context.fits_all(&given, budget) {
                    for line in given {
                        context.push(line);
                    }
                    warned |= warning.is_some();
                    lines += 1;
                    handed.push(*seq);
                    continue;
                }
                if lines == 0 {
                    // It is the last line the run gives.
                    if let Some(warning) = warning {
                        context.push(warning);
                    }
                    let tail = cut_tail(reference);
                    let room = budget.saturating_sub(context.cost() + 2 + context.cost_of(&tail));
                    context.push(&cut(line, room));
                    context.push(&tail);
                    handed.push(*seq);
                }
                break;
            }
        }
    }
    if context.lines.is_empty() {
        return None;
    }
    Some(Given {
        output: output(event, &context.lines.join("\n")),
        statuses,
        handed,
        signalled,
    })
}

/// The JSON object a run on `event` prints to give `context`.
fn output(event: Event, context: &str) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": event.name(),
            "additionalContext": context,
        }
    })
    .to_string()
}

/// `text` as a JSON string, without its quotes.
fn escaped(text: &str) -> String {
    let quoted = serde_json::to_string(text).expect("a string encodes");
    quoted[1..quoted.len() - 1].to_owned()
}

/// The lines a run gives, and the characters they take in its output.
#[derive(Default)]
struct Context {
    lines: Vec<String>,
    taken: usize,
}

impl Context {
    /// The characters the lines take in the output, with the line ends between them.
    fn cost(&self) -> usize {
        self.taken
    }

    /// The characters `line` would add to the output.
    fn cost_of(&self, line: &str) -> usize {
        let end = if self.lines.is_empty() { 0 } else { 2 };
        end + escaped(line).chars().count()
    }

    /// Whether `line` fits within `budget` after the lines.
    fn fits(&self, line: &str, budget: usize) -> bool {
        self.fits_all(&[line], budget)
    }

    /// Whether `lines` fit within `budget` after the lines.
    fn fits_all(&self, lines: &[&str], budget: usize) -> bool {
        let mut taken = self.taken;
        for (at, line) in lines.iter().enumerate() {
            let end = if self.lines.is_empty() && at == 0 {
                0
            } else {
                2
            };
            taken += end + escaped(line).chars().count();
        }
        taken <= budget
    }

    fn push(&mut self, line: &str) {
        self.taken += self.cost_of(line);
        self.lines.push(line.to_owned());
    }
}

/// The head of `line` that takes at most `room` characters in the output, with its line end.
fn cut(line: &str, room: usize) -> String {
    let mut head = String::new();
    let mut taken = 2;
    for char in line.chars() {
        taken += escaped(char.encode_utf8(&mut [0; 4])).chars().count();
        if taken > room {
            break;
        }
        head.push(char);
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(seq: u64, length: usize) -> Waiting {
        let head = format!("[ada/api · ws · own] #r{seq}: ");
        let line = format!("{head}{}", "x".repeat(length - head.chars().count()));
        Waiting::Own {
            seq,
            line,
            reference: format!("r{seq}"),
        }
    }

    fn context(given: &Given) -> String {
        let output: serde_json::Value = serde_json::from_str(&given.output).unwrap();
        output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The input's event, folder and the cap; another event is none; a cap that is no number is
    /// the default.
    #[test]
    fn input_takes_the_event_the_folder_and_the_cap() {
        let stop = r#"{"hook_event_name":"Stop","cwd":"/w","stop_hook_active":true}"#;
        let taken = input(stop.as_bytes(), Some("3")).unwrap();
        assert_eq!(taken.event, Some(Event::Stop { continued: true }));
        assert_eq!((taken.cwd, taken.cap), (PathBuf::from("/w"), 3));
        let other = r#"{"hook_event_name":"PreToolUse","cwd":"/w"}"#;
        let taken = input(other.as_bytes(), Some("many")).unwrap();
        assert_eq!((taken.event, taken.cap), (None, CAP));
        assert!(input(&b"{\"cwd\""[..], None).is_err());
    }

    /// The input is its first object: the run reads no further, as on an input a shell between
    /// the CLI and the hook never ends.
    #[test]
    fn input_ends_with_its_object() {
        struct Endless(&'static [u8]);
        impl Read for Endless {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                assert!(!self.0.is_empty(), "read past the object");
                let taken = self.0.len().min(buf.len());
                buf[..taken].copy_from_slice(&self.0[..taken]);
                self.0 = &self.0[taken..];
                Ok(taken)
            }
        }
        let given = Endless(br#" {"hook_event_name":"PostToolUse","cwd":"/w"}"#);
        let taken = input(given, None).unwrap();
        assert_eq!(taken.event, Some(Event::PostToolUse));
    }

    /// Nothing waiting, or only messages whose signal went, gives nothing.
    #[test]
    fn nothing_new_gives_nothing() {
        assert_eq!(compose(Event::PostToolUse, &[]), None);
        let waiting = [
            Waiting::Below {
                seq: 1,
                signalled: true,
            },
            Waiting::Status {
                position: 4,
                line: None,
            },
        ];
        assert_eq!(compose(Event::PostToolUse, &waiting), None);
    }

    /// The signal goes first, once for the messages below `own` whose signal did not go, which
    /// stay waiting; each status and `own` line follows.
    #[test]
    fn signal_comes_first_and_hands_nothing_over() {
        let waiting = [
            Waiting::Below {
                seq: 1,
                signalled: false,
            },
            Waiting::Status {
                position: 7,
                line: Some("skald: #abcd expired".to_owned()),
            },
            own(2, 50),
            Waiting::Below {
                seq: 3,
                signalled: false,
            },
        ];
        let given = compose(Event::Stop { continued: false }, &waiting).unwrap();
        let shown = context(&given);
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(lines.len(), 3, "{shown}");
        assert_eq!(lines[0], SIGNAL);
        assert_eq!(lines[1], "skald: #abcd expired");
        assert_eq!((given.statuses, given.handed), (vec![7], vec![2]));
        assert_eq!(given.signalled, vec![1, 3]);
        assert!(given.output.contains(r#""hookEventName":"Stop""#));
    }

    fn inline(seq: u64, length: usize) -> Waiting {
        let head = format!("[bob/review · bs · peer] #r{seq}: ");
        let line = format!("{head}{}", "y".repeat(length - head.chars().count()));
        Waiting::Inline {
            seq,
            line,
            reference: format!("r{seq}"),
        }
    }

    /// `inline` lines go in, handed over, after one warning line before the first of them; `own`
    /// lines before it get none.
    #[test]
    fn inline_lines_follow_one_warning() {
        let waiting = [own(1, 40), inline(2, 40), own(3, 40), inline(4, 40)];
        let given = compose(Event::PostToolUse, &waiting).unwrap();
        let shown = context(&given);
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(lines.len(), 5, "{shown}");
        assert!(lines[0].contains("#r1"), "{shown}");
        assert_eq!(lines[1], DATA_WARNING);
        assert!(
            lines[2].contains("#r2") && lines[4].contains("#r4"),
            "{shown}"
        );
        assert_eq!(given.handed, vec![1, 2, 3, 4]);
        assert!(given.signalled.is_empty());

        let given = compose(Event::PostToolUse, &[own(1, 100)]).unwrap();
        assert!(!context(&given).contains(DATA_WARNING));
    }

    /// An `inline` line that does not fit after its warning waits; one longer than the limit alone
    /// is cut after its warning, within the limit.
    #[test]
    fn inline_lines_stay_within_the_limit() {
        let given = compose(Event::PostToolUse, &[own(1, 9_850), inline(2, 100)]).unwrap();
        assert_eq!(given.handed, vec![1]);
        assert!(!context(&given).contains(DATA_WARNING));

        let given = compose(Event::PostToolUse, &[inline(5, 12_000)]).unwrap();
        assert_eq!(given.handed, vec![5]);
        assert!(given.output.chars().count() <= LIMIT);
        let shown = context(&given);
        assert!(shown.starts_with(DATA_WARNING), "{shown}");
        assert!(
            shown
                .lines()
                .last()
                .unwrap()
                .contains("inbox with message #r5")
        );
    }

    /// Lines that do not fit wait, in order; one longer than the limit alone is cut within it and
    /// ends with how to read it whole.
    #[test]
    fn output_stays_within_the_limit() {
        let given = compose(Event::PostToolUse, &[own(1, 6_000), own(2, 6_000)]).unwrap();
        assert_eq!(given.handed, vec![1]);
        assert!(given.output.chars().count() <= LIMIT);

        let long = own(5, 12_000);
        let given = compose(Event::PostToolUse, std::slice::from_ref(&long)).unwrap();
        assert_eq!(given.handed, vec![5]);
        assert!(
            given.output.chars().count() <= LIMIT,
            "{}",
            given.output.len()
        );
        let shown = context(&given);
        assert!(shown.starts_with("[ada/api · ws · own] #r5: xxx"));
        assert!(
            shown
                .lines()
                .last()
                .unwrap()
                .contains("inbox with message #r5")
        );

        let given = compose(Event::PostToolUse, &[own(1, 100), long]).unwrap();
        assert_eq!(
            given.handed,
            vec![1],
            "the long one waits for a run of its own"
        );
    }

    /// Characters JSON escapes count as their escapes.
    #[test]
    fn escapes_count_against_the_limit() {
        let head = "[ada/api · ws · own] #r1: ";
        let line = format!("{head}{}", "\"".repeat(9_000));
        let waiting = [Waiting::Own {
            seq: 1,
            line,
            reference: "r1".to_owned(),
        }];
        let given = compose(Event::PostToolUse, &waiting).unwrap();
        assert!(given.output.chars().count() <= LIMIT);
        assert!(context(&given).lines().count() == 2);
    }
}
