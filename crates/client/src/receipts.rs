//! `read` and `refused` receipts the device issues (`docs/protocol/delivery.md`, "Device to agent"
//! and "Receipts").
//!
//! - Once the device marks a kept message whose `to` is an agent of this device handed over to
//!   that agent, by any run, it owes the message's `read`, kept with the mark ([`crate::state`]).
//!   None for a message without `to` or to a person, and none for the person's run.
//! - Once it keeps a message whose `to` agent's inbound mode refuses it, it owes its `refused`,
//!   with the reason, kept with the message ([`crate::inbound`]).
//! - A run then puts one receipt in the outbox for the messages it owes `read`, or `refused` for one
//!   reason, of from one sending device and one `from` in a channel, in the same transaction that
//!   clears what it owed, so a crash neither loses a receipt nor sends it twice. The receipt is from the agent, to the
//!   messages' `from`, with `hops` 0 and the device's next `counter` in the channel, encrypted in
//!   the channel's group once the channel takes this device's application messages: until then
//!   it waits for a later run ([`groups::encrypt_receipt`]).
//! - A receipt goes with `report` false, and no run reports it as a message the device's user sent.
//!   It goes out with every envelope queued before it, in the outbox's order, so a channel's
//!   envelopes are stored in `counter` order ("Gap detection").

use std::collections::BTreeMap;

use skald_proto::{
    Address, Body, ChannelId, Content, DeviceId, MessageId, NonEmpty, Receipt, ReceiptStatus,
    SpaceId, Unknown,
};

use crate::devices::text;
use crate::groups::{self, Rosters};
use crate::send::{self, Shown as _};
use crate::spaces::{Follower, Scope};
use crate::state::ReadDue;

/// Puts in the outbox a receipt of each `read` and `refused` the device owes within `scope`, in a
/// space of `rosters`, the spaces the run synced, once the channel takes it, then sends the
/// receipts that wait in the outbox with every envelope queued before them, for as long as the
/// request timeout. What is left waits for a later run.
pub fn send(follower: &mut Follower, rosters: &Rosters, scope: Scope) {
    if let Err(err) = issue(follower, rosters, scope) {
        eprintln!("skald: warning: {err}");
    }
    let queued = match send::queued(&follower.state, scope) {
        Ok(queued) => queued,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    let Some(last) = queued.iter().rposition(|queued| !queued.reports()) else {
        return;
    };
    let timeout = follower.timeout;
    send::deliver(follower, &queued[..=last], timeout, None);
}

/// The receipts the device owes of one receipt: in a space and channel, of the messages of a
/// sending device, from the agent they went to, to their `from`, `refused` for a reason or `read`.
type Batch = (SpaceId, ChannelId, DeviceId, String, String, Option<String>);

/// Encrypts a receipt of each `read` and `refused` the device owes within `scope`, in a space of
/// `rosters`, into the outbox, those of one [`Batch`] in one; one whose channel takes none yet
/// waits.
fn issue(follower: &Follower, rosters: &Rosters, scope: Scope) -> Result<(), String> {
    let mut batches: BTreeMap<Batch, Vec<(u64, MessageId)>> = BTreeMap::new();
    for ReadDue {
        seq,
        msg_id,
        space,
        channel,
        sender,
        author,
        addressee,
        reason,
    } in follower.state.reads_due().map_err(text)?
    {
        let batch = (space, channel, sender, author, addressee, reason);
        batches.entry(batch).or_default().push((seq, msg_id));
    }
    for ((space, channel, _, author, addressee, reason), reads) in batches {
        if scope.excludes(space) {
            continue;
        }
        let Some((roster, devices)) = rosters.get(&space) else {
            continue;
        };
        let (Ok(from), Ok(to)) = (addressee.parse::<Address>(), author.parse::<Address>()) else {
            return Err(format!(
                "the local state is malformed: the addresses of a receipt, {addressee} and \
                 {author}"
            ));
        };
        let (issued, messages): (Vec<u64>, Vec<MessageId>) = reads.into_iter().unzip();
        let Some(messages) = NonEmpty::from_vec(messages) else {
            continue;
        };
        // One whose channel takes none yet waits for a later run.
        let _ = groups::encrypt_receipt(
            follower,
            (roster, devices),
            (channel, issued),
            |_, counter| Content {
                from,
                to: Some(to),
                counter: Some(counter),
                hops: 0,
                reply_to: None,
                body: Some(Body::Receipt(Receipt {
                    status: match reason {
                        Some(_) => ReceiptStatus::Refused,
                        None => ReceiptStatus::Read,
                    },
                    messages,
                    reason,
                    unknown: Unknown::default(),
                })),
                delegation: None,
                grant: None,
                attachments: None,
                unknown: Unknown::default(),
            },
        )?;
    }
    Ok(())
}
