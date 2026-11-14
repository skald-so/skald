//! Storing sent envelopes: `docs/protocol/delivery.md`, "Send" and "Mailbox". The server answers a
//! device's `send` with `stored` once the envelope and its mailbox entry are committed, or with
//! `send-refused` and its problem, storing nothing of the envelope. It stores at most one envelope
//! per idempotency key and answers a retry as it answered the first send.

use std::num::NonZeroU64;

use skald_proto::{
    AccountId, Cbor as _, DeviceId, Frame, Header, RawEnvelope, Roster, Scheme, SendKey, SpaceId,
    Stored,
};
use sqlx::{PgConnection, PgPool};

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
    fn name(self) -> &'static str {
        match self {
            Self::VersionMismatch => "version-mismatch",
            Self::NotAMember => "not-a-member",
            Self::UnknownRecipient => "unknown-recipient",
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

/// The answer to `sender`'s send of `envelope`.
pub(crate) async fn send(
    db: &PgPool,
    sender: &Sender,
    envelope: &RawEnvelope,
) -> Result<Frame, sqlx::Error> {
    answer(
        db,
        sender,
        &envelope.envelope().header,
        Some(envelope.bytes()),
    )
    .await
}

/// The answer to `sender`'s send of an `hpke` envelope with `header`, whose `recipients` is not one
/// device: [`Frame::misaddressed_send`].
pub(crate) async fn misaddressed(
    db: &PgPool,
    sender: &Sender,
    header: &Header,
) -> Result<Frame, sqlx::Error> {
    answer(db, sender, header, None).await
}

/// The answer to `sender`'s send of an envelope with `header`, whose bytes are `envelope`, or
/// `None` if it cannot be stored. A retry gets the first answer, whatever its bytes.
async fn answer(
    db: &PgPool,
    sender: &Sender,
    header: &Header,
    envelope: Option<&[u8]>,
) -> Result<Frame, sqlx::Error> {
    let key = send_key(header);
    let outcome = match (recipient(db, sender, header).await?, envelope) {
        (Ok(recipient), Some(envelope)) => Ok((recipient, envelope)),
        // `recipient` refuses a send without one recipient; this is in case it does not.
        (Ok(_), None) => Err(Problem::UnknownRecipient),
        (Err(problem), _) => Err(problem),
    };
    // A send answered before keeps its answer: keeping this one finds its key taken.
    loop {
        let kept = match outcome {
            Ok((recipient, envelope)) => store(db, sender, &key, recipient, envelope).await?,
            Err(problem) => refuse(db, sender, &key, problem).await?,
        };
        if let Some(answer) = kept {
            return Ok(answer);
        }
        if let Some(answer) = answered(db, sender, &key).await? {
            return Ok(answer);
        }
    }
}

/// The answer to the send with `key`: its refusal for `problem`, or else its `stored` at
/// `timestamp`, in seconds since the Unix epoch.
fn frame(key: &SendKey, timestamp: i64, problem: Option<String>) -> Frame {
    match problem {
        Some(problem) => Frame::SendRefused {
            send_key: key.clone(),
            problem,
        },
        None => Frame::Stored(Stored {
            send_key: key.clone(),
            timestamp: timestamp.try_into().expect("a time after 1970"),
        }),
    }
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

/// The answer the send of `sender` with `key` got, if it was answered.
async fn answered(
    db: &PgPool,
    sender: &Sender,
    key: &SendKey,
) -> Result<Option<Frame>, sqlx::Error> {
    let found: Option<(i64, Option<String>)> = sqlx::query_as(
        "SELECT floor(extract(epoch FROM answered_at))::bigint, problem FROM sends
         WHERE sender_id = $1::uuid AND msg_id = $2::uuid
           AND recipient_id IS NOT DISTINCT FROM $3::uuid",
    )
    .bind(sender.device.to_string())
    .bind(key.msg_id.to_string())
    .bind(key.recipient.map(|device| device.to_string()))
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(timestamp, problem)| frame(key, timestamp, problem)))
}

/// Keeps the answer to the send of `sender` with `key` on `conn`: its refusal for `problem`, or
/// else its `stored`. Gives the answer; `None` if a send with `key` is answered already.
async fn keep(
    conn: &mut PgConnection,
    sender: &Sender,
    key: &SendKey,
    problem: Option<Problem>,
) -> Result<Option<Frame>, sqlx::Error> {
    let problem = problem.map(Problem::name);
    // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
    let inserted: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO sends (sender_id, msg_id, recipient_id, answered_at, problem)
         VALUES ($1::uuid, $2::uuid, $3::uuid, now(), $4)
         ON CONFLICT DO NOTHING RETURNING floor(extract(epoch FROM answered_at))::bigint",
    )
    .bind(sender.device.to_string())
    .bind(key.msg_id.to_string())
    .bind(key.recipient.map(|device| device.to_string()))
    .bind(problem)
    .fetch_optional(conn)
    .await?;
    Ok(inserted.map(|(timestamp,)| frame(key, timestamp, problem.map(str::to_owned))))
}

/// Keeps the refusal of the send of `sender` with `key` for `problem`, and gives it; `None` if a
/// send with `key` is answered already.
async fn refuse(
    db: &PgPool,
    sender: &Sender,
    key: &SendKey,
    problem: Problem,
) -> Result<Option<Frame>, sqlx::Error> {
    keep(&mut *db.acquire().await?, sender, key, Some(problem)).await
}

/// Stores `envelope`, the send of `sender` with `key`, with its entry in `recipient`'s mailbox, and
/// gives its `stored`; `None` if a send with `key` is answered already.
async fn store(
    db: &PgPool,
    sender: &Sender,
    key: &SendKey,
    recipient: DeviceId,
    envelope: &[u8],
) -> Result<Option<Frame>, sqlx::Error> {
    let mut tx = db.begin().await?;
    let Some(stored) = keep(&mut tx, sender, key, None).await? else {
        return Ok(None);
    };
    // The mailbox's row lock makes stores to it wait for each other, so its entries are numbered in
    // the order they are committed.
    sqlx::query(
        "WITH envelope AS (INSERT INTO envelopes (envelope) VALUES ($1) RETURNING id),
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
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(stored))
}
