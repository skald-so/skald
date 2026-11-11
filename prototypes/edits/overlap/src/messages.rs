//! An in-process stand-in for the end-to-end encrypted messages between the machines of a space.
//! They bypass the server stand-in: it never receives them.

use std::collections::BTreeMap;

use crate::MachineId;

/// A message between two machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Asks a machine for its changes. `base` is the asking machine's merge base with the
    /// upstream's default branch, so that the answer leaves out what the asking machine has.
    ChangesRequest { base: String },
    /// A machine's changes: a git bundle of a commit of its working tree, committed changes and
    /// not, on top of their history since a base the asking machine has.
    Changes { bundle: Vec<u8> },
}

/// A message with the machine that sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub from: MachineId,
    pub message: Message,
}

/// Each machine's inbox.
#[derive(Debug, Default)]
pub struct Messages {
    inboxes: BTreeMap<MachineId, Vec<Envelope>>,
}

impl Messages {
    /// Puts `message` from `from` into the inbox of `to`.
    pub fn send(&mut self, from: &str, to: &str, message: Message) {
        self.inboxes
            .entry(to.to_owned())
            .or_default()
            .push(Envelope {
                from: from.to_owned(),
                message,
            });
    }

    /// Takes the requests in `machine`'s inbox, in the order they came, and leaves the rest:
    /// each sender with its base.
    pub fn take_requests(&mut self, machine: &str) -> Vec<(MachineId, String)> {
        self.take(machine, |message| match message {
            Message::ChangesRequest { base } => Ok(base),
            other => Err(other),
        })
    }

    /// Takes the changes in `machine`'s inbox, in the order they came, and leaves the rest: each
    /// sender with its bundle.
    pub fn take_changes(&mut self, machine: &str) -> Vec<(MachineId, Vec<u8>)> {
        self.take(machine, |message| match message {
            Message::Changes { bundle } => Ok(bundle),
            other => Err(other),
        })
    }

    /// Takes the messages in `machine`'s inbox that `pick` takes, and leaves those it gives back.
    fn take<T>(
        &mut self,
        machine: &str,
        pick: fn(Message) -> Result<T, Message>,
    ) -> Vec<(MachineId, T)> {
        let Some(inbox) = self.inboxes.get_mut(machine) else {
            return Vec::new();
        };
        let mut taken = Vec::new();
        for Envelope { from, message } in std::mem::take(inbox) {
            match pick(message) {
                Ok(value) => taken.push((from, value)),
                Err(message) => inbox.push(Envelope { from, message }),
            }
        }
        taken
    }

    /// Every message waiting in an inbox, by its recipient.
    pub fn waiting(&self) -> &BTreeMap<MachineId, Vec<Envelope>> {
        &self.inboxes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_machine_takes_its_own_messages_of_one_kind_in_order() {
        let request = |base: &str| Message::ChangesRequest { base: base.into() };
        let changes = |bundle: &[u8]| Message::Changes {
            bundle: bundle.to_vec(),
        };
        let mut messages = Messages::default();
        messages.send("a", "b", request("1"));
        messages.send("c", "b", changes(b"2"));
        messages.send("c", "b", request("3"));
        messages.send("b", "a", changes(b"4"));

        assert_eq!(messages.take_requests("c"), []);
        assert_eq!(
            messages.take_requests("b"),
            [("a".into(), "1".into()), ("c".into(), "3".into())]
        );
        assert_eq!(messages.take_requests("b"), []);
        assert_eq!(
            messages.waiting()["b"],
            [Envelope {
                from: "c".into(),
                message: changes(b"2")
            }]
        );
        assert_eq!(messages.take_changes("b"), [("c".into(), b"2".to_vec())]);
        assert_eq!(messages.take_changes("b"), []);
        assert_eq!(messages.take_changes("a"), [("b".into(), b"4".to_vec())]);
    }
}
