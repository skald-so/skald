//! Linking a new device with the recovery key (`docs/protocol/membership.md`, "Linking a device",
//! "Anchors" and "Revocation"), only by its person at a terminal, who types the key without echo.
//!
//! The device pins the master key only from an account record it signs and a master backup that
//! opens in PSK mode with the key, issues its own certificate with it and shows the account's
//! devices. Then, in this order, it revokes with the master key the devices its person names
//! stolen, has the server store its certificate, starts from the anchors objects of the devices no
//! master-key revocation marks as stolen, takes the history keys wrapped to the recovery key
//! ([`crate::recovery_wraps`]), and revokes with its own key those its person names lost or wiped.
//! The master private key and the recovery key never reach the disk. Each revocation is written to
//! the local state before it is sent and sent again by later runs; a run that stops before it is
//! done leaves the rest to a later run with the recovery key, which signs no revocation twice.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{
    HpkeKey, RecoveryKey, SigningKey, check_account, check_anchors, check_device, counts,
    marked_stolen, open_master_backup, sign_revocation, verify,
};
use skald_proto::{
    AccountId, AccountRecord, Anchors, Cbor as _, Cert, DeviceId, DeviceKind, MasterBackup,
    NonEmpty, REVOCATION_LABEL, Revocation, RevocationBody, Roster, SpaceId, Unknown,
};
use zeroize::Zeroizing;

use crate::connection::{Connection, printable};
use crate::device::{self, Keys, MasterKey};
use crate::devices::{self, text};
use crate::follow::{self, Own, SpaceDevices};
use crate::link_code;
use crate::output::{self, Stream};
use crate::recovery_wraps;
use crate::revocations::{self, certified_by};
use crate::secrets::Secrets;
use crate::sign_in;
use crate::spaces::now;
use crate::state::{self, Link, State};

/// The devices its person names, by the options of `skald sign-in`; with none, it asks.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Named {
    /// Revoked with the master key.
    pub stolen: Vec<DeviceId>,
    /// Revoked with this device's key.
    pub lost: Vec<DeviceId>,
    /// Certified by a device named stolen or lost, and valid all the same: a revocation's `keep`.
    pub keep: Vec<DeviceId>,
    /// Revoked by a revocation that a master-key revocation voids, and revoked all the same: its
    /// `also`.
    pub keep_revoked: Vec<DeviceId>,
}

impl Named {
    fn given(&self) -> bool {
        !(self.stolen.is_empty()
            && self.lost.is_empty()
            && self.keep.is_empty()
            && self.keep_revoked.is_empty())
    }
}

/// The new device, waiting for linking: its connection, local state, secrets and session, its
/// account, itself and its keys.
pub struct NewDevice<'a> {
    pub connection: &'a mut Connection,
    pub state: &'a State,
    pub secrets: &'a Secrets,
    pub token: &'a str,
    pub account: AccountId,
    pub device: &'a state::Device,
    pub keys: &'a Keys,
}

/// The run that sends again what a run with the recovery key signed and did not send.
const AGAIN: &str = "the next skald sign-in --first-device no --with-recovery-key";

/// What the run says when it stops: nothing is signed but what it says it signed.
fn refused(why: &str) -> sign_in::Error {
    sign_in::Error::Recovery(format!("{why}; this device signs nothing"))
}

/// `AccountAnchors` in the API spec.
#[derive(Deserialize)]
struct AccountAnchors {
    anchors: Vec<String>,
}

