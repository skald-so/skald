//! A person's device the tests play as its own client would, with its machine's keys and an MLS
//! state of the tests' own: it keeps KeyPackages it made on the server, goes through its mailbox
//! in order, joining from Welcomes, processing Commits and decrypting application messages, and
//! sends `mls` envelopes.

use serde_json::json;
use skald_crypto::{Decrypted, GroupChange, Joining, MlsContent, MlsDevice, MlsFraming, StateKey};
use skald_proto::{
    Cbor as _, ChannelId, DeviceId, Envelope, Frame, Header, MessageId, NonEmpty, PROTOCOL_VERSION,
    Push, RawEnvelope, Scheme, SpaceId, Unknown,
};

use crate::answers::handed_of;
use crate::cloud::Cloud;
use crate::fixtures::cert_of;
use crate::kit::{b64, upload_group_info};
use crate::peer::Peer;
use crate::world::{Person, now, post_as};

/// How many KeyPackages, beside its last-resort one, a played device keeps on the server.
const KEY_PACKAGES: usize = 3;

/// A Welcome or Commit a played device took from its mailbox, and what it did with it.
#[derive(Debug, Clone)]
pub enum Took {
    /// It joined a channel's group from a Welcome.
    Joined(Joining),
    /// It entered a new epoch of a channel's group from another device's Commit.
    Changed(ChannelId, GroupChange),
    /// A Commit removed it from a channel's group.
    Removed(ChannelId),
    /// It decrypted an application message of a channel's group, of the envelope's `msg_id`.
    Read(ChannelId, MessageId, Decrypted),
    /// It could not use the entry.
    Failed(String),
}

/// A person's device, played by the test.
pub struct Played {
    pub person: Person,
    pub mls: MlsDevice,
    /// The highest `seq` it acked.
    cursor: u64,
    /// Everything it took from its mailbox so far, in order.
    pub took: Vec<Took>,
}

impl Played {
    /// `person`'s machine's device, whose KeyPackages on the server the test replaces by ones the
    /// played device made, and a last-resort one.
    pub fn new(cloud: &Cloud, person: Person) -> Self {
        without_key_packages(cloud, person.device);
        let mut mls = MlsDevice::new(&person.key, &StateKey::generate());
        let made: Vec<String> = (0..KEY_PACKAGES)
            .map(|_| b64(mls.key_package(now()).expect("a KeyPackage").as_ref()))
            .collect();
        let last = mls
            .last_resort_key_package(now())
            .expect("a last-resort KeyPackage");
        let request = json!({ "key_packages": made, "last_resort": b64(last.as_ref()) });
        let path = format!("/devices/{}/key-packages", person.device);
        let (status, body) = post_as(cloud, &person.token, &path, Some(&request));
        assert_eq!(status, 200, "POST {path}: {body}");
        Played {
            person,
            mls,
            cursor: 0,
            took: Vec::new(),
        }
    }

    pub fn device(&self) -> DeviceId {
        self.person.device
    }

    /// The device's connection.
    pub fn connect(&self, cloud: &Cloud) -> Peer {
        Peer::connect(cloud, &self.person, &self.person.key)
    }

    /// Goes through the device's mailbox from its cursor in order, joining from each Welcome,
    /// processing each Commit and decrypting each application message, and acks it; gives what it
    /// took this time.
    pub fn take_in(&mut self, cloud: &Cloud) -> Vec<Took> {
        let mut peer = self.connect(cloud);
        let entries = peer.catch_up(self.cursor);
        let mut took = Vec::new();
        for push in &entries {
            match push {
                Push::Message { seq, envelope } => {
                    self.cursor = self.cursor.max(seq.get());
                    if let Some(one) = self.take(envelope) {
                        took.push(one);
                    }
                }
                Push::Status { seq, .. } => self.cursor = self.cursor.max(seq.get()),
                _ => {}
            }
        }
        if self.cursor > 0 {
            peer.ack(self.cursor);
        }
        self.took.extend(took.iter().cloned());
        took
    }

    /// Takes `envelopes` in order, as a compromised server hands them to the device beside its
    /// mailbox, as it takes its mailbox's entries; gives what it took of each, `None` for one that
    /// is no `mls` envelope.
    pub fn handed(&mut self, envelopes: &[RawEnvelope]) -> Vec<Option<Took>> {
        let took: Vec<Option<Took>> = envelopes.iter().map(|one| self.take(one)).collect();
        self.took.extend(took.iter().flatten().cloned());
        took
    }

