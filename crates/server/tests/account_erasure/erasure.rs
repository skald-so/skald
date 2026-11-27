//! What the tests share: moving a deletion back, the account's data as the server gave it before
//! the deletion, what a shared space keeps of it, and the rows of the database that hold any of
//! that data beyond what is kept.

use std::collections::BTreeSet;
use std::fmt::Display;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::{AccountId, DeviceId, SpaceId};

use crate::api::{Person, Session};
use crate::common::device_flow::{json, text};
use crate::common::devices::{devices_of, unb64};
use crate::common::{TestDb, hex};
use crate::history::Item;

/// How old a deletion is once the server erases the account.
pub const ERASED_AFTER: &str = "30 days";
/// A deletion just short of that.
pub const NOT_YET: &str = "29 days 23 hours";
/// How soon the server erases an account once its deletion is old enough.
pub const ERASURE: Duration = Duration::from_secs(30);
/// How often a test looks whether it has.
const LOOK: Duration = Duration::from_millis(250);

/// Moves the deletion of `account`, a deleted account, back by `interval`, such as `30 days`, as if
/// that much time had passed since it.
pub fn move_deletion_back(db: &TestDb, account: AccountId, interval: &str) {
    db.execute(&format!(
        "DO $$
         BEGIN
           UPDATE accounts SET deleted_at = deleted_at - interval '{interval}'
           WHERE id = '{account}'::uuid AND deleted_at IS NOT NULL;
           IF NOT FOUND THEN
             RAISE EXCEPTION 'no deletion of the account %', '{account}';
           END IF;
         END $$"
    ));
}

/// The forms in which a row holds the ID `id` of `bytes`: as text and as the hex of its bytes.
fn forms(id: impl Display, bytes: &[u8; 16]) -> [String; 2] {
    [id.to_string(), hex(bytes)]
}

/// Whether `row` holds any of `forms`.
fn holds(row: &str, forms: &[String]) -> bool {
    forms.iter().any(|form| row.contains(form.as_str()))
}

/// The bytes `field` of `body` holds as base64url; fails if it holds none.
fn bytes_of(body: &Value, field: &str) -> Vec<u8> {
    let given = body[field].as_str();
    unb64(given.unwrap_or_else(|| panic!("no {field} in {body}")))
}

/// An account's data, as the server gave it before the account's deletion.
pub struct Traces {
    /// The account's handle, for the tests' messages.
    pub who: String,
    pub account: AccountId,
    account_forms: [String; 2],
    devices: Vec<(DeviceId, [String; 2])>,
    /// Whether a row may still hold its devices' IDs, as a channel's group state does while its
    /// devices are in the group.
    devices_in_groups: bool,
    /// Texts no row may hold: its username and its sign-in method's subject.
    texts: Vec<String>,
    /// What no row may hold, each named, as hex: its account record, anchors objects and the wraps
    /// to its recovery key.
    gone: Vec<(String, String)>,
    /// What only a row that holds what a space keeps may hold, as hex: its master public key.
    master: String,
    /// What no row that holds its ID may hold, as hex: its master backup.
    backup: String,
}

impl Traces {
    /// The data of `person`'s account, the GitHub user `github`, with its devices `devices`: read
    /// from the server through its session, which must still work.
    #[track_caller]
    pub fn of(person: &Person, github: u64, devices: &[DeviceId]) -> Self {
        let session = &person.session;
        let response = session.call("get", "/session", None);
        assert_eq!(response.status, 200, "GET /session: {}", text(&response));
        let username = json(&response)["username"].as_str().map(str::to_owned);
        let username = username.unwrap_or_else(|| panic!("no username: {}", text(&response)));
        let response = session.call("get", "/account/master", None);
        assert_eq!(
            response.status,
            200,
            "GET /account/master: {}",
            text(&response)
        );
        let master = json(&response);
        let mut gone = vec![(
            "its account record".to_owned(),
            hex(&bytes_of(&master, "record")),
        )];
        for anchors in session.anchors() {
            gone.push(("its anchors object".to_owned(), hex(&anchors)));
        }
        Traces {
            who: person.handle.to_string(),
            account: person.account,
            account_forms: forms(person.account, person.account.as_bytes()),
            devices: (devices.iter())
                .map(|&device| (device, forms(device, device.as_bytes())))
                .collect(),
            devices_in_groups: false,
            texts: vec![username, github.to_string()],
            gone,
            master: hex(&bytes_of(&master, "master")),
            backup: hex(&bytes_of(&master, "backup")),
        }
    }

    /// These traces with `wraps`, wraps to the account's recovery key, which no row may hold.
    pub fn with_wraps(mut self, wraps: &[&Item]) -> Self {
        for wrap in wraps {
            (self.gone).push(("a wrap to its recovery key".to_owned(), hex(wrap.bytes())));
        }
        self
    }

    /// These traces, a row allowed to hold the account's devices' IDs.
    pub fn devices_in_groups(mut self) -> Self {
        self.devices_in_groups = true;
        self
    }