/// Links the device with the recovery key its person types, revoking the devices of `named`.
pub fn link(new: &mut NewDevice<'_>, named: &Named) -> Result<(), sign_in::Error> {
    // What a run that stopped signed and did not send.
    revocations::send_pending(new.connection, new.token, new.state);
    let given = device::master_key(new.connection, new.token)?;
    let (master, record, backup) = account_master(&given, new.account)?;
    if let Link::Recovering { master: pinned, .. } = &new.device.link
        && *pinned != master
    {
        return Err(refused(
            "the server gives another master key than the one this device pinned at an earlier \
             run with the recovery key",
        ));
    }
    check_account(&record, &master).map_err(|err| {
        refused(&format!(
            "the account record the server gives fails: {err}; someone may have changed it"
        ))
    })?;
    let typed = read_hidden(
        "Type the account's recovery key; it does not show as you type, and Enter ends it: ",
    )
    .map_err(|err| sign_in::Error::Io(format!("cannot read the terminal: {err}")))?
    .ok_or_else(|| refused("no recovery key was typed"))?;
    let key = RecoveryKey::parse(typed.trim()).map_err(|_| {
        refused("what was typed is not a recovery key: 13 groups of four letters and digits")
    })?;
    drop(typed);
    let signer = open_master_backup(&backup, &key, new.account, &master).map_err(|_| {
        refused(
            "the recovery key does not open the account's master backup: it is not this \
             account's recovery key, or someone changed the backup",
        )
    })?;
    // The wraps to the recovery key open with its HPKE key pair, kept until the wraps are taken.
    let recovery = HpkeKey::from_recovery_key(&key.to_bytes());
    drop(key);
    let cert = new.own_cert(&signer)?;
    new.state.set_link(&Link::Recovering {
        master,
        cert: cert.to_cbor(),
    })?;

    let mut devices = devices::own(new.connection, new.token, new.state, &master)
        .map_err(sign_in::Error::Recovery)?;
    println!("The account's devices:");
    print!(
        "{}",
        devices::listing(&devices::check(&master, &devices), new.device.id)
    );
    let named = if named.given() {
        named.clone()
    } else {
        ask(&master, &signer, &devices, (new.account, new.device.id))
            .map_err(|err| sign_in::Error::Io(format!("cannot ask the user: {err}")))?
            .ok_or_else(|| refused("no answer"))?
    };
    let plan = Plan::new(&master, &devices, (new.account, new.device.id), &named)
        .map_err(|err| refused(&err))?;

    // 1. The stolen devices, revoked with the master key.
    let by_master = plan.by_master(&master, &signer, &devices, new.account)?;
    // The master private key has signed all it signs.
    drop(signer);
    let sent = new.send(&by_master)?;
    for revocation in &sent {
        say_revoked(revocation, &devices, "stolen");
    }
    devices.revocations.extend(sent);

    // Stored after the master-key revocations, which free the stolen devices' places under the
    // devices limit.
    match link_code::upload(new.connection, new.token, new.device.id, &cert.to_cbor()) {
        Err(sign_in::Error::Device(device::Error::DevicesLimit)) => {
            return Err(sign_in::Error::Recovery(
                "the account has as many devices as it may on this server, so the server does not \
                 store this device's certificate. Name a device stolen, or revoke one on a linked \
                 device, then run skald sign-in --first-device no --with-recovery-key again"
                    .to_owned(),
            ));
        }
        stored => stored?,
    }
    println!("The server stored this device's certificate, issued with the account's master key.");

    // 2. The anchors.
    let own = Own {
        account: new.account,
        master,
    };
    new.anchor(&own, &devices)?;
    recovery_wraps::Taking {
        connection: new.connection,
        token: new.token,
        state: new.state,
        secrets: new.secrets,
        keys: new.keys,
    }
    .take(&own, &recovery)
    .map_err(|err| {
        sign_in::Error::Recovery(format!(
            "this device has not taken the history keys wrapped to the recovery key: {err}; {AGAIN} \
             takes them"
        ))
    })?;
    drop(recovery);

    // 3. The lost or wiped devices, revoked with this device's key.
    let by_device = plan.by_device(new.keys.signing(), &devices, new.account, new.device.id)?;
    new.state
        .add_revocations(
            &by_device
                .iter()
                .map(Revocation::to_cbor)
                .collect::<Vec<_>>(),
            true,
        )
        .map_err(text)
        .map_err(sign_in::Error::Recovery)?;
    new.state.set_link(&Link::Certified {
        master,
        cert: cert.to_cbor(),
    })?;
    for revocation in &by_device {
        let later = "the next skald command that connects";
        revocations::store_or(
            new.connection,
            new.token,
            new.state,
            &revocation.to_cbor(),
            later,
        )
        .map_err(sign_in::Error::Recovery)?;
        say_revoked(revocation, &devices, "lost or wiped");
    }
    println!("This device is linked with the recovery key, which it keeps nowhere.");
    Ok(())
}

/// The master key, the account record and the master backup the server gives for `account`.
fn account_master(
    given: &MasterKey,
    account: AccountId,
) -> Result<([u8; 32], AccountRecord, MasterBackup), sign_in::Error> {
    let (Some(master), Some(record), Some(backup)) = (&given.master, &given.record, &given.backup)
    else {
        return Err(refused(if given.master.is_none() {
            "the server reports no master key for the account"
        } else {
            "this account has no recovery key: it was made before recovery keys, so link this \
             device from a linked device, with a link code"
        }));
    };
    let malformed =
        || refused("the server gives the account's master key in a form it cannot read");
    let decoded = |text: &str| URL_SAFE_NO_PAD.decode(text).map_err(|_| malformed());
    let master: [u8; 32] = decoded(master)?.try_into().map_err(|_| malformed())?;
    let record = AccountRecord::from_cbor(&decoded(record)?).map_err(|_| malformed())?;
    let backup = MasterBackup::from_cbor(&decoded(backup)?).map_err(|_| malformed())?;
    if record.body().account != account {
        return Err(refused(
            "the server gives the account record of another account",
        ));
    }
    Ok((master, record, backup))
}

