//! Resending to a Claude Code session (`docs/protocol/delivery.md`, "Device to agent";
//! `docs/dev/architecture.md`, "Delivery to agents"): the helper's run for an agent's sessions
//! ([`crate::recv::to_sessions`]) gives each kept message that waits for the agent to the first
//! path its session has, in the order of [`Way::ORDER`]:
//!
//! - the session's cross-session socket ([`crate::claude_socket`]);
//! - Channels: an event of the session's `skald mcp` ([`crate::mcp`]);
//! - the `asyncRewake` hook, `skald hook claude --wait`, which then starts a turn;
//! - the mod, which, while no turn runs, submits only [`MOD_PROMPT`], never a message's text.
//!
//! Each gives an `own` message as its line, within the hooks' limit ([`hook::fitted`]), and a
//! message below `own` only as the signal ([`hook::SIGNAL`]). A message a path took waits for its
//! confirmation there ([`crate::claude_socket::Scan`]) until a timeout, [`RESEND_SETTING`] or
//! [`RESEND_AFTER`], and goes by the next path then, or at once on a receipt that says it did not
//! reach the model; after the last it waits for the hooks and `inbox`, which give a signal again.
//! A signal goes again so until the agent calls `inbox`. While a path waits for its confirmation,
//! the hooks and `inbox` do not give an `own` message ([`State::for_agent`]); one the session file
//! shows going to the model goes by no other path ([`State::shown`]). The timeouts live in the
//! local state, so a helper that restarts goes on with them.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde_json::json;

use skald_proto::{MessageId, Name, SpaceId};

use crate::devices::text;
use crate::forms::References;
use crate::output::{self, Stream};
use crate::spaces::Follower;
use crate::state::{self, HOOKS, Pending, State};
use crate::{claude_socket, folders, groups, hook, recv};

/// The setting of how long a message a path took waits for its confirmation, in seconds; the
/// helper takes it from the command that starts it.
pub const RESEND_SETTING: &str = "SKALD_RESEND_AFTER";

/// How long a message a path took waits for its confirmation by default.
pub const RESEND_AFTER: Duration = Duration::from_secs(30);

/// The prompt the mod submits: it names the `inbox` tool and holds no message.
pub const MOD_PROMPT: &str =
    "skald: messages wait for you: call the inbox tool, or skald recv, to read them";

/// How long a message a path took waits for its confirmation: [`RESEND_SETTING`] if set.
pub fn resend_after() -> Duration {
    (std::env::var(RESEND_SETTING).ok())
        .and_then(|seconds| seconds.trim().parse().ok())
        .map_or(RESEND_AFTER, Duration::from_secs)
}

/// Prints what the mod's poll gives, as `agent` in the working directory's space, by the state in
/// `dir`: as JSON, how many messages wait for the agent, `waiting`, and [`MOD_PROMPT`] as `prompt`
/// if `prompt`.
pub fn mod_poll(dir: &Path, agent: &Name, prompt: bool) -> Result<(), String> {
    let waiting = match folders::working_space(dir) {
        Ok(space) => (State::open(dir).and_then(|state| state.waiting(space.id, agent.as_str())))
            .map_err(text)?,
        Err(_) => 0,
    };
    let mut polled = json!({ "waiting": waiting });
    if prompt {
        polled["prompt"] = json!(MOD_PROMPT);
    }
    output::write(Stream::Out, format!("{polled}\n").as_bytes())
        .map_err(|err| format!("cannot write the poll's answer: {err}"))
}

/// A path to a Claude Code session, in the order resending takes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Way {
    Socket,
    Channel,
    Rewake,
    Mod,
}

impl Way {
    /// Every path, in order.
    pub const ORDER: [Way; 4] = [Way::Socket, Way::Channel, Way::Rewake, Way::Mod];

    /// Its name in the local state.
    pub fn name(self) -> &'static str {
        match self {
            Way::Socket => "socket",
            Way::Channel => "channel",
            Way::Rewake => "rewake",
            Way::Mod => "mod",
        }
    }

    fn of(name: &str) -> Option<Way> {
        Way::ORDER.into_iter().find(|way| way.name() == name)
    }
}

/// The paths beside the socket that the helper holds for an agent's session in a space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// A `skald mcp` of Claude Code's.
    pub channel: bool,
    /// An `asyncRewake` hook that waits.
    pub rewake: bool,
    /// A mod that polls.
    pub mods: bool,
}

