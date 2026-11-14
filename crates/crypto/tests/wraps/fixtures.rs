//! Devices, keys, certificates, a channel's group and wraps made apart from the wrapping under test:
//! sealed with the single-shot HPKE seal, encoded with the tests' CBOR writer and signed over
//! `[label, body]` with the tests' keys.

use skald_crypto::{HistoryKey, HpkeKey, MlsDevice, issue, open_wrap};
use skald_proto::{
    AccountId, Cbor, Cert, CertBody, ChannelId, DeviceId, DeviceKind, HistoryWrap, Unknown,
    WrapRecipient,
};

use crate::cbor::{Value, bytes};
use crate::fields::signed_cbor;

use crate::devices::state_key;
pub use crate::devices::{NOW, device_id, signing_key};

/// The label of a wrap's signature.
pub const LABEL: &str = "skald wrap";
/// `info` of a wrap's HPKE seal.
pub const INFO: &[u8] = b"skald history wrap";
/// The account of the tests' devices.
pub const ACCOUNT: AccountId = AccountId::from_bytes([0xac; 16]);
/// Another account.
pub const OTHER_ACCOUNT: AccountId = AccountId::from_bytes([0xad; 16]);
/// The account's recovery key, 32 bytes as the user enters it.
pub const RECOVERY: [u8; 32] = [0x7e; 32];

/// The seed of the master key that issues the certificates of client devices.
const MASTER: u8 = 255;
/// The client device that issues the certificates of browsers.
const ISSUER: u8 = 1;

fn issued(seed: u8, kind: DeviceKind, hpke: [u8; 32], issuer: Option<u8>) -> Cert {
    let body = CertBody {
        account: ACCOUNT,
        device: device_id(seed),
        kind,
        signing: signing_key(seed).public_key(),
        hpke,
        issuer: issuer.map(device_id),
        issued: NOW,
        ssh: None,
        name: format!("Device {seed}"),
        unknown: Unknown::default(),
    };
    let issuer_key = signing_key(issuer.unwrap_or(MASTER));
    issue(&body, &issuer_key, &signing_key(seed)).expect("a test device's certificate")
}

/// The valid certificate of client device `seed`, as the caller has checked it.
pub fn cert(seed: u8) -> Cert {
    issued(
        seed,
        DeviceKind::Client,
        HpkeKey::from_bytes(&[seed; 32]).public_key(),
        None,
    )
}

/// A certificate of client device `seed` that names the signing key of device `key`, as no valid
/// certificate does.
pub fn cert_with_key_of(seed: u8, key: u8) -> Cert {
    let body = CertBody {
        signing: signing_key(key).public_key(),
        ..cert(seed).body().clone()
    };
    issue(&body, &signing_key(MASTER), &signing_key(key)).expect("a certificate")
}

/// The HPKE key of browser `seed`, as it comes from its key storage.
pub fn browser_key(seed: u8) -> HpkeKey {
    HpkeKey::from_bytes(&[seed; 32])
}

/// The valid certificate of browser `seed`, issued by a client device of the account.
pub fn browser_cert(seed: u8) -> Cert {
    issued(
        seed,
        DeviceKind::Browser,
        browser_key(seed).public_key(),
        Some(ISSUER),
    )
}

/// The recipient browser `seed` of a wrap.
pub fn browser(seed: u8) -> WrapRecipient {
    WrapRecipient::Browser(device_id(seed))
}

/// The recovery HPKE key of `recovery`.
pub fn recovery_key(recovery: &[u8; 32]) -> HpkeKey {
    HpkeKey::from_recovery_key(recovery)
}

/// A recipient of a wrap with its X25519 public key and the HPKE key it opens wraps with.
pub struct Reader {
    pub recipient: WrapRecipient,
    pub public: [u8; 32],
    pub key: HpkeKey,
}

/// Browser `seed`, with the `hpke` key of its certificate.
pub fn browser_reader(seed: u8) -> Reader {
    Reader {
        recipient: browser(seed),
        public: browser_cert(seed).body().hpke,
        key: browser_key(seed),
    }
}