impl NewDevice<'_> {
    /// The device's certificate, issued with `master`, the master private key: the one an earlier
    /// run issued, so that the device has one certificate, or a new one, checked.
    fn own_cert(&self, master: &SigningKey) -> Result<Cert, sign_in::Error> {
        if let Link::Recovering { cert, .. } = &self.device.link {
            return Cert::from_cbor(cert)
                .map_err(|err| state::Error::Malformed(format!("the issued certificate: {err}")))
                .map_err(Into::into);
        }
        let body = self
            .keys
            .master_cert_body(self.account, self.device.name.clone());
        let cert = skald_crypto::issue(&body, master, self.keys.signing())
            .map_err(|err| device::Error::Cert(err.to_string()))?;
        check_device(
            &master.public_key(),
            std::slice::from_ref(&cert),
            body.device,
        )
        .map_err(|err| device::Error::Cert(err.to_string()))?;
        Ok(cert)
    }

    /// Writes `signed`, revocations, to the state as pending, then has the server store each;
    /// gives them.
    fn send(&mut self, signed: &[Revocation]) -> Result<Vec<Revocation>, sign_in::Error> {
        let bytes: Vec<Vec<u8>> = signed.iter().map(Revocation::to_cbor).collect();
        self.state
            .add_revocations(&bytes, true)
            .map_err(text)
            .map_err(sign_in::Error::Recovery)?;
        for revocation in &bytes {
            revocations::store_or(self.connection, self.token, self.state, revocation, AGAIN)
                .map_err(sign_in::Error::Recovery)?;
        }
        Ok(signed.to_vec())
    }

    /// Joins the spaces of the anchors objects of the account's devices, `devices`, that no
    /// master-key revocation marks as stolen ("Anchors"), and pins the personal space they name;
    /// warns of each space it does not join and of personal spaces that differ.
    fn anchor(&mut self, own: &Own, devices: &SpaceDevices) -> Result<(), sign_in::Error> {
        let body = self
            .connection
            .get_as("/account/anchors", self.token)
            .map_err(sign_in::Error::Server)?;
        let listed: AccountAnchors = serde_json::from_slice(&body).map_err(|_| {
            sign_in::Error::Unexpected("an anchors list without its anchors".to_owned())
        })?;
        let mut by_space: BTreeMap<SpaceId, Vec<[u8; 32]>> = BTreeMap::new();
        let mut personal: Vec<SpaceId> = Vec::new();
        for anchors in listed.anchors.iter().filter_map(|text| {
            let bytes = URL_SAFE_NO_PAD.decode(text).ok()?;
            Anchors::from_cbor(&bytes).ok()
        }) {
            if !valid_anchors(&anchors, own, devices, self.device.id) {
                continue;
            }
            let body = anchors.body();
            for (space, hash) in &body.anchors {
                by_space.entry(*space).or_default().push(*hash);
            }
            if let Some(space) = body.personal
                && !personal.contains(&space)
            {
                personal.push(space);
            }
        }
        if personal.len() > 1 {
            eprintln!(
                "skald: warning: the anchors objects of the account's devices name different \
                 personal spaces, so this device pins none as its personal space"
            );
        }
        let personal = match personal[..] {
            [space] => Some(space),
            _ => None,
        };
        for (space, hashes) in &by_space {
            match self.start(*space, hashes, own)? {
                Ok(start) => self.state.add_linked_space(
                    *space,
                    start.body().name.as_str(),
                    &start.to_cbor(),
                    personal == Some(*space),
                )?,
                Err(warning) => eprintln!("skald: warning: {warning}"),
            }
        }
        Ok(())
    }

    /// The revision of `space` the device starts from: of those whose `hashes` the anchors give,
    /// the one with the lowest number, if every other one is a revision it accepts after it
    /// ("Roster"); otherwise why it does not join the space.
    fn start(
        &mut self,
        space: SpaceId,
        hashes: &[[u8; 32]],
        own: &Own,
    ) -> Result<Result<Roster, String>, sign_in::Error> {
        let given =
            follow::given(self.connection, self.token, space).map_err(sign_in::Error::Io)?;
        let anchored: Option<Vec<&Roster>> = hashes
            .iter()
            .map(|hash| {
                given
                    .iter()
                    .find(|revision| revision.hash() == *hash && revision.body().space == space)
            })
            .collect();
        let Some(start) = anchored.as_ref().and_then(|anchored| {
            anchored
                .iter()
                .min_by_key(|revision| revision.body().revision)
        }) else {
            return Ok(Err(format!(
                "the server does not give a revision of the space {space} that the anchors of the \
                 account's devices name, so this device does not join the space"
            )));
        };
        let devices = follow::space_devices(self.connection, self.token, self.state, space)
            .map_err(sign_in::Error::Io)?;
        let followed = follow::follow(start, &BTreeMap::new(), &given, &devices, own);
        let accepted: Vec<[u8; 32]> = std::iter::once(*start)
            .chain(&followed.accepted)
            .map(Roster::hash)
            .collect();
        if hashes.iter().all(|hash| accepted.contains(hash)) {
            return Ok(Ok((*start).clone()));
        }
        Ok(Err(format!(
            "the anchors of the account's devices of the space {} ({space}) disagree: one names a \
             revision this device does not accept after the earliest one, so it does not join the \
             space",
            start.body().name.as_str()
        )))
    }
}

