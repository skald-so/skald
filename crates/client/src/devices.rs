//! `skald devices`: the account's devices, as the server gives them, checked against the master key
//! the machine pinned in its local state, never against the one the server gives, and against the
//! revocations the server gives with them and those the device holds
//! (`docs/protocol/membership.md`, "Device certificate" and "Device checks"). A device shows by its
//! certificate's `name`, `kind` and `issued` only when checks 1 and 2 pass, marked if it is
//! revoked; the others show by ID only. It warns of each device revoked, or valid again, since the
//! device last checked it.

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{ChainProblem, DeviceCheckError, check_device, check_valid_device};
use skald_proto::{AccountRecord, Cbor as _, Cert, DeviceId, DeviceKind, Revocation, VersionRange};

use crate::connection::{self, Connection, printable};
use crate::follow::{self, Own, SpaceDevices};
use crate::kept_certs;
use crate::key_changes;
use crate::revocations;
use crate::secrets::{self, Secrets};
use crate::sign_in;
use crate::ssh_checks::{self, Results};
use crate::state::{self, Link, State, StatusChange};

/// What `skald devices` and `skald set-username` say when the machine has no session.
pub const NOT_SIGNED_IN: &str = "this machine is not signed in; run skald sign-in --server <URL>";

/// `Devices` in the API spec, or `SpaceDevices` with its `records`.
#[derive(Deserialize)]
struct Devices {
    devices: Vec<Entry>,
    #[serde(default)]
    revocations: Vec<String>,
    #[serde(default)]
    records: Vec<String>,
    #[serde(default)]
    deleted: Vec<String>,
    /// The server's web address, in `Devices` alone.
    #[serde(default)]
    web: Option<String>,
}

/// `Device` in the API spec: anything else the server gives with it, such as a name, is ignored.
#[derive(Deserialize)]
struct Entry {
    device: String,
    cert: Option<String>,
}

/// Lists the devices of the account the machine whose state is in `dir` signed in to, from the
/// server of its session, its secrets in `storage`, with what their SSH signatures showed
/// ([`ssh_checks`]). A `person`'s run warns too of master keys its spaces give
/// ([`follow::master_warnings`]); an agent's names nothing of them.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    person: bool,
) -> Result<(), String> {
    let (state, server, token) = signed_in(dir, storage)?;

    let Some(state::Device {
        id: this,
        link: Link::Certified { master, cert },
        ..
    }) = state.device().map_err(text)?
    else {
        println!(
            "This device is not linked yet: the server has taken no certificate of it, so it lists \
             no device as linked."
        );
        return Ok(());
    };

    let mut connection = to_session(&server, speaks, timeout)?;
    revocations::send_pending(&mut connection, &token, &state);
    let devices = own(&mut connection, &token, &state, &master)?;

    let checked = check(&master, &devices);
    let shown: Vec<&Cert> = checked
        .linked
        .iter()
        .chain(&checked.revoked)
        .copied()
        .collect();
    let (signatures, key_changes) = ssh_checks::check(&state, &shown)?;
    print!("{}", listing(&checked, this, &signatures));
    let changed = statuses(&state, &checked, None, "this account")?;
    let masters = if person {
        let cert = Cert::from_cbor(&cert).map_err(text)?;
        let own = Own {
            account: cert.body().account,
            master,
        };
        follow::master_warnings(&state, &own, None)?
    } else {
        Vec::new()
    };
    for warning in (two_bodies_warnings(&state, &checked)?.into_iter())
        .chain(shared_name_warnings(&checked.linked))
        .chain(key_changes)
        .chain(changed)
        .chain(masters)
    {
        eprintln!("skald: warning: {warning}");
    }
    Ok(())
}

/// The devices of the account of the session `token`, with the revocations the server gives and
/// those `state` holds; keeps in `state` those the server gives that count against the pinned
/// `master` key.
pub fn own(
    connection: &mut Connection,
    token: &str,
    state: &State,
    master: &[u8; 32],
) -> Result<SpaceDevices, String> {
    let devices = fetch(connection, "/devices", token, state)?;
    revocations::keep(state, &devices, |_| Some(*master))?;
    Ok(devices)
}

