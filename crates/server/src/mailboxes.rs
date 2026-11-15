//! Storing sent envelopes: `docs/protocol/delivery.md`, "Send" and "Mailbox". The server answers a
//! device's `send` with `stored` once the envelope and its mailbox entry are committed, or with
//! `send-refused` and its problem, storing nothing of the envelope. It stores at most one envelope
//! per idempotency key and answers a retry as it answered the first send. With the server's cloud
//! limits (`docs/dev/architecture.md`, "Cloud limits"), it refuses a payload over the size limit,
//! and a message past its space's number for the month.

use std::num::NonZeroU64;

use skald_proto::{
    AccountId, Cbor as _, DeviceId, Frame, Header, RawEnvelope, Roster, Scheme, SendKey, SpaceId,
    Stored,
};
use sqlx::{PgConnection, PgPool};

use crate::MAILBOX_LONGEST;
use crate::delivery::Wakers;

/// The device a connection is authenticated as.
pub(crate) struct Sender {
    pub(crate) device: DeviceId,
    pub(crate) account: AccountId,
    /// The protocol version of its connection.
    pub(crate) version: NonZeroU64,
}

/// The server's limits on sends; none without a setting.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Limits {
    /// The longest payload, in bytes.
    pub(crate) message_size: Option<u64>,
    /// The messages a space stores per calendar month in UTC.
    pub(crate) monthly_messages: Option<u64>,
}

/// Why a send is refused: the problems of "Send" that the server finds so far.
#[derive(Debug, Clone, Copy)]
enum Problem {
    VersionMismatch,
    NotAMember,
    UnknownRecipient,
    TooLarge,
    QuotaExceeded,
}

