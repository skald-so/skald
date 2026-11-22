//! `skald held`: the person decides each message an agent in `hold` keeps for that decision
//! (`docs/protocol/delivery.md`, "Device to agent"), only at a terminal, as `skald link` takes its
//! person's answers ([`link_code::at_terminal`]); as an agent, or without a terminal, it decides
//! nothing.
//!
//! - It takes the messages one at a time, in the order the device took them in: it shows the
//!   message, with its agent, space, channel, sender with device and trust level, when the device
//!   took it in, `#<ref>` and text, then reads `accept` or `refuse`, and after `refuse` a reason,
//!   an empty line giving none.
//! - Each step after the person's answers goes to the helper, or runs here holding the local
//!   state's lock, as other commands do ([`Step`]). The state keeps a decision with the receipt it
//!   owes, so a crash neither loses nor repeats them: accepted, the message waits for the agent as
//!   any other and gets `read` once handed over; refused, it gets `refused` with the reason, which
//!   the step sends ([`receipts::send`]).
//! - A message whose wait ends undecided gets `refused` saying no one decided
//!   ([`receipts::NO_ONE_DECIDED`]). The person's `skald recv` says how many wait
//!   ([`say_waiting`]).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use skald_proto::{MessageId, VersionRange};

use crate::devices::{signed_in_with_secrets, text};
use crate::forms::{self, Form, References};
use crate::key_packages;
use crate::link_code;
use crate::output::{self, Stream};
use crate::person::Person;
use crate::receipts;
use crate::recv;
use crate::secrets;
use crate::spaces::{self, Scope};
use crate::state::{self, Held, State};

/// What the person types to accept a held message, and to refuse it.
const ACCEPT: &str = "accept";
const REFUSE: &str = "refuse";

/// Why a decision finds no message to decide.
const GONE: &str = "the message no longer waits for a decision: another run decided it, or its \
                    wait ended";

/// A step of `skald held` after its person's answers, on the message of the kept entry `seq` for
/// the agent `agent`, which its process hands to the helper or runs here holding the local state's
/// lock (`docs/dev/architecture.md`, "Local state").
#[derive(Serialize, Deserialize)]
pub enum Step {
    /// Shows the message.
    Show { seq: u64, agent: String },
    /// Hands the message over to the agent.
    Accept { seq: u64, agent: String },
    /// Refuses the message with `reason`, if any, and sends its `refused`.
    Refuse {
        seq: u64,
        agent: String,
        reason: Option<String>,
    },
}

/// The person's decision on a held message.
enum Decision {
    Accept,
    /// With the reason, if any.
    Refuse(Option<String>),
}

/// `skald held` on the machine whose state is in `dir`: takes its person's decisions at the
/// terminal in this process and has `step` run each step that follows them ([`Step`]), stopping at
/// the first that fails; gives the exit code.
pub fn run(dir: &Path, mut step: impl FnMut(Step) -> ExitCode) -> ExitCode {
    if !link_code::at_terminal() {
        return crate::exit(Err(
            "skald held takes its person's decisions only at a terminal; nothing is decided"
                .to_owned(),
        ));
    }
    let mut taken = BTreeSet::new();
    loop {
        // Another run may have decided one or taken in another meanwhile.
        let next = State::open(dir)
            .and_then(|state| state.held(state::unix_now()))
            .map_err(text)
            .map(|held| {
                (held.into_iter()).find(|held| !taken.contains(&(held.seq, held.agent.clone())))
            });
        let held = match next {
            Ok(Some(held)) => held,
            Ok(None) if taken.is_empty() => {
                println!("No message waits for a decision.");
                return ExitCode::SUCCESS;
            }
            Ok(None) => return ExitCode::SUCCESS,
            Err(err) => return crate::exit(Err(err)),
        };
        let (seq, agent) = (held.seq, held.agent);
        taken.insert((seq, agent.clone()));
        let shown = step(Step::Show {
            seq,
            agent: agent.clone(),
        });
        if shown != ExitCode::SUCCESS {
            return shown;
        }
        let decided = match decision(Person::Terminal) {
            Ok(Some(Decision::Accept)) => step(Step::Accept { seq, agent }),
            Ok(Some(Decision::Refuse(reason))) => step(Step::Refuse { seq, agent, reason }),
            Ok(None) => {
                return crate::exit(Err(
                    "the terminal's input ended; the messages not decided still wait".to_owned(),
                ));
            }
            Err(err) => return crate::exit(Err(err)),
        };
        if decided != ExitCode::SUCCESS {
            return decided;
        }
    }
}

/// The decision `person` gives, asked again until `accept` or `refuse`, and after `refuse` the
/// reason, an empty one giving none; `None` once the terminal's input ends.
fn decision(person: Person) -> Result<Option<Decision>, String> {
    loop {
        let Some(answer) = person.enter(&format!("{ACCEPT} or {REFUSE}"), "")? else {
            return Ok(None);
        };
        match answer.trim().to_lowercase().as_str() {
            ACCEPT => return Ok(Some(Decision::Accept)),
            REFUSE => break,
            _ => println!("Please answer {ACCEPT} or {REFUSE}."),
        }
    }
    let Some(reason) = person.enter("Reason, or nothing for none", "")? else {
        return Ok(None);
    };
    let reason = reason.trim();
    Ok(Some(Decision::Refuse(
        (!reason.is_empty()).then(|| reason.to_owned()),
    )))
}