/// Records in `state` whether each device `checked` holds with a certificate that passes checks 1
/// and 2, of `account` if given, is revoked, keeps its certificate ([`kept_certs::keep`]) and
/// checks its SSH signature ([`ssh_checks::check`]); gives the warning of each key change that
/// check finds and of each device revoked, valid again or new since the device last checked it, as
/// a device of `whose` ("Device checks").
pub fn statuses(
    state: &State,
    checked: &Checked,
    account: Option<skald_proto::AccountId>,
    whose: &str,
) -> Result<Vec<String>, String> {
    let of = |cert: &Cert| account.is_none_or(|account| cert.body().account == account);
    let trusted: Vec<(&Cert, bool)> = (checked.linked.iter().map(|cert| (*cert, false)))
        .chain(checked.revoked.iter().map(|cert| (*cert, true)))
        .filter(|(cert, _)| of(cert))
        .collect();
    let certs: Vec<&Cert> = trusted.iter().map(|(cert, _)| *cert).collect();
    kept_certs::keep(state, &certs)?;
    let (_, key_changes) = ssh_checks::check(state, &certs)?;
    let seen: Vec<(DeviceId, bool)> = (trusted.iter())
        .map(|(cert, revoked)| (cert.body().device, *revoked))
        .collect();
    let changed = state.device_statuses(&seen).map_err(text)?;
    let changed = changed.into_iter().map(|(device, change)| match change {
        StatusChange::Revoked => format!("device {device} of {whose} is revoked"),
        StatusChange::Valid => format!("device {device} of {whose} is valid again"),
        StatusChange::Added => format!("device {device} of {whose} is new"),
    });
    Ok(key_changes.into_iter().chain(changed).collect())
}

/// The warning of each device in `checked` with certificates of two bodies
/// (`docs/protocol/membership.md`, "Device certificate"), each kept in `state` as a key change the
/// first time.
pub fn two_bodies_warnings(state: &State, checked: &Checked) -> Result<Vec<String>, String> {
    (checked.two_bodies.iter())
        .map(|id| {
            let warning = format!(
                "the key of device {id} signed two different certificates of it, so neither \
                 counts; the device or its key may be compromised"
            );
            key_changes::keep_once(state, &warning, &warning)?;
            Ok(warning)
        })
        .collect()
}

/// The warning of each name that two or more of the `linked` certificates, those of one account,
/// hold ("Device checks").
pub fn shared_name_warnings(linked: &[&Cert]) -> Vec<String> {
    shared_names(linked)
        .into_iter()
        .map(|(name, ids)| {
            let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
            format!(
                "linked devices {} have the same name \"{}\"; tell them apart by ID",
                ids.join(", "),
                printable(&name)
            )
        })
        .collect()
}

/// `GET <path>` as the session `token`: a device list, as the IDs of its devices, the
/// certificates it gives with those `state` keeps of devices with two bodies, and the revocations
/// it gives with those `state` holds, and the accounts it reports deleted; a certificate,
/// revocation or account ID that does not decode counts as none. Prints the warning of each device
/// given with a certificate other than the one `state` keeps ([`kept_certs`]). Records the server's
/// web address the list names ([`crate::acceptances::record_web`]).
pub fn fetch(
    connection: &mut Connection,
    path: &str,
    token: &str,
    state: &State,
) -> Result<SpaceDevices, String> {
    let body = connection.get_as(path, token).map_err(text)?;
    let (given, web) = decoded(&body)?;
    crate::acceptances::record_web(state, connection.server(), web.as_deref());
    let (certs, swapped) = kept_certs::settle(state, given.certs)?;
    for warning in swapped {
        eprintln!("skald: warning: {warning}");
    }
    let revocations = revocations::with_held(state, given.revocations)?;
    Ok(SpaceDevices {
        certs,
        revocations,
        ..given
    })
}

/// The device list `body`, a `Devices` answer or one with its fields, as it gives it: the IDs of
/// its devices, the certificates, revocations and account records that decode.
pub fn decode(body: &[u8]) -> Result<SpaceDevices, String> {
    decoded(body).map(|(devices, _)| devices)
}