impl Problem {
    fn name(self) -> &'static str {
        match self {
            Self::VersionMismatch => "version-mismatch",
            Self::NotAMember => "not-a-member",
            Self::UnknownRecipient => "unknown-recipient",
            Self::TooLarge => "too-large",
            Self::QuotaExceeded => "quota-exceeded",
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

/// The answer to `sender`'s send of `envelope` with `report`, under `limits`; a stored envelope
/// wakes its recipient's connections in `wakers`, on each instance.
pub(crate) async fn send(
    db: &PgPool,
    wakers: &Wakers,
    limits: Limits,
    sender: &Sender,
    envelope: &RawEnvelope,
    report: bool,
) -> Result<Frame, sqlx::Error> {
    let header = &envelope.envelope().header;
    let outgoing = Outgoing {
        bytes: envelope.bytes(),
        size: envelope.envelope().payload.len(),
        report,
        ttl: header.ttl,
    };
    let answer = answer(db, wakers, limits, sender, header, Some(outgoing)).await?;
    // A retry's wake finds no new entry.
    if let Frame::Stored(Stored {
        send_key: SendKey {
            recipient: Some(recipient),
            ..
        },
        ..
    }) = &answer
    {
        wakers.wake(*recipient);
    }
    Ok(answer)
}

/// The answer to `sender`'s send of an `hpke` envelope with `header`, whose `recipients` is not one
/// device: [`Frame::misaddressed_send`].
pub(crate) async fn misaddressed(
    db: &PgPool,
    wakers: &Wakers,
    sender: &Sender,
    header: &Header,
) -> Result<Frame, sqlx::Error> {
    answer(db, wakers, Limits::default(), sender, header, None).await
}

/// The answer to `sender`'s send of an envelope with `header`, which is `envelope`, or `None` if
/// it cannot be stored, under `limits`; a stored envelope tells the other instances in `wakers`. A
/// retry gets the first answer, whatever its bytes.
async fn answer(
    db: &PgPool,
    wakers: &Wakers,
    limits: Limits,
    sender: &Sender,
    header: &Header,
    envelope: Option<Outgoing<'_>>,
) -> Result<Frame, sqlx::Error> {
    let key = send_key(header);
    let outcome = match (recipient(db, sender, header).await?, envelope) {
        (Ok(_), Some(envelope))
            if limits
                .message_size
                .is_some_and(|limit| envelope.size as u64 > limit) =>
        {
            Err(Problem::TooLarge)
        }
        (Ok(recipient), Some(envelope)) => Ok((recipient, envelope)),
        // `recipient` refuses a send without one recipient; this is in case it does not.
        (Ok(_), None) => Err(Problem::UnknownRecipient),
        (Err(problem), _) => Err(problem),
    };
    // A send answered before keeps its answer: keeping this one finds its key taken.
    loop {
        let kept = match outcome {
            Ok((recipient, envelope)) => {
                let counted = limits.monthly_messages.map(|limit| (header.space, limit));
                match store(db, wakers, sender, &key, recipient, envelope, counted).await? {
                    Kept::Answer(answer) => answer,
                    Kept::OverQuota => refuse(db, sender, &key, Problem::QuotaExceeded).await?,
                }
            }
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

/// An envelope to store: its bytes, its payload's length, whether it gets statuses, and its `ttl`.
#[derive(Clone, Copy)]
struct Outgoing<'a> {
    bytes: &'a [u8],
    size: usize,
    report: bool,
    ttl: Option<u64>,
}

/// What storing a send gave.
enum Kept {
    /// Its `stored`; `None` if a send with its key is answered already.
    Answer(Option<Frame>),
    /// Nothing: its message is past its space's number for the month.
    OverQuota,
}

/// Stores `envelope`, the send of `sender` with `key`, with its entry in `recipient`'s mailbox, and
/// gives its `stored`; tells the other instances in `wakers`. With `counted`, a space and its
/// monthly limit, a message not counted yet counts in the space's month, and is not stored past the
/// limit.
async fn store(
    db: &PgPool,
    wakers: &Wakers,
    sender: &Sender,
    key: &SendKey,
    recipient: DeviceId,
    envelope: Outgoing<'_>,
    counted: Option<(SpaceId, u64)>,
) -> Result<Kept, sqlx::Error> {
    let mut tx = db.begin().await?;
    let Some(stored) = keep(&mut tx, sender, key, None).await? else {
        return Ok(Kept::Answer(None));
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
         INSERT INTO entries (device_id, seq, envelope_id, ttl_ends_at)
         SELECT $2::uuid, mailbox.last_seq, envelope.id, now() + make_interval(secs => $6)
         FROM envelope, mailbox",
    )
    .bind(envelope.bytes)
    .bind(recipient.to_string())
    .bind(sender.device.to_string())
    .bind(key.msg_id.to_string())
    .bind(envelope.report)
    // Past `MAILBOX_LONGEST`, a `ttl` changes nothing.
    .bind(envelope.ttl.map(|ttl| ttl.min(MAILBOX_LONGEST) as f64))
    .execute(&mut *tx)
    .await?;
    if let Some((space, limit)) = counted
        && !count(&mut tx, sender, key, space, limit).await?
    {
        return Ok(Kept::OverQuota);
    }
    wakers.tell(&mut tx, [recipient]).await?;
    tx.commit().await?;
    Ok(Kept::Answer(Some(stored)))
}

/// Counts the message of `sender`'s send with `key` in `space`'s month on `conn`, unless it is
/// counted already; `false` if it is not, as the space has stored `limit` messages this month.
async fn count(
    conn: &mut PgConnection,
    sender: &Sender,
    key: &SendKey,
    space: SpaceId,
    limit: u64,
) -> Result<bool, sqlx::Error> {
    // Of the message's envelopes stored at once, the second waits for the first to commit, and
    // finds it counted.
    let new = sqlx::query(
        "INSERT INTO counted_messages (sender_id, msg_id, space_id)
         VALUES ($1::uuid, $2::uuid, $3::uuid) ON CONFLICT DO NOTHING",
    )
    .bind(sender.device.to_string())
    .bind(key.msg_id.to_string())
    .bind(space.to_string())
    .execute(&mut *conn)
    .await?;
    if new.rows_affected() == 0 {
        return Ok(true);
    }
    // Last in the transaction: the month's row lock makes the space's counts wait for each other.
    let counted: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO message_counts AS c (space_id, month, messages)
         VALUES ($1::uuid, date_trunc('month', now() AT TIME ZONE 'UTC')::date, 1)
         ON CONFLICT (space_id, month) DO UPDATE SET messages = c.messages + 1
         WHERE c.messages < $2
         RETURNING c.messages",
    )
    .bind(space.to_string())
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_optional(conn)
    .await?;
    Ok(counted.is_some())
}
