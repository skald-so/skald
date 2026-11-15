//! Server instances waking each other: `docs/dev/architecture.md`, "Network". An instance that adds
//! mailbox entries, stores a roster revision or ends a session tells the others through PostgreSQL
//! `NOTIFY`, in the transaction that does it. Each instance listens, and wakes the device's
//! connections, sends the `space-notice` or closes that session's connections. A notification names
//! what changed and carries no envelope. An instance whose listening connection drops listens
//! again, then sends its connections what they missed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_proto::{AccountId, DeviceId, SpaceId};
use sqlx::postgres::{PgListener, PgNotification, PgPoolOptions};
use sqlx::{PgConnection, PgPool};
use tokio::sync::watch;

use crate::DATABASE_TIMEOUT;
use crate::connections::Connections;
use crate::delivery::Wakers;
use crate::notices::Notices;

/// The channel the instances notify each other on, and the statement that listens on it.
const CHANNEL: &str = "skald_instances";
const LISTEN: &str = r#"LISTEN "skald_instances""#;
/// How long an instance waits before it tries again to listen.
const RETRY: Duration = Duration::from_secs(2);
/// How long the listening connection may go without a notification before the instance checks it,
/// and how long the check may take: a connection lost without a close sends nothing.
const CHECK_AFTER: Duration = Duration::from_secs(3);
const CHECK_WAIT: Duration = Duration::from_secs(2);
/// This instance: how it names itself in its notifications, the latest revision of each space it
/// knows of, and whether it has listened yet.
#[derive(Clone)]
pub(crate) struct Instance {
    id: Arc<str>,
    latest: Arc<Mutex<HashMap<SpaceId, i64>>>,
    listened: Arc<watch::Sender<bool>>,
}

impl Instance {
    /// An instance with a random name.
    pub(crate) fn new() -> Self {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).expect("the operating system's random generator failed");
        let id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Instance {
            id: id.into(),
            latest: Arc::default(),
            listened: Arc::new(watch::Sender::new(false)),
        }
    }

    /// Waits until this instance has listened to the others, so that a connection it takes from
    /// then on misses nothing they tell.
    pub(crate) async fn listening(&self) {
        // The sender lives as long as this instance.
        let _ = self
            .listened
            .subscribe()
            .wait_for(|listened| *listened)
            .await;
    }

    /// Records that `space` has the revision `revision`.
    pub(crate) fn record(&self, space: SpaceId, revision: i64) {
        let mut latest = self.latest.lock().expect("the latest revisions");
        let known = latest.entry(space).or_default();
        *known = (*known).max(revision);
    }

    /// Whether this instance knows of the revision `revision` of `space`, or a later one.
    fn knows(&self, space: SpaceId, revision: i64) -> bool {
        let latest = self.latest.lock().expect("the latest revisions");
        latest.get(&space).is_some_and(|known| *known >= revision)
    }

    /// Tells the other instances of `events` on `conn`, once its transaction commits.
    pub(crate) async fn tell(
        &self,
        conn: &mut PgConnection,
        events: &[Event],
    ) -> Result<(), sqlx::Error> {
        if events.is_empty() {
            return Ok(());
        }
        let payloads: Vec<String> = events.iter().map(|event| event.payload(self)).collect();
        sqlx::query("SELECT pg_notify($1, payload) FROM unnest($2::text[]) AS payload")
            .bind(CHANNEL)
            .bind(payloads)
            .execute(conn)
            .await?;
        Ok(())
    }
}

/// What an instance tells the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    /// The device's mailbox has new entries.
    Entries(DeviceId),
    /// The space's revision with this number is stored.
    Revision(SpaceId, i64),
    /// The session whose token's SHA-256 this is has ended.
    SessionEnded(Vec<u8>),
}

impl Event {
    /// The notification's payload: the instance's name, then the event.
    fn payload(&self, instance: &Instance) -> String {
        let id = &instance.id;
        match self {
            Self::Entries(device) => format!("{id} entries {device}"),
            Self::Revision(space, revision) => format!("{id} revision {space} {revision}"),
            Self::SessionEnded(session) => {
                format!("{id} session {}", URL_SAFE_NO_PAD.encode(session))
            }
        }
    }