/// Whether `anchors` counts for a device linked with the recovery key ("Anchors"): of `own`'s
/// account, signed by a `client` device of it other than `this`, whose certificate among
/// `devices` passes checks 1 and 2 and that no master-key revocation marks as stolen.
fn valid_anchors(anchors: &Anchors, own: &Own, devices: &SpaceDevices, this: DeviceId) -> bool {
    let body = anchors.body();
    let Ok(cert) = check_device(&own.master, &devices.certs, body.device) else {
        return false;
    };
    let signer = cert.body();
    body.account == own.account
        && body.device != this
        && signer.account == own.account
        && signer.kind == DeviceKind::Client
        && check_anchors(anchors, &signer.signing).is_ok()
        && !marked_stolen(
            &own.master,
            &devices.certs,
            &devices.revocations,
            body.device,
        )
}

/// What the device revokes, checked before it signs anything.
struct Plan {
    named: Named,
    /// The devices each device of `named.stolen` and `named.lost` certified.
    certified: BTreeMap<DeviceId, Vec<DeviceId>>,
}

impl Plan {
    /// The plan of `named` on `this` device of `account`, among the account's `devices` checked
    /// against the pinned `master` key; why not if it names a device the account has not, this
    /// device, a device both stolen and lost, a device kept that no device named stolen or lost
    /// certified, or one both kept and named.
    fn new(
        master: &[u8; 32],
        devices: &SpaceDevices,
        (account, this): (AccountId, DeviceId),
        named: &Named,
    ) -> Result<Self, String> {
        let all = [&named.stolen, &named.lost, &named.keep, &named.keep_revoked];
        for device in all.into_iter().flatten() {
            let chained = check_device(master, &devices.certs, *device);
            if *device == this || !chained.is_ok_and(|cert| cert.body().account == account) {
                return Err(format!(
                    "the server lists no other device {device} of this account with a certificate \
                     chained to its master key"
                ));
            }
        }
        if let Some(device) = named.stolen.iter().find(|id| named.lost.contains(id)) {
            return Err(format!("the device {device} is named both stolen and lost"));
        }
        let mut certified = BTreeMap::new();
        for device in named.stolen.iter().chain(&named.lost) {
            let ids: Vec<DeviceId> = (certified_by(master, &devices.certs, *device).into_iter())
                .map(|(id, _)| id)
                .collect();
            certified.insert(*device, ids);
        }
        for kept in &named.keep {
            if named.stolen.contains(kept) || named.lost.contains(kept) {
                return Err(format!("the device {kept} is named both kept and revoked"));
            }
            if !certified.values().any(|ids| ids.contains(kept)) {
                return Err(format!(
                    "the device {kept} is not one a device named stolen or lost certified, so it \
                     cannot be kept"
                ));
            }
        }
        Ok(Plan {
            named: named.clone(),
            certified,
        })
    }

    /// The `keep` of a revocation of `device`.
    fn keep_of(&self, device: DeviceId) -> Option<NonEmpty<DeviceId>> {
        let certified = self.certified.get(&device).cloned().unwrap_or_default();
        NonEmpty::from_vec(
            (certified.into_iter())
                .filter(|id| self.named.keep.contains(id))
                .collect(),
        )
    }