/// As [`decode`], with the server's web address `body` names, if any.
fn decoded(body: &[u8]) -> Result<(SpaceDevices, Option<String>), String> {
    let devices: Devices = serde_json::from_slice(body)
        .map_err(|_| "unexpected answer from the server: a device list without its devices")?;
    let ids = devices
        .devices
        .iter()
        .filter_map(|entry| entry.device.parse().ok())
        .collect();
    let certs = devices
        .devices
        .iter()
        .filter_map(|entry| {
            let bytes = URL_SAFE_NO_PAD.decode(entry.cert.as_ref()?).ok()?;
            Cert::from_cbor(&bytes).ok()
        })
        .collect();
    let decoded = |list: &[String]| -> Vec<Vec<u8>> {
        (list.iter())
            .filter_map(|text| URL_SAFE_NO_PAD.decode(text).ok())
            .collect()
    };
    let revocations = (decoded(&devices.revocations).iter())
        .filter_map(|bytes| Revocation::from_cbor(bytes).ok())
        .collect();
    let records = (decoded(&devices.records).iter())
        .filter_map(|bytes| AccountRecord::from_cbor(bytes).ok())
        .collect();
    let deleted = (devices.deleted.iter())
        .filter_map(|id| id.parse().ok())
        .collect();
    let given = SpaceDevices {
        ids,
        certs,
        revocations,
        records,
        deleted,
    };
    Ok((given, devices.web))
}

/// The local state of the machine whose state is in `dir`, its secrets in `storage`, with the
/// server of its session and the session token; [`NOT_SIGNED_IN`] if it has no session.
pub fn signed_in(dir: &Path, storage: secrets::Storage) -> Result<(State, String, String), String> {
    signed_in_with_secrets(dir, storage).map(|(state, server, token, _, _)| (state, server, token))
}

/// As [`signed_in`], with the secrets too: as stored, and where.
pub fn signed_in_with_secrets(
    dir: &Path,
    storage: secrets::Storage,
) -> Result<SessionSecrets, String> {
    session_with_secrets(dir, storage)?.ok_or_else(|| NOT_SIGNED_IN.to_owned())
}

type SessionSecrets = (State, String, String, secrets::Map, Secrets);

/// The stored session with its secrets, if the machine has one.
pub fn session_with_secrets(
    dir: &Path,
    storage: secrets::Storage,
) -> Result<Option<SessionSecrets>, String> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let (state, secrets) = sign_in::open(dir, storage).map_err(text)?;
    let Some(session) = state.session().map_err(text)? else {
        return Ok(None);
    };
    let stored = secrets.load().map_err(text)?;
    let token = sign_in::token_for(&stored, &session.server).cloned();
    Ok(token.map(|token| (state, session.server, token, stored, secrets)))
}

/// A connection to `server`, the server of the machine's stored session, as [`Connection::new`]
/// gives it; a URL in plain HTTP to another machine asks to sign in again over HTTPS.
pub fn to_session(
    server: &str,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<Connection, String> {
    Connection::new(server, speaks, timeout).map_err(|err| match err {
        connection::Error::Cleartext(_) => stored_cleartext(server),
        err => err.to_string(),
    })
}

/// What a command says of a session stored for `server`, plain HTTP to another machine.
pub fn stored_cleartext(server: &str) -> String {
    format!(
        "{server:?}, the server of this machine's session, is plain HTTP to another machine: sign \
         in again with an https:// URL: run skald sign-in --server <URL>"
    )
}

/// Each name that two or more of the `linked` certificates hold, with their devices, in the order
/// of the name's first certificate.
fn shared_names(linked: &[&Cert]) -> Vec<(String, Vec<DeviceId>)> {
    let mut names: Vec<(String, Vec<DeviceId>)> = Vec::new();
    for cert in linked {
        let body = cert.body();
        match names.iter_mut().find(|(name, _)| *name == body.name) {
            Some((_, ids)) => ids.push(body.device),
            None => names.push((body.name.clone(), vec![body.device])),
        }
    }
    names.retain(|(_, ids)| ids.len() > 1);
    names
}

/// The text of `err`, as `skald` prints it.
pub fn text(err: impl std::fmt::Display) -> String {
    err.to_string()
}

/// The devices of a list, checked.
#[derive(Debug, PartialEq)]
pub struct Checked<'c> {
    /// The valid certificates, one per device, in the list's order.
    pub linked: Vec<&'c Cert>,
    /// The certificates that pass checks 1 and 2 of revoked devices, one per device, in the list's
    /// order.
    pub revoked: Vec<&'c Cert>,
    /// The devices without a certificate that passes checks 1 and 2, in the list's order.
    apart: Vec<DeviceId>,
    /// The devices, listed or issuers in a chain, with certificates of two bodies.
    two_bodies: Vec<DeviceId>,
}

