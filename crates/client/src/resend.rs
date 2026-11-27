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
//! Each gives an `own` message, or one under `inline`, as its line, within the hooks' limit
//! ([`hook::fitted`]), and any other only as the signal ([`hook::SIGNAL`]), as the person's delivery
//! settings say ([`crate::delivery`]), which may hold a message no path took back to the end of a
//! window of `batch`, or leave it to `inbox` under `pull`. Each counts one wake for each channel
//! whose messages it takes. A message a path took waits for its confirmation there
//! ([`crate::claude_socket::Scan`]) until a timeout, [`RESEND_SETTING`] or [`RESEND_AFTER`], and
//! goes by the next path then, or at once on a receipt that says it did not reach the model; after
//! the last it waits for the hooks and `inbox`, which give a signal again.
//! A signal goes again so until the agent calls `inbox`. While a path waits for its confirmation,
//! the hooks and `inbox` do not give an `own` message ([`State::for_agent`]); one the session file
//! shows going to the model goes by no other path ([`State::shown`]). The timeouts live in the
//! local state, so a helper that restarts goes on with them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use serde_json::json;

use skald_proto::{MessageId, Name, SpaceId};

use crate::devices::text;
use crate::forms::{self, Form, References};
use crate::output::{self, Stream};
use crate::recv::Goes;
use crate::spaces::Follower;
use crate::state::{self, HOOKS, Pending, State};
use crate::{claude_socket, folders, hook, recv};

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
    pub channel: Vec<Push>,
    /// What the `asyncRewake` hook gives, if anything.
    pub rewake: Option<Push>,
    /// Whether the mod submits its prompt.
    pub prompt: bool,
    /// When, in Unix milliseconds, the next message a path took times out, if any does.
    pub next: Option<i64>,
    /// When, in Unix milliseconds, the first window of `batch` that holds back a message no path
    /// took ends, if one does: a run then pushes it.
    pub batch: Option<u64>,
}

/// The output for one path, with the identity of a single own message if it carries one.
#[derive(Debug, PartialEq, Eq)]
pub struct Push {
    pub content: String,
    pub own: Option<state::ClaudeStopDelivery>,
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

/// A waiting message a path takes now, opened: its `seq`, its channel, and its line if it goes in
/// as its text.
struct Opened {
    seq: u64,
    channel: Option<String>,
    own: Option<Text>,
}

/// The line of a message that goes in as its text, its `msg_id`, and whether it is `inline`, after
/// the warning line.
struct Text {
    line: String,
    msg_id: MessageId,
    inline: bool,
    whole: Option<String>,
}

impl Text {
    /// Its line as a push path gives it alone, within the limit.
    fn fitted(&self, references: &References) -> String {
        let line = match self.inline {
            true => format!("{}\n{}", forms::DATA_WARNING, self.line),
            false => self.line.clone(),
        };
        hook::fitted(&line, &self.reference(references))
    }

