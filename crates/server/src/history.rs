//! A channel's history on the server: `docs/protocol/membership.md`, "Uploads" and "History
//! entry". A device uploads links, wraps and entries on its connection, which proves its key, so a
//! session alone uploads none. The server keeps the first link per channel, epoch and `prev` and
//! the first wrap per channel, epoch and recipient, each from a member device of its epoch, and the
//! first entry per sending device and `msg_id`, of the channel's current or previous epoch, from a
//! member device of the current one. It gives them, as the bytes it got, to sessions of the
//! accounts the latest revision lists as the channel's members; to those without `history`, only
//! from their account's join ("Access"). It refuses links and wraps of epochs before the channel's
//! cutoff, which the history retention sets ([`crate::history_retention`]).

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use skald_proto::{
    Cbor as _, ChannelId, Frame, HistoryEntry, HistoryItem, HistoryLink, HistoryWrap, Roster,
    SpaceId, WrapRecipient,
};
use sqlx::{PgConnection, PgPool};

use crate::AppState;
use crate::devices::base64;
use crate::groups::members_channel;
use crate::mailboxes::{Sender, receiving};
use crate::refusal::{Answer, Refusal, SignedIn};

/// Why an upload is refused: the problems of "Uploads".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Problem {
    Malformed,
    NotAMember,
    EpochMismatch,
}

impl Problem {
    fn name(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::NotAMember => "not-a-member",
            Self::EpochMismatch => "epoch-mismatch",
        }
    }
}

/// An item to keep, decoded as far as the server reads it.
enum Upload<'a> {
    Link {
        epoch: u64,
        prev: u64,
        bytes: &'a [u8],
    },
    Wrap {
        epoch: u64,
        recipient: WrapRecipient,
        bytes: &'a [u8],
    },
    Entry {
        epoch: u64,
        msg_id: String,
        bytes: &'a [u8],
    },
}

impl Upload<'_> {
    /// The epoch whose member devices may upload it, given the channel's current epoch.
    fn uploaders(&self, current: u64) -> u64 {
        match self {
            Self::Link { epoch, .. } | Self::Wrap { epoch, .. } => *epoch,
            Self::Entry { .. } => current,
        }
    }

    /// Whether its epoch is one the channel may take it for, given its current epoch and its
    /// `cutoff`, the earliest epoch whose links and wraps the retention left.
    fn in_epoch(&self, current: u64, cutoff: u64) -> bool {
        match self {
            Self::Link { epoch, .. } | Self::Wrap { epoch, .. } => {
                (cutoff..=current).contains(epoch)
            }
            Self::Entry { epoch, .. } => *epoch == current || *epoch + 1 == current,
        }
    }
}

/// `item` decoded as its kind, of `channel`; `None` if it does not decode, is of another channel
/// or has a `prev` that is not below its epoch.
fn decode(item: &HistoryItem, channel: ChannelId) -> Option<Upload<'_>> {
    let (of, upload) = match item {
        HistoryItem::Link(bytes) => {
            let link = HistoryLink::from_cbor(bytes).ok()?;
            if link.prev >= link.epoch {
                return None;
            }
            let upload = Upload::Link {
                epoch: link.epoch,
                prev: link.prev,
                bytes,
            };
            (link.channel, upload)
        }
        HistoryItem::Wrap(bytes) => {
            let wrap = HistoryWrap::from_cbor(bytes).ok()?;
            let body = wrap.body();
            let upload = Upload::Wrap {
                epoch: body.epoch,
                recipient: body.recipient,
                bytes,
            };
            (body.channel, upload)
        }
        HistoryItem::Entry { msg_id, entry } => {
            let decoded = HistoryEntry::from_cbor(entry).ok()?;
            let upload = Upload::Entry {
                epoch: decoded.epoch,
                msg_id: msg_id.to_string(),
                bytes: entry,
            };
            (decoded.channel, upload)
        }
    };
    (of == channel).then_some(upload)
}

/// The answer to `sender`'s upload `upload` of `item` in `channel` of `space`: `history-stored`
/// once it is kept, now or before, otherwise `history-refused`, keeping nothing.
pub(crate) async fn upload(
    db: &PgPool,
    sender: &Sender,
    upload: u64,
    space: SpaceId,
    channel: ChannelId,
    item: &HistoryItem,
) -> Result<Frame, sqlx::Error> {
    let mut tx = db.begin().await?;
    let answer = match keep(&mut tx, sender, space, channel, item).await? {
        Ok(()) => Frame::HistoryStored { upload },
        Err(problem) => Frame::HistoryRefused {
            upload,
            problem: problem.name().to_owned(),
        },
    };
    tx.commit().await?;
    Ok(answer)
}

