//! `skald reply` and the MCP reply tool ([`crate::mcp`]): the device answers a message in its
//! channel, `to` its author; like any message, every member device of the channel gets it
//! (`docs/protocol/README.md`, "Content").
//!
//! - The device keeps what a reply needs of each message its person's run showed or it handed to an
//!   agent, for the message's wait: its envelope's `ttl`, at most [`recv::AGENT_WAIT`], after the
//!   device wrote it ([`crate::state`]). That is the message's `msg_id`, space and channel, and its
//!   `from` and `hops`, sealed under the state key. References count those messages
//!   ([`crate::forms`]).
//! - A reply names the message by `#<ref>`, `<ref>`, as a shell drops an unquoted `#…`, or its
//!   `msg_id`. The person replies to a message its run showed; an agent only to one handed over to
//!   it in its folder's space. An unknown message, or a reference two kept messages share, sends
//!   nothing and says why, naming nothing of another space.
//! - It sends as `skald send` does ([`send::send_synced`]), in the message's channel, `to` its
//!   `from`, a person's address included (`docs/protocol/agents.md`, "Person"), with `reply_to`
//!   its `msg_id` and `hops` 0 from the person, one more than the message's from an agent.
//! - A reply the device takes in gives one of its messages `replied` ([`replied`]) only if it is in
//!   that message's channel and from its `to` agent (`docs/protocol/delivery.md`, "Receipts").

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use skald_crypto::StateKey;
use skald_proto::{Address, ChannelId, DeviceId, MessageId, Name, SpaceId, VersionRange};

use crate::connection::printable;
use crate::devices::text;
use crate::forms::{self, SHORTEST};
use crate::groups;
use crate::recv;
use crate::secrets;
use crate::send::{self, Draft};
use crate::spaces::Scope;
use crate::state::{self, Replied, State, Target};

/// What a reply needs of a message besides its `msg_id`, space and channel, as the device seals it.
#[derive(Serialize, Deserialize)]
struct Sealed {
    /// The message's `from`, which the reply is to.
    from: String,
    hops: u64,
}

/// What a reply names a message by.
#[derive(Debug, PartialEq)]
enum Given {
    Id(MessageId),
    /// Leading characters of its reference, lowercased.
    Reference(String),
}

/// Replies `text` to the message `message` names, by `#<ref>`, `<ref>` or its `msg_id`, as
/// `agent` if given, whose run holds its warnings for the person, or as the person, by the state in
/// `dir`, its secrets in `storage`; reports as `skald send` does, as JSON if `json`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    (message, text): (&str, &str),
    agent: Option<&Name>,
    json: bool,
) -> Result<(), String> {
    recv::holding_warnings(dir, agent.is_some(), || {
        let named = given(message)?;
        let scope = Scope::of_run(dir, agent.is_some())?;
        let reader = match (scope, agent) {
            (Scope::Only(space), Some(agent)) => Some((space, agent.as_str())),
            _ => None,
        };
        let target = target(dir, &named, reader, message)?;
        let mut synced = send::of_channel(dir, storage, settings, target.channel, scope)?;
        let opened = open(
            &groups::inbox_key(&synced.follower)?,
            synced.follower.this,
            &target,
        )
        .ok_or("what this device kept of the message does not open; nothing is sent")?;
        let to: Address = (opened.from.parse()).map_err(|err| {
            format!("what this device kept of the message is malformed: {err}; nothing is sent")
        })?;
        let server = send::server_of(&mut synced.follower)?;
        let draft = Draft {
            to: Some(to),
            text,
            reply_to: Some(target.msg_id),
            hops: match agent {
                Some(_) => opened.hops.saturating_add(1),
                None => 0,
            },
        };
        let reported = send::reported(json, agent);
        let channel = target.channel;
        send::send_synced(
            dir,
            (synced, channel),
            (scope, server),
            &draft,
            agent,
            reported,
        )
    })
}

