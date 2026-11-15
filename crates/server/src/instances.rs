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
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::{PgConnection, PgPool};

use crate::DATABASE_TIMEOUT;
use crate::connections::Connections;
use crate::delivery::Wakers;
use crate::notices::Notices;

/// The channel the instances notify each other on.
const CHANNEL: &str = "skald_instances";
/// How long an instance waits before it tries to listen again, at first, and at most.
const RETRY_FIRST: Duration = Duration::from_millis(250);
const RETRY_LONGEST: Duration = Duration::from_secs(5);
/// This instance, as it names itself in its notifications, and the latest revision of each space
/// it knows of.
#[derive(Clone)]
pub(crate) struct Instance {
    id: Arc<str>,
    latest: Arc<Mutex<HashMap<SpaceId, i64>>>,
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
        }
    }

    /// Records that `space` has the revision `revision`; `true` if it is later than any this
    /// instance knew of, so that its notice is not sent yet.
    pub(crate) fn stored(&self, space: SpaceId, revision: i64) -> bool {
        let mut latest = self.latest.lock().expect("the latest revisions");
        let known = latest.entry(space).or_default();
        let later = revision > *known;
        *known = (*known).max(revision);
        later
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
/// `local`. When the listening connection drops, it listens again, then sends what was missed.
pub(crate) async fn listen(instance: Instance, local: Local) {
    let mut listened = false;
    let mut retry = RETRY_FIRST;
    let mut failing = false;
    loop {
        match open(&instance, &local, listened).await {
            Ok(mut listener) => {
                listened = true;
                retry = RETRY_FIRST;
                failing = false;
                // A lost connection or any other failure leaves the listener for a new one.
                while let Ok(Some(notification)) = listener.try_recv().await {
                    act(&instance, &local, notification.payload()).await;
                }
            }
            Err(err) => {
                // Once per outage, so that the server's output does not fill up.
                if !failing {
                    eprintln!("skald-server: cannot listen to the other instances: {err}");
                    failing = true;
                }
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(RETRY_LONGEST);
            }
        }
    }
}

/// Listens on a connection of its own, then records the latest revision of each space; if
/// `listened` before, first sends `local` what it missed.
async fn open(
    instance: &Instance,
    local: &Local,
    listened: bool,
) -> Result<PgListener, sqlx::Error> {
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
    if listened {
        catch_up(local).await?;
    }
    // What is stored from here on comes as a notification; a revision stored before, also as one
    // that is late, sends no notice twice.
    let latest: Vec<(String, i64)> = sqlx::query_as(
        "SELECT space_id::text, max(revision) FROM roster_revisions GROUP BY space_id",
    )
    .fetch_all(&local.db)
    .await?;
    for (space, revision) in latest {
        let space = space.parse().expect("a stored space ID");
        if instance.stored(space, revision) && listened {
            // A revision that no notification told of.
            let accounts = members(&local.db, space, revision).await?;
            local.notices.notify(accounts, space);
        }
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
        Event::Revision(space, revision) if instance.stored(space, revision) => {
            match members(&local.db, space, revision).await {
                Ok(accounts) => local.notices.notify(accounts, space),
                Err(err) => eprintln!("skald-server: cannot read a revision's members: {err}"),
            }
        }
        // The notice of that revision is sent.
        Event::Revision(..) => {}
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

/// Sends `local`'s connections the entries and closes the other instances may have told of while
/// this one did not listen: wakes each device's connections, and closes each connection whose
/// session has ended.
async fn catch_up(local: &Local) -> Result<(), sqlx::Error> {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance() -> Instance {
        Instance {
            id: "a1".into(),
            latest: Arc::default(),
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
    fn only_a_later_revision_is_new() {
        let instance = instance();
        let (first, second) = (SpaceId::from_bytes([1; 16]), SpaceId::from_bytes([2; 16]));
        assert!(instance.stored(first, 2));
        assert!(!instance.stored(first, 2));
        assert!(!instance.stored(first, 1));
        assert!(instance.stored(second, 1));
        assert!(instance.stored(first, 3));
        assert!(
            !instance.clone().stored(first, 3),
            "a clone shares what it knows"
        );
    }

    #[test]
    fn instances_have_names_of_their_own() {
        let (first, second) = (Instance::new(), Instance::new());
        assert_eq!(first.id.len(), 32);
        assert_ne!(first.id, second.id);
    }
}