    /// Step 1: a master-key revocation, signed by `signer`, of each stolen device `devices` holds
    /// none of, with `also` the devices kept revoked of the revocations it voids. Fails if a
    /// device kept revoked is none of those.
    fn by_master(
        &self,
        master: &[u8; 32],
        signer: &SigningKey,
        devices: &SpaceDevices,
        account: AccountId,
    ) -> Result<Vec<Revocation>, sign_in::Error> {
        let mut all = devices.revocations.clone();
        let mut signed = Vec::new();
        let mut also_listed: Vec<DeviceId> = Vec::new();
        for &device in &self.named.stolen {
            if let Some(held) = held_by(&all, (account, device), (None, master)) {
                also_listed.extend(held.body().also.iter().flat_map(|also| also.iter()));
                continue;
            }
            let mut body = RevocationBody {
                account,
                device,
                keep: self.keep_of(device),
                signer: None,
                timestamp: now(),
                also: None,
                unknown: Unknown::default(),
            };
            let first = sign(&body, signer)?;
            let named = voided_names(master, &devices.certs, &all, &first);
            body.also = NonEmpty::from_vec(
                (self.named.keep_revoked.iter().copied())
                    .filter(|id| named.contains(id))
                    .collect(),
            );
            let revocation = if body.also.is_some() {
                sign(&body, signer)?
            } else {
                first
            };
            also_listed.extend(body.also.iter().flat_map(|also| also.iter()));
            all.push(revocation.clone());
            signed.push(revocation);
        }
        if let Some(device) = (self.named.keep_revoked.iter()).find(|id| !also_listed.contains(id))
        {
            return Err(refused(&format!(
                "the device {device} is not one that a revocation by a device named stolen \
                 revoked, so it cannot be kept revoked"
            )));
        }
        Ok(signed)
    }

    /// Step 3: a revocation, signed by `key` of `this` device, of each lost device `devices`
    /// holds none of by this device.
    fn by_device(
        &self,
        key: &SigningKey,
        devices: &SpaceDevices,
        account: AccountId,
        this: DeviceId,
    ) -> Result<Vec<Revocation>, sign_in::Error> {
        let mut signed = Vec::new();
        for &device in &self.named.lost {
            let held = (Some(this), &key.public_key());
            if held_by(&devices.revocations, (account, device), held).is_some() {
                continue;
            }
            let body = RevocationBody {
                account,
                device,
                keep: self.keep_of(device),
                signer: Some(this),
                timestamp: now(),
                also: None,
                unknown: Unknown::default(),
            };
            signed.push(sign(&body, key)?);
        }
        Ok(signed)
    }
}

/// The revocation of `body`, signed by `key`.
fn sign(body: &RevocationBody, key: &SigningKey) -> Result<Revocation, sign_in::Error> {
    sign_revocation(body, key)
        .map_err(|err| sign_in::Error::Recovery(format!("a revocation does not encode: {err}")))
}

/// A revocation among `revocations` of `device` of `account` by `signer`, the master key if
/// `None`, that verifies with `key`, the signer's public key: one the server lists that does not
/// verify counts as none.
fn held_by<'r>(
    revocations: &'r [Revocation],
    (account, device): (AccountId, DeviceId),
    (signer, key): (Option<DeviceId>, &[u8; 32]),
) -> Option<&'r Revocation> {
    revocations.iter().find(|revocation| {
        let body = revocation.body();
        body.account == account
            && body.device == device
            && body.signer == signer
            && verify(
                key,
                REVOCATION_LABEL,
                &[revocation.body_bytes()],
                revocation.signature(),
            )
            .is_ok()
    })
}

/// The devices named by the revocations of `all` that `stolen`, a master-key revocation, voids:
/// those that count beside the others and no longer do beside `stolen` too.
fn voided_names(
    master: &[u8; 32],
    certs: &[Cert],
    all: &[Revocation],
    stolen: &Revocation,
) -> Vec<DeviceId> {
    let mut named = Vec::new();
    for (at, revocation) in all.iter().enumerate() {
        let mut others: Vec<Revocation> = all.to_vec();
        others.remove(at);
        let before = counts(master, certs, &others, revocation);
        others.push(stolen.clone());
        if before && !counts(master, certs, &others, revocation) {
            let body = revocation.body();
            named.push(body.device);
            named.extend(body.also.iter().flat_map(|also| also.iter()));
        }
    }
    named
}

/// Says that the device revoked by `revocation`, of `devices`, is revoked as `how`.
fn say_revoked(revocation: &Revocation, devices: &SpaceDevices, how: &str) {
    let device = revocation.body().device;
    let name = (devices.certs.iter())
        .find(|cert| cert.body().device == device)
        .map(|cert| printable(&cert.body().name))
        .unwrap_or_default();
    let by = if revocation.body().signer.is_none() {
        "the account's master key"
    } else {
        "this device's key"
    };
    println!("Revoked the device {device} ({name}) as {how}, with {by}.");
}

