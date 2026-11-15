//! The scene of the tests: the fixture of storing sent envelopes with dave in its channel
//! `general` too, each person's laptop as an MLS device with its connection and session, the
//! channel's `mls` envelopes, its GroupInfo on the server, and its group as "Creating a channel"
//! forms it.

use serde_json::json;
use skald_crypto::{MlsDevice, MlsError, Processed, SigningKey, StateKey};
use skald_proto::{
    Cbor, Cert, ChannelId, DeviceId, Envelope, Header, MessageId, NonEmpty, PROTOCOL_VERSION,
    Scheme, SendKey, Unknown,
};

use crate::api::{Person, Session, assert_refused};
use crate::common::cbor::{array_of, byte_item, head, uint};
use crate::common::device_flow::{json, text};
use crate::common::devices::{assert_taken, b64, unb64};
use crate::common::{Response, Server, TestDb};
use crate::fixture::{Fixture, channel_members};
use crate::wire::{Answer, Socket, assert_nowhere, mailbox, now, send_frame};

pub const EPOCH_MISMATCH: &str = "epoch-mismatch";

/// Revokes a device: `POST`.
pub const REVOCATIONS: &str = "/account/revocations";
/// The label of a revocation's signature.
const REVOCATION_LABEL: &str = "skald revocation";

/// A person's certified laptop as an MLS device, with its connection, which states no cursor, and
/// its person's session.
pub struct Peer {
    pub id: DeviceId,
    pub mls: MlsDevice,
    signing: SigningKey,
    state_key: StateKey,
    pub socket: Socket,
    pub session: Session,
}

impl Peer {
    /// `person`'s laptop on the fixture's server.
    pub fn of(fixture: &Fixture, person: &Person) -> Self {
        Self::on(&fixture.team.flow.stand.server, fixture, person)
    }

    /// `person`'s laptop on `server`, an instance of the fixture's server.
    pub fn on(server: &Server, fixture: &Fixture, person: &Person) -> Self {
        let signing = SigningKey::from_bytes(&person.laptop.signing.secret);
        let state_key = StateKey::generate();
        Peer {
            id: person.laptop.id,
            mls: MlsDevice::new(&signing, &state_key),
            signing,
            state_key,
            socket: Socket::to(server, &fixture.team.flow, person, &person.laptop),
            session: person.session.on(server),
        }
    }

    /// Connects `person`'s laptop to `server` instead, keeping its MLS state.
    pub fn connect(&mut self, server: &Server, fixture: &Fixture, person: &Person) {
        self.socket = Socket::to(server, &fixture.team.flow, person, &person.laptop);
        self.session = person.session.on(server);
    }

    /// Another MLS device with the laptop's keys and a copy of its MLS state, as a copy of its
    /// disk holds it.
    pub fn copy(&self) -> MlsDevice {
        MlsDevice::load(&self.signing, &self.state_key, &self.mls.seal())
            .expect("the device's own sealed state")
    }

    /// Another MLS device with the laptop's keys and no MLS state, to form groups of its own.
    pub fn twin(&self) -> MlsDevice {
        MlsDevice::new(&self.signing, &StateKey::generate())
    }

    /// A new KeyPackage of the laptop.
    pub fn key_package(&mut self) -> Vec<u8> {
        let package = self.mls.key_package(now()).expect("a KeyPackage");
        package.as_ref().to_vec()
    }

    /// The laptop's GroupInfo of its current epoch of the channel.
    pub fn group_info(&self, fixture: &Fixture) -> Vec<u8> {
        (self.mls.group_info(fixture.general)).expect("a member's GroupInfo")
    }