    fn take(&mut self, envelope: &RawEnvelope) -> Option<Took> {
        let Envelope { header, payload } = envelope.envelope().clone();
        if header.scheme != Scheme::Mls {
            return None;
        }
        Some(match MlsFraming::read(&payload) {
            Some(MlsFraming::Welcome) => {
                let mut joining = None;
                match self.mls.join(&payload, now(), |seen| {
                    joining = Some(seen.clone());
                    true
                }) {
                    Ok(_) => Took::Joined(joining.expect("the Welcome's group")),
                    Err(err) => Took::Failed(format!("a Welcome: {err:?}")),
                }
            }
            Some(MlsFraming::Message {
                content: MlsContent::Commit,
                ..
            }) => {
                let channel = header.channel;
                let mut change = None;
                match self.mls.process_commit(channel, &payload, now(), |seen| {
                    change = Some(seen.clone());
                    true
                }) {
                    Ok(skald_crypto::Processed::Removed) => Took::Removed(channel),
                    Ok(_) => Took::Changed(channel, change.expect("the Commit's change")),
                    Err(err) => Took::Failed(format!("a Commit of {channel}: {err:?}")),
                }
            }
            Some(MlsFraming::Message {
                content: MlsContent::Application,
                ..
            }) => {
                let channel = header.channel;
                match self.mls.decrypt(channel, &payload) {
                    Ok(read) => Took::Read(channel, header.msg_id, read),
                    Err(err) => Took::Failed(format!("a message of {channel}: {err:?}")),
                }
            }
            _ => return None,
        })
    }

    /// The channels whose groups it joined from Welcomes, with what each Welcome showed.
    pub fn joined(&self) -> Vec<Joining> {
        self.took
            .iter()
            .filter_map(|took| match took {
                Took::Joined(joining) => Some(joining.clone()),
                _ => None,
            })
            .collect()
    }

    /// The Welcomes of `channel` it joined from.
    pub fn joined_to(&self, channel: ChannelId) -> Vec<Joining> {
        self.joined()
            .into_iter()
            .filter(|joining| joining.channel == channel)
            .collect()
    }

    /// The changes of `channel`'s group it entered from other devices' Commits, in order.
    pub fn changes_of(&self, channel: ChannelId) -> Vec<GroupChange> {
        self.took
            .iter()
            .filter_map(|took| match took {
                Took::Changed(of, change) if *of == channel => Some(change.clone()),
                _ => None,
            })
            .collect()
    }

    /// The application messages of `channel` it decrypted, in order, each with its envelope's
    /// `msg_id`.
    pub fn read_in(&self, channel: ChannelId) -> Vec<(MessageId, Decrypted)> {
        self.took
            .iter()
            .filter_map(|took| match took {
                Took::Read(of, msg_id, read) if *of == channel => Some((*msg_id, read.clone())),
                _ => None,
            })
            .collect()
    }

    /// Whether a Commit removed it from `channel`'s group.
    pub fn removed_from(&self, channel: ChannelId) -> bool {
        self.took
            .iter()
            .any(|took| matches!(took, Took::Removed(of) if *of == channel))
    }

    /// The member devices of `channel`'s group as the device holds it.
    pub fn members(&self, channel: ChannelId) -> Vec<DeviceId> {
        self.mls
            .members(channel)
            .unwrap_or_else(|err| panic!("no group of {channel}: {err:?}"))
            .iter()
            .map(|member| member.device)
            .collect()
    }

    /// Sends `payload` in a new `mls` envelope of `channel` in `space` to `recipients`, none for the
    /// whole channel; gives the server's answer.
    pub fn send(
        &self,
        cloud: &Cloud,
        (space, channel): (SpaceId, ChannelId),
        payload: &[u8],
        recipients: &[DeviceId],
    ) -> Frame {
        let envelope = mls_envelope((space, channel), payload, recipients);
        self.connect(cloud).send(&envelope)
    }

    /// As [`Played::send`]; fails unless the server stores the envelope.
    #[track_caller]
    pub fn stores(
        &self,
        cloud: &Cloud,
        at: (SpaceId, ChannelId),
        payload: &[u8],
        recipients: &[DeviceId],
    ) {
        let answer = self.send(cloud, at, payload, recipients);
        assert!(matches!(answer, Frame::Stored(_)), "not stored: {answer:?}");
    }

    /// The device commits `commit`, which it made in `channel`'s group; once the server stores it,
    /// it merges it and gives its Welcome, if any. The device uploads no GroupInfo.
    #[track_caller]
    pub fn commits(
        &mut self,
        cloud: &Cloud,
        at: (SpaceId, ChannelId),
        commit: &[u8],
    ) -> Option<Vec<u8>> {
        self.stores(cloud, at, commit, &[]);
        self.mls
            .commit_stored(at.1)
            .expect("the stored Commit merges")
    }

