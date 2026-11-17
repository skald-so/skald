//! What the tests share: a second machine of an account, linked by hand as a link code's link will,
//! and the lines the played devices say.

use std::fs;

use rusqlite::{Connection, params};
use skald_crypto::issue;
use skald_proto::{
    Cbor as _, CertBody, ChannelId, DeviceId, DeviceKind, MessageId, SpaceId, Unknown,
};

use crate::fixtures::hex;
use crate::keys;
use crate::machine::{FIRST_DEVICE, Machine, NO};
use crate::reading::{Said, says};
use crate::talk::{Line, Talk};
use crate::world::{Person, World, now};

/// Bob's device, the talk's first played one, and Carol's, the second.
pub const BOB: usize = 0;
pub const CAROL: usize = 1;

/// The local state's database in a machine's state directory.
const STATE_FILE: &str = "state.sqlite3";

/// The talk's played device `who` says `text` in `at`'s channel, its `counter`th line there, as
/// [`says`] does: its message, then the message's entry. Gives the message, its `msg_id` and the
/// entry's bytes.
#[track_caller]
pub fn said_by(
    talk: &mut Talk,
    who: usize,
    (at, counter): ((SpaceId, ChannelId), u64),
    text: &'static str,
) -> (Said, MessageId, Vec<u8>) {
    let from = talk.address(&talk.played[who].person);
    let line = Line::new(&from, counter, text);
    says(&talk.world.cloud, &mut talk.played[who], at, &line)
}

/// A second machine of `first`'s account, with the host name `host`, linked by hand as a link
/// code's link will: it signs in as no first device; the test certifies its device with the keys
/// of `first`'s machine, sets the certificate up in the server's database, and writes it, the
/// pinned master key and the spaces of `first`'s machine into the second machine's local state.
/// `first`'s machine accepted the latest revision of each space at its last run, so those are the
/// revisions the second machine starts from. Gives the person on the second machine.
#[track_caller]
pub fn second_machine(world: &World, first: &Person, host: &'static str) -> Person {
    let cloud = &world.cloud;
    let machine = Machine::new().with_host_name(host);
    let page = world.browser.page();
    let signed_in = cloud.signed_in(&page, &machine, &first.user, &[FIRST_DEVICE, NO]);
    assert!(signed_in.success, "{signed_in:?}");
    let token = cloud.last_token();
    let waiting: Vec<DeviceId> = (cloud.devices(&token).into_iter())
        .filter(|device| device.cert.is_none())
        .map(|device| device.id)
        .collect();
    assert_eq!(waiting.len(), 1, "the devices waiting for a certificate");
    let device = waiting[0];
    let holders = machine.files_holding(&token);
    assert_eq!(holders.len(), 1, "the secrets file: {holders:?}");
    let secrets = fs::read(&holders[0]).expect("read the secrets file");
    let signing = (keys::public_keys(&secrets, keys::ed25519).into_iter())
        .find(|public| DeviceId::from_signing_key(public) == device)
        .expect("the secrets file holds the device's signing key");
    let key = keys::signing_key(&holders, &signing);
    // The device's HPKE public key, as it registered it: the one the server's database holds as a
    // device's `hpke` whose private key the secrets file holds.
    let tables: Vec<String> = (cloud.rows().iter())
        .map(|table| table.to_lowercase())
        .filter(|table| table.contains(&device.to_string()))
        .collect();
    let hpke = (keys::public_keys(&secrets, keys::x25519).into_iter())
        .find(|public| {
            let column = format!("<hpke>{}</hpke>", hex(public));
            tables.iter().any(|table| table.contains(&column))
        })
        .expect("the secrets file holds the device's registered HPKE key");
    let body = CertBody {
        account: first.account,
        device,
        kind: DeviceKind::Client,
        signing,
        hpke,
        issuer: Some(first.device),
        issued: now(),
        ssh: None,
        name: host.to_owned(),
        unknown: Unknown::default(),
    };
    let cert = issue(&body, &first.key, &key)
        .expect("a certificate")
        .to_cbor();
    cloud.execute(&format!(
        "UPDATE devices SET cert = '\\x{}'::bytea WHERE id = '{device}'::uuid",
        hex(&cert)
    ));
    write_link(&machine, first, &cert);
    Person {
        user: first.user.clone(),
        machine,
        token,
        account: first.account,
        master: first.master,
        handle: first.handle.clone(),
        device,
        host,
        key,
    }
}

/// Writes into the local state of `machine`, whose device waits for linking, its certificate
/// `cert`, in CBOR, the master key `first`'s machine pinned, and each space as `first`'s machine
/// accepted it.
fn write_link(machine: &Machine, first: &Person, cert: &[u8]) {
    let state = Connection::open(machine.state_dir().join(STATE_FILE))
        .expect("open the second machine's local state");
    let linked = state
        .execute(
            "UPDATE device SET link = 'certified', master = ?1, cert = ?2 WHERE link = 'waiting'",
            params![first.master.as_slice(), cert],
        )
        .expect("write the certificate and the master key");
    assert_eq!(linked, 1, "no device waiting for linking");
    let accepted = first.machine.state_dir().join(STATE_FILE);
    state
        .execute(
            "ATTACH DATABASE ?1 AS accepted",
            params![accepted.to_string_lossy()],
        )
        .expect("open the first machine's local state");
    state
        .execute_batch(
            "INSERT INTO space (space, name, anchor, accepted, personal)
                 SELECT space, name, anchor, accepted, personal FROM accepted.space;
             INSERT INTO accepted_revision (space, revision, hash)
                 SELECT space, revision, hash FROM accepted.accepted_revision;
             DETACH DATABASE accepted;",
        )
        .expect("write the spaces");
}
