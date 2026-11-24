//! Unmeasured synthetic accounts, signed rosters and an MLS channel.

use std::net::SocketAddrV4;
use std::num::NonZeroU64;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use skald_crypto::{
    HpkeKey, MlsDevice, RecoveryKey, SigningKey, StateKey, issue, new_recovery, sign_account,
    sign_roster,
};
use skald_github_stand_in::{GitHub, User};
use skald_proto::{
    AUTH_LABEL, AccountId, Address, Body, Cbor, Cert, CertBody, Channel, ChannelId, ChannelMember,
    Content, DeviceId, DeviceKind, Envelope, Frame, Header, HistoryBody, MessageId, Name, NonEmpty,
    PROTOCOL_VERSION, Push, RawEnvelope, Role, RosterBody, Scheme, SpaceId,
};

use crate::http::{Http, b64, json as read_json, text, unb64};
use crate::socket::Socket;
use crate::{FIXTURE_SERVER_NAME, Result};

const SPACE_NAME: &str = "load-test";

pub struct Device {
    pub id: DeviceId,
    pub token: String,
    pub signing: SigningKey,
    account: AccountId,
    master: SigningKey,
    handle: Name,
    cert: Cert,
    mls: MlsDevice,
    counter: u64,
}

#[derive(Clone, Copy)]
pub struct ChannelAt {
    pub space: SpaceId,
    pub id: ChannelId,
}

impl ChannelAt {
    pub fn envelope(
        self,
        msg_id: MessageId,
        payload: Vec<u8>,
        recipients: Option<NonEmpty<DeviceId>>,
    ) -> Result<RawEnvelope> {
        let header = Header {
            v: PROTOCOL_VERSION,
            space: self.space,
            channel: self.id,
            msg_id,
            scheme: Scheme::Mls,
            recipients,
            ttl: None,
            unknown: Default::default(),
        };
        Ok(RawEnvelope::from_bytes(
            Envelope { header, payload }.to_cbor(),
        )?)
    }

