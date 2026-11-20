//! Storing sent envelopes: `docs/protocol/delivery.md`, "Send" and "Mailbox". The server answers a
//! device's `send` with `stored` once the envelope and its mailbox entries are committed, or with
//! `send-refused` and its problem, storing nothing of the envelope. An `hpke` envelope goes to its
//! one recipient, a `service` device only while a token of it that has not ended receives in the
//! envelope's channel; the channel's group routes an `mls` one ([`groups`](crate::groups)). It stores
//! at most one envelope per idempotency key and answers a retry as it answered the first send. It
//! counts each space's messages per month; with the server's cloud limits
//! (`docs/dev/architecture.md`, "Cloud limits"), it refuses a payload over the size limit, a
//! message past its space's number for the month, and one past its device's send rate.

use std::num::NonZeroU64;
use std::time::Duration;

use skald_crypto::MlsFraming;
use skald_proto::{
    AccountId, Cbor as _, Channel, ChannelId, DeviceId, Frame, Header, RawEnvelope, Roster,
    RosterBody, Scheme, SendKey, SpaceId, Stored,
};
use sqlx::{PgConnection, PgPool};

use crate::MAILBOX_LONGEST;
use crate::delivery::Wakers;
use crate::groups;
use crate::service_tokens::Token;

/// The device a connection is authenticated as.
pub(crate) struct Sender {
    pub(crate) device: DeviceId,
    pub(crate) account: AccountId,
    /// The protocol version of its connection.
    pub(crate) version: NonZeroU64,
    /// The service token of its connection, for a `service` device.
    pub(crate) token: Option<Token>,
}

/// The server's limits on sends; none without a setting.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Limits {
    /// The longest payload, in bytes.
    pub(crate) message_size: Option<u64>,
    /// The messages a space stores per calendar month in UTC.
    pub(crate) monthly_messages: Option<u64>,
    /// The messages a device stores within a sliding window.
    pub(crate) send_rate: Option<SendRate>,
}

/// A send rate: `messages` per `window`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SendRate {
    pub(crate) messages: u64,
    pub(crate) window: Duration,
}

/// Why a send is refused: the problems of "Send".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Problem {
    VersionMismatch,
    NotAMember,
    EpochMismatch,
    UnknownRecipient,
    TooLarge,
    QuotaExceeded,
    RateLimited,
}