/// Keeps `sender`'s `item` in `channel` of `space` on `tx`, unless one is kept in its place, or
/// gives the problem it is refused with.
async fn keep(
    tx: &mut PgConnection,
    sender: &Sender,
    space: SpaceId,
    channel: ChannelId,
    item: &HistoryItem,
) -> Result<Result<(), Problem>, sqlx::Error> {
    let Some(upload) = decode(item, channel) else {
        return Ok(Err(Problem::Malformed));
    };
    // The group's row lock makes a Commit or the retention's deletion wait for the upload, or the
    // upload see it.
    let current: Option<(i64, i64)> = sqlx::query_as(
        "SELECT g.epoch, g.history_cutoff FROM channel_groups g
         JOIN channels c ON c.id = g.channel_id
         WHERE g.channel_id = $1::uuid AND c.space_id = $2::uuid AND g.view IS NOT NULL
         FOR SHARE OF g",
    )
    .bind(channel.to_string())
    .bind(space.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let Some((current, cutoff)) = current else {
        return Ok(Err(Problem::NotAMember));
    };
    let current = u64::try_from(current).expect("a stored epoch from 1");
    let cutoff = u64::try_from(cutoff).expect("a stored cutoff from 0");
    if !upload.in_epoch(current, cutoff) {
        return Ok(Err(Problem::EpochMismatch));
    }
    let member: Option<(bool,)> = sqlx::query_as(
        "SELECT $3::uuid = ANY(members) FROM channel_epochs
         WHERE channel_id = $1::uuid AND epoch = $2",
    )
    .bind(channel.to_string())
    .bind(epoch(upload.uploaders(current)))
    .bind(sender.device.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    // A revoked device is no member: its row lock makes a revocation wait for the upload.
    if member != Some((true,)) || receiving(tx, &[sender.device]).await?.is_empty() {
        return Ok(Err(Problem::NotAMember));
    }
    // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
    let channel = channel.to_string();
    let insert = match &upload {
        Upload::Link {
            epoch: e,
            prev,
            bytes,
        } => sqlx::query(
            "INSERT INTO history_links (channel_id, epoch, prev, link)
             VALUES ($1::uuid, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(channel)
        .bind(epoch(*e))
        .bind(epoch(*prev))
        .bind(*bytes),
        Upload::Wrap {
            epoch: e,
            recipient,
            bytes,
        } => {
            let (kind, id) = match recipient {
                WrapRecipient::Browser(device) => (1_i16, device.to_string()),
                WrapRecipient::Recovery(account) => (2, account.to_string()),
            };
            sqlx::query(
                "INSERT INTO history_wraps (channel_id, epoch, recipient_kind, recipient_id, wrap)
                 VALUES ($1::uuid, $2, $3, $4::uuid, $5) ON CONFLICT DO NOTHING",
            )
            .bind(channel)
            .bind(epoch(*e))
            .bind(kind)
            .bind(id)
            .bind(*bytes)
        }
        Upload::Entry { msg_id, bytes, .. } => sqlx::query(
            "INSERT INTO history_entries (sender_id, msg_id, channel_id, entry)
             VALUES ($1::uuid, $2::uuid, $3::uuid, $4) ON CONFLICT DO NOTHING",
        )
        .bind(sender.device.to_string())
        .bind(msg_id.as_str())
        .bind(channel)
        .bind(*bytes),
    };
    insert.execute(&mut *tx).await?;
    Ok(Ok(()))
}

/// `epoch` as the database keeps it.
pub(crate) fn epoch(epoch: u64) -> i64 {
    i64::try_from(epoch).expect("an epoch below 2^63")
}

/// What a channel's history holds, each a path's last segment.
pub(crate) const LINKS: u8 = 0;
pub(crate) const WRAPS: u8 = 1;
pub(crate) const ENTRIES: u8 = 2;

/// `GET /spaces/<space>/channels/<channel>/history/<links|wraps|entries>`: the channel's items of
/// `KIND`, base64url without padding, in a JSON object under the kind's name; for sessions of the
/// channel's members in the latest revision. A member without `history` gets only the links whose
/// `prev`, and the wraps and entries whose epoch, is at or after its account's join ("Access").
pub(crate) async fn given<const KIND: u8>(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(path): Path<(String, String)>,
) -> Answer {
    let (name, query) = match KIND {
        LINKS => (
            "links",
            "SELECT link FROM history_links WHERE channel_id = $1::uuid AND prev >= $2
             ORDER BY epoch, prev",
        ),
        WRAPS => (
            "wraps",
            "SELECT wrap FROM history_wraps WHERE channel_id = $1::uuid AND epoch >= $2
             ORDER BY epoch",
        ),
        _ => (
            "entries",
            "SELECT entry FROM history_entries WHERE channel_id = $1::uuid",
        ),
    };
    let (channel, history) = members_channel(&state, &account, &path.0, &path.1).await?;
    let failed = |err| Refusal::Database("read a channel's history", err);
    let from = if history {
        0
    } else {
        joined(&state.db, channel, &account)
            .await
            .map_err(failed)?
            .unwrap_or(i64::MAX)
    };
    let mut items = sqlx::query_as(query).bind(channel.to_string());
    if KIND != ENTRIES {
        items = items.bind(from);
    }
    let items: Vec<(Vec<u8>,)> = items.fetch_all(&state.db).await.map_err(failed)?;
    let given = |bytes: &[u8]| {
        KIND != ENTRIES
            || from == 0
            || HistoryEntry::from_cbor(bytes).is_ok_and(|entry| epoch(entry.epoch) >= from)
    };
    let items: Vec<String> = (items.iter())
        .filter(|(bytes,)| given(bytes))
        .map(|(bytes,)| base64(bytes))
        .collect();
    Ok(Json(serde_json::json!({ name: items })).into_response())
}

/// The epoch whose Commit added `account` to `channel`: the first epoch with a device of the
/// account among its members after the revision that last added the account to the channel.
/// `None` if none has one yet.
async fn joined(
    db: &PgPool,
    channel: ChannelId,
    account: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let (joined,): (Option<i64>,) = sqlx::query_as(
        "SELECT min(e.epoch) FROM channel_epochs e
         WHERE e.channel_id = $1::uuid
           AND e.epoch > coalesce((
               SELECT a.epoch FROM channel_additions a
               WHERE a.channel_id = $1::uuid AND a.account_id = $2::uuid
           ), 0)
           AND EXISTS (
               SELECT 1 FROM devices d WHERE d.id = ANY(e.members) AND d.account_id = $2::uuid
           )",
    )
    .bind(channel.to_string())
    .bind(account)
    .fetch_one(db)
    .await?;
    Ok(joined)
}

/// Keeps, on `tx`, the current epoch of each channel of `roster` for each account it lists among
/// the channel's members that `previous` does not: the account's join comes after it ("Access").
pub(crate) async fn added(
    tx: &mut PgConnection,
    roster: &Roster,
    previous: Option<&Roster>,
) -> Result<(), sqlx::Error> {
    let (mut channels, mut accounts) = (Vec::new(), Vec::new());
    for channel in &roster.body().channels {
        let before = previous.and_then(|previous| {
            (previous.body().channels.iter()).find(|before| before.id == channel.id)
        });
        let listed = |account| {
            before.is_some_and(|before| before.members.iter().any(|m| m.account == account))
        };
        for member in channel.members.iter().filter(|m| !listed(m.account)) {
            channels.push(channel.id.to_string());
            accounts.push(member.account.to_string());
        }
    }
    sqlx::query(
        "INSERT INTO channel_additions (channel_id, account_id, epoch)
         SELECT a.channel::uuid, a.account::uuid, coalesce(g.epoch, 0)
         FROM unnest($1::text[], $2::text[]) AS a (channel, account)
         LEFT JOIN channel_groups g ON g.channel_id = a.channel::uuid
         ON CONFLICT (channel_id, account_id) DO UPDATE SET epoch = excluded.epoch",
    )
    .bind(&channels)
    .bind(&accounts)
    .execute(tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use skald_proto::Unknown;

    use super::*;

    /// A link of `channel`'s epoch `epoch` to `prev`.
    fn link(channel: ChannelId, epoch: u64, prev: u64) -> HistoryItem {
        let link = HistoryLink {
            channel,
            epoch,
            prev,
            nonce: [0; 12],
            ciphertext: vec![1; 32],
            unknown: Unknown::default(),
        };
        HistoryItem::Link(link.to_cbor())
    }

    #[test]
    fn a_link_decodes_only_of_its_channel_with_prev_below_its_epoch() {
        let channel = ChannelId::generate();
        assert!(matches!(
            decode(&link(channel, 3, 2), channel),
            Some(Upload::Link {
                epoch: 3,
                prev: 2,
                ..
            })
        ));
        for (item, what) in [
            (link(channel, 3, 3), "prev at its epoch"),
            (link(channel, 3, 4), "prev above its epoch"),
            (link(ChannelId::generate(), 3, 2), "of another channel"),
        ] {
            assert!(decode(&item, channel).is_none(), "{what}");
        }
    }

    #[test]
    fn an_entry_is_taken_for_the_current_or_previous_epoch_only() {
        let entry = |epoch| Upload::Entry {
            epoch,
            msg_id: String::new(),
            bytes: &[],
        };
        assert!(entry(3).in_epoch(3, 0) && entry(2).in_epoch(3, 0));
        assert!(!entry(1).in_epoch(3, 0) && !entry(4).in_epoch(3, 0));
        assert_eq!(entry(2).uploaders(3), 3, "members of the current epoch");
    }

    #[test]
    fn a_link_or_wrap_is_taken_from_the_cutoff_to_the_current_epoch() {
        let link = |epoch| Upload::Link {
            epoch,
            prev: 1,
            bytes: &[],
        };
        assert!(link(3).in_epoch(4, 3) && link(4).in_epoch(4, 3));
        assert!(!link(2).in_epoch(4, 3) && !link(5).in_epoch(4, 3));
    }
}