    /// The device adds `person`'s device to `channel`'s group with one of its KeyPackages on the
    /// server, commits, uploads the new epoch's GroupInfo and sends the device the Welcome; fails
    /// unless the server stores each.
    #[track_caller]
    pub fn welcomes(&mut self, cloud: &Cloud, at: (SpaceId, ChannelId), person: &Person) {
        let package = take_key_package(cloud, &self.person.token, person.device);
        let commit = (self.mls)
            .add_devices(at.1, &[package], &[cert_of(cloud, person)], now())
            .expect("a Commit adding the device");
        let welcome = self.commits(cloud, at, &commit).expect("a Welcome");
        self.uploads_group_info(cloud, at);
        self.stores(cloud, at, &welcome, &[person.device]);
    }

    /// The device uploads its GroupInfo of `channel`'s current epoch; fails unless the server takes
    /// it.
    #[track_caller]
    pub fn uploads_group_info(&self, cloud: &Cloud, at: (SpaceId, ChannelId)) {
        let group_info = self.mls.group_info(at.1).expect("a member's GroupInfo");
        let status = upload_group_info(cloud, &self.person.token, at, &group_info);
        assert!(
            (200..300).contains(&status),
            "GroupInfo not taken: {status}"
        );
    }

    /// The device creates `channel`'s group with no other device and commits; once the server
    /// stores the Commit, it merges it, and uploads no GroupInfo.
    #[track_caller]
    pub fn creates_alone(&mut self, cloud: &Cloud, at: (SpaceId, ChannelId)) {
        self.mls.create_group(at.1, now()).expect("a new group");
        let none: [Vec<u8>; 0] = [];
        let commit = (self.mls.add_devices(at.1, &none, &[], now())).expect("a Commit");
        self.commits(cloud, at, &commit);
    }
}

/// A new `mls` envelope of `channel` in `space` with `payload`, to `recipients`, none for the whole
/// channel.
pub fn mls_envelope(
    (space, channel): (SpaceId, ChannelId),
    payload: &[u8],
    recipients: &[DeviceId],
) -> RawEnvelope {
    let header = Header {
        v: PROTOCOL_VERSION,
        space,
        channel,
        msg_id: MessageId::generate(),
        scheme: Scheme::Mls,
        recipients: NonEmpty::from_vec(recipients.to_vec()),
        ttl: None,
        unknown: Unknown::default(),
    };
    let bytes = Envelope {
        header,
        payload: payload.to_vec(),
    }
    .to_cbor();
    RawEnvelope::from_bytes(bytes).expect("an envelope")
}

/// One KeyPackage of `device` that the server hands the session `token`.
#[track_caller]
pub fn take_key_package(cloud: &Cloud, token: &str, device: DeviceId) -> Vec<u8> {
    let request = json!({ "devices": [device.to_string()] });
    let (status, body) = post_as(cloud, token, "/key-packages", Some(&request));
    assert_eq!(status, 200, "POST /key-packages: {body}");
    match handed_of(&body).as_slice() {
        [(of, bytes)] if *of == device => bytes.clone(),
        _ => panic!("not one KeyPackage of {device}: {body}"),
    }
}

/// A device of no account, which no revision admits: an MLS device with a key of its own.
pub fn stranger() -> MlsDevice {
    MlsDevice::new(&skald_crypto::SigningKey::generate(), &StateKey::generate())
}

/// The Welcome of a group of `channel` that `signer` forms, adding `person`'s device with one of
/// its KeyPackages that the session `token` takes from the server.
#[track_caller]
pub fn welcome_from(
    cloud: &Cloud,
    signer: &mut MlsDevice,
    channel: ChannelId,
    (person, token): (&Person, &str),
) -> Vec<u8> {
    let key_package = take_key_package(cloud, token, person.device);
    let cert = cert_of(cloud, person);
    signer.create_group(channel, now()).expect("a group");
    (signer.add_devices(channel, &[key_package], &[cert], now())).expect("a Commit");
    (signer.commit_stored(channel))
        .expect("the Commit merges")
        .expect("a Welcome")
}

/// Removes every KeyPackage of `device` from the server, so no device can add it to a group.
pub fn without_key_packages(cloud: &Cloud, device: DeviceId) {
    cloud.execute(&format!(
        "DELETE FROM key_packages WHERE device_id = '{device}'::uuid"
    ));
}