    pub fn path(self, suffix: &str) -> String {
        format!("/spaces/{}/channels/{}{suffix}", self.space, self.id)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

impl Device {
    fn create(http: &Http, github: &GitHub, index: usize) -> Result<Self> {
        let handle = format!("load-{index}");
        let token = http.sign_in(github, User::new(index as u64 + 1, &handle))?;
        let who = read_json(http.api("GET", "/session", &token, None)?)?;
        let account = text(&who, "account")?.parse()?;
        let signing = SigningKey::generate();
        let id = DeviceId::from_signing_key(&signing.public_key());
        let hpke = HpkeKey::generate().public_key();
        let registration = json!({
            "device": id.to_string(),
            "kind": "client",
            "signing": b64(&signing.public_key()),
            "hpke": b64(&hpke),
            "name": &handle,
        });
        http.api("POST", "/devices", &token, Some(&registration))?;
        let master = SigningKey::generate();
        let body = CertBody {
            account,
            device: id,
            kind: DeviceKind::Client,
            signing: signing.public_key(),
            hpke,
            issuer: None,
            issued: now(),
            ssh: None,
            github: None,
            name: handle.clone(),
            unknown: Default::default(),
        };
        let cert = issue(&body, &master, &signing)?;
        let (account_body, backup) = new_recovery(&master, &RecoveryKey::generate(), account);
        let record = sign_account(&account_body, &master)?;
        let request = json!({
            "master": b64(&master.public_key()),
            "cert": b64(&cert.to_cbor()),
            "record": b64(&record.to_cbor()),
            "backup": b64(&backup.to_cbor()),
        });
        http.api("PUT", "/account/master", &token, Some(&request))?;
        let mls = MlsDevice::new(&signing, &StateKey::generate());
        Ok(Self {
            id,
            token,
            signing,
            account,
            master,
            handle: handle.parse()?,
            cert,
            mls,
            counter: 0,
        })
    }

    fn revision(&self, http: &Http, roster: &RosterBody) -> Result<[u8; 32]> {
        let signed = sign_roster(roster, &self.signing)?;
        let request = json!({ "revision": b64(&signed.to_cbor()) });
        http.api(
            "POST",
            &format!("/spaces/{}/revisions", roster.space),
            &self.token,
            Some(&request),
        )?;
        Ok(signed.hash())
    }

    fn packages(&mut self, http: &Http) -> Result<()> {
        let package = self.mls.key_package(now())?;
        let last_resort = self.mls.last_resort_key_package(now())?;
        let request = json!({
            "key_packages": [b64(package.as_ref())],
            "last_resort": b64(last_resort.as_ref()),
        });
        http.api(
            "POST",
            &format!("/devices/{}/key-packages", self.id),
            &self.token,
            Some(&request),
        )?;
        Ok(())
    }

    async fn connect(&self, http: &Http, address: SocketAddrV4) -> Result<Socket> {
        let response = read_json(http.api("POST", "/session/challenge", &self.token, None)?)?;
        let challenge = unb64(text(&response, "challenge")?)?;
        let signature = self
            .signing
            .sign(AUTH_LABEL, &[&challenge, http.origin.as_bytes()]);
        let mut socket = Socket::connect(address).await?;
        socket
            .ready(Frame::Auth {
                challenge,
                device: self.id,
                signature,
            })
            .await?;
        Ok(socket)
    }

    pub fn message(&mut self, at: ChannelAt, text: &str) -> Result<(RawEnvelope, Vec<u8>)> {
        self.counter += 1;
        let msg_id = MessageId::generate();
        let content = Content {
            from: Address::person(
                SPACE_NAME.parse()?,
                self.handle.clone(),
                FIXTURE_SERVER_NAME.parse()?,
            ),
            to: None,
            counter: NonZeroU64::new(self.counter),
            hops: 0,
            reply_to: None,
            body: Some(Body::A2a(
                json!({
                    "messageId": msg_id.to_string(),
                    "role": "ROLE_AGENT",
                    "parts": [{ "text": text }],
                })
                .to_string(),
            )),
            delegation: None,
            grant: None,
            attachments: None,
            unknown: Default::default(),
        };
        let payload = self.mls.encrypt(at.id, &content.to_cbor())?;
        let epoch = self.mls.epoch(at.id)?;
        let history = self
            .mls
            .history_key(at.id, epoch)
            .ok_or("missing history key")?;
        let body = HistoryBody {
            sender: self.id,
            channel: at.id,
            msg_id,
            timestamp: now(),
            content,
            unknown: Default::default(),
        };
        let entry = history.seal_entry(epoch, &body, &self.signing)?.to_cbor();
        Ok((at.envelope(msg_id, payload, None)?, entry))
    }
}

fn space(http: &Http, devices: &[Device]) -> Result<ChannelAt> {
    let owner = &devices[0];
    let value = read_json(http.api(
        "POST",
        "/spaces",
        &owner.token,
        Some(&json!({ "name": SPACE_NAME })),
    )?)?;
    let space = text(&value, "space")?.parse()?;
    let mut roster = RosterBody::first(
        space,
        SPACE_NAME.parse()?,
        owner.account,
        owner.handle.clone(),
        owner.master.public_key(),
        owner.id,
        now(),
    );
    let previous = owner.revision(http, &roster)?;
    let value = read_json(http.api(
        "POST",
        &format!("/spaces/{space}/channels"),
        &owner.token,
        None,
    )?)?;
    let id = text(&value, "channel")?.parse()?;
    roster.revision += 1;
    roster.prev = Some(previous);
    for member in &devices[1..] {
        roster.members.push(skald_proto::Member {
            account: member.account,
            handle: member.handle.clone(),
            role: Role::Member,
            master: member.master.public_key(),
            invite: None,
            excluded: None,
            unknown: Default::default(),
        });
    }
    roster.channels.push(Channel {
        id,
        members: devices
            .iter()
            .map(|device| ChannelMember {
                account: device.account,
                history: false,
            })
            .collect(),
        services: Vec::new(),
        unknown: Default::default(),
    });
    owner.revision(http, &roster)?;
    Ok(ChannelAt { space, id })
}

async fn stored(socket: &mut Socket, envelope: RawEnvelope) -> Result<()> {
    let msg_id = envelope.envelope().header.msg_id;
    socket
        .send(&Frame::Send {
            envelope,
            report: false,
        })
        .await?;
    match socket
        .within(|frame| {
            matches!(frame, Frame::Stored(stored) if stored.send_key.msg_id == msg_id)
                || matches!(frame, Frame::SendRefused { send_key, .. } if send_key.msg_id == msg_id)
        })
        .await?
    {
        Frame::Stored(_) => Ok(()),
        Frame::SendRefused { problem, .. } => {
            Err(format!("the MLS setup send was refused: {problem}").into())
        }
        _ => unreachable!("only answers to this send matched"),
    }
}

async fn form(
    http: &Http,
    at: ChannelAt,
    devices: &mut [Device],
    sockets: &mut [Socket],
) -> Result<()> {
    let admitted: Vec<_> = devices.iter().map(|device| device.cert.clone()).collect();
    let (owner, joiners) = devices.split_first_mut().ok_or("missing group owner")?;
    let ids: Vec<_> = joiners.iter().map(|device| device.id.to_string()).collect();
    for joiner in joiners.iter_mut() {
        joiner.packages(http)?;
    }
    let response = read_json(http.api(
        "POST",
        "/key-packages",
        &owner.token,
        Some(&json!({ "devices": ids })),
    )?)?;
    let packages = response["key_packages"]
        .as_array()
        .ok_or("missing key packages")?
        .iter()
        .map(|package| unb64(text(package, "key_package")?))
        .collect::<Result<Vec<_>>>()?;
    if packages.len() != joiners.len() {
        return Err("the fixture did not provide every KeyPackage".into());
    }
    owner.mls.create_group(at.id, now())?;
    let commit = owner.mls.add_devices(at.id, &packages, &admitted, now())?;
    stored(
        &mut sockets[0],
        at.envelope(MessageId::generate(), commit, None)?,
    )
    .await?;
    let welcome = owner
        .mls
        .commit_stored(at.id)?
        .ok_or("MLS Add did not make a Welcome")?;
    let group_info = owner.mls.group_info(at.id)?;
    http.api(
        "PUT",
        &at.path("/group-info"),
        &owner.token,
        Some(&json!({ "group_info": b64(&group_info) })),
    )?;
    let members = NonEmpty::from_vec(joiners.iter().map(|device| device.id).collect());
    let welcome_id = MessageId::generate();
    stored(&mut sockets[0], at.envelope(welcome_id, welcome, members)?).await?;

    for (device, socket) in joiners.iter_mut().zip(&mut sockets[1..]) {
        let frame = socket
            .within(|frame| {
                matches!(frame, Frame::Push(Push::Message { envelope, .. })
            if envelope.envelope().header.msg_id == welcome_id)
            })
            .await?;
        let Frame::Push(Push::Message { seq, envelope }) = frame else {
            unreachable!()
        };
        let joined = device
            .mls
            .join(&envelope.envelope().payload, now(), |joining| {
                joining.channel == at.id
                    && joining.signer.device == owner.id
                    && joining.members.len() == admitted.len()
                    && joining.members.iter().all(|member| {
                        admitted.iter().any(|cert| {
                            cert.body().device == member.device
                                && cert.body().signing == member.signing
                        })
                    })
            })?;
        if joined != at.id {
            return Err("the Welcome joined a different channel".into());
        }
        socket.send(&Frame::Ack { seq }).await?;
    }
    Ok(())
}

pub async fn prepare(
    address: SocketAddrV4,
    http: &Http,
    github: &GitHub,
    count: usize,
) -> Result<(ChannelAt, Vec<Device>, Vec<Socket>)> {
    let mut devices = Vec::with_capacity(count);
    for index in 0..count {
        devices.push(Device::create(http, github, index)?);
    }
    let at = space(http, &devices)?;
    let mut sockets = Vec::with_capacity(count);
    for device in &devices {
        sockets.push(device.connect(http, address).await?);
    }
    form(http, at, &mut devices, &mut sockets).await?;
    Ok((at, devices, sockets))
}