impl Problem {
    fn name(self) -> &'static str {
        match self {
            Self::VersionMismatch => "version-mismatch",
            Self::NotAMember => "not-a-member",
            Self::EpochMismatch => "epoch-mismatch",
            Self::UnknownRecipient => "unknown-recipient",
            Self::TooLarge => "too-large",
            Self::QuotaExceeded => "quota-exceeded",
            Self::RateLimited => "rate-limited",
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
/// wakes its recipients' connections in `wakers`, on each instance.
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
        payload: &envelope.envelope().payload,
    };
    answer(db, wakers, limits, sender, header, Some(outgoing)).await
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
    let outcome = match (route(db, sender, header, envelope).await?, envelope) {
        (Ok(_), Some(envelope))
            if limits
                .message_size
                .is_some_and(|limit| envelope.size as u64 > limit) =>
        {
            Err(Problem::TooLarge)
        }
        (Ok(route), Some(envelope)) => Ok((route, envelope)),
        // `route` refuses a send without one recipient; this is in case it does not.
        (Ok(_), None) => Err(Problem::UnknownRecipient),
        (Err(problem), _) => Err(problem),
    };
    // A send answered before keeps its answer: keeping this one finds its key taken.
    loop {
        let kept = match &outcome {
            Ok((route, envelope)) => {
                // A handshake message is no message of the space's to count. The others count
                // also without a limit, for the space's usage.
                let counted = (!route.lasting()).then_some((header.space, limits));
                match store(db, wakers, sender, header, &key, route, *envelope, counted).await? {
                    Kept::Answer(answer) => answer,
                    Kept::Refused(problem) => refuse(db, sender, &key, problem).await?,
                }
            }
            Err(problem) => refuse(db, sender, &key, *problem).await?,
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

/// Where `sender`'s envelope with `header`, which is `envelope` if it can be stored, goes, or the
/// problem it is refused with.
async fn route(
    db: &PgPool,
    sender: &Sender,
    header: &Header,
    envelope: Option<Outgoing<'_>>,
) -> Result<Result<Route, Problem>, sqlx::Error> {
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
    let in_channel = |account, device| in_channel(body, channel, account, device);
    // A `service` device sends with its token only where the token's send permission and channels
    // let it.
    let sends = match &sender.token {
        Some(token) => token.sends_in(header.space, header.channel),
        None => in_channel(sender.account, sender.device),
    };
    if !sends {
        return Ok(Err(Problem::NotAMember));
    }
    if header.scheme == Scheme::Mls {
        let framing = envelope.and_then(|envelope| MlsFraming::read(envelope.payload));
        return Ok(groups::addressed(header, framing).map(Route::Group));
    }
    let Some(recipient) = send_key(header).recipient else {
        return Ok(Err(Problem::UnknownRecipient));
    };
    // A `service` device receives in a channel only with a token: `store` checks it.
    match certified(db, recipient).await? {
        Some((_, true)) => Ok(Ok(Route::Service(recipient))),
        Some((account, false)) if in_channel(account, recipient) => {
            Ok(Ok(Route::Device(recipient)))
        }
        _ => Ok(Err(Problem::UnknownRecipient)),
    }
}

/// Whether `device` of `account` is in `channel` of the revision `body`: its account a member of
/// the channel, the device not excluded.
pub(crate) fn in_channel(
    body: &RosterBody,
    channel: &Channel,
    account: AccountId,
    device: DeviceId,
) -> bool {
    channel.members.iter().any(|m| m.account == account)
        && body.members.iter().any(|m| {
            m.account == account && !m.excluded.as_ref().is_some_and(|e| e.contains(&device))
        })
}

/// Where an envelope goes.
enum Route {
    /// To its one recipient device, under `hpke`.
    Device(DeviceId),
    /// To its one recipient, a `service` device, under `hpke`, while a token of it that has not
    /// ended and has the receive permission lists the envelope's channel.
    Service(DeviceId),
    /// Where the channel's group routes it, under `mls`: an `MLSMessage` of this framing.
    Group(MlsFraming),
}

impl Route {
    /// Whether its entries do not expire by time: those of a handshake message.
    fn lasting(&self) -> bool {
        matches!(self, Self::Group(framing) if framing.is_handshake())
    }
}

/// The latest roster revision of `space`, if it has one.
pub(crate) async fn latest(
    db: impl sqlx::PgExecutor<'_>,
    space: SpaceId,
) -> Result<Option<Roster>, sqlx::Error> {
    let found: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT roster FROM roster_revisions WHERE space_id = $1::uuid
         ORDER BY revision DESC LIMIT 1",
    )
    .bind(space.to_string())
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(roster,)| Roster::from_cbor(&roster).expect("a stored revision decodes")))
}

/// The account of `device`, and whether it is a `service` device, if it has a certificate on the
/// server and is not revoked.
async fn certified(
    db: &PgPool,
    device: DeviceId,
) -> Result<Option<(AccountId, bool)>, sqlx::Error> {
    let found: Option<(String, bool)> = sqlx::query_as(
        "SELECT account_id::text, kind = 'service' FROM devices
         WHERE id = $1::uuid AND cert IS NOT NULL AND NOT revoked",
    )
    .bind(device.to_string())
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(account, service)| (account.parse().expect("a stored account ID"), service)))
}

/// Whether `device` has a token that has not ended and has the receive permission, of `space` and
/// listing `channel`. Locked on `conn` until it commits: a revocation of the token, or a revision
/// that ends it, waits for the envelope stored meanwhile.
async fn receives_in(
    conn: &mut PgConnection,
    device: DeviceId,
    (space, channel): (SpaceId, ChannelId),
) -> Result<bool, sqlx::Error> {
    let found: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM service_tokens
         WHERE device_id = $1::uuid AND space_id = $2::uuid AND $3::uuid = ANY(channels)
             AND 'receive' = ANY(permissions) AND expires_at > now()
         LIMIT 1 FOR SHARE",
    )
    .bind(device.to_string())
    .bind(space.to_string())
    .bind(channel.to_string())
    .fetch_optional(conn)
    .await?;
    Ok(found.is_some())
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