/// The recovery key `recovery` of `account`.
pub fn recovery_reader(account: AccountId, recovery: &[u8; 32]) -> Reader {
    Reader {
        recipient: WrapRecipient::Recovery(account),
        public: recovery_key(recovery).public_key(),
        key: recovery_key(recovery),
    }
}

/// The readers of the tests: browser 50 and the account's recovery key.
pub fn readers() -> [Reader; 2] {
    [browser_reader(50), recovery_reader(ACCOUNT, &RECOVERY)]
}

/// The certificates of the devices a reader knows, the signers of its wraps among them.
pub fn known() -> Vec<Cert> {
    (0..100).map(cert).collect()
}

/// What a reader gets from `wrap` with `key`, as the caller does it: the epoch's key once it opens
/// and its signature verifies with the certificate of its `signer` among the known devices; the
/// first refusal otherwise.
pub fn read(wrap: &HistoryWrap, key: &HpkeKey) -> Result<[u8; 16], String> {
    let opened = open_wrap(wrap, key).map_err(|error| format!("open: {error:?}"))?;
    let signer = opened.signer();
    let cert = known()
        .into_iter()
        .find(|cert| cert.body().device == signer)
        .ok_or_else(|| format!("no certificate of {signer}"))?;
    opened
        .verify(&cert)
        .map(|key| *key.to_bytes())
        .map_err(|error| format!("verify: {error:?}"))
}

/// A channel's group of devices held in memory, each step on all of them.
pub struct Channel {
    pub id: ChannelId,
    pub seeds: Vec<u8>,
    pub devices: Vec<MlsDevice>,
}

impl Channel {
    /// Device `seeds[0]` creates the group, adds the others from a KeyPackage each and commits; the
    /// server stores the Commit and the others join from its Welcome: epoch 1.
    pub fn new(seeds: &[u8]) -> Self {
        let id = ChannelId::generate();
        let device = |seed: u8| MlsDevice::new(&signing_key(seed), &state_key(seed));
        let mut creator = device(seeds[0]);
        creator.create_group(id, NOW).expect("a new group");
        let mut joiners: Vec<MlsDevice> = seeds[1..].iter().map(|&seed| device(seed)).collect();
        let packages: Vec<Vec<u8>> = joiners
            .iter_mut()
            .map(|joiner| {
                joiner
                    .key_package(NOW)
                    .expect("a KeyPackage")
                    .as_ref()
                    .to_vec()
            })
            .collect();
        let admitted: Vec<Cert> = seeds.iter().map(|&seed| cert(seed)).collect();
        creator
            .add_devices(id, &packages, &admitted, NOW)
            .expect("a Commit of the Adds");
        let welcome = creator
            .commit_stored(id)
            .expect("the stored Commit merges")
            .expect("a Welcome");
        for joiner in &mut joiners {
            let joined = joiner
                .join(&welcome, NOW, |_| true)
                .expect("the device joins");
            assert_eq!(joined, id);
        }
        let mut devices = vec![creator];
        devices.extend(joiners);
        let channel = Self {
            id,
            seeds: seeds.to_vec(),
            devices,
        };
        assert_eq!(channel.epoch(), 1);
        channel
    }

    /// Device `seeds[0]` commits an empty Commit, the server stores it and the others process it:
    /// the next epoch, with the same members.
    pub fn next_epoch(&mut self) {
        let id = self.id;
        let (first, others) = self.devices.split_first_mut().expect("a member");
        let commit = first.commit_empty(id, b"", NOW).expect("an empty Commit");
        first.commit_stored(id).expect("the stored Commit merges");
        for device in others {
            device
                .process_commit(id, &commit, NOW, |_| true)
                .expect("the Commit is processed");
        }
    }

    pub fn epoch(&self) -> u64 {
        self.devices[0].epoch(self.id).expect("the group")
    }

    /// The history key of `epoch` as the member at `at` holds it.
    pub fn key(&self, at: usize, epoch: u64) -> &HistoryKey {
        self.devices[at]
            .history_key(self.id, epoch)
            .unwrap_or_else(|| panic!("member {at} holds the history key of epoch {epoch}"))
    }