    /// Whether `row` holds any of this data.
    fn in_row(&self, row: &str) -> bool {
        holds(row, &self.account_forms)
            || self.devices.iter().any(|(_, forms)| holds(row, forms))
            || self.texts.iter().any(|text| row.contains(text.as_str()))
            || self
                .gone
                .iter()
                .any(|(_, hexed)| row.contains(hexed.as_str()))
            || row.contains(self.master.as_str())
    }
}

/// What the tests let a row keep of an account's data: the hex of the items a shared space keeps,
/// the IDs of those spaces, and how many rows may hold the account's ID alone, as the mark that it
/// was deleted.
pub struct Kept {
    items: Vec<String>,
    spaces: Vec<[String; 2]>,
    marks: usize,
}

impl Kept {
    /// Nothing: once no space keeps the account, no row holds anything of it.
    pub fn nothing() -> Self {
        Kept {
            items: Vec::new(),
            spaces: Vec::new(),
            marks: 0,
        }
    }

    /// The mark that the account was deleted alone, for an account that shared no space.
    pub fn mark() -> Self {
        Kept {
            marks: 1,
            ..Self::nothing()
        }
    }

    /// What `space` keeps of an account whose devices are `devices`, as `session`, a member's, gets
    /// it: the space's revisions, the certificates of those devices and the space's revocations,
    /// and the mark.
    #[track_caller]
    pub fn of_space(session: &Session, space: SpaceId, devices: &[DeviceId]) -> Self {
        let mut items: Vec<String> = session.revisions(space).iter().map(|r| hex(r)).collect();
        let certs = devices_of(&session.devices_response(space));
        for device in devices {
            if let Some(Some(cert)) = certs.get(&device.to_string()) {
                items.push(hex(cert));
            }
        }
        let revocations = session.space_revocations(space);
        items.extend(revocations.iter().map(|revocation| hex(revocation)));
        Kept {
            items,
            spaces: vec![forms(space, space.as_bytes())],
            marks: 1,
        }
    }

    /// This, with `entries`, history entries the account's devices wrote in the space.
    pub fn with_entries(mut self, entries: &[&Item]) -> Self {
        (self.items).extend(entries.iter().map(|entry| hex(entry.bytes())));
        self
    }
}

/// Each row of `db` that holds `traces`' data beyond what `kept` lets it keep, with what it holds;
/// none once the account is erased. Every table counts, those later migrations add too.
pub fn leftovers(db: &TestDb, traces: &Traces, kept: &Kept) -> Vec<String> {
    let mut found = Vec::new();
    let mut marks = Vec::new();
    for row in db.rows() {
        let mut held: Vec<String> = Vec::new();
        for text in &traces.texts {
            if row.contains(text.as_str()) {
                held.push(format!("{text:?}"));
            }
        }
        for (what, hexed) in &traces.gone {
            if row.contains(hexed.as_str()) {
                held.push(what.clone());
            }
        }
        let keeps = kept.items.iter().any(|item| row.contains(item.as_str()));
        if !keeps {
            if row.contains(traces.master.as_str()) {
                held.push("its master key".to_owned());
            }
            if !traces.devices_in_groups {
                for (device, forms) in &traces.devices {
                    if holds(&row, forms) {
                        held.push(format!("its device {device}"));
                    }
                }
            }
            if holds(&row, &traces.account_forms) {
                if row.contains(traces.backup.as_str()) {
                    held.push("its master backup".to_owned());
                }
                let beside_space = row.starts_with("roster_members: ")
                    && kept.spaces.iter().any(|space| holds(&row, space));
                if held.is_empty() && !beside_space {
                    marks.push(row);
                    continue;
                }
            }
        }
        if !held.is_empty() {
            found.push(format!("{}: {row}", held.join(", ")));
        }
    }
    if marks.len() > kept.marks {
        let allowed = kept.marks;
        found.extend(
            marks
                .iter()
                .map(|row| format!("its ID, in more than {allowed} row: {row}")),
        );
    }
    found
}

/// Waits for up to [`ERASURE`] until no row of `db` holds `traces`' data beyond what `kept` lets it
/// keep; fails naming the rows that still do.
#[track_caller]
pub fn erased_within(db: &TestDb, traces: &Traces, kept: &Kept) {
    let deadline = Instant::now() + ERASURE;
    loop {
        let left = leftovers(db, traces, kept);
        if left.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}'s data after {ERASURE:?}, in rows of these tables:\n{}",
            traces.who,
            left.join("\n")
        );
        thread::sleep(LOOK);
    }
}

/// The rows of `db` that hold any of `traces`' data.
pub fn rows_with(db: &TestDb, traces: &Traces) -> BTreeSet<String> {
    let mut rows: BTreeSet<String> = db.rows().into_iter().collect();
    rows.retain(|row| traces.in_row(row));
    rows
}