    /// The name of the instance that sent `payload`, and its event; `None` if it is none of those
    /// this instance knows.
    fn parse(payload: &str) -> Option<(&str, Event)> {
        let mut words = payload.split(' ');
        let instance = words.next()?;
        let event = match (words.next()?, words.next()?, words.next()) {
            ("entries", device, None) => Self::Entries(device.parse().ok()?),
            ("revision", space, Some(revision)) => {
                Self::Revision(space.parse().ok()?, revision.parse().ok()?)
            }
            ("session", session, None) => Self::SessionEnded(URL_SAFE_NO_PAD.decode(session).ok()?),
            _ => return None,
        };
        words.next().is_none().then_some((instance, event))
    }
}

/// What an instance's connections get from the others.
pub(crate) struct Local {
    pub(crate) db: PgPool,
    pub(crate) wakers: Arc<Wakers>,
    pub(crate) notices: Arc<Notices>,
    pub(crate) connections: Arc<Connections>,
}

/// Listens to the other instances for good, as `instance`, and acts on their notifications for
/// `local`. Each time it starts to listen, also again after the listening connection dropped, it
/// sends what its connections may have missed.
pub(crate) async fn listen(instance: Instance, local: Local) {
    loop {
        match open(&instance, &local).await {
            Ok(mut listener) => {
                instance.listened.send_replace(true);
                while let Some(notification) = next(&mut listener).await {
                    act(&instance, &local, notification.payload()).await;
                }
            }
            Err(err) => {
                eprintln!("skald-server: cannot listen to the other instances: {err}");
                tokio::time::sleep(RETRY).await;
            }
        }
    }
}

/// The next notification on `listener`; `None` once its connection is lost, also without a close.
async fn next(listener: &mut PgListener) -> Option<PgNotification> {
    loop {
        match tokio::time::timeout(CHECK_AFTER, listener.try_recv()).await {
            Ok(Ok(Some(notification))) => return Some(notification),
            Ok(Ok(None) | Err(_)) => return None,
            Err(_) => {
                // Listening again changes nothing, so the connection still shows as listening.
                let check = sqlx::raw_sql(LISTEN).execute(&mut *listener);
                if !matches!(tokio::time::timeout(CHECK_WAIT, check).await, Ok(Ok(_))) {
                    return None;
                }
            }
        }
    }
}

/// Listens on a connection of its own, sends `local` what it may have missed, then records the
/// latest revision of each space.
async fn open(instance: &Instance, local: &Local) -> Result<PgListener, sqlx::Error> {
    // Its own pool, so that the listening connection takes none of the server's.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(DATABASE_TIMEOUT)
        .max_lifetime(None)
        .idle_timeout(None)
        .connect_lazy_with((*local.db.connect_options()).clone());
    let mut listener = PgListener::connect_with(&pool).await?;
    // A lost connection ends the listener, and `listen` opens a new one.
    listener.eager_reconnect(false);
    listener.listen(CHANNEL).await?;
    // A revision stored from here on comes as a notification too.
    let latest: Vec<(String, i64)> = sqlx::query_as(
        "SELECT space_id::text, max(revision) FROM roster_revisions GROUP BY space_id",
    )
    .fetch_all(&local.db)
    .await?;
    let latest: Vec<(SpaceId, i64)> = latest
        .into_iter()
        .map(|(space, revision)| (space.parse().expect("a stored space ID"), revision))
        .collect();
    catch_up(instance, local, &latest).await?;
    for (space, revision) in latest {
        instance.record(space, revision);
    }
    Ok(listener)
}

/// Acts on the notification `payload` for `local`, unless `instance` sent it: wakes, notifies or
/// closes connections.
async fn act(instance: &Instance, local: &Local, payload: &str) {
    let Some((from, event)) = Event::parse(payload) else {
        // A notification of a later server version.
        return;
    };
    if *from == *instance.id {
        return;
    }
    match event {
        Event::Entries(device) => local.wakers.wake(device),
        Event::Revision(space, revision) => {
            instance.record(space, revision);
            match members(&local.db, space, revision).await {
                Ok(accounts) => local.notices.notify(accounts, space),
                Err(err) => eprintln!("skald-server: cannot read a revision's members: {err}"),
            }
        }
        Event::SessionEnded(session) => local.connections.end_session(&session),
    }
}