    /// Sends `payload` in the channel's `mls` envelope of `header`; gives the envelope and the
    /// answer, checking that it names the envelope's send-key.
    pub fn send_with(&mut self, header: &Header, payload: &[u8]) -> (Vec<u8>, Answer) {
        let envelope = mls_envelope(header, payload);
        self.socket.frame(send_frame(&envelope, false));
        let answer = self.socket.answer();
        let key = match &answer {
            Answer::Stored(stored) => &stored.send_key,
            Answer::Refused(key, _) => key,
        };
        let expected = SendKey {
            msg_id: header.msg_id,
            recipient: None,
        };
        assert_eq!(*key, expected, "an mls send-key: the msg_id alone");
        (envelope, answer)
    }

    /// Sends `payload` in a new `mls` envelope of the channel to `recipients`, none for the whole
    /// channel; gives the envelope and the answer.
    pub fn send(
        &mut self,
        fixture: &Fixture,
        payload: &[u8],
        recipients: &[DeviceId],
    ) -> (Vec<u8>, Answer) {
        self.send_with(&mls_header(fixture, recipients), payload)
    }

    /// As [`Peer::send`], checking that the server stores the envelope; gives it.
    #[track_caller]
    pub fn stores(
        &mut self,
        fixture: &Fixture,
        payload: &[u8],
        recipients: &[DeviceId],
    ) -> Vec<u8> {
        self.stores_with(&mls_header(fixture, recipients), payload)
    }

    /// As [`Peer::send_with`], checking that the server stores the envelope; gives it.
    #[track_caller]
    pub fn stores_with(&mut self, header: &Header, payload: &[u8]) -> Vec<u8> {
        let (envelope, answer) = self.send_with(header, payload);
        assert!(
            matches!(answer, Answer::Stored(_)),
            "stored, not {answer:?}"
        );
        envelope
    }

    /// As [`Peer::send`], checking that the server refuses the envelope and stores nothing of it;
    /// gives the problem.
    #[track_caller]
    pub fn refusal(
        &mut self,
        fixture: &Fixture,
        payload: &[u8],
        recipients: &[DeviceId],
    ) -> String {
        let (envelope, answer) = self.send(fixture, payload, recipients);
        let Answer::Refused(_, problem) = answer else {
            panic!("send-refused, not {answer:?}");
        };
        assert_nowhere(fixture.db(), &envelope);
        problem
    }

    /// Checks that the server refuses `payload` sent as [`Peer::refusal`] does, with `problem`.
    #[track_caller]
    pub fn assert_refused(
        &mut self,
        fixture: &Fixture,
        payload: &[u8],
        recipients: &[DeviceId],
        problem: &str,
        what: &str,
    ) {
        let found = self.refusal(fixture, payload, recipients);
        assert_eq!(found, problem, "{what}");
    }

    /// The laptop creates the channel's group at epoch 0, adds the devices of `packages`, which
    /// `admitted` admits, and commits; gives the Commit, pending until the laptop merges or drops
    /// it.
    pub fn creates(
        &mut self,
        fixture: &Fixture,
        packages: &[Vec<u8>],
        admitted: &[Cert],
    ) -> Vec<u8> {
        let channel = fixture.general;
        self.mls.create_group(channel, now()).expect("a new group");
        (self.mls.add_devices(channel, packages, admitted, now())).expect("a Commit of the Adds")
    }

    /// The server stored the laptop's pending Commit: the laptop merges it; gives its Welcome, if
    /// it adds devices.
    pub fn merges(&mut self, fixture: &Fixture) -> Option<Vec<u8>> {
        (self.mls.commit_stored(fixture.general)).expect("the stored Commit merges")
    }

    /// As [`Peer::merges`], then the laptop uploads its GroupInfo of the new epoch, which the
    /// server takes; gives the Welcome, if any, and the GroupInfo.
    #[track_caller]
    pub fn merges_and_uploads(&mut self, fixture: &Fixture) -> (Option<Vec<u8>>, Vec<u8>) {
        let welcome = self.merges(fixture);
        let group_info = self.group_info(fixture);
        let response = upload(&self.session, fixture, &group_info);
        assert_uploaded(&response, "the committer's");
        (welcome, group_info)
    }