/// What `message` names: a `msg_id`, or else a reference, with or without its `#`, of at least
/// [`SHORTEST`] characters.
fn given(message: &str) -> Result<Given, String> {
    let trimmed = message.trim();
    if let Ok(msg_id) = trimmed.parse::<MessageId>() {
        return Ok(Given::Id(msg_id));
    }
    let reference = trimmed.strip_prefix('#').unwrap_or(trimmed).to_lowercase();
    if reference.len() >= SHORTEST && reference.bytes().all(|byte| forms::BASE32.contains(&byte)) {
        return Ok(Given::Reference(reference));
    }
    Err(format!(
        "\"{}\" names no message: give its #<ref>, as skald recv shows it, or its msg_id; nothing \
         is sent",
        printable(message)
    ))
}

/// The message `named`, as `message` names it, that the person, or else the agent `reader` in its
/// space, can reply to by the state in `dir`; fails, saying why, for one it cannot reply to and for
/// a reference two kept messages share.
fn target(
    dir: &Path,
    named: &Given,
    reader: Option<(skald_proto::SpaceId, &str)>,
    message: &str,
) -> Result<Target, String> {
    let shown = printable(message.trim());
    let unknown = || match reader {
        Some(_) => format!(
            "{shown} is no message handed over to this agent in its folder's space, or its time \
             for a reply ended; nothing is sent"
        ),
        None => format!(
            "{shown} is no message skald recv showed, or its time for a reply ended; nothing is \
             sent"
        ),
    };
    if !dir.join(state::FILE).is_file() {
        return Err(unknown());
    }
    let state = State::open(dir).map_err(text)?;
    let targets = state.targets(reader, state::unix_now()).map_err(text)?;
    let kept = state.kept_ids().map_err(text)?;
    chosen(named, targets, &kept).map_err(|shared| {
        if shared {
            format!(
                "{shown} is the reference of more than one message this device keeps: give more \
                 of it, or the msg_id; nothing is sent"
            )
        } else {
            unknown()
        }
    })
}

/// The one of `targets` that `named` names; fails with whether more than one message shares it:
/// a reference two of `kept`, the messages the device keeps, start with, or a `msg_id` two
/// targets have.
fn chosen(named: &Given, targets: Vec<Target>, kept: &[MessageId]) -> Result<Target, bool> {
    let mut found: Vec<Target> = match named {
        Given::Id(msg_id) => (targets.into_iter())
            .filter(|target| target.msg_id == *msg_id)
            .collect(),
        Given::Reference(reference) => {
            let mut sharing: Vec<MessageId> = (kept.iter().copied())
                .filter(|msg_id| forms::whole_reference(*msg_id).starts_with(reference))
                .collect();
            sharing.sort_unstable();
            sharing.dedup();
            if sharing.len() > 1 {
                return Err(true);
            }
            (targets.into_iter())
                .filter(|target| forms::whole_reference(target.msg_id).starts_with(reference))
                .collect()
        }
    };
    match found.len() {
        0 => Err(false),
        1 => Ok(found.remove(0)),
        _ => Err(true),
    }
}

/// The name the state key seals what a reply needs of the kept message of the entry `seq` under.
fn item(seq: u64) -> Vec<u8> {
    [b"target:".as_slice(), &seq.to_be_bytes()].concat()
}

/// What a reply needs of the message of the entry `seq`, `from` and with `hops`, sealed with `key`
/// for the device `this`.
pub fn seal_target(
    key: &StateKey,
    this: DeviceId,
    seq: u64,
    (from, hops): (&Address, u64),
) -> Vec<u8> {
    let sealed = Sealed {
        from: from.to_string(),
        hops,
    };
    let bytes = serde_json::to_vec(&sealed).expect("a target encodes");
    key.seal(this, &item(seq), &bytes)
}

/// What a reply needs of `target`, opened with `key` for the device `this`, if it opens.
fn open(key: &StateKey, this: DeviceId, target: &Target) -> Option<Sealed> {
    let bytes = key.open(this, &item(target.seq), &target.sealed)?;
    serde_json::from_slice(&bytes).ok()
}

