//! A minimal in-process Delivery Service that stands in for `skald-server`: it orders each
//! channel's Commits and keeps its latest GroupInfo, as `docs/protocol/membership.md`, "Commit
//! ordering", "Creating a channel" and "MLS profile", say. It holds no group secrets: it follows a
//! channel's epoch and member devices from its GroupInfo and Commits only, with the library's
//! [`Observer`].

use std::collections::BTreeMap;

use crate::{DeviceId, Leaf, Observer, Result, wire};

/// The `send-refused` problem of a refused Commit (`docs/protocol/delivery.md`, "Send").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The Commit is for an epoch other than the channel's current one.
    EpochMismatch,
    /// The Commit is not one the channel's view verifies as sent by the member device sending it.
    NotAMember,
}

pub struct DeliveryService<O> {
    channels: BTreeMap<Vec<u8>, Channel<O>>,
}

struct Channel<O> {
    epoch: u64,
    /// The device whose Commit of epoch 0 the service kept.
    creator: DeviceId,
    /// The group in the current epoch: from the first GroupInfo, then each accepted Commit.
    view: Option<O>,
    group_info: Option<Vec<u8>>,
    /// The accepted Commits, in the order members process them.
    commits: Vec<Vec<u8>>,
}

impl<O> Default for DeliveryService<O> {
    fn default() -> Self {
        Self {
            channels: BTreeMap::new(),
        }
    }
}

impl<O: Observer> DeliveryService<O> {
    /// Device `sender` sends `commit`, an encoded `MLSMessage` with a PublicMessage. The service
    /// keeps the first Commit of epoch 0 of a channel it does not know; it accepts a later one
    /// only for the channel's current epoch, from the member it names as its sender, if its view
    /// verifies the Commit. An accepted Commit moves the channel to its new epoch.
    pub fn send(&mut self, sender: &DeviceId, commit: &[u8]) -> Result<(), Refusal> {
        let framing = wire::framing(commit).map_err(|_| Refusal::NotAMember)?;
        if framing.wire_format != wire::PUBLIC_MESSAGE || framing.content_type != wire::COMMIT {
            return Err(Refusal::NotAMember);
        }
        let Some(channel) = self.channels.get_mut(&framing.group_id) else {
            if framing.epoch != 0 {
                return Err(Refusal::EpochMismatch);
            }
            self.channels.insert(
                framing.group_id,
                Channel {
                    epoch: 1,
                    creator: *sender,
                    view: None,
                    group_info: None,
                    commits: vec![commit.to_vec()],
                },
            );
            return Ok(());
        };
        if framing.epoch != channel.epoch {
            return Err(Refusal::EpochMismatch);
        }
        // Until a GroupInfo arrives, the service cannot tell who the members are.
        let view = channel.view.as_mut().ok_or(Refusal::NotAMember)?;
        view.process_commit(commit, |leaf| {
            if leaf.identity == sender {
                Ok(())
            } else {
                Err("the Commit's sender is another device".into())
            }
        })
        .map_err(|_| Refusal::NotAMember)?;
        channel.epoch = view.epoch();
        channel.commits.push(commit.to_vec());
        Ok(())
    }

    /// Device `sender` uploads `group_info`, an encoded `MLSMessage` with a GroupInfo. The service
    /// keeps it as the channel's latest if it is of the channel's current epoch, has the
    /// `ratchet_tree` and `external_pub` extensions, verifies, and lists `sender` as a member;
    /// and if it is the group the service follows, or, before the first one, if `sender` sent the
    /// kept Commit of epoch 0.
    pub fn upload_group_info(&mut self, sender: &DeviceId, group_info: &[u8]) -> Result<()> {
        let fields = wire::group_info(group_info)?;
        let channel = self
            .channels
            .get_mut(&fields.group_id)
            .ok_or("GroupInfo: no such channel")?;
        if fields.epoch != channel.epoch {
            return Err("GroupInfo: not of the current epoch".into());
        }
        if !(fields.has(wire::RATCHET_TREE) && fields.has(wire::EXTERNAL_PUB)) {
            return Err("GroupInfo: without ratchet_tree or external_pub".into());
        }
        let observed = O::observe(group_info)?;
        if !observed
            .members()?
            .iter()
            .any(|leaf| leaf.identity == sender)
        {
            return Err("GroupInfo: from a device that is not a member".into());
        }
        match &channel.view {
            Some(view) => {
                if view.context()? != observed.context()? {
                    return Err("GroupInfo: of another group state".into());
                }
            }
            None if *sender == channel.creator => channel.view = Some(observed),
            None => return Err("GroupInfo: not from the device of the kept Commit".into()),
        }
        channel.group_info = Some(group_info.to_vec());
        Ok(())
    }

