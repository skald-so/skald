//! What the tests of the cloud limits share: the settings and problems, envelopes of a payload
//! length of their own, sends of any length, a server started again with the settings as
//! environment variables, months passing as the database shows them, and devices written to the
//! database.

use std::thread;
use std::time::Duration;

use skald_proto::{Cbor, DeviceKind, Envelope, Header, HpkePayload, Stored};

use crate::api::Person;
use crate::common::TestDb;
use crate::common::device_flow::{DeviceFlow, INTERVAL};
use crate::common::devices::{Device, random, write_devices};
use crate::common::sign_in::SignIn;
use crate::common::times::moved_back;
use crate::fixture::Fixture;
use crate::wire::{Answer, Socket, assert_nowhere, key_of, query};

/// The settings, as arguments and as environment variables.
pub const SIZE: &str = "--message-size-limit";
pub const SIZE_ENV: &str = "SKALD_MESSAGE_SIZE_LIMIT";
pub const MONTHLY: &str = "--monthly-messages-limit";
pub const MONTHLY_ENV: &str = "SKALD_MONTHLY_MESSAGES_LIMIT";
pub const DEVICES: &str = "--devices-limit";
pub const DEVICES_ENV: &str = "SKALD_DEVICES_LIMIT";

/// The values of `docs/dev/architecture.md`, "Cloud limits": a message text of 64 KB and 50
/// devices per member.
pub const KB_64: usize = 64 * 1024;
pub const DEVICES_50: usize = 50;

/// The problems of `send-refused` over a limit.
pub const TOO_LARGE: &str = "too-large";
pub const QUOTA_EXCEEDED: &str = "quota-exceeded";
/// The problem of a registration over the devices limit.
pub const OVER_DEVICES: &str = "https://skald.so/problems/devices-limit";

/// An envelope of `header` whose payload, in the form of an `hpke` payload with random bytes, is
/// exactly `len` bytes long.
pub fn sized(header: &Header, len: usize) -> Vec<u8> {
    let noise: Vec<u8> = (0..len.div_ceil(32)).flat_map(|_| random()).collect();
    let enc = random();
    let payload = (len.saturating_sub(48)..=len)
        .map(|ct_len| {
            let ct = noise[..ct_len].to_vec();
            HpkePayload { enc, ct }.to_cbor()
        })
        .find(|payload| payload.len() == len)
        .unwrap_or_else(|| panic!("no hpke payload of {len} bytes"));
    Envelope {
        header: header.clone(),
        payload,
    }
    .to_cbor()
}

/// Sends `envelope` of `header`, of any length, with `report` true, and gives the server's answer,
/// checking that it names the envelope's send-key.
#[track_caller]
pub fn answer_to(socket: &mut Socket, header: &Header, envelope: &[u8]) -> Answer {
    socket.send(envelope);
    let answer = socket.answer();
    let key = match &answer {
        Answer::Stored(stored) => &stored.send_key,
        Answer::Refused(key, _) => key,
    };
    assert_eq!(key, &key_of(header), "the answer's send-key");
    answer
}

/// Sends `envelope` of `header` and checks that the server stores it; gives the `stored`.
#[track_caller]
pub fn assert_stored(socket: &mut Socket, header: &Header, envelope: &[u8]) -> Stored {
    match answer_to(socket, header, envelope) {
        Answer::Stored(stored) => stored,
        refused => panic!("stored, not {refused:?}"),
    }
}

/// Sends `envelope` of `header` and checks that the server refuses it with `problem` and stores
/// nothing of it in `db`; `what` names the case.
#[track_caller]
pub fn assert_refused_with(
    db: &TestDb,
    socket: &mut Socket,
    header: &Header,
    envelope: &[u8],
    problem: &str,
    what: &str,
) {
    match answer_to(socket, header, envelope) {
        Answer::Refused(_, refused) => assert_eq!(refused, problem, "{what}: the problem"),
        stored => panic!("{what}: send-refused with {problem}, not {stored:?}"),
    }
    assert_nowhere(db, envelope);
}

/// The fixture of storing sent envelopes on a server with the extra arguments `args`, and alice's
/// connection; GitHub user IDs from `github` on.
pub fn with_alice(github: u64, args: &[&str]) -> (Fixture, Socket) {
    let fixture = Fixture::with(github, args);
    let alice = Socket::of(&fixture.team.flow, &fixture.team.alice);
    (fixture, alice)
}

/// Stops `stand`'s server and starts another on its database, its codes polling every second, with
/// the environment variables `envs`: the settings it applies; gives its device flow.
pub fn restarted(stand: SignIn, envs: &[(&str, &str)]) -> DeviceFlow {
    let SignIn { db, server, .. } = stand;
    drop(server);
    DeviceFlow::of(SignIn::on_with(db, &[INTERVAL, "1"], envs))
}

/// The tables whose times and months the server counts a month's messages by.
const COUNTED: [&str; 4] = ["envelopes", "sends", "entries", "message_counts"];

/// Moves every time and month the server keeps of envelopes, sends, entries and message counts one
/// month back, as if the month had ended: each `timestamptz` and `date` column of [`COUNTED`].
pub fn to_the_month_before(db: &TestDb) {
    db.execute(&moved_back(&COUNTED, "1 month"));
}

/// How long a test of the monthly count takes at most, past its setup.
const COUNTING: Duration = Duration::from_secs(180);

/// Waits until the month in UTC, by the database's clock, ends no sooner than [`COUNTING`] from
/// now, so that a test's messages fall in one month.
pub fn clear_of_the_month_end(db: &TestDb) {
    let left: f64 = query(db, async |conn| {
        sqlx::query_scalar(
            "SELECT EXTRACT(EPOCH FROM date_trunc('month', now() AT TIME ZONE 'UTC')
                    + interval '1 month' - (now() AT TIME ZONE 'UTC'))::float8",
        )
        .fetch_one(conn)
        .await
    });
    if left < COUNTING.as_secs_f64() {
        thread::sleep(Duration::from_secs_f64(left.max(0.0) + 2.0));
    }
}

/// Writes to `db` `count` devices of `kind` of `person`'s account, each with a certificate issued
/// by the account's master key if `certified`, as linking registers them; gives them.
pub fn add_devices(
    db: &TestDb,
    person: &Person,
    count: usize,
    kind: DeviceKind,
    certified: bool,
) -> Vec<Device> {
    let devices: Vec<Device> = (0..count)
        .map(|at| Device::new(&format!("added-{at}")))
        .collect();
    let certs: Vec<Option<Vec<u8>>> = devices
        .iter()
        .map(|device| certified.then(|| device.cert_as(person.account, kind, &person.master)))
        .collect();
    let rows: Vec<(&Device, Option<&[u8]>)> = devices
        .iter()
        .zip(&certs)
        .map(|(device, cert)| (device, cert.as_deref()))
        .collect();
    write_devices(db, person.account, kind, &rows);
    devices
}