    /// Checks that the laptop's mailbox holds `welcome` alone, and joins the channel's group from
    /// it.
    #[track_caller]
    pub fn joins(&mut self, fixture: &Fixture, welcome: &[u8]) {
        let held = payloads(fixture.db(), self.id);
        assert!(
            held == [welcome],
            "the Welcome alone in the mailbox, not {} entries",
            held.len()
        );
        let joined = (self.mls.join(welcome, now(), |_| true)).expect("joins from the Welcome");
        assert_eq!(joined, fixture.general, "the channel joined");
    }

    /// The laptop processes `commit`, another device's Commit of the channel, and enters its
    /// epoch; gives the epoch.
    #[track_caller]
    pub fn process(&mut self, fixture: &Fixture, commit: &[u8]) -> u64 {
        let processed = (self
            .mls
            .process_commit(fixture.general, commit, now(), |_| true))
        .expect("the device processes the Commit");
        let Processed::Entered { epoch } = processed else {
            panic!("the device enters the Commit's epoch, not {processed:?}");
        };
        epoch
    }

    /// The laptop's Commit that `make` makes, which the server stores; the laptop merges it and
    /// uploads its GroupInfo of the new epoch, which the server takes. Gives the Commit.
    #[track_caller]
    pub fn commits(
        &mut self,
        fixture: &Fixture,
        make: impl FnOnce(&mut MlsDevice, ChannelId) -> Result<Vec<u8>, MlsError>,
    ) -> Vec<u8> {
        self.commits_with_welcome(fixture, make).0
    }

    /// As [`Peer::commits`]; gives the Commit and its Welcome, if it adds devices.
    #[track_caller]
    pub fn commits_with_welcome(
        &mut self,
        fixture: &Fixture,
        make: impl FnOnce(&mut MlsDevice, ChannelId) -> Result<Vec<u8>, MlsError>,
    ) -> (Vec<u8>, Option<Vec<u8>>) {
        let commit = make(&mut self.mls, fixture.general).expect("a Commit");
        self.stores(fixture, &commit, &[]);
        (commit, self.merges_and_uploads(fixture).0)
    }
}

/// An empty Commit of `device` in `channel`.
pub fn empty(device: &mut MlsDevice, channel: ChannelId) -> Result<Vec<u8>, MlsError> {
    device.commit_empty(channel, b"", now())
}

/// A Commit of `device` in `channel` that removes `devices`.
pub fn removing(
    devices: &[DeviceId],
) -> impl FnOnce(&mut MlsDevice, ChannelId) -> Result<Vec<u8>, MlsError> {
    move |device, channel| device.remove_devices(channel, devices, b"", now())
}

/// The header of a new `mls` envelope of the fixture's channel to `recipients`, none for the whole
/// channel.
pub fn mls_header(fixture: &Fixture, recipients: &[DeviceId]) -> Header {
    Header {
        v: PROTOCOL_VERSION,
        space: fixture.team.space,
        channel: fixture.general,
        msg_id: MessageId::generate(),
        scheme: Scheme::Mls,
        recipients: NonEmpty::from_vec(recipients.to_vec()),
        ttl: None,
        unknown: Unknown::default(),
    }
}

/// The envelope of `header` with `payload`.
pub fn mls_envelope(header: &Header, payload: &[u8]) -> Vec<u8> {
    Envelope {
        header: header.clone(),
        payload: payload.to_vec(),
    }
    .to_cbor()
}

/// The payloads of the envelopes in `device`'s mailbox, in `seq` order.
pub fn payloads(db: &TestDb, device: DeviceId) -> Vec<Vec<u8>> {
    (mailbox(db, device).into_iter())
        .map(|(_, envelope)| {
            (Envelope::from_cbor(&envelope).expect("an envelope that decodes")).payload
        })
        .collect()
}