/// What a run gives the paths the helper holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pushed {
    /// The space of the run's agent.
    pub space: Option<SpaceId>,
    /// The channel events, each one's text.
    pub channel: Vec<String>,
    /// What the `asyncRewake` hook gives, if anything.
    pub rewake: Option<String>,
    /// Whether the mod submits its prompt.
    pub prompt: bool,
    /// When, in Unix milliseconds, the next message a path took times out, if any does.
    pub next: Option<i64>,
}

/// What resending does with a waiting message now.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Leaves it where it is.
    Stay,
    /// Gives it to the path.
    To(Way),
    /// Leaves it to the hooks and `inbox`.
    Release,
}

/// What resending does now, at `now` in Unix milliseconds, with `pending`, by the paths for which
/// `has` holds.
fn step(pending: &Pending, now: i64, has: impl Fn(Way) -> bool) -> Step {
    let after = match pending.path.as_deref() {
        None if pending.signalled => return Step::Stay,
        None => None,
        Some(HOOKS) => return Step::Stay,
        Some(name) => match Way::of(name) {
            Some(_) if pending.due.is_some_and(|due| due > now) => return Step::Stay,
            Some(way) => Some(way),
            None => return Step::Stay,
        },
    };
    let next =
        (Way::ORDER.into_iter()).find(|way| after.is_none_or(|after| *way > after) && has(*way));
    match (next, after) {
        (Some(way), _) => Step::To(way),
        (None, Some(_)) => Step::Release,
        (None, None) => Step::Stay,
    }
}

/// When the first of `pending` that waits on a path for its confirmation times out, in Unix
/// milliseconds; one the session file shows waits with no timeout.
fn next_due<'p>(pending: impl IntoIterator<Item = &'p Pending>) -> Option<i64> {
    (pending.into_iter())
        .filter(|pending| (pending.path.as_deref()).is_some_and(|path| path != HOOKS))
        .filter_map(|pending| pending.due)
        .filter(|due| *due < i64::MAX)
        .min()
}

/// A waiting message, opened: its `seq`, and its line and `msg_id` if it is at `own`.
struct Opened {
    seq: u64,
    own: Option<(String, MessageId)>,
}

/// Gives each kept message that waits for `agent` in `space` to the next path its session has:
/// the socket, by its registration in the state; the paths the helper holds, as `held` says. A
/// path takes what it gives for `resend`. What it gives the paths the helper holds goes into
/// `pushed`. Hands nothing over: a confirmation, the hooks or `inbox` do. Gives 0.
pub fn hand_over(
    follower: &Follower,
    (space, agent): (SpaceId, &Name),
    (held, resend): (Held, Duration),
    pushed: &mut Pushed,
) -> Result<usize, String> {
    let state = &follower.state;
    pushed.space = Some(space);
    let now = state::unix_ms();
    let due = now.saturating_add(i64::try_from(resend.as_millis()).unwrap_or(i64::MAX));
    let socket = live_socket(state, (space, agent))?;
    let has = |way| match way {
        Way::Socket => socket.is_some(),
        Way::Channel => held.channel,
        Way::Rewake => held.rewake,
        Way::Mod => held.mods,
    };
    let pending = (state.pending(space, agent.as_str(), state::unix_now())).map_err(text)?;
    let mut ways: BTreeMap<Way, Vec<&Pending>> = BTreeMap::new();
    let mut released = Vec::new();
    for waiting in &pending {
        match step(waiting, now, has) {
            Step::Stay => {}
            Step::To(way) => ways.entry(way).or_default().push(waiting),
            Step::Release => released.push(waiting.seq),
        }
    }
    let agent_name = agent.as_str();
    if !released.is_empty() {
        (state.pushed((&released, &[]), agent_name, (HOOKS, None))).map_err(text)?;
    }
    let mut next = next_due(pending.iter().filter(|waiting| {
        !released.contains(&waiting.seq) && !ways.values().flatten().any(|to| to.seq == waiting.seq)
    }));
    if ways.is_empty() {
        pushed.next = next;
        return Ok(0);
    }
    let references = References::new(state.kept_ids().map_err(text)?);
    for (way, taken) in ways {
        let given = match way {
            Way::Mod => {
                let seqs: Vec<u64> = taken.iter().map(|waiting| waiting.seq).collect();
                (state.pushed((&seqs, &[]), agent_name, (way.name(), Some(due)))).map_err(text)?;
                pushed.prompt = true;
                true
            }
            _ => {
                let opened = opened(follower, &taken, &references)?;
                match way {
                    Way::Socket => {
                        let session = socket.as_ref().expect("a socket path has a session");
                        to_socket(state, (session, agent_name), &opened, (&references, due))?
                    }
                    Way::Channel => {
                        to_channel(state, agent_name, &opened, (&references, due), pushed)?
                    }
                    _ => to_rewake(state, agent_name, &opened, (&references, due), pushed)?,
                }
            }
        };
        if given {
            next = Some(next.map_or(due, |next| next.min(due)));
        }
    }
    pushed.next = next;
    Ok(0)
}

