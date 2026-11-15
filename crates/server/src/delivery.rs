//! Delivering a device's mailbox: `docs/protocol/delivery.md`, "Mailbox" and "Receipts". A
//! connection reads its device's entries in `seq` order once its first `sync` states the cursor,
//! and each new entry wakes it. An ack deletes every entry up to its cursor, adds `delivered` to
//! the sending device's mailbox for each message sent with `report` true, and deletes an envelope
//! once no entry refers to it.

use std::collections::HashMap;
use std::future::pending;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use skald_proto::{DeliveryStatus, DeviceId, Id, Kind, Push, RawEnvelope, ServerStatus, Unknown};
use sqlx::postgres::PgArguments;
use sqlx::query::QueryAs;
use sqlx::{PgConnection, PgPool, Postgres};
use tokio::sync::watch;

use crate::expiry;
use crate::instances::{Event, Instance};

/// How many entries the server reads from a mailbox at a time.
const PAGE: i64 = 100;

/// Wakes a device's connections when an entry is added to its mailbox, on this instance, and tells
/// the other instances.
pub(crate) struct Wakers {
    by_device: Mutex<HashMap<DeviceId, watch::Sender<()>>>,
    instance: Instance,
}

impl Wakers {
    pub(crate) fn new(instance: Instance) -> Self {
        Wakers {
            by_device: Mutex::default(),
            instance,
        }
    }

    /// Tells the other instances on `conn`, once its transaction commits, that the mailboxes of
    /// `devices` have new entries.
    pub(crate) async fn tell(
        &self,
        conn: &mut PgConnection,
        devices: impl IntoIterator<Item = DeviceId>,
    ) -> Result<(), sqlx::Error> {
        let events: Vec<Event> = devices.into_iter().map(Event::Entries).collect();
        self.instance.tell(conn, &events).await
    }

    /// Wakes the connections of every device, which may have missed wakes.
    pub(crate) fn wake_all(&self) {
        for sender in self.by_device.lock().expect("the wakers").values() {
            sender.send_replace(());
        }
    }

    /// Wakes the connections of `device`, whose mailbox has a new entry.
    pub(crate) fn wake(&self, device: DeviceId) {
        if let Some(sender) = self.by_device.lock().expect("the wakers").get(&device) {
            sender.send_replace(());
        }
    }

    fn subscribe(self: &Arc<Self>, device: DeviceId) -> Subscription {
        let woken = self
            .by_device
            .lock()
            .expect("the wakers")
            .entry(device)
            .or_insert_with(|| watch::channel(()).0)
            .subscribe();
        Subscription {
            wakers: Arc::clone(self),
            device,
            woken: Some(woken),
        }
    }
}

/// The wakes of a device's mailbox for one connection; dropping it unsubscribes.
struct Subscription {
    wakers: Arc<Wakers>,
    device: DeviceId,
    /// `None` only while it drops.
    woken: Option<watch::Receiver<()>>,
}

impl Subscription {
    /// Waits for a wake since the last one.
    async fn woken(&mut self) {
        let woken = self.woken.as_mut().expect("a live subscription");
        // The sender lives while it has a receiver.
        if woken.changed().await.is_err() {
            pending::<()>().await;
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut by_device = self.wakers.by_device.lock().expect("the wakers");
        drop(self.woken.take());
        if by_device
            .get(&self.device)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            by_device.remove(&self.device);
        }
    }
}

/// A connection's reading of its device's mailbox.
pub(crate) struct Reader {
    device: DeviceId,
    /// The protocol version of the connection.
    version: NonZeroU64,
    /// The server's mailbox retention, as [`expiry::seconds`] gives it.
    retention: Option<f64>,
    subscription: Subscription,
    /// Whether the connection's first `sync` came.
    synced: bool,
    /// The highest `seq` sent on the connection. Catch-up starts after the first `sync`'s cursor,
    /// as its ack deletes the entries up to it.
    sent: i64,
}

impl Reader {
    /// The reading of `device`'s mailbox on a connection of the protocol version `version`, woken
    /// by `wakers`, on a server with the mailbox retention `retention`.
    pub(crate) fn new(
        wakers: &Arc<Wakers>,
        device: DeviceId,
        version: NonZeroU64,
        retention: Option<Duration>,
    ) -> Self {
        Reader {
            device,
            version,
            retention: expiry::seconds(retention),
            subscription: wakers.subscribe(device),
            synced: false,
            sent: 0,
        }
    }

    pub(crate) fn device(&self) -> DeviceId {
        self.device
    }

    /// Starts catch-up at the connection's first `sync`, once the `sync` acked: gives the highest
    /// `seq` the mailbox has held so far; `None` at a later `sync`.
    pub(crate) async fn start(&mut self, db: &PgPool) -> Result<Option<i64>, sqlx::Error> {
        if self.synced {
            return Ok(None);
        }
        let held: Option<(i64,)> =
            sqlx::query_as("SELECT last_seq FROM mailboxes WHERE device_id = $1::uuid")
                .bind(self.device.to_string())
                .fetch_optional(db)
                .await?;
        self.synced = true;
        Ok(Some(held.map_or(0, |(seq,)| seq)))
    }