    /// The channel's current epoch, if the service knows the channel.
    pub fn epoch(&self, group_id: &[u8]) -> Option<u64> {
        self.channels.get(group_id).map(|channel| channel.epoch)
    }

    /// The channel's members in its current epoch, once the service has a GroupInfo of it.
    pub fn members(&self, group_id: &[u8]) -> Result<Option<Vec<Leaf>>> {
        self.channels
            .get(group_id)
            .and_then(|channel| channel.view.as_ref())
            .map(Observer::members)
            .transpose()
    }

    /// The channel's latest GroupInfo.
    pub fn group_info(&self, group_id: &[u8]) -> Option<&[u8]> {
        self.channels
            .get(group_id)
            .and_then(|channel| channel.group_info.as_deref())
    }

    /// The channel's accepted Commits, in the order they were accepted.
    pub fn commits(&self, group_id: &[u8]) -> &[Vec<u8>] {
        self.channels
            .get(group_id)
            .map_or(&[], |channel| &channel.commits)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An observer the tests never reach.
    pub(crate) struct Unused;

    impl Observer for Unused {
        fn observe(_: &[u8]) -> Result<Self> {
            unimplemented!()
        }
        fn process_commit(&mut self, _: &[u8], _: impl FnOnce(&Leaf) -> Result<()>) -> Result<()> {
            unimplemented!()
        }
        fn epoch(&self) -> u64 {
            unimplemented!()
        }
        fn context(&self) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn members(&self) -> Result<Vec<Leaf>> {
            unimplemented!()
        }
    }

    const SENDER: DeviceId = [0xd1; 16];

    /// The plaintext header of a message of group `g` at `epoch` with `content_type`: a member's
    /// PublicMessage, or a PrivateMessage.
    fn message(public: bool, epoch: u64, content_type: u8) -> Vec<u8> {
        let wire_format = if public {
            wire::PUBLIC_MESSAGE
        } else {
            wire::PRIVATE_MESSAGE
        };
        let mut message = [wire::PROTOCOL_VERSION, wire_format]
            .map(u16::to_be_bytes)
            .concat();
        message.extend_from_slice(b"\x01g");
        message.extend_from_slice(&epoch.to_be_bytes());
        if public {
            message.extend_from_slice(&[1, 0, 0, 0, 0, 0]);
        }
        message.push(content_type);
        message
    }

    #[test]
    fn keeps_the_first_commit_of_epoch_zero_only() {
        let mut service = DeliveryService::<Unused>::default();
        for (message, why) in [
            (message(false, 0, wire::COMMIT), "a PrivateMessage"),
            (message(true, 0, wire::APPLICATION), "not a Commit"),
            (b"\x00\x01".to_vec(), "not an MLSMessage"),
        ] {
            assert_eq!(
                service.send(&SENDER, &message),
                Err(Refusal::NotAMember),
                "{why}"
            );
        }
        assert_eq!(
            service.send(&SENDER, &message(true, 1, wire::COMMIT)),
            Err(Refusal::EpochMismatch),
            "a Commit of epoch 1 of a new channel"
        );
        assert_eq!(service.epoch(b"g"), None);
        assert!(service.commits(b"g").is_empty());

        let first = message(true, 0, wire::COMMIT);
        assert_eq!(service.send(&SENDER, &first), Ok(()));
        assert_eq!(service.epoch(b"g"), Some(1));
        assert_eq!(service.commits(b"g"), std::slice::from_ref(&first));
        assert_eq!(service.members(b"g").unwrap(), None);
        assert_eq!(service.group_info(b"g"), None);

        // Without a GroupInfo of epoch 1, the service cannot tell a member's Commit.
        for (message, want) in [
            (first.clone(), Refusal::EpochMismatch),
            (message(true, 1, wire::COMMIT), Refusal::NotAMember),
        ] {
            assert_eq!(service.send(&SENDER, &message), Err(want));
        }
        assert_eq!(service.epoch(b"g"), Some(1));
        assert_eq!(service.commits(b"g"), [first]);
    }
}