/// The last registered Claude Code session of `agent` in `space` that still runs
/// ([`claude_socket::runs`]); a newer one that does not is no longer registered.
fn live_socket(
    state: &state::State,
    (space, agent): (SpaceId, &Name),
) -> Result<Option<state::ClaudeSession>, String> {
    let sessions = state.claude_sessions().map_err(text)?;
    let theirs = (sessions.into_iter().rev())
        .filter(|session| session.space == space && session.agent == agent.as_str());
    for session in theirs {
        if claude_socket::runs(&session.socket) {
            return Ok(Some(session));
        }
        state.end_claude(&session.session).map_err(text)?;
    }
    Ok(None)
}

/// Opens each of `taken`; one that does not open waits for the hooks, which warn of it.
fn opened(
    follower: &Follower,
    taken: &[&Pending],
    references: &References,
) -> Result<Vec<Opened>, String> {
    let key = groups::inbox_key(follower)?;
    Ok((taken.iter())
        .filter_map(|waiting| {
            let opened = key.open(follower.this, &recv::kept_item(waiting.seq), &waiting.line)?;
            let (line, msg_id) = recv::own_line(&opened, references);
            Some(Opened {
                seq: waiting.seq,
                own: msg_id.map(|msg_id| (line, msg_id)),
            })
        })
        .collect())
}

/// The `seq`s of the `own` messages of `opened`, and of those below `own`.
fn split(opened: &[Opened]) -> (Vec<u64>, Vec<u64>) {
    let own = (opened.iter()).filter(|each| each.own.is_some());
    let below = (opened.iter()).filter(|each| each.own.is_none());
    (
        own.map(|each| each.seq).collect(),
        below.map(|each| each.seq).collect(),
    )
}

/// Writes `opened` into the socket of `session`, the agent `agent`'s, with
/// [`claude_socket::write`]: each `own` message as its line, those below `own` as one signal.
/// Each is recorded as the socket's before the write, waiting until `due`, and as before it if
/// the write fails, which ends the registration. Gives whether it wrote.
fn to_socket(
    state: &state::State,
    (session, agent): (&state::ClaudeSession, &str),
    opened: &[Opened],
    (references, due): (&References, i64),
) -> Result<bool, String> {
    let mut lines: Vec<(String, String)> = (opened.iter())
        .filter_map(|each| each.own.as_ref())
        .map(|(line, msg_id)| {
            (
                msg_id.to_string(),
                hook::fitted(line, &references.of(*msg_id)),
            )
        })
        .collect();
    let (own, below) = split(opened);
    let signal = MessageId::generate().to_string();
    if !below.is_empty() {
        lines.push((signal.clone(), hook::SIGNAL.to_owned()));
    }
    if lines.is_empty() {
        return Ok(false);
    }
    let taken = (&session.session[..], agent);
    (state.socket_takes(taken, &own, (&below, &signal), due)).map_err(text)?;
    if claude_socket::write(session, &lines).is_err() {
        let seqs: Vec<u64> = own.into_iter().chain(below).collect();
        state.socket_failed(&seqs, agent).map_err(text)?;
        state.end_claude(&session.session).map_err(text)?;
        return Ok(false);
    }
    Ok(true)
}

/// Gives `opened` to the agent `agent`'s `skald mcp` as channel events, waiting until `due`: each
/// `own` message as its line, those below `own` as one signal. Gives whether it gave any.
fn to_channel(
    state: &state::State,
    agent: &str,
    opened: &[Opened],
    (references, due): (&References, i64),
    pushed: &mut Pushed,
) -> Result<bool, String> {
    let (own, below) = split(opened);
    let mut events: Vec<String> = (opened.iter())
        .filter_map(|each| each.own.as_ref())
        .map(|(line, msg_id)| hook::fitted(line, &references.of(*msg_id)))
        .collect();
    if !below.is_empty() {
        events.push(hook::SIGNAL.to_owned());
    }
    if events.is_empty() {
        return Ok(false);
    }
    (state.pushed((&own, &below), agent, (Way::Channel.name(), Some(due)))).map_err(text)?;
    pushed.channel.extend(events);
    Ok(true)
}