/// Asks its person which of the account's `devices`, checked against `master`, are stolen and
/// which lost or wiped, which of those they certified stay valid and which of the devices the
/// revocations by stolen devices revoked stay revoked; `None` once the input ends. `signer`, the
/// master private key, signs the revocations that show which those are, never sent or kept.
fn ask(
    master: &[u8; 32],
    signer: &SigningKey,
    devices: &SpaceDevices,
    (account, this): (AccountId, DeviceId),
) -> io::Result<Option<Named>> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout();
    let shown: Vec<DeviceId> = (devices.ids.iter().copied())
        .filter(|id| *id != this && check_device(master, &devices.certs, *id).is_ok())
        .collect();
    let Some(stolen) = revocations::ask_ids(
        &mut input,
        &mut output,
        "Which of these devices are stolen? The master key revokes them.",
        &shown,
    )?
    else {
        return Ok(None);
    };
    let rest: Vec<DeviceId> = (shown.iter().copied())
        .filter(|id| !stolen.contains(id))
        .collect();
    let Some(lost) = revocations::ask_ids(
        &mut input,
        &mut output,
        "Which of the others are lost or wiped? This device revokes them.",
        &rest,
    )?
    else {
        return Ok(None);
    };
    let mut certified: Vec<DeviceId> = Vec::new();
    for device in stolen.iter().chain(&lost) {
        for (id, _) in certified_by(master, &devices.certs, *device) {
            if !stolen.contains(&id) && !lost.contains(&id) && !certified.contains(&id) {
                certified.push(id);
            }
        }
    }
    let mut named = Named {
        stolen,
        lost,
        ..Named::default()
    };
    if !certified.is_empty() {
        writeln!(
            output,
            "The devices named certified these, which are revoked with them unless kept:"
        )?;
        for id in &certified {
            writeln!(output, "  {id}")?;
        }
        let Some(keep) = revocations::ask_keep(&mut input, &mut output, &certified)? else {
            return Ok(None);
        };
        named.keep = keep;
    }
    let restored = Plan::new(master, devices, (account, this), &named)
        .and_then(|plan| {
            let signed = plan
                .by_master(master, signer, devices, account)
                .map_err(|err| err.to_string())?;
            Ok(signed
                .iter()
                .flat_map(|revocation| {
                    voided_names(master, &devices.certs, &devices.revocations, revocation)
                })
                .collect::<Vec<DeviceId>>())
        })
        .unwrap_or_default();
    if !restored.is_empty() {
        writeln!(
            output,
            "Revocations by the devices named stolen revoked these, which are valid again unless \
             kept revoked:"
        )?;
        for id in &restored {
            writeln!(output, "  {id}")?;
        }
        let Some(kept) = revocations::ask_ids(
            &mut input,
            &mut output,
            "Which of them stay revoked?",
            &restored,
        )?
        else {
            return Ok(None);
        };
        named.keep_revoked = kept;
    }
    Ok(Some(named))
}

/// Reads a line its person types at the terminal, with echo off, after showing `prompt`; `None`
/// once the input ends.
fn read_hidden(prompt: &str) -> io::Result<Option<Zeroizing<String>>> {
    let echo = EchoOff::new()?;
    output::write(Stream::Out, prompt.as_bytes())?;
    let mut line = Zeroizing::new(String::with_capacity(256));
    let read = io::stdin().lock().read_line(&mut line);
    drop(echo);
    println!();
    Ok((read? > 0).then_some(line))
}

/// The terminal's echo, off until it is dropped.
#[cfg(unix)]
struct EchoOff(libc::termios);

#[cfg(unix)]
impl EchoOff {
    fn new() -> io::Result<Self> {
        // SAFETY: termios is plain data, and tcgetattr fills it in for the standard input.
        let mut mode: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: the standard input's descriptor and a termios to fill in.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut mode) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let before = mode;
        mode.c_lflag &= !libc::ECHO;
        // SAFETY: the standard input's descriptor and the mode read from it, changed.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &mode) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(EchoOff(before))
    }
}

#[cfg(unix)]
impl Drop for EchoOff {
    fn drop(&mut self) {
        // SAFETY: the standard input's descriptor and its mode before.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0) };
    }
}

/// The console's echo, off until it is dropped.
#[cfg(windows)]
struct EchoOff {
    input: windows_sys::Win32::Foundation::HANDLE,
    mode: windows_sys::Win32::System::Console::CONSOLE_MODE,
}

