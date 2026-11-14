//! Storing sent envelopes: `docs/protocol/delivery.md`, "Send" and "Mailbox". The server answers a
//! device's `send` with `stored` once the envelope and its mailbox entry are committed, or with
//! `send-refused` and its problem, storing nothing. It stores at most one envelope per idempotency
//! key and answers a retry as it answered the first send.

use std::num::NonZeroU64;

use skald_proto::{
    AccountId, Cbor as _, DeviceId, Frame, Header, RawEnvelope, Roster, Scheme, SendKey, SpaceId,
    Stored,
};
use sqlx::PgPool;

use crate::delivery::Wakers;

/// The device a connection is authenticated as.
pub(crate) struct Sender {
    pub(crate) device: DeviceId,
    pub(crate) account: AccountId,
    /// The protocol version of its connection.
    pub(crate) version: NonZeroU64,
}

/// Why a send is refused: the problems of "Send" that the server finds so far.
#[derive(Debug, Clone, Copy)]
enum Problem {
    VersionMismatch,
    NotAMember,
    UnknownRecipient,
}

impl Problem {
    fn refusal(self, send_key: SendKey) -> Frame {
        let problem = match self {
            Self::VersionMismatch => "version-mismatch",
            Self::NotAMember => "not-a-member",
            Self::UnknownRecipient => "unknown-recipient",
        };
        Frame::SendRefused {
            send_key,
            problem: problem.to_owned(),
        }
    }
}

/// The send-key of an envelope with `header`: its `msg_id` and, under `hpke`, its one recipient.
fn send_key(header: &Header) -> SendKey {
    let recipient = match (header.scheme, &header.recipients) {
        (Scheme::Hpke, Some(devices)) if devices.len() == 1 => Some(devices[0]),
        _ => None,
    };
    SendKey {
        msg_id: header.msg_id,
        recipient,
    }
}

/// The answer to `sender`'s send of `envelope` with `report`; a stored envelope wakes its
/// recipient's connections in `wakers`.
pub(crate) async fn send(
    db: &PgPool,
    wakers: &Wakers,
    sender: &Sender,
    envelope: &RawEnvelope,
    report: bool,
) -> Result<Frame, sqlx::Error> {
    let header = &envelope.envelope().header;
    let key = send_key(header);
    let answer = |timestamp| {
        Frame::Stored(Stored {
            send_key: key.clone(),
            timestamp,
        })
    };
    // A retry gets the first answer, whatever its bytes.
    if let Some(timestamp) = stored_at(db, sender, &key).await? {
        return Ok(answer(timestamp));
    }
    let recipient = match recipient(db, sender, header).await? {
        Ok(recipient) => recipient,
        Err(problem) => return Ok(problem.refusal(key)),
    };
    loop {
        let stored = store(db, sender, &key, recipient, envelope.bytes(), report).await?;
        if let Some(timestamp) = stored {
            wakers.wake(recipient);
            return Ok(answer(timestamp));
        }
        // A send of the same key stored it meanwhile.
        if let Some(timestamp) = stored_at(db, sender, &key).await? {
            return Ok(answer(timestamp));
        }
    }
}

/// The answer to `sender`'s send of an `hpke` envelope with `header`, whose `recipients` is not one
/// device: [`Frame::misaddressed_send`].
pub(crate) async fn misaddressed(
    db: &PgPool,
    sender: &Sender,
    header: &Header,
) -> Result<Frame, sqlx::Error> {
    let problem = recipient(db, sender, header)
        .await?
        .err()
        .unwrap_or(Problem::UnknownRecipient);
    Ok(problem.refusal(send_key(header)))
}