/// An envelope to store: its bytes, its payload's length, whether it gets statuses, its `ttl`, and
/// its payload.
#[derive(Clone, Copy)]
struct Outgoing<'a> {
    bytes: &'a [u8],
    size: usize,
    report: bool,
    ttl: Option<u64>,
    payload: &'a [u8],
}

/// What storing a send gave.
enum Kept {
    /// Its `stored`; `None` if a send with its key is answered already.
    Answer(Option<Frame>),
    /// Nothing, for this problem: its `hpke` recipient is revoked or, a `service` device, has no
    /// token that receives in the channel, the channel's group refuses it, or its message is past
    /// its space's number for the month or its device's send rate.
    Refused(Problem),
}

/// Stores `envelope`, the send of `sender` with `key` and `header`, with an entry in the mailbox of
/// each device `route` gives, and gives its `stored`; tells the other instances in `wakers`, and
/// wakes the recipients' connections on this one. With `counted`, a space and the server's limits, a
/// message not counted yet counts in the space's month and towards its device's send rate, and is
/// not stored past either limit. Nothing is stored for a revoked `hpke` recipient, and no entry for
/// a revoked device.
#[allow(clippy::too_many_arguments)]
async fn store(
    db: &PgPool,
    wakers: &Wakers,
    sender: &Sender,
    header: &Header,
    key: &SendKey,
    route: &Route,
    envelope: Outgoing<'_>,
    counted: Option<(SpaceId, Limits)>,
) -> Result<Kept, sqlx::Error> {
    let mut tx = db.begin().await?;
    let routed = match route {
        Route::Service(device)
            if !receives_in(&mut tx, *device, (header.space, header.channel)).await? =>
        {
            Err(Problem::UnknownRecipient)
        }
        Route::Device(device) | Route::Service(device) => {
            let open = receiving(&mut tx, &[*device]).await?;
            if open.is_empty() {
                Err(Problem::UnknownRecipient)
            } else {
                Ok(open)
            }
        }
        // Locks the channel's group: its envelopes are stored one at a time.
        Route::Group(framing) => {
            groups::route(&mut tx, sender, header, framing, envelope.payload).await?
        }
    };
    let recipients = match routed {
        Ok(devices) => devices,
        Err(problem) => return Ok(Kept::Refused(problem)),
    };
    let Some(stored) = keep(&mut tx, sender, key, None).await? else {
        return Ok(Kept::Answer(None));
    };
    // An envelope no entry refers to is not kept: a channel's first Commit goes to no one.
    let envelope_id: Option<(i64,)> = if recipients.is_empty() {
        None
    } else {
        Some(
            sqlx::query_as(
                "INSERT INTO envelopes (envelope, sender_id, msg_id, report)
         VALUES ($1, $2::uuid, $3::uuid, $4) RETURNING id",
            )
            .bind(envelope.bytes)
            .bind(sender.device.to_string())
            .bind(key.msg_id.to_string())
            .bind(envelope.report)
            .fetch_one(&mut *tx)
            .await?,
        )
    };
    let lasting = route.lasting();
    // Past `MAILBOX_LONGEST`, a `ttl` changes nothing; a handshake message's entry has none.
    let ttl = (envelope.ttl)
        .filter(|_| !lasting)
        .map(|ttl| ttl.min(MAILBOX_LONGEST) as f64);
    // `receiving` gives each recipient once, in the order of their IDs: stores to the same
    // mailboxes at once lock them in one order.
    for recipient in &recipients {
        // The mailbox's row lock makes stores to it wait for each other, so its entries are
        // numbered in the order they are committed.
        sqlx::query(
            "WITH mailbox AS (
                 INSERT INTO mailboxes (device_id, last_seq) VALUES ($1::uuid, 1)
                 ON CONFLICT (device_id) DO UPDATE SET last_seq = mailboxes.last_seq + 1
                 RETURNING last_seq
             )
             INSERT INTO entries (device_id, seq, envelope_id, ttl_ends_at, lasting)
             SELECT $1::uuid, mailbox.last_seq, $2, now() + make_interval(secs => $3), $4
             FROM mailbox",
        )
        .bind(recipient.to_string())
        .bind(envelope_id.map(|(id,)| id))
        .bind(ttl)
        .bind(lasting)
        .execute(&mut *tx)
        .await?;
    }
    if let Some((space, limits)) = counted
        && let Some(problem) = count(&mut tx, sender, key, space, limits).await?
    {
        return Ok(Kept::Refused(problem));
    }
    wakers.tell(&mut tx, recipients.iter().copied()).await?;
    tx.commit().await?;
    for recipient in recipients {
        wakers.wake(recipient);
    }
    Ok(Kept::Answer(Some(stored)))
}