    /// Waits until an entry is added to the mailbox, once the first `sync` came.
    pub(crate) async fn woken(&mut self) {
        if !self.synced {
            return pending().await;
        }
        self.subscription.woken().await;
    }

    /// The next page of entries after those sent, up to `seq` `upto`, in `seq` order; they count
    /// as sent. `None` once none is left. Expired entries are left out: those in the page's range
    /// that expired by time, and each `message` of a version not deliverable on the connection,
    /// expire as the page reaches them, so no ack of the connection covers one as `delivered`.
    pub(crate) async fn next(
        &mut self,
        db: &PgPool,
        upto: i64,
    ) -> Result<Option<Vec<Push>>, sqlx::Error> {
        let wakers = &self.subscription.wakers;
        expiry::expire_due(db, wakers, self.device, (self.sent, upto), self.retention).await?;
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT e.seq, v.envelope, e.status, e.msg_id::text, e.recipient_id::text,
                 floor(extract(epoch FROM e.happened_at))::bigint
             FROM entries e LEFT JOIN envelopes v ON v.id = e.envelope_id
             WHERE e.device_id = $1::uuid AND e.seq > $2 AND e.seq <= $3
             ORDER BY e.seq LIMIT $4",
        )
        .bind(self.device.to_string())
        .bind(self.sent)
        .bind(upto)
        .bind(PAGE)
        .fetch_all(db)
        .await?;
        let Some((last, ..)) = rows.last() else {
            return Ok(None);
        };
        self.sent = *last;
        let mut pushes = Vec::new();
        for push in rows.into_iter().map(push) {
            match push {
                Push::Message { seq, envelope }
                    if !expiry::deliverable(envelope.envelope().header.v, self.version) =>
                {
                    expiry::expire(db, wakers, self.device, seq.get()).await?;
                }
                push => pushes.push(push),
            }
        }
        Ok(Some(pushes))
    }
}

/// An entry as [`Reader::next`] reads it: its `seq`, and its envelope or its status, recipient
/// device, msg_id and the time it happened.
type Row = (
    i64,
    Option<Vec<u8>>,
    Option<i16>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// The frame of the entry `row`.
fn push((seq, envelope, status, msg_id, device, happened): Row) -> Push {
    let seq = u64::try_from(seq)
        .ok()
        .and_then(|seq| seq.try_into().ok())
        .expect("a stored seq from 1");
    if let Some(envelope) = envelope {
        return Push::Message {
            seq,
            envelope: RawEnvelope::from_bytes(envelope).expect("a stored envelope decodes"),
        };
    }
    Push::Status {
        seq,
        status: DeliveryStatus {
            msg_id: parsed(msg_id),
            device: parsed(device),
            status: server_status(status.expect("a stored status")),
            timestamp: happened
                .and_then(|at| at.try_into().ok())
                .expect("a stored time after 1970"),
            unknown: Unknown::default(),
        },
    }
}

/// The ID a `status` entry stores as `id`.
fn parsed<K: Kind>(id: Option<String>) -> Id<K> {
    id.and_then(|id| id.parse().ok()).expect("a stored ID")
}

/// The status a `status` entry stores as `code`.
fn server_status(code: i16) -> ServerStatus {
    match code {
        1 => ServerStatus::Delivered,
        2 => ServerStatus::Expired,
        other => panic!("a stored status: {other}"),
    }
}

/// Acks `device`'s mailbox up to `cursor`: deletes every entry up to it, adds `delivered` to the
/// sending device's mailbox for each message sent with `report` true, and deletes the envelopes
/// that no entry refers to any more. Wakes the sending devices' connections in `wakers`.
pub(crate) async fn ack(
    db: &PgPool,
    wakers: &Wakers,
    device: DeviceId,
    cursor: u64,
) -> Result<(), sqlx::Error> {
    let query = sqlx::query_as(removing!(
        "DELETE FROM entries WHERE device_id = $1::uuid AND seq <= $2
         RETURNING device_id, seq, envelope_id, now() AS at"
    ))
    .bind(device.to_string())
    .bind(i64::try_from(cursor).unwrap_or(i64::MAX));
    remove(db, wakers, query, ServerStatus::Delivered).await?;
    Ok(())
}

/// A mailbox entry an ack or an expiry deleted: its device; its envelope, for a `message`; the
/// sending device and the msg_id of a message to report on; and when it was acked or expired, in
/// seconds since the Unix epoch.
pub(crate) type Removed = (String, Option<i64>, Option<String>, Option<String>, f64);

/// The query that runs the statement `$gone`, which deletes mailbox entries and returns the
/// `device_id`, `seq`, `envelope_id` and `at` of each, and gives each as [`Removed`]: in the order
/// of the senders' mailboxes, so that removals at once lock them in one order.
macro_rules! removing {
    ($gone:literal) => {
        concat!(
            "WITH gone AS (",
            $gone,
            ")
             SELECT gone.device_id::text, gone.envelope_id,
                 CASE WHEN v.report THEN v.sender_id::text END, v.msg_id::text,
                 extract(epoch FROM gone.at)::float8
             FROM gone LEFT JOIN envelopes v ON v.id = gone.envelope_id
             ORDER BY v.sender_id, gone.device_id, gone.seq"
        )
    };
}
pub(crate) use removing;

/// Runs `query`, a [`removing!`] one, in a transaction that also adds `status` to the sending
/// device's mailbox for each message to report on, and deletes the envelopes that no entry refers
/// to any more; wakes the sending devices' connections in `wakers`, on each instance. Gives the
/// number of entries it removed.
pub(crate) async fn remove<'q>(
    db: &PgPool,
    wakers: &Wakers,
    query: QueryAs<'q, Postgres, Removed, PgArguments>,
    status: ServerStatus,
) -> Result<usize, sqlx::Error> {
    let mut tx = db.begin().await?;
    let removed = remove_on(&mut tx, wakers, query, status).await?;
    tx.commit().await?;
    Ok(removed.woken(wakers))
}