    fn reference(&self, references: &References) -> String {
        self.whole
            .clone()
            .unwrap_or_else(|| references.of(self.msg_id))
    }
}

/// Gives each kept message that waits for `agent` in `space` to the next path its session has:
/// the socket, by its registration in the state; the paths the helper holds, as `held` says; one no
/// path took only once the delivery settings push it ([`recv::pushed`]). A path takes what it gives
/// for `resend`. What it gives the paths the helper holds goes into `pushed`. Hands nothing over: a
/// confirmation, the hooks or `inbox` do. Gives 0.
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
    let (opened, batch) = opened(follower, space, &ways, &references)?;
    pushed.batch = batch;
    for (way, taken) in ways {
        let opened: Vec<&Opened> = (taken.iter())
            .filter_map(|waiting| opened.get(&waiting.seq))
            .collect();
        if opened.is_empty() {
            continue;
        }
        let to = To {
            state,
            agent: agent_name,
            references: &references,
            due,
        };
        let given = match way {
            Way::Mod => {
                let seqs: Vec<u64> = opened.iter().map(|each| each.seq).collect();
                (state.pushed((&seqs, &[]), agent_name, (way.name(), Some(due)))).map_err(text)?;
                state.count_wakes(&channels(&opened)).map_err(text)?;
                pushed.prompt = true;
                true
            }
            Way::Socket => {
                let session = socket.as_ref().expect("a socket path has a session");
                to_socket(&to, session, &opened)?
            }
            Way::Channel => to_channel(&to, &opened, pushed)?,
            Way::Rewake => to_rewake(&to, &opened, pushed)?,
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

/// Opens each message of `ways`; one that does not open waits for the hooks, which warn of it.
/// Of those no path took, keeps those the delivery settings push now, as they say for them alone
/// ([`recv::pushed`]); gives when the first window of `batch` that holds one back ends.
fn opened(
    follower: &Follower,
    space: SpaceId,
    ways: &BTreeMap<Way, Vec<&Pending>>,
    references: &References,
) -> Result<(BTreeMap<u64, Opened>, Option<u64>), String> {
    let (fresh, again): (Vec<&Pending>, Vec<&Pending>) =
        (ways.values().flatten()).partition(|waiting| waiting.path.is_none());
    let sealed = |taken: Vec<&Pending>| {
        (taken.into_iter())
            .map(|waiting| (waiting.seq, waiting.line.clone()))
            .collect::<Vec<_>>()
    };
    let fresh = recv::pushed(follower, space, sealed(fresh))?;
    let again = recv::pushed_again(follower, space, sealed(again))?;
    let deferred: BTreeSet<_> = [Way::Channel, Way::Rewake]
        .iter()
        .filter_map(|way| ways.get(way))
        .flatten()
        .map(|pending| pending.seq)
        .collect();
    let opened = (fresh.messages.into_iter().chain(again.messages))
        .map(|message| {
            let whole = if message.goes == Goes::Own && deferred.contains(&message.seq) {
                message.msg_id
            } else {
                None
            };
            let (line, _, whole) =
                recv::kept_line_with_whole(&message.opened, Form::Short, references, whole);
            let own = match (message.goes, message.msg_id) {
                (Goes::Own | Goes::Inline, Some(msg_id)) => Some(Text {
                    line,
                    msg_id,
                    inline: message.goes == Goes::Inline,
                    whole,
                }),
                _ => None,
            };
            let opened = Opened {
                seq: message.seq,
                channel: message.channel,
                own,
            };
            (message.seq, opened)
        })
        .collect();
    Ok((opened, fresh.due))
}

/// The channels of `opened`.
fn channels(opened: &[&Opened]) -> BTreeSet<String> {
    (opened.iter())
        .filter_map(|each| each.channel.clone())
        .collect()
}

/// The `seq`s of the messages of `opened` that go in as their text, and of the others.
fn split(opened: &[&Opened]) -> (Vec<u64>, Vec<u64>) {
    let own = (opened.iter()).filter(|each| each.own.is_some());
    let below = (opened.iter()).filter(|each| each.own.is_none());
    (
        own.map(|each| each.seq).collect(),
        below.map(|each| each.seq).collect(),
    )
}

/// Where a run gives messages: the state, the agent they wait for, their references, and until
/// when, in Unix milliseconds, a path that takes them waits for their confirmation.
struct To<'a> {
    state: &'a state::State,
    agent: &'a str,
    references: &'a References,
    due: i64,
}

/// The delivery identity of an own line this path writes with its whole reference.
fn own_push(to: &To, opened: &Opened) -> Option<state::ClaudeStopDelivery> {
    let own = opened.own.as_ref()?;
    own.whole.as_ref()?;
    Some(state::ClaudeStopDelivery {
        seq: opened.seq,
        msg_id: own.msg_id,
        due: to.due,
    })
}

/// Writes `opened` into the socket of `session`, the agent's, with [`claude_socket::write`]: each
/// message that goes in as its text as its line, the others as one signal. Each is recorded as the
/// socket's before the write, and as before it if the write fails, which ends the registration; a
/// write counts its wakes unless the session says none of its lines reached the model
/// ([`State::socket_wakes`]). Gives whether it wrote.
fn to_socket(to: &To, session: &state::ClaudeSession, opened: &[&Opened]) -> Result<bool, String> {
    let To {
        state,
        agent,
        references,
        due,
    } = *to;
    let mut lines: Vec<(String, String)> = (opened.iter())
        .filter_map(|each| each.own.as_ref())
        .map(|own| (own.msg_id.to_string(), own.fitted(references)))
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
    let ids: Vec<String> = lines.into_iter().map(|(id, _)| id).collect();
    (state.socket_wakes(&session.session, &ids, &channels(opened))).map_err(text)?;
    Ok(true)
}

/// Gives `opened` to the agent's `skald mcp` as channel events: each message that goes in as its
/// text as its line, the others as one signal. Gives whether it gave any.
fn to_channel(to: &To, opened: &[&Opened], pushed: &mut Pushed) -> Result<bool, String> {
    let (own, below) = split(opened);
    let mut events: Vec<Push> = (opened.iter())
        .filter_map(|each| {
            Some(Push {
                content: each.own.as_ref()?.fitted(to.references),
                own: own_push(to, each),
            })
        })
        .collect();
    if !below.is_empty() {
        events.push(Push {
            content: hook::SIGNAL.to_owned(),
            own: None,
        });
    }
    if events.is_empty() {
        return Ok(false);
    }
    let path = (Way::Channel.name(), Some(to.due));
    (to.state.pushed((&own, &below), to.agent, path)).map_err(text)?;
    (to.state.count_wakes(&channels(opened))).map_err(text)?;
    pushed.channel.extend(events);
    Ok(true)
}

/// Gives what of `opened` fits within the hooks' limit to the agent's `asyncRewake` hook
/// ([`hook::plain`]). Gives whether it gave any.
fn to_rewake(to: &To, opened: &[&Opened], pushed: &mut Pushed) -> Result<bool, String> {
    let waiting: Vec<hook::Waiting> = (opened.iter())
        .map(|each| match &each.own {
            Some(own) if own.inline => hook::Waiting::Inline {
                seq: each.seq,
                line: own.line.clone(),
                reference: own.reference(to.references),
            },
            Some(own) => hook::Waiting::Own {
                seq: each.seq,
                line: own.line.clone(),
                reference: own.reference(to.references),
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
    let path = (Way::Rewake.name(), Some(to.due));
    (to.state
        .pushed((&given.handed, &given.signalled), to.agent, path))
    .map_err(text)?;
    let woken: Vec<&Opened> = (opened.iter().copied())
        .filter(|each| given.handed.contains(&each.seq) || given.signalled.contains(&each.seq))
        .collect();
    (to.state.count_wakes(&channels(&woken))).map_err(text)?;
    let own = match given.handed.as_slice() {
        [seq] => woken
            .iter()
            .find(|each| each.seq == *seq)
            .and_then(|each| own_push(to, each)),
        _ => None,
    };
    pushed.rewake = Some(Push {
        content: given.output,
        own,
    });
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