    /// The bytes of [`Channel::key`].
    pub fn key_bytes(&self, at: usize, epoch: u64) -> [u8; 16] {
        *self.key(at, epoch).to_bytes()
    }

    /// The member at `at` wraps the key of `epoch` to `reader`.
    pub fn wrap(&self, at: usize, epoch: u64, reader: &Reader) -> HistoryWrap {
        self.key(at, epoch)
            .wrap(
                self.id,
                epoch,
                reader.recipient,
                &reader.public,
                &signing_key(self.seeds[at]),
            )
            .expect("a wrap")
    }
}

/// `recipient` as the CDDL encodes it: `[1, device]` or `[2, account]`.
pub fn recipient_value(recipient: &WrapRecipient) -> Value {
    match recipient {
        WrapRecipient::Browser(device) => {
            Value::Array(vec![Value::Uint(1), bytes(device.as_bytes())])
        }
        WrapRecipient::Recovery(account) => {
            Value::Array(vec![Value::Uint(2), bytes(account.as_bytes())])
        }
    }
}

/// The associated data of a wrap's HPKE seal: the encoded `[channel, epoch, recipient]`.
pub fn aad(channel: ChannelId, epoch: u64, recipient: &WrapRecipient) -> Vec<u8> {
    Value::Array(vec![
        bytes(channel.as_bytes()),
        Value::Uint(epoch),
        recipient_value(recipient),
    ])
    .encode()
}

/// The fields of a `history-wrap-body`.
#[derive(Debug, Clone)]
pub struct Fields {
    pub channel: ChannelId,
    pub epoch: u64,
    pub recipient: WrapRecipient,
    pub signer: DeviceId,
    pub enc: Vec<u8>,
    pub ct: Vec<u8>,
}

impl Fields {
    /// The fields of `wrap`'s body.
    pub fn of(wrap: &HistoryWrap) -> Self {
        let body = wrap.body();
        Self {
            channel: body.channel,
            epoch: body.epoch,
            recipient: body.recipient,
            signer: body.signer,
            enc: body.enc.to_vec(),
            ct: body.ct.clone(),
        }
    }

    /// The map entries, as the CDDL numbers them.
    pub fn entries(&self) -> Vec<(i64, Value)> {
        vec![
            (1, bytes(self.channel.as_bytes())),
            (2, Value::Uint(self.epoch)),
            (3, recipient_value(&self.recipient)),
            (4, bytes(self.signer.as_bytes())),
            (5, bytes(&self.enc)),
            (6, bytes(&self.ct)),
        ]
    }

    /// The encoded body, with `unknown` fields.
    pub fn encode(&self, unknown: Vec<(i64, Value)>) -> Vec<u8> {
        let mut entries = self.entries();
        entries.extend(unknown);
        Value::Map(entries).encode()
    }
}

/// The wrap of the encoded `body`, signed by device `seed` over `[label, body]`.
pub fn signed(seed: u8, label: &str, body: &[u8]) -> HistoryWrap {
    HistoryWrap::from_cbor(&signed_cbor(body, &signing_key(seed).sign(label, &[body])))
        .expect("a wrap")
}

/// `fields` signed by device `seed` as the spec says.
pub fn resigned(seed: u8, fields: &Fields) -> HistoryWrap {
    signed(seed, LABEL, &fields.encode(vec![]))
}

/// A wrap of `plaintext` in `channel` and `epoch` to `reader`, sealed and signed by device `seed`
/// apart from the wrapping under test, its body with `unknown` fields.
pub fn made_apart(
    seed: u8,
    channel: ChannelId,
    epoch: u64,
    reader: &Reader,
    plaintext: &[u8],
    unknown: Vec<(i64, Value)>,
) -> HistoryWrap {
    let (enc, ct) = HpkeKey::seal(
        &reader.public,
        INFO,
        &aad(channel, epoch, &reader.recipient),
        plaintext,
    )
    .expect("a single-shot seal");
    let fields = Fields {
        channel,
        epoch,
        recipient: reader.recipient,
        signer: device_id(seed),
        enc: enc.to_vec(),
        ct,
    };
    signed(seed, LABEL, &fields.encode(unknown))
}
