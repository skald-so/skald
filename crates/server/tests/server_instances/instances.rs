//! What the tests share: other instances on the fixture's database, how soon an instance acts on
//! another's change, an instance that expires entries, and a database role of its own for an
//! instance whose connections a test ends.

use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{DeviceId, Header};

use crate::api::Person;
use crate::common::devices::random;
use crate::common::times::{RETENTION, WEEK};
use crate::common::{Server, TestDb, hex};
use crate::fixture::Fixture;
use crate::link::{Entry, seqs};
use crate::watch::Watch;
use crate::wire::{Socket, query};

/// How soon an instance acts on what another one stored, added or ended: at once.
pub const WAKE: Duration = Duration::from_secs(1);
/// How soon an instance closes the connections of a session another one ended.
pub const CLOSE: Duration = Duration::from_secs(3);
/// How soon an instance whose listening connection dropped listens again once the database lets
/// it, and sends its connections what they missed.
pub const RELISTEN: Duration = Duration::from_secs(15);
/// The extra arguments of an instance that expires entries after [`WEEK`].
pub fn expiring() -> [String; 2] {
    [RETENTION.to_owned(), WEEK.to_string()]
}

/// Another instance of the fixture's server on its database, with its public address, its sign-in
/// and the extra arguments `args`.
pub fn instance(fixture: &Fixture, args: &[String]) -> Server {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    fixture.team.flow.stand.instance(&args, &[])
}

/// A connection of `person`'s laptop to `server` that only sends: it states no cursor, so no entry
/// comes on it.
pub fn sender(server: &Server, fixture: &Fixture, person: &Person) -> Socket {
    Socket::to(server, &fixture.team.flow, person, &person.laptop)
}

/// A connection of `person`'s laptop to `server` from cursor 0, whose mailbox is empty.
#[track_caller]
pub fn watch(server: &Server, fixture: &Fixture, person: &Person) -> Watch {
    Watch::from(server, &fixture.team.flow, person, 0, &[])
}

/// Stores on `sender` an envelope to `recipient` in the fixture's channel, with `report` true;
/// gives its header and bytes.
#[track_caller]
pub fn store(sender: &mut Socket, fixture: &Fixture, recipient: DeviceId) -> (Header, Vec<u8>) {
    let header = fixture.to(recipient);
    let sent = crate::wire::envelope(&header);
    sender.store(&header, &sent);
    (header, sent)
}

/// Waits until `device`'s mailbox holds `count` entries or more, for up to `within`; then for
/// another `QUIET`, so that an entry added twice would be there too. Gives the `seq`s it holds.
#[track_caller]
pub fn settled(db: &TestDb, device: DeviceId, count: usize, within: Duration) -> Vec<i64> {
    let deadline = Instant::now() + within;
    loop {
        let found = seqs(db, device);
        if found.len() >= count {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{device}'s mailbox holds {found:?}, not {count} entries, after {within:?}"
        );
        thread::sleep(Duration::from_millis(100));
    }
    thread::sleep(crate::link::QUIET);
    seqs(db, device)
}

/// A database role that can sign in to the test database as a superuser; dropped on drop.
pub struct Role<'a> {
    db: &'a TestDb,
    pub name: String,
    password: String,
}

impl<'a> Role<'a> {
    pub fn create(db: &'a TestDb) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "skald_instance_{}_{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let password = hex(&random());
        db.execute(&format!(
            "CREATE ROLE {name} LOGIN SUPERUSER PASSWORD '{password}'"
        ));
        Role { db, name, password }
    }

    /// Another instance of the fixture's server, connecting to its database as this role.
    pub fn instance(&self, fixture: &Fixture) -> Server {
        let url = self.db.url_as(&self.name, &self.password);
        fixture.team.flow.stand.instance_at(&url, &[], &[])
    }

    /// Lets this role sign in to the database, or not; its open connections stay.
    pub fn set_login(&self, login: bool) {
        let login = if login { "LOGIN" } else { "NOLOGIN" };
        self.db
            .execute(&format!("ALTER ROLE {} {login}", self.name));
    }

    /// The process IDs of this role's listening connections: backends whose last statement is
    /// `LISTEN` or `UNLISTEN`.
    pub fn listening(&self) -> Vec<i32> {
        let name = self.name.clone();
        query(self.db, async |conn| {
            sqlx::query_scalar(
                r"SELECT pid FROM pg_stat_activity
                  WHERE usename = $1 AND query ~* '^\s*(un)?listen\s'",
            )
            .bind(name)
            .fetch_all(conn)
            .await
        })
    }

    /// Waits up to `RELISTEN` for this role's listening connections, then ends them in
    /// PostgreSQL; gives their process IDs.
    #[track_caller]
    pub fn end_listening(&self) -> Vec<i32> {
        let deadline = Instant::now() + RELISTEN;
        let pids = loop {
            let pids = self.listening();
            if !pids.is_empty() {
                break pids;
            }
            assert!(
                Instant::now() < deadline,
                "no listening connection of the instance after {RELISTEN:?}"
            );
            thread::sleep(Duration::from_millis(100));
        };
        let list: Vec<String> = pids.iter().map(i32::to_string).collect();
        self.db.execute(&format!(
            "SELECT pg_terminate_backend(pid, 5000) FROM pg_stat_activity
             WHERE pid IN ({})",
            list.join(", ")
        ));
        pids
    }
}

impl Drop for Role<'_> {
    fn drop(&mut self) {
        if thread::panicking() {
            // The role goes with the test PostgreSQL server.
            return;
        }
        let name = &self.name;
        self.db.execute(&format!(
            "SELECT pg_terminate_backend(pid, 5000) FROM pg_stat_activity WHERE usename = '{name}'"
        ));
        self.db.execute(&format!("DROP ROLE IF EXISTS {name}"));
    }
}

/// `role`'s instance stops listening, alice's `sender` stores an envelope to bob's laptop, and the
/// instance listens again: `bob` gets it, the `seq`th, and then nothing, no notice again.
#[track_caller]
pub fn relisten_quietly(
    role: &Role,
    fixture: &Fixture,
    sender: &mut Socket,
    bob: &mut Watch,
    seq: u64,
) {
    role.set_login(false);
    role.end_listening();
    let (_, missed) = store(sender, fixture, fixture.team.bob.laptop.id);
    role.set_login(true);
    assert_eq!(bob.entry_within(RELISTEN), Entry::Message(seq, missed));
    bob.assert_quiet();
}