/// Checks each device of `devices` once against the pinned `master` key, with all the
/// certificates and revocations it holds.
pub fn check<'c>(master: &[u8; 32], devices: &'c SpaceDevices) -> Checked<'c> {
    let SpaceDevices {
        ids,
        certs,
        revocations,
        ..
    } = devices;
    let mut checked = Checked {
        linked: Vec::new(),
        revoked: Vec::new(),
        apart: Vec::new(),
        two_bodies: Vec::new(),
    };
    let mut seen = Vec::new();
    for &device in ids {
        if seen.contains(&device) {
            continue;
        }
        seen.push(device);
        let twice = match check_device(master, certs, device) {
            Ok(cert) => {
                match check_valid_device(master, certs, revocations, device) {
                    Ok(_) => checked.linked.push(cert),
                    Err(_) => checked.revoked.push(cert),
                }
                continue;
            }
            Err(DeviceCheckError::TwoBodies) => Some(device),
            Err(DeviceCheckError::Chain {
                device,
                problem: ChainProblem::TwoBodies,
            }) => Some(device),
            Err(_) => None,
        };
        checked.apart.push(device);
        if let Some(twice) = twice
            && !checked.two_bodies.contains(&twice)
        {
            checked.two_bodies.push(twice);
        }
    }
    checked
}

/// The list of `checked` on `this` device: the linked devices, the revoked ones, then those not
/// linked, each part under its heading if it has any; each device's line ends with what
/// `signatures` shows of its SSH signature ([`ssh_checks::shown`]).
pub fn listing(checked: &Checked, this: DeviceId, signatures: &Results) -> String {
    let mut out = String::new();
    if !checked.linked.is_empty() {
        out.push_str(
            "Linked: certificates chained to the account's master key this machine pinned\n",
        );
        for cert in &checked.linked {
            let signature = ssh_checks::shown(signatures, cert);
            out.push_str(&format!("  {}{signature}\n", linked_line(cert, this)));
        }
    }
    if !checked.revoked.is_empty() {
        out.push_str("Revoked: chained to that key, and revoked since\n");
        for cert in &checked.revoked {
            let signature = ssh_checks::shown(signatures, cert);
            out.push_str(&format!("  {}{signature}\n", revoked_line(cert, this)));
        }
    }
    if !checked.apart.is_empty() {
        out.push_str("Not linked: no valid certificate, so the server's word alone\n");
        for id in &checked.apart {
            out.push_str(&format!("  {id}\n"));
        }
    }
    out
}

/// The name of `kind`, as the protocol names it.
pub fn kind_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Client => "client",
        DeviceKind::Browser => "browser",
        DeviceKind::Service => "service",
    }
}

/// The line of a device with its valid certificate `cert`, marked if it is `this` device.
pub fn linked_line(cert: &Cert, this: DeviceId) -> String {
    let body = cert.body();
    let kind = kind_name(body.kind);
    let mut line = format!(
        "{}  {}  {kind}  issued {}",
        body.device,
        printable(&body.name),
        date(body.issued)
    );
    if body.device == this {
        line.push_str("  (this device)");
    }
    line
}

/// The line of a revoked device with `cert`, its certificate that passes checks 1 and 2, marked if
/// it is `this` device.
pub fn revoked_line(cert: &Cert, this: DeviceId) -> String {
    format!("{}  revoked", linked_line(cert, this))
}