/// The `replied` a message in `channel` of `space` from the device `sender`, named `name`, from
/// `from`, a reply to `original`, gives the message `original` the device sent, by `state`: only if
/// it is in that message's space and channel, and from its `to`, an agent.
pub fn replied(
    state: &State,
    ((space, channel), sender, from, name): ((SpaceId, ChannelId), DeviceId, &Address, &str),
    original: MessageId,
) -> Result<Option<Replied>, String> {
    let Some(sent) = state.sent(original).map_err(text)? else {
        return Ok(None);
    };
    let to = (sent.addressee.as_deref()).and_then(|to| to.parse::<Address>().ok());
    let answers = sent.space == space
        && sent.channel == channel
        && to.is_some_and(|to| !to.is_person() && &to == from);
    if !answers {
        return Ok(None);
    }
    let status = forms::Status {
        msg_id: original.to_string(),
        space: Some(space.to_string()),
        channel: Some(channel.to_string()),
        status: forms::REPLIED.to_owned(),
        device: Some(sender.to_string()),
        device_name: Some(name.to_owned()),
        problem: None,
        timestamp: state::unix_now(),
    };
    Ok(Some(Replied {
        msg_id: original,
        space,
        status: serde_json::to_string(&status).expect("a status encodes"),
        agent: sent.agent,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message to reply to, of `msg_id`, by its entry `seq`.
    fn target(seq: u64, msg_id: MessageId) -> Target {
        Target {
            seq,
            msg_id,
            space: SpaceId::generate(),
            channel: ChannelId::generate(),
            sealed: Vec::new(),
        }
    }

    /// A message is named by its `msg_id`, or by a reference of at least 4 characters with or
    /// without `#`, in either case; anything else names none.
    #[test]
    fn message_is_named_by_msg_id_or_reference() {
        let msg_id = MessageId::generate();
        assert_eq!(given(&msg_id.to_string()), Ok(Given::Id(msg_id)));
        for named in ["#k2m4", "k2m4", " #K2M4 "] {
            assert_eq!(
                given(named),
                Ok(Given::Reference("k2m4".to_owned())),
                "{named}"
            );
        }
        for wrong in ["#k2m", "", "#", "k2m1", "#k2m4!", "##k2m4"] {
            assert!(given(wrong).is_err(), "{wrong}");
        }
    }

    /// A reference names the one target it starts, unless two kept messages start with it; a
    /// `msg_id`, the target that has it; none, or two targets of one `msg_id`, name none.
    #[test]
    fn reference_names_one_message_unless_two_kept_ones_share_it() {
        let ids: Vec<MessageId> = std::iter::repeat_with(MessageId::generate)
            .take(3)
            .collect();
        let reference = |msg_id: MessageId, length: usize| {
            Given::Reference(forms::whole_reference(msg_id)[..length].to_owned())
        };
        let targets = || vec![target(1, ids[0]), target(2, ids[1])];

        let chose = chosen(&reference(ids[0], 52), targets(), &ids).unwrap();
        assert_eq!((chose.seq, chose.msg_id), (1, ids[0]));
        assert_eq!(chosen(&Given::Id(ids[1]), targets(), &ids).unwrap().seq, 2);
        assert_eq!(chosen(&reference(ids[2], 52), targets(), &ids), Err(false));
        assert_eq!(chosen(&Given::Id(ids[2]), targets(), &ids), Err(false));
        let twice = vec![target(1, ids[0]), target(5, ids[0])];
        assert_eq!(chosen(&Given::Id(ids[0]), twice, &ids), Err(true));

        // Two kept messages whose references start alike, one of them no target.
        let mut seen = std::collections::HashMap::new();
        let (one, other) = std::iter::repeat_with(MessageId::generate)
            .find_map(|msg_id| {
                let prefix = forms::whole_reference(msg_id)[..SHORTEST].to_owned();
                seen.insert(prefix, msg_id).map(|earlier| (earlier, msg_id))
            })
            .unwrap();
        let alike = vec![target(1, one)];
        let shared = reference(one, SHORTEST);
        assert_eq!(chosen(&shared, alike.clone(), &[one, other]), Err(true));
        assert_eq!(chosen(&shared, alike.clone(), &[one]).unwrap().seq, 1);
        assert_eq!(
            chosen(&reference(one, 52), alike, &[one, other])
                .unwrap()
                .seq,
            1
        );
    }
}