/// The one recipient device that `sender`'s envelope with `header` is stored for, or the problem
/// it is refused with.
async fn recipient(
    db: &PgPool,
    sender: &Sender,
    header: &Header,
) -> Result<Result<DeviceId, Problem>, sqlx::Error> {
    if header.v != sender.version {
        return Ok(Err(Problem::VersionMismatch));
    }
    let Some(roster) = latest(db, header.space).await? else {
        return Ok(Err(Problem::NotAMember));
    };
    let body = roster.body();
    let Some(channel) = body.channels.iter().find(|c| c.id == header.channel) else {
        return Ok(Err(Problem::NotAMember));
    };
    // Whether `device` of `account` is in the channel: its account a member of it, not excluded.
    let in_channel = |account: AccountId, device: DeviceId| {
        channel.members.iter().any(|m| m.account == account)
            && body.members.iter().any(|m| {
                m.account == account && !m.excluded.as_ref().is_some_and(|e| e.contains(&device))
            })
    };
    // No channel has an MLS group yet, so no device may send under `mls`.
    if !in_channel(sender.account, sender.device) || header.scheme == Scheme::Mls {
        return Ok(Err(Problem::NotAMember));
    }
    let Some(recipient) = send_key(header).recipient else {
        return Ok(Err(Problem::UnknownRecipient));
    };
    match certified_account(db, recipient).await? {
        Some(account) if in_channel(account, recipient) => Ok(Ok(recipient)),
        _ => Ok(Err(Problem::UnknownRecipient)),
    }
}

/// The latest roster revision of `space`, if it has one.
async fn latest(db: &PgPool, space: SpaceId) -> Result<Option<Roster>, sqlx::Error> {
    let found: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT roster FROM roster_revisions WHERE space_id = $1::uuid
         ORDER BY revision DESC LIMIT 1",
    )
    .bind(space.to_string())
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(roster,)| Roster::from_cbor(&roster).expect("a stored revision decodes")))
}

/// The account of `device` if it has a certificate on the server.
async fn certified_account(
    db: &PgPool,
    device: DeviceId,
) -> Result<Option<AccountId>, sqlx::Error> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT account_id::text FROM devices WHERE id = $1::uuid AND cert IS NOT NULL",
    )
    .bind(device.to_string())
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(account,)| account.parse().expect("a stored account ID")))
}

/// When the send of `sender` with `key` was stored, in seconds since the Unix epoch, if it was.
async fn stored_at(
    db: &PgPool,
    sender: &Sender,
    key: &SendKey,
) -> Result<Option<u64>, sqlx::Error> {
    let found: Option<(i64,)> = sqlx::query_as(
        "SELECT floor(extract(epoch FROM stored_at))::bigint FROM sends
         WHERE sender_id = $1::uuid AND msg_id = $2::uuid
           AND recipient_id IS NOT DISTINCT FROM $3::uuid",
    )
    .bind(sender.device.to_string())
    .bind(key.msg_id.to_string())
    .bind(key.recipient.map(|device| device.to_string()))
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(timestamp,)| timestamp.try_into().expect("a time after 1970")))
}

/// Stores `envelope`, the send of `sender` with `key` and `report`, with its entry in `recipient`'s
/// mailbox, and gives when, in seconds since the Unix epoch; `None` if a send with `key` is stored
/// already.
async fn store(
    db: &PgPool,
    sender: &Sender,
    key: &SendKey,
    recipient: DeviceId,
    envelope: &[u8],
    report: bool,
) -> Result<Option<u64>, sqlx::Error> {
    let mut tx = db.begin().await?;
    // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
    let inserted: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO sends (sender_id, msg_id, recipient_id, stored_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, now())
         ON CONFLICT DO NOTHING RETURNING floor(extract(epoch FROM stored_at))::bigint",
    )
    .bind(sender.device.to_string())
    .bind(key.msg_id.to_string())
    .bind(key.recipient.map(|device| device.to_string()))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((timestamp,)) = inserted else {
        return Ok(None);
    };
    // The mailbox's row lock makes stores to it wait for each other, so its entries are numbered in
    // the order they are committed.
    sqlx::query(
        "WITH envelope AS (
             INSERT INTO envelopes (envelope, sender_id, msg_id, report)
             VALUES ($1, $3::uuid, $4::uuid, $5) RETURNING id
         ),
         mailbox AS (
             INSERT INTO mailboxes (device_id, last_seq) VALUES ($2::uuid, 1)
             ON CONFLICT (device_id) DO UPDATE SET last_seq = mailboxes.last_seq + 1
             RETURNING last_seq
         )
         INSERT INTO entries (device_id, seq, envelope_id)
         SELECT $2::uuid, mailbox.last_seq, envelope.id FROM envelope, mailbox",
    )
    .bind(envelope)
    .bind(recipient.to_string())
    .bind(sender.device.to_string())
    .bind(key.msg_id.to_string())
    .bind(report)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(timestamp.try_into().expect("a time after 1970")))
}