/// Of `devices`, those that get a mailbox entry: with a certificate, not revoked. Locked on `conn`
/// until it commits: a device's row lock makes its revocation wait for the entries stored for it,
/// which the revocation then expires, or these stores see the revocation.
pub(crate) async fn receiving(
    conn: &mut PgConnection,
    devices: &[DeviceId],
) -> Result<Vec<DeviceId>, sqlx::Error> {
    let devices: Vec<String> = devices.iter().map(ToString::to_string).collect();
    let found: Vec<(String,)> = sqlx::query_as(
        "SELECT id::text FROM devices
         WHERE id = ANY($1::text[]::uuid[]) AND cert IS NOT NULL AND NOT revoked
         ORDER BY id FOR SHARE",
    )
    .bind(devices)
    .fetch_all(conn)
    .await?;
    Ok(found
        .iter()
        .map(|(device,)| device.parse().expect("a stored device ID"))
        .collect())
}

/// Counts the message of `sender`'s send with `key` in `space`'s month and towards `sender`'s send
/// rate on `conn`, unless it is counted already; the problem if it is past either of `limits`.
async fn count(
    conn: &mut PgConnection,
    sender: &Sender,
    key: &SendKey,
    space: SpaceId,
    limits: Limits,
) -> Result<Option<Problem>, sqlx::Error> {
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
        return Ok(None);
    }
    if let Some(rate) = limits.send_rate
        && !within_rate(conn, sender, rate).await?
    {
        return Ok(Some(Problem::RateLimited));
    }
    let limit = limits.monthly_messages;
    // Last in the transaction: the month's row lock makes the space's counts wait for each other.
    let counted: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO message_counts AS c (space_id, month, messages)
         VALUES ($1::uuid, date_trunc('month', now() AT TIME ZONE 'UTC')::date, 1)
         ON CONFLICT (space_id, month) DO UPDATE SET messages = c.messages + 1
         WHERE $2::bigint IS NULL OR c.messages < $2
         RETURNING c.messages",
    )
    .bind(space.to_string())
    .bind(limit.map(|limit| i64::try_from(limit).unwrap_or(i64::MAX)))
    .fetch_optional(conn)
    .await?;
    Ok(counted.is_none().then_some(Problem::QuotaExceeded))
}

/// Whether `sender`'s messages counted within `rate`'s window, the one just counted on `conn`
/// included, are at most its number.
async fn within_rate(
    conn: &mut PgConnection,
    sender: &Sender,
    rate: SendRate,
) -> Result<bool, sqlx::Error> {
    // The device's counts, on any instance, wait for each other: each sees the ones before it.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('send-rate ' || $1, 0))")
        .bind(sender.device.to_string())
        .execute(&mut *conn)
        .await?;
    let (counted,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM counted_messages
         WHERE sender_id = $1::uuid AND counted_at > now() - make_interval(secs => $2)",
    )
    .bind(sender.device.to_string())
    .bind(rate.window.as_secs_f64())
    .fetch_one(conn)
    .await?;
    Ok(u64::try_from(counted).unwrap_or(0) <= rate.messages)
}