/// The accounts that the revision `revision` of `space` lists as members.
async fn members(
    db: &PgPool,
    space: SpaceId,
    revision: i64,
) -> Result<Vec<AccountId>, sqlx::Error> {
    let found: Vec<(String,)> = sqlx::query_as(
        "SELECT account_id::text FROM roster_members WHERE space_id = $1::uuid AND revision = $2",
    )
    .bind(space.to_string())
    .bind(revision)
    .fetch_all(db)
    .await?;
    Ok(found
        .into_iter()
        .map(|(account,)| account.parse().expect("a stored account ID"))
        .collect())
}

/// Sends `local`'s connections what the other instances may have told while `instance` did not
/// listen: wakes each device's connections, closes each connection whose session has ended, and
/// sends each connected account the notices of the revisions in `latest`, the latest of each space,
/// that the instance does not know, all of them before it first listens.
async fn catch_up(
    instance: &Instance,
    local: &Local,
    latest: &[(SpaceId, i64)],
) -> Result<(), sqlx::Error> {
    local.wakers.wake_all();
    let sessions = local.connections.sessions();
    let live: Vec<(Vec<u8>,)> =
        sqlx::query_as("SELECT token_hash FROM sessions WHERE token_hash = ANY($1)")
            .bind(&sessions)
            .fetch_all(&local.db)
            .await?;
    for session in sessions {
        if !live.iter().any(|(live,)| *live == session) {
            local.connections.end_session(&session);
        }
    }
    let (spaces, revisions): (Vec<String>, Vec<i64>) = latest
        .iter()
        .filter(|(space, revision)| !instance.knows(*space, *revision))
        .map(|(space, revision)| (space.to_string(), *revision))
        .unzip();
    let accounts: Vec<String> = local
        .notices
        .accounts()
        .iter()
        .map(ToString::to_string)
        .collect();
    let notified: Vec<(String, String)> = sqlx::query_as(
        "SELECT m.space_id::text, m.account_id::text
         FROM roster_members m
         JOIN unnest($1::uuid[], $2::bigint[]) AS l (space_id, revision) USING (space_id, revision)
         WHERE m.account_id = ANY($3::uuid[])",
    )
    .bind(spaces)
    .bind(revisions)
    .bind(accounts)
    .fetch_all(&local.db)
    .await?;
    for (space, account) in notified {
        local.notices.notify(
            [account.parse().expect("a stored account ID")],
            space.parse().expect("a stored space ID"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance() -> Instance {
        Instance {
            id: "a1".into(),
            latest: Arc::default(),
            listened: Arc::new(watch::Sender::new(false)),
        }
    }

    #[test]
    fn each_event_reads_back_from_its_payload() {
        let device = DeviceId::from_signing_key(&[1; 32]);
        let space = SpaceId::from_bytes([2; 16]);
        for event in [
            Event::Entries(device),
            Event::Revision(space, 7),
            Event::SessionEnded(vec![0, 15, 16, 255]),
        ] {
            let payload = event.payload(&instance());
            assert_eq!(Event::parse(&payload), Some(("a1", event)), "{payload}");
        }
    }

    #[test]
    fn a_payload_of_another_form_is_none() {
        let device = DeviceId::from_signing_key(&[1; 32]);
        let space = SpaceId::from_bytes([2; 16]);
        for payload in [
            String::new(),
            "a1".to_owned(),
            "a1 entries".to_owned(),
            format!("a1 entries {device} more"),
            "a1 entries nodevice".to_owned(),
            format!("a1 revision {space}"),
            format!("a1 revision {space} seven"),
            format!("a1 revision {space} 7 more"),
            "a1 session a".to_owned(),
            "a1 session a=".to_owned(),
            format!("a1 later {device}"),
        ] {
            assert_eq!(Event::parse(&payload), None, "{payload}");
        }
    }

    #[test]
    fn an_instance_knows_the_revisions_up_to_the_latest_it_recorded() {
        let instance = instance();
        let (first, second) = (SpaceId::from_bytes([1; 16]), SpaceId::from_bytes([2; 16]));
        assert!(!instance.knows(first, 1));
        instance.record(first, 2);
        instance.record(first, 1);
        assert!(instance.knows(first, 1));
        assert!(instance.knows(first, 2));
        assert!(!instance.knows(first, 3));
        assert!(!instance.knows(second, 1));
        instance.clone().record(first, 3);
        assert!(instance.knows(first, 3), "a clone shares what it records");
    }

    #[test]
    fn instances_have_names_of_their_own() {
        let (first, second) = (Instance::new(), Instance::new());
        assert_eq!(first.id.len(), 32);
        assert_ne!(first.id, second.id);
    }
}
