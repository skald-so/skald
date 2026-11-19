//! What Ada's agents say: `skald send` as an agent in `~/work/acme`, and what the front saw it send.

use skald_proto::RawEnvelope;

use crate::agents::Agents;
use crate::fixtures::{send_args, strs};
use crate::machine::Finished;
use crate::talk::messages_sent;
use crate::world::AGENT;

/// Runs `skald --agent <agent> send <flags> --all --channel <channel> <text>` in `~/work/acme` to
/// the talk's channel; gives the run and the message envelopes the front saw it send.
pub fn send_as(
    agents: &Agents,
    (agent, flags): (&str, &[&str]),
    text: &str,
) -> (Finished, Vec<RawEnvelope>) {
    let talk = &agents.talk;
    let from = talk.front.sent().len();
    let mut args: Vec<String> = [AGENT, agent].map(str::to_owned).to_vec();
    let mut send = send_args(talk.channel, text);
    send.splice(1..1, flags.iter().map(|flag| (*flag).to_owned()));
    args.extend(send);
    let finished = (talk.ada.machine)
        .start_in(&agents.folder, &strs(&args))
        .finish();
    let sent = (messages_sent(&talk.front, talk.channel, from).into_iter())
        .map(|sent| sent.envelope)
        .collect();
    (finished, sent)
}

/// As [`send_as`], without flags; fails unless the run succeeds and the front saw it send one
/// message. Gives that message's envelope.
#[track_caller]
pub fn sent_as(agents: &Agents, agent: &str, text: &str) -> RawEnvelope {
    let (finished, sent) = send_as(agents, (agent, &[]), text);
    assert!(finished.success, "{agent}: {finished:?}");
    match <[RawEnvelope; 1]>::try_from(sent) {
        Ok([one]) => one,
        Err(sent) => panic!("not one message sent: {sent:?}"),
    }
}