#[cfg(windows)]
impl EchoOff {
    fn new() -> io::Result<Self> {
        use windows_sys::Win32::System::Console::{
            ENABLE_ECHO_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE, SetConsoleMode,
        };
        // SAFETY: the process's own standard input, and a mode to fill in.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(input, &mut mode) == 0
                || SetConsoleMode(input, mode & !ENABLE_ECHO_INPUT) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(EchoOff { input, mode })
        }
    }
}

#[cfg(windows)]
impl Drop for EchoOff {
    fn drop(&mut self) {
        // SAFETY: the standard input and its mode before.
        unsafe { windows_sys::Win32::System::Console::SetConsoleMode(self.input, self.mode) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skald_crypto::issue;
    use skald_proto::CertBody;

    /// An account with its master key and `client` devices: A and C certified by the master key,
    /// B by A; A revoked C. `this` is the new device, without a certificate yet.
    struct Account {
        id: AccountId,
        master: SigningKey,
        keys: [SigningKey; 3],
        devices: SpaceDevices,
        this: DeviceId,
    }

    const A: usize = 0;
    const B: usize = 1;
    const C: usize = 2;

    impl Account {
        fn new() -> Self {
            let (id, master) = (AccountId::generate(), SigningKey::generate());
            let keys = [(); 3].map(|()| SigningKey::generate());
            let device = |key: &SigningKey| DeviceId::from_signing_key(&key.public_key());
            let cert = |at: usize, issuer: Option<usize>| {
                let body = CertBody {
                    account: id,
                    device: device(&keys[at]),
                    kind: DeviceKind::Client,
                    signing: keys[at].public_key(),
                    hpke: [3; 32],
                    issuer: issuer.map(|by| device(&keys[by])),
                    issued: 1_800_000_000,
                    ssh: None,
                    name: format!("device-{at}"),
                    unknown: Unknown::default(),
                };
                issue(&body, issuer.map_or(&master, |by| &keys[by]), &keys[at])
                    .expect("a certificate")
            };
            let certs = vec![cert(A, None), cert(B, Some(A)), cert(C, None)];
            let body = RevocationBody {
                account: id,
                device: device(&keys[C]),
                keep: None,
                signer: Some(device(&keys[A])),
                timestamp: 1_800_000_000,
                also: None,
                unknown: Unknown::default(),
            };
            let by_a = sign_revocation(&body, &keys[A]).expect("a revocation");
            let devices = SpaceDevices {
                ids: keys.iter().map(device).collect(),
                certs,
                revocations: vec![by_a],
                records: Vec::new(),
            };
            Account {
                id,
                master,
                keys,
                devices,
                this: DeviceId::from_signing_key(&SigningKey::generate().public_key()),
            }
        }

        fn device(&self, at: usize) -> DeviceId {
            DeviceId::from_signing_key(&self.keys[at].public_key())
        }

        fn plan(&self, named: &Named) -> Result<Plan, String> {
            let master = self.master.public_key();
            Plan::new(&master, &self.devices, (self.id, self.this), named)
        }

        fn by_master(&self, named: &Named) -> Result<Vec<Revocation>, String> {
            let master = self.master.public_key();
            let plan = self.plan(named)?;
            (plan.by_master(&master, &self.master, &self.devices, self.id))
                .map_err(|err| err.to_string())
        }
    }

    fn ids(
        revocation: &Revocation,
        of: fn(&RevocationBody) -> &Option<NonEmpty<DeviceId>>,
    ) -> Vec<DeviceId> {
        of(revocation.body())
            .as_deref()
            .unwrap_or_default()
            .to_vec()
    }

    #[test]
    fn a_plan_names_only_other_devices_of_the_account_and_keeps_only_those_they_certified() {
        let ada = Account::new();
        let (a, b, c) = (ada.device(A), ada.device(B), ada.device(C));
        for refused in [
            Named {
                lost: vec![ada.this],
                ..Named::default()
            },
            Named {
                stolen: vec![ada.this],
                ..Named::default()
            },
            Named {
                stolen: vec![a],
                lost: vec![a],
                ..Named::default()
            },
            Named {
                stolen: vec![a],
                keep: vec![c],
                ..Named::default()
            },
            Named {
                stolen: vec![a],
                keep: vec![a],
                ..Named::default()
            },
            Named {
                lost: vec![DeviceId::from_signing_key(&[9; 32])],
                ..Named::default()
            },
        ] {
            assert!(ada.plan(&refused).is_err(), "{refused:?}");
        }
        let keeping = Named {
            stolen: vec![a],
            keep: vec![b],
            ..Named::default()
        };
        let plan = ada.plan(&keeping).expect("a plan");
        assert_eq!(plan.keep_of(a).as_deref(), Some(&[b][..]));
        assert_eq!(plan.keep_of(c), None);
    }

    #[test]
    fn a_stolen_device_is_revoked_with_the_master_key_once_listing_in_also_the_devices_kept_revoked()
     {
        let ada = Account::new();
        let master = ada.master.public_key();
        let (a, b, c) = (ada.device(A), ada.device(B), ada.device(C));
        let named = Named {
            stolen: vec![a],
            keep: vec![b],
            keep_revoked: vec![c],
            ..Named::default()
        };

        let signed = ada.by_master(&named).expect("signed");

        assert_eq!(signed.len(), 1, "{signed:?}");
        let body = signed[0].body();
        assert_eq!((body.device, body.signer, body.account), (a, None, ada.id));
        assert_eq!(ids(&signed[0], |body| &body.keep), vec![b]);
        assert_eq!(ids(&signed[0], |body| &body.also), vec![c]);
        assert!(counts(&master, &ada.devices.certs, &[], &signed[0]));
        // Without kept revoked, A's revocation of C is voided; B is no device A's revocations named.
        let voiding = ada
            .by_master(&Named {
                stolen: vec![a],
                ..Named::default()
            })
            .expect("signed");
        assert_eq!(ids(&voiding[0], |body| &body.also), Vec::<DeviceId>::new());
        let wrong = Named {
            stolen: vec![a],
            keep_revoked: vec![b],
            ..Named::default()
        };
        assert!(ada.by_master(&wrong).is_err());
        // Held already, it is not signed again; what it kept revoked still counts as listed.
        let mut held = ada;
        held.devices.revocations.push(signed[0].clone());
        assert_eq!(held.by_master(&named).expect("none signed"), vec![]);
    }

    #[test]
    fn a_lost_device_is_revoked_with_this_devices_key_unless_it_revoked_it_already() {
        let ada = Account::new();
        let key = SigningKey::generate();
        let this = DeviceId::from_signing_key(&key.public_key());
        let mut ada = Account { this, ..ada };
        let (a, b) = (ada.device(A), ada.device(B));
        let named = Named {
            lost: vec![a],
            keep: vec![b],
            ..Named::default()
        };
        let plan = ada.plan(&named).expect("a plan");

        let signed = plan
            .by_device(&key, &ada.devices, ada.id, this)
            .expect("signed");

        assert_eq!(signed.len(), 1, "{signed:?}");
        let body = signed[0].body();
        assert_eq!((body.device, body.signer), (a, Some(this)));
        assert_eq!(ids(&signed[0], |body| &body.keep), vec![b]);
        assert!(signed_by(&signed[0], &key));
        ada.devices.revocations.push(signed[0].clone());
        let again = plan
            .by_device(&key, &ada.devices, ada.id, this)
            .expect("none");
        assert_eq!(again, vec![]);
    }

    #[test]
    fn a_listed_revocation_that_does_not_verify_does_not_stop_the_device_revoking() {
        let key = SigningKey::generate();
        let this = DeviceId::from_signing_key(&key.public_key());
        let mut ada = Account {
            this,
            ..Account::new()
        };
        let (a, c) = (ada.device(A), ada.device(C));
        let other = SigningKey::generate();
        let forged = |device, signer| {
            let body = RevocationBody {
                account: ada.id,
                device,
                keep: None,
                signer,
                timestamp: 1_800_000_000,
                also: None,
                unknown: Unknown::default(),
            };
            sign_revocation(&body, &other).expect("a revocation")
        };
        let listed = [forged(a, None), forged(c, Some(this))];
        ada.devices.revocations.extend(listed);
        let named = Named {
            stolen: vec![a],
            lost: vec![c],
            ..Named::default()
        };

        let by_master = ada.by_master(&named).expect("signed");
        let plan = ada.plan(&named).expect("a plan");
        let by_device = (plan.by_device(&key, &ada.devices, ada.id, this)).expect("signed");

        assert_eq!(by_master.len(), 1, "{by_master:?}");
        assert!(signed_by(&by_master[0], &ada.master), "{by_master:?}");
        assert_eq!(by_device.len(), 1, "{by_device:?}");
        assert!(signed_by(&by_device[0], &key), "{by_device:?}");
    }

    /// Whether `revocation` verifies with `key`.
    fn signed_by(revocation: &Revocation, key: &SigningKey) -> bool {
        skald_crypto::verify(
            &key.public_key(),
            skald_proto::REVOCATION_LABEL,
            &[revocation.body_bytes()],
            revocation.signature(),
        )
        .is_ok()
    }
}