/// The channel's GroupInfo on the server.
fn group_info_path(fixture: &Fixture) -> String {
    format!(
        "/spaces/{}/channels/{}/group-info",
        fixture.team.space, fixture.general
    )
}

/// `PUT` of `group_info` as the channel's GroupInfo.
pub fn upload(session: &Session, fixture: &Fixture, group_info: &[u8]) -> Response {
    let body = json!({ "group_info": b64(group_info) });
    session.call("put", &group_info_path(fixture), Some(&body))
}

/// `GET` of the channel's GroupInfo.
pub fn latest(session: &Session, fixture: &Fixture) -> Response {
    session.call("get", &group_info_path(fixture), None)
}

/// What `session` gets of the channel's group: whether it has one, and its GroupInfo if the server
/// has one of the current epoch.
#[track_caller]
pub fn latest_of(session: &Session, fixture: &Fixture) -> (bool, Option<Vec<u8>>) {
    let response = latest(session, fixture);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let group = body["group"]
        .as_bool()
        .unwrap_or_else(|| panic!("no group in {body}"));
    let group_info = match &body["group_info"] {
        serde_json::Value::Null => None,
        bytes => Some(unb64(
            bytes
                .as_str()
                .unwrap_or_else(|| panic!("a GroupInfo in {body}")),
        )),
    };
    (group, group_info)
}

/// Checks that `session` gets `group_info` as the channel's GroupInfo.
#[track_caller]
pub fn assert_latest(session: &Session, fixture: &Fixture, group_info: &[u8], what: &str) {
    let found = latest_of(session, fixture);
    assert!(
        found == (true, Some(group_info.to_vec())),
        "{what}: the GroupInfo given is not the one expected: {found:?}"
    );
}

/// Checks that `response` takes an upload.
#[track_caller]
pub fn assert_uploaded(response: &Response, what: &str) {
    assert!(
        (200..300).contains(&response.status),
        "{what} GroupInfo taken: {}",
        text(response)
    );
}

/// Checks that the server refuses `group_info` uploaded on `session`.
#[track_caller]
pub fn assert_upload_refused(session: &Session, fixture: &Fixture, group_info: &[u8], what: &str) {
    assert_refused(&upload(session, fixture, group_info), what);
}

/// The test's people and their laptops: alice, the space's admin, and bob, carol and dave, members
/// of the channel, and eve, outside the space.
pub struct Scene {
    pub fixture: Fixture,
    pub alice: Peer,
    pub bob: Peer,
    pub carol: Peer,
    pub dave: Peer,
    pub eve: Peer,
}

/// What forming the channel's group gave: alice's Commit of epoch 0, its Welcome and her GroupInfo
/// of epoch 1.
pub struct Formed {
    pub commit: Vec<u8>,
    pub welcome: Vec<u8>,
    pub group_info: Vec<u8>,
}

impl Scene {
    /// The scene, the channel not formed; GitHub user IDs from `github` on.
    pub fn new(github: u64) -> Self {
        Self::with(github, &[])
    }