/// The date of `seconds` since the Unix epoch, as `YYYY-MM-DD` in UTC.
pub fn date(seconds: u64) -> String {
    // The Gregorian calendar repeats every 400 years, of 146 097 days.
    const CYCLE: u64 = 146_097;
    let days = seconds / 86_400;
    let mut year = 1970 + 400 * (days / CYCLE);
    let mut day = days % CYCLE;
    let leap = |year: u64| {
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    };
    // A cycle holds 400 years.
    for _ in 0..400 {
        let length = if leap(year) { 366 } else { 365 };
        if day < length {
            break;
        }
        day -= length;
        year += 1;
    }
    let february = if leap(year) { 29 } else { 28 };
    let mut month = 1;
    for length in [31, february, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] {
        if day < length {
            break;
        }
        day -= length;
        month += 1;
    }
    format!("{year:04}-{month:02}-{:02}", day + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skald_crypto::{SigningKey, issue};
    use skald_proto::{AccountId, CertBody, Unknown};

    /// The list of `ids` with `certs` and `revocations`.
    fn list(ids: &[DeviceId], certs: Vec<Cert>, revocations: Vec<Revocation>) -> SpaceDevices {
        SpaceDevices {
            ids: ids.to_vec(),
            certs,
            revocations,
            records: Vec::new(),
            deleted: Default::default(),
        }
    }

    /// A device with keys of its own.
    struct Device {
        key: SigningKey,
    }

    impl Device {
        fn new() -> Self {
            Device {
                key: SigningKey::generate(),
            }
        }

        fn id(&self) -> DeviceId {
            DeviceId::from_signing_key(&self.key.public_key())
        }

        /// Its certificate on `account`, signed by `signer` as `issuer`, named `name`.
        fn cert(
            &self,
            account: AccountId,
            issuer: Option<DeviceId>,
            signer: &SigningKey,
            name: &str,
        ) -> Cert {
            let body = CertBody {
                account,
                device: self.id(),
                kind: DeviceKind::Client,
                signing: self.key.public_key(),
                hpke: [7; 32],
                issuer,
                issued: 1_800_000_000,
                ssh: None,
                github: None,
                name: name.to_owned(),
                unknown: Unknown::default(),
            };
            issue(&body, signer, &self.key).expect("a certificate")
        }
    }

    #[test]
    fn valid_certificates_are_linked_and_others_apart_each_once_in_order() {
        let account = AccountId::generate();
        let master = SigningKey::generate();
        let (first, second, server_made, bare) =
            (Device::new(), Device::new(), Device::new(), Device::new());
        let certs = vec![
            first.cert(account, None, &master, "first"),
            second.cert(account, Some(first.id()), &first.key, "second"),
            server_made.cert(account, None, &SigningKey::generate(), "server"),
        ];
        let ids = [
            server_made.id(),
            second.id(),
            first.id(),
            bare.id(),
            second.id(),
            server_made.id(),
        ];

        let devices = list(&ids, certs, Vec::new());

        let checked = check(&master.public_key(), &devices);

        let certs = &devices.certs;
        assert_eq!(
            checked,
            Checked {
                linked: vec![&certs[1], &certs[0]],
                revoked: vec![],
                apart: vec![server_made.id(), bare.id()],
                two_bodies: vec![],
            }
        );
    }

    #[test]
    fn revoked_devices_and_those_they_certified_are_apart_from_the_valid_ones() {
        let account = AccountId::generate();
        let master = SigningKey::generate();
        let (first, second, third) = (Device::new(), Device::new(), Device::new());
        let certs = vec![
            third.cert(account, None, &master, "third"),
            first.cert(account, None, &master, "revoked"),
            second.cert(account, Some(first.id()), &first.key, "certified"),
        ];
        let revocation = |device: DeviceId, key: &SigningKey, signer: DeviceId| {
            let body = skald_proto::RevocationBody {
                account,
                device,
                keep: None,
                signer: Some(signer),
                timestamp: 1_800_000_000,
                also: None,
                unknown: Unknown::default(),
            };
            skald_crypto::sign_revocation(&body, key).expect("a revocation")
        };
        let ids = [first.id(), second.id(), third.id()];
        // By the third device, which counts; and by the second for the third, signed by another key.
        let counting = revocation(first.id(), &third.key, third.id());
        let forged = revocation(third.id(), &first.key, second.id());

        let devices = list(&ids, certs, vec![counting, forged]);
        let checked = check(&master.public_key(), &devices);

        let certs = &devices.certs;
        assert_eq!(
            checked,
            Checked {
                linked: vec![&certs[0]],
                revoked: vec![&certs[1], &certs[2]],
                apart: vec![],
                two_bodies: vec![],
            }
        );
    }

    #[test]
    fn two_bodies_of_a_device_or_of_its_issuer_are_reported_once() {
        let account = AccountId::generate();
        let master = SigningKey::generate();
        let (issuer, issued) = (Device::new(), Device::new());
        let certs = vec![
            issuer.cert(account, None, &master, "one"),
            issuer.cert(account, None, &master, "two"),
            issued.cert(account, Some(issuer.id()), &issuer.key, "issued"),
        ];

        let devices = list(&[issued.id(), issuer.id()], certs, Vec::new());

        let checked = check(&master.public_key(), &devices);

        assert_eq!(
            checked,
            Checked {
                linked: vec![],
                revoked: vec![],
                apart: vec![issued.id(), issuer.id()],
                two_bodies: vec![issuer.id()],
            }
        );
    }

    #[test]
    fn names_shared_by_linked_devices_are_each_reported_once_with_their_devices() {
        let (account, master) = (AccountId::generate(), SigningKey::generate());
        let devices: Vec<Device> = (0..5).map(|_| Device::new()).collect();
        let certs: Vec<Cert> = ["ada", "bob", "ada", "eve", "bob"]
            .iter()
            .zip(&devices)
            .map(|(name, device)| device.cert(account, None, &master, name))
            .collect();
        let linked: Vec<&Cert> = certs.iter().collect();

        assert_eq!(
            shared_names(&linked),
            vec![
                ("ada".to_owned(), vec![devices[0].id(), devices[2].id()]),
                ("bob".to_owned(), vec![devices[1].id(), devices[4].id()]),
            ]
        );
        assert_eq!(shared_names(&linked[..2]), vec![]);
    }

    #[test]
    fn listing_heads_only_the_parts_it_has() {
        let master = SigningKey::generate();
        let device = Device::new();
        let cert = device.cert(AccountId::generate(), None, &master, "ada");
        let (id, other) = (device.id(), Device::new().id());
        let linked = format!(
            "Linked: certificates chained to the account's master key this machine pinned\n  {}\n",
            linked_line(&cert, id)
        );
        let apart =
            format!("Not linked: no valid certificate, so the server's word alone\n  {other}\n");
        let gone = Device::new();
        let gone_cert = gone.cert(AccountId::generate(), None, &master, "gone");
        let revoked = format!(
            "Revoked: chained to that key, and revoked since\n  {}  revoked\n",
            linked_line(&gone_cert, id)
        );
        let listed = |linked: Vec<&Cert>, revoked: Vec<&Cert>, apart: Vec<DeviceId>| {
            let checked = Checked {
                linked,
                revoked,
                apart,
                two_bodies: vec![],
            };
            listing(&checked, id, &Results::new())
        };
        assert_eq!(
            listed(vec![&cert], vec![&gone_cert], vec![other]),
            format!("{linked}{revoked}{apart}")
        );
        assert_eq!(
            listed(vec![&cert], vec![], vec![other]),
            format!("{linked}{apart}")
        );
        assert_eq!(listed(vec![&cert], vec![], vec![]), linked);
        assert_eq!(listed(vec![], vec![&gone_cert], vec![]), revoked);
        assert_eq!(listed(vec![], vec![], vec![other]), apart);
        assert_eq!(listed(vec![], vec![], vec![]), "");
    }

    #[test]
    fn line_shows_the_certificate_and_marks_this_device() {
        let master = SigningKey::generate();
        let device = Device::new();
        let cert = device.cert(AccountId::generate(), None, &master, "ada\u{1b}[2J");
        let id = device.id();
        assert_eq!(
            linked_line(&cert, id),
            format!("{id}  ada\\u{{1b}}[2J  client  issued 2027-01-15  (this device)")
        );
        assert!(!linked_line(&cert, Device::new().id()).contains("this device"));
    }

    #[test]
    fn date_is_the_utc_day() {
        for (seconds, day) in [
            (0, "1970-01-01"),
            (86_399, "1970-01-01"),
            (86_400, "1970-01-02"),
            (2_678_400, "1970-02-01"),
            (31_536_000, "1971-01-01"),
            (94_608_000, "1972-12-31"),
            (94_694_400, "1973-01-01"),
            (978_220_800, "2000-12-31"),
            (978_307_200, "2001-01-01"),
            (1_009_756_800, "2001-12-31"),
            (951_782_400, "2000-02-29"),
            (951_868_800, "2000-03-01"),
            (981_201_600, "2001-02-03"),
            (1_800_000_000, "2027-01-15"),
            (4_107_456_000, "2100-02-28"),
            (4_107_542_400, "2100-03-01"),
            (13_574_563_200, "2400-02-29"),
        ] {
            assert_eq!(date(seconds), day, "{seconds}");
        }
        // The latest `issued` gives a date, too.
        assert!(date(u64::MAX).len() >= 10);
    }
}