/// Runs `step` of `skald held` on the machine whose state is in `dir`, its secrets in `storage`.
pub fn step(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    step: Step,
) -> Result<(), String> {
    let now = state::unix_now();
    match step {
        Step::Show { seq, agent } => show(dir, storage, seq, &agent),
        Step::Accept { seq, agent } => {
            let state = State::open(dir).map_err(text)?;
            if !state.accept_held(seq, &agent, now).map_err(text)? {
                return Err(GONE.to_owned());
            }
            println!("Accepted: the agent {agent} gets it.");
            Ok(())
        }
        Step::Refuse { seq, agent, reason } => {
            let state = State::open(dir).map_err(text)?;
            let space = waiting(&state, seq, &agent, now)?.space;
            if !(state.refuse_held(seq, &agent, reason.as_deref(), now)).map_err(text)? {
                return Err(GONE.to_owned());
            }
            println!("Refused: the agent {agent} never gets it.");
            let due = state.reads_due().map_err(text)?;
            if due.iter().any(|due| due.space == space) {
                drop(state);
                send_refused(dir, storage, (speaks, timeout), space)?;
            }
            Ok(())
        }
    }
}

/// Shows the message of the kept entry `seq` that waits for the person's decision for `agent`, by
/// the state in `dir`, its secrets in `storage`.
fn show(dir: &Path, storage: secrets::Storage, seq: u64, agent: &str) -> Result<(), String> {
    let (state, _, _, _, secrets) = signed_in_with_secrets(dir, storage)?;
    let held = waiting(&state, seq, agent, state::unix_now())?;
    let this = (state.device().map_err(text)?)
        .map(|device| device.id)
        .ok_or_else(|| "this machine has no device".to_owned())?;
    let key = key_packages::local_state_key(&state, &secrets)?;
    let message = (key.open(this, &recv::kept_item(seq), &held.line))
        .and_then(|opened| serde_json::from_slice::<forms::Message>(&opened).ok())
        .ok_or_else(|| "the held message this device kept does not open".to_owned())?;
    let space = (state.spaces().map_err(text)?.into_iter())
        .find(|space| space.id == held.space)
        .map_or_else(|| held.space.to_string(), |space| space.name);
    let references = References::new(state.kept_ids().map_err(text)?);
    let reference = (message.msg_id.parse::<MessageId>().ok())
        .map(|msg_id| references.of(msg_id))
        .unwrap_or_default();
    println!("{}", shown(&held, &space, &message, &reference));
    Ok(())
}

/// The message of the kept entry `seq` that waits for the person's decision for `agent` by `now`,
/// in Unix seconds, by `state`; fails with [`GONE`] if none does.
fn waiting(state: &State, seq: u64, agent: &str, now: u64) -> Result<Held, String> {
    (state.held(now).map_err(text)?.into_iter())
        .find(|held| held.seq == seq && held.agent == agent)
        .ok_or_else(|| GONE.to_owned())
}

/// How `skald held` shows `message`, `held` for its agent in the space named `space`, by its
/// reference `reference`.
fn shown(held: &Held, space: &str, message: &forms::Message, reference: &str) -> String {
    format!(
        "The message {} (#{reference}) for the agent {} in the space {space}, channel {}, from {} \
         ({}), by the device {} ({}), taken in {}:\n  {}",
        message.msg_id,
        held.agent,
        message.channel,
        message.from,
        message.level,
        crate::connection::printable(&message.device_name),
        message.device,
        crate::update::utc(held.taken),
        crate::connection::printable(&message.text)
    )
}

/// Sends the receipts the device owes in `space`, the `refused` of a refusal among them, by the
/// state in `dir`, its secrets in `storage`, once the space is synced.
fn send_refused(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    space: skald_proto::SpaceId,
) -> Result<(), String> {
    let scope = Scope::Only(space);
    let mut follower = spaces::following(dir, storage, speaks, timeout, scope)?;
    let followed = (follower.state.spaces().map_err(text)?.into_iter())
        .find(|followed| followed.id == space)
        .ok_or_else(|| format!("this device follows no space {space}"))?;
    let synced = follower.sync(&followed)?;
    let rosters = [(space, synced)].into_iter().collect();
    receipts::send(&mut follower, &rosters, scope);
    Ok(())
}

/// Says, in `form`, how many decisions on held messages wait in the state `state`, and that
/// `skald held` takes them; nothing with none.
pub fn say_waiting(state: &State, form: Form) -> Result<(), String> {
    let waiting = state.held(state::unix_now()).map_err(text)?.len();
    let line = match (waiting, form) {
        (0, _) => return Ok(()),
        (_, Form::Json) => {
            let line = forms::object("held", json!({ "waiting": waiting }));
            return output::write(Stream::Out, format!("{line}\n").as_bytes())
                .map_err(|err| format!("cannot write to the output: {err}"));
        }
        (1, _) => "skald: 1 message waits for your decision: run skald held at a terminal to \
                   accept or refuse it"
            .to_owned(),
        _ => format!(
            "skald: {waiting} messages wait for your decision: run skald held at a terminal to \
             accept or refuse them"
        ),
    };
    output::write(Stream::Err, format!("{line}\n").as_bytes())
        .map_err(|err| format!("cannot write to the output: {err}"))
}