    /// As [`Scene::new`], on a server with the extra arguments `args`.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let mut fixture = Fixture::with(github, args);
        let t = &fixture.team;
        let accounts = [
            t.alice.account,
            t.bob.account,
            t.carol.account,
            t.dave.account,
        ];
        let general = fixture.general;
        fixture.revise(|body| channel_members(body, general, &accounts));
        let t = &fixture.team;
        let [alice, bob, carol, dave, eve] = [&t.alice, &t.bob, &t.carol, &t.dave, &fixture.eve]
            .map(|person| Peer::of(&fixture, person));
        Scene {
            fixture,
            alice,
            bob,
            carol,
            dave,
            eve,
        }
    }

    /// The scene with the channel's group formed: alice creates it with bob and carol, as
    /// [`Scene::form`] does. dave is a member of the channel, but not of its group.
    pub fn formed(github: u64) -> (Self, Formed) {
        Self::formed_with(github, &[])
    }

    /// As [`Scene::formed`], on a server with the extra arguments `args`.
    pub fn formed_with(github: u64, args: &[&str]) -> (Self, Formed) {
        let mut scene = Self::with(github, args);
        let formed = scene.form();
        (scene, formed)
    }

    /// The valid certificates of the channel members' laptops.
    pub fn admitted(&self) -> Vec<Cert> {
        let t = &self.fixture.team;
        [&t.alice, &t.bob, &t.carol, &t.dave]
            .map(|person| Cert::from_cbor(&person.cert).expect("a certificate"))
            .to_vec()
    }

    /// alice creates the channel's group at epoch 0, adds bob and carol from their KeyPackages and
    /// commits; gives her Commit, not sent yet.
    pub fn creates(&mut self) -> Vec<u8> {
        let packages = [self.bob.key_package(), self.carol.key_package()];
        let admitted = self.admitted();
        self.alice.creates(&self.fixture, &packages, &admitted)
    }

    /// Forms the channel's group: alice creates it as [`Scene::creates`] does; the server stores
    /// the Commit; alice uploads her GroupInfo of epoch 1, then sends the Welcome to bob and carol,
    /// who join from it.
    #[track_caller]
    pub fn form(&mut self) -> Formed {
        let commit = self.creates();
        let f = &self.fixture;
        self.alice.stores(f, &commit, &[]);
        let (welcome, group_info) = self.alice.merges_and_uploads(f);
        let welcome = welcome.expect("a Welcome");
        self.alice
            .stores(f, &welcome, &[self.bob.id, self.carol.id]);
        for peer in [&mut self.bob, &mut self.carol] {
            peer.joins(f, &welcome);
        }
        Formed {
            commit,
            welcome,
            group_info,
        }
    }

    /// Checks that alice, bob and carol hold the channel's group with them as its members, in leaf
    /// order.
    #[track_caller]
    pub fn assert_formed(&self) {
        let all = [self.alice.id, self.bob.id, self.carol.id];
        for peer in [&self.alice, &self.bob, &self.carol] {
            let members = (peer.mls.members(self.fixture.general)).expect("the group");
            let devices: Vec<DeviceId> = members.iter().map(|member| member.device).collect();
            assert_eq!(devices, all, "{}'s members", peer.id);
        }
    }

    /// alice adds dave: the server stores her Commit, she uploads her GroupInfo of the new epoch
    /// and sends the Welcome to dave, who joins from it; bob and carol process the Commit. Gives the
    /// Commit and the Welcome.
    #[track_caller]
    pub fn add_dave(&mut self) -> (Vec<u8>, Vec<u8>) {
        let package = self.dave.key_package();
        let admitted = self.admitted();
        let (commit, welcome) = self
            .alice
            .commits_with_welcome(&self.fixture, |device, channel| {
                device.add_devices(channel, &[&package], &admitted, now())
            });
        let welcome = welcome.expect("a Welcome");
        self.alice.stores(&self.fixture, &welcome, &[self.dave.id]);
        self.dave.joins(&self.fixture, &welcome);
        for peer in [&mut self.bob, &mut self.carol] {
            peer.process(&self.fixture, &commit);
        }
        (commit, welcome)
    }
}

/// Revokes `person`'s laptop by a revocation its master key signs, which the server takes.
#[track_caller]
pub fn revoke(person: &Person) {
    let body = [
        head(5, 3),
        uint(1),
        byte_item(person.account.as_bytes()),
        uint(2),
        byte_item(person.laptop.id.as_bytes()),
        uint(5),
        uint(now()),
    ]
    .concat();
    let signature = person.master.key.sign(REVOCATION_LABEL, &[&body]);
    let revocation = array_of(&[byte_item(&body), byte_item(&signature)]);
    let request = json!({ "revocation": b64(&revocation) });
    assert_taken(&person.session.call("post", REVOCATIONS, Some(&request)));
}