/// What [`remove_on`] removed: the number of entries, and the sending devices whose mailboxes got
/// a status.
pub(crate) struct Statuses {
    removed: usize,
    senders: Vec<DeviceId>,
}

impl Statuses {
    /// Wakes the sending devices' connections in `wakers`, once the transaction has committed;
    /// gives the number of entries removed.
    pub(crate) fn woken(self, wakers: &Wakers) -> usize {
        for sender in self.senders {
            wakers.wake(sender);
        }
        self.removed
    }
}

/// [`remove`] on `conn`, in its transaction: tells the other instances once it commits, after which
/// the caller wakes this instance's connections with [`Statuses::woken`]. A revoked sending device
/// gets no status.
pub(crate) async fn remove_on<'q>(
    conn: &mut PgConnection,
    wakers: &Wakers,
    query: QueryAs<'q, Postgres, Removed, PgArguments>,
    status: ServerStatus,
) -> Result<Statuses, sqlx::Error> {
    let gone = query.fetch_all(&mut *conn).await?;
    let mut senders = Vec::new();
    for (device, _, sender, msg_id, at) in &gone {
        // A message sent with `report` false, or by a device that is gone, gets no status.
        let (Some(sender), Some(msg_id)) = (sender, msg_id) else {
            continue;
        };
        // The sending device's row lock makes its revocation wait for this status, which the
        // revocation then expires, or this insert see the revocation.
        let added: Option<(String,)> = sqlx::query_as(
            "WITH sender AS (
                 SELECT id FROM devices WHERE id = $1::uuid AND NOT revoked FOR SHARE
             ),
             mailbox AS (
                 INSERT INTO mailboxes (device_id, last_seq) SELECT id, 1 FROM sender
                 ON CONFLICT (device_id) DO UPDATE SET last_seq = mailboxes.last_seq + 1
                 RETURNING last_seq
             )
             INSERT INTO entries (device_id, seq, status, msg_id, recipient_id, happened_at)
             SELECT $1::uuid, mailbox.last_seq, $2, $3::uuid, $4::uuid, to_timestamp($5)
             FROM mailbox RETURNING device_id::text",
        )
        .bind(sender)
        .bind(status as i16)
        .bind(msg_id)
        .bind(device)
        .bind(at)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((sender,)) = added {
            senders.push(sender.parse().expect("a stored device ID"));
        }
    }
    let envelopes: Vec<i64> = gone
        .iter()
        .filter_map(|(_, envelope, ..)| *envelope)
        .collect();
    // An envelope's row lock makes removals of its entries at once check for the others one after
    // another, so the last one sees none left, as its check runs once the others committed.
    sqlx::query("SELECT 1 FROM envelopes WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(&envelopes)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM envelopes v WHERE v.id = ANY($1)
         AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.envelope_id = v.id)",
    )
    .bind(envelopes)
    .execute(&mut *conn)
    .await?;
    wakers.tell(conn, senders.iter().copied()).await?;
    Ok(Statuses {
        removed: gone.len(),
        senders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_signing_key(&[byte; 32])
    }

    /// The devices with subscriptions.
    fn subscribed(wakers: &Wakers) -> Vec<DeviceId> {
        wakers.by_device.lock().unwrap().keys().copied().collect()
    }

    #[test]
    fn a_device_is_unsubscribed_once_its_last_subscription_drops() {
        let wakers = Arc::new(Wakers::new(Instance::new()));
        let first = wakers.subscribe(device(1));
        let second = wakers.subscribe(device(1));
        assert_eq!(subscribed(&wakers), [device(1)]);

        drop(first);
        assert_eq!(subscribed(&wakers), [device(1)]);
        drop(second);
        assert_eq!(subscribed(&wakers), []);
    }

    #[test]
    fn a_stored_status_reads_as_its_server_status() {
        assert_eq!(
            server_status(ServerStatus::Delivered as i16),
            ServerStatus::Delivered
        );
        assert_eq!(
            server_status(ServerStatus::Expired as i16),
            ServerStatus::Expired
        );
    }
}
