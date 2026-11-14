//! Delivering a device's mailbox: `docs/protocol/delivery.md`, "Mailbox" and "Receipts". A
//! connection reads its device's entries in `seq` order once its first `sync` states the cursor,
//! and each new entry wakes it. An ack deletes every entry up to its cursor, adds `delivered` to
//! the sending device's mailbox for each message sent with `report` true, and deletes an envelope
//! once no entry refers to it.

use std::collections::HashMap;
use std::future::pending;
use std::sync::{Arc, Mutex};

use skald_proto::{DeliveryStatus, DeviceId, Id, Kind, Push, RawEnvelope, ServerStatus, Unknown};
use sqlx::PgPool;
use tokio::sync::watch;

/// How many entries the server reads from a mailbox at a time.
const PAGE: i64 = 100;

/// Wakes a device's connections when an entry is added to its mailbox, on this instance.
#[derive(Default)]
pub(crate) struct Wakers {
    by_device: Mutex<HashMap<DeviceId, watch::Sender<()>>>,
}

impl Wakers {
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
    subscription: Subscription,
    /// Whether the connection's first `sync` came.
    synced: bool,
    /// The highest `seq` sent on the connection, or the cursor of its first `sync` if higher.
    sent: i64,
}

impl Reader {
    /// The reading of `device`'s mailbox, woken by `wakers`.
    pub(crate) fn new(wakers: &Arc<Wakers>, device: DeviceId) -> Self {
        Reader {
            device,
            subscription: wakers.subscribe(device),
            synced: false,
            sent: 0,
        }
    }

    pub(crate) fn device(&self) -> DeviceId {
        self.device
    }

    /// Starts catch-up after `cursor` at the connection's first `sync`: gives the highest `seq` the
    /// mailbox has held so far; `None` at a later `sync`.
    pub(crate) async fn start(
        &mut self,
        db: &PgPool,
        cursor: u64,
    ) -> Result<Option<i64>, sqlx::Error> {
        if self.synced {
            return Ok(None);
        }
        let held: Option<(i64,)> =
            sqlx::query_as("SELECT last_seq FROM mailboxes WHERE device_id = $1::uuid")
                .bind(self.device.to_string())
                .fetch_optional(db)
                .await?;
        self.synced = true;
        self.sent = i64::try_from(cursor).unwrap_or(i64::MAX);
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
    /// as sent.
    pub(crate) async fn next(&mut self, db: &PgPool, upto: i64) -> Result<Vec<Push>, sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT e.seq, v.envelope, e.status, e.msg_id::text, e.recipient_id::text,
                 floor(extract(epoch FROM e.added_at))::bigint
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
        if let Some((seq, ..)) = rows.last() {
            self.sent = *seq;
        }
        Ok(rows.into_iter().map(push).collect())
    }
}

/// An entry as [`Reader::next`] reads it: its `seq`, and its envelope or its status, recipient
/// device, msg_id and adding time.
type Row = (
    i64,
    Option<Vec<u8>>,
    Option<i16>,
    Option<String>,
    Option<String>,
    i64,
);

/// The frame of the entry `row`.
fn push((seq, envelope, status, msg_id, device, added): Row) -> Push {
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
            timestamp: added.try_into().expect("a time after 1970"),
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
    let mut tx = db.begin().await?;
    // Each deleted entry's envelope, and the sending device and msg_id of a message to report on;
    // in the order of the senders' mailboxes, so that acks at once lock them in one order.
    let gone: Vec<(Option<i64>, Option<String>, Option<String>)> = sqlx::query_as(
        "WITH gone AS (
             DELETE FROM entries WHERE device_id = $1::uuid AND seq <= $2 RETURNING seq, envelope_id
         )
         SELECT gone.envelope_id, CASE WHEN v.report THEN v.sender_id::text END, v.msg_id::text
         FROM gone LEFT JOIN envelopes v ON v.id = gone.envelope_id
         ORDER BY v.sender_id, gone.seq",
    )
    .bind(device.to_string())
    .bind(i64::try_from(cursor).unwrap_or(i64::MAX))
    .fetch_all(&mut *tx)
    .await?;
    let mut senders = Vec::new();
    for (_, sender, msg_id) in &gone {
        // A message sent with `report` false, or by a device that is gone, gets no status.
        let (Some(sender), Some(msg_id)) = (sender, msg_id) else {
            continue;
        };
        sqlx::query(
            "WITH mailbox AS (
                 INSERT INTO mailboxes (device_id, last_seq) VALUES ($1::uuid, 1)
                 ON CONFLICT (device_id) DO UPDATE SET last_seq = mailboxes.last_seq + 1
                 RETURNING last_seq
             )
             INSERT INTO entries (device_id, seq, status, msg_id, recipient_id)
             SELECT $1::uuid, mailbox.last_seq, $2, $3::uuid, $4::uuid FROM mailbox",
        )
        .bind(sender)
        .bind(ServerStatus::Delivered as i16)
        .bind(msg_id)
        .bind(device.to_string())
        .execute(&mut *tx)
        .await?;
        senders.push(sender.parse().expect("a stored device ID"));
    }
    let envelopes: Vec<i64> = gone.iter().filter_map(|(envelope, ..)| *envelope).collect();
    sqlx::query(
        "DELETE FROM envelopes v WHERE v.id = ANY($1)
         AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.envelope_id = v.id)",
    )
    .bind(envelopes)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    for sender in senders {
        wakers.wake(sender);
    }
    Ok(())
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
        let wakers = Arc::new(Wakers::default());
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