/// Gives what of `opened` fits within the hooks' limit to the agent `agent`'s `asyncRewake` hook,
/// waiting until `due` ([`hook::plain`]). Gives whether it gave any.
fn to_rewake(
    state: &state::State,
    agent: &str,
    opened: &[Opened],
    (references, due): (&References, i64),
    pushed: &mut Pushed,
) -> Result<bool, String> {
    let waiting: Vec<hook::Waiting> = (opened.iter())
        .map(|each| match &each.own {
            Some((line, msg_id)) => hook::Waiting::Own {
                seq: each.seq,
                line: line.clone(),
                reference: references.of(*msg_id),
            },
            None => hook::Waiting::Below {
                seq: each.seq,
                signalled: false,
            },
        })
        .collect();
    let Some(given) = hook::plain(&waiting) else {
        return Ok(false);
    };
    let path = (Way::Rewake.name(), Some(due));
    (state.pushed((&given.handed, &given.signalled), agent, path)).map_err(text)?;
    pushed.rewake = Some(given.output);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A waiting message on `path`, if any, until `due`.
    fn waiting(path: Option<&str>, due: Option<i64>, signalled: bool) -> Pending {
        Pending {
            seq: 1,
            line: Vec::new(),
            msg_id: None,
            path: path.map(str::to_owned),
            due,
            signalled,
        }
    }

    /// The paths of a session that has those of `ways`.
    fn having(ways: &[Way]) -> impl Fn(Way) -> bool + '_ {
        |way| ways.contains(&way)
    }

    const NOW: i64 = 1_000_000;

    /// A message no path took goes by the session's first path, in the order socket, Channels,
    /// `asyncRewake`, mod; with none, it waits for the hooks and `inbox`.
    #[test]
    fn a_new_message_goes_by_the_first_path_the_session_has() {
        let new = waiting(None, None, false);
        for (ways, first) in [
            (&Way::ORDER[..], Step::To(Way::Socket)),
            (
                &[Way::Mod, Way::Rewake, Way::Channel][..],
                Step::To(Way::Channel),
            ),
            (&[Way::Mod, Way::Rewake][..], Step::To(Way::Rewake)),
            (&[Way::Mod][..], Step::To(Way::Mod)),
            (&[][..], Step::Stay),
        ] {
            assert_eq!(step(&new, NOW, having(ways)), first, "{ways:?}");
        }
    }

    /// A message waits on its path until its timeout; then it goes by the next path the session
    /// has, never an earlier one, and after the last it is left to the hooks and `inbox`.
    #[test]
    fn a_message_goes_by_the_next_path_once_its_timeout_passed() {
        for (path, next) in [
            ("socket", Step::To(Way::Channel)),
            ("channel", Step::To(Way::Rewake)),
            ("rewake", Step::To(Way::Mod)),
            ("mod", Step::Release),
        ] {
            let on = waiting(Some(path), Some(NOW + 1), false);
            assert_eq!(step(&on, NOW, having(&Way::ORDER)), Step::Stay, "{path}");
            let timed_out = waiting(Some(path), Some(NOW), false);
            assert_eq!(step(&timed_out, NOW, having(&Way::ORDER)), next, "{path}");
        }
        let skipping = waiting(Some("socket"), Some(NOW - 1), false);
        assert_eq!(
            step(&skipping, NOW, having(&[Way::Mod])),
            Step::To(Way::Mod)
        );
        let gone = waiting(Some("channel"), Some(NOW - 1), false);
        assert_eq!(step(&gone, NOW, having(&[Way::Socket])), Step::Release);
    }

    /// A message the session file shows waits with no timeout; one left to the hooks, or whose
    /// signal the hooks gave, stays theirs.
    #[test]
    fn a_shown_or_released_message_stays() {
        let shown = waiting(Some("socket"), Some(i64::MAX), false);
        assert_eq!(step(&shown, NOW, having(&Way::ORDER)), Step::Stay);
        let released = waiting(Some(HOOKS), None, false);
        assert_eq!(step(&released, NOW, having(&Way::ORDER)), Step::Stay);
        let signalled = waiting(None, None, true);
        assert_eq!(step(&signalled, NOW, having(&Way::ORDER)), Step::Stay);
    }

    /// The next timeout is the first of the messages on a path, but those shown or released.
    #[test]
    fn the_next_timeout_is_the_first_on_a_path() {
        let pending = [
            waiting(Some("socket"), Some(NOW + 5), false),
            waiting(Some("channel"), Some(NOW + 2), false),
            waiting(Some("rewake"), Some(i64::MAX), false),
            waiting(Some(HOOKS), Some(NOW), false),
            waiting(None, None, false),
        ];
        assert_eq!(next_due(&pending), Some(NOW + 2));
        assert_eq!(next_due(&pending[2..]), None);
    }
}
