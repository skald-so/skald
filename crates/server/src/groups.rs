//! The server as each channel's MLS Delivery Service (`docs/protocol/membership.md`, "Creating a
//! channel", "Commit ordering" and "MLS profile"). It follows a channel's group from outside it,
//! with [`ChannelView`], kept in the database so that every instance and a restart go on from it:
//! it keeps the channel's first Commit of epoch 0, starts the view from the GroupInfo of epoch 1
//! that the Commit's sender uploads, then accepts a Commit only of the current epoch from the
//! device of its sender leaf, or its joiner. It routes the channel's `mls` envelopes to the member
//! devices of the group ("Envelope"; `docs/protocol/delivery.md`, "Send"), one at a time under the
//! group's row lock, so every mailbox holds them in one order ("Order"). It keeps the latest
//! GroupInfo of the current epoch and gives it to the channel's members, and the member devices of
//! each epoch, which may upload its history ([`history`](crate::history)).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use skald_crypto::{ChannelView, MlsContent, MlsFraming};
use skald_proto::{AccountId, ChannelId, ChannelMember, DeviceId, Header, Roster};
use sqlx::PgConnection;

use crate::devices::{base64, bytes};
use crate::mailboxes::{Problem, Sender, latest as latest_revision, receiving};
use crate::refusal::{Answer, Caller, Refusal, parse};
use crate::spaces::readable;
use crate::{AppState, presence};

/// Checks how an `mls` envelope with `header` and a payload of `framing`, `None` if it is no
/// `MLSMessage`, is addressed: a Welcome to `recipients`, a message of the channel's group to the
/// channel. [`route`] refuses anything else.
pub(crate) fn addressed(
    header: &Header,
    framing: Option<MlsFraming>,
) -> Result<MlsFraming, Problem> {
    let framing = framing.ok_or(Problem::NotAMember)?;
    match &framing {
        MlsFraming::Message { group, .. } if group != header.channel.as_bytes() => {
            Err(Problem::NotAMember)
        }
        MlsFraming::Welcome if header.recipients.is_none() => Err(Problem::UnknownRecipient),
        MlsFraming::Message { .. } if header.recipients.is_some() => Err(Problem::UnknownRecipient),
        _ => Ok(framing),
    }
}

/// A channel's group as the server follows it.
struct Group {
    /// The device whose Commit of epoch 0 the server kept, until the server erases its account.
    creator: Option<DeviceId>,
    epoch: u64,
    /// The saved view, from the first GroupInfo on.
    view: Option<Vec<u8>>,
    /// The view's member devices.
    members: Vec<DeviceId>,
}

/// A group as [`locked`] reads it: its creator, epoch, view and members.
type Row = (Option<String>, i64, Option<Vec<u8>>, Vec<String>);

/// The group of `channel`, if it has one, locked on `tx` until it commits.
async fn locked(tx: &mut PgConnection, channel: ChannelId) -> Result<Option<Group>, sqlx::Error> {
    let found: Option<Row> = sqlx::query_as(
        "SELECT creator_id::text, epoch, view, members::text[] FROM channel_groups
         WHERE channel_id = $1::uuid FOR UPDATE",
    )
    .bind(channel.to_string())
    .fetch_optional(tx)
    .await?;
    Ok(found.map(|(creator, epoch, view, members)| Group {
        creator: creator.map(|creator| creator.parse().expect("a stored device ID")),
        epoch: u64::try_from(epoch).expect("a stored epoch from 1"),
        view,
        members: members
            .iter()
            .map(|member| member.parse().expect("a stored device ID"))
            .collect(),
    }))
}

/// The devices that `sender`'s `mls` envelope with `header`, of `framing` as [`addressed`] took
/// it, goes to, or the problem it is refused with; an accepted Commit moves the channel's group on
/// `tx` to its new epoch. Locks the channel's group on `tx` until it commits.
pub(crate) async fn route(
    tx: &mut PgConnection,
    sender: &Sender,
    header: &Header,
    framing: &MlsFraming,
    payload: &[u8],
) -> Result<Result<Vec<DeviceId>, Problem>, sqlx::Error> {
    let channel = header.channel;
    let group = locked(tx, channel).await?;
    let routed = match framing {
        MlsFraming::Welcome => welcome(group.as_ref(), header),
        MlsFraming::Message {
            epoch,
            public: true,
            content: MlsContent::Commit,
            ..
        } => commit(tx, sender, channel, group, *epoch, payload).await?,
        MlsFraming::Message {
            public: false,
            content: MlsContent::Application,
            ..
        } => application(group.as_ref(), sender.device),
        // A proposal, a PrivateMessage Commit and a PublicMessage application message, which the
        // profile does not send ("MLS profile").
        _ => Err(Problem::NotAMember),
    };
    Ok(match routed {
        Ok(devices) => Ok(receiving(tx, &devices).await?),
        Err(problem) => Err(problem),
    })
}

/// A Welcome's recipients: each a member device of the current epoch, otherwise
/// `unknown-recipient`.
fn welcome(group: Option<&Group>, header: &Header) -> Result<Vec<DeviceId>, Problem> {
    let members = followed(group)?;
    let recipients = header.recipients.as_deref().unwrap_or_default();
    if recipients.iter().all(|device| members.contains(device)) {
        Ok(recipients.to_vec())
    } else {
        Err(Problem::UnknownRecipient)
    }
}

/// An application message's recipients: each member device of the current epoch but `sender`,
/// which must be one.
fn application(group: Option<&Group>, sender: DeviceId) -> Result<Vec<DeviceId>, Problem> {
    let members = followed(group)?;
    if !members.contains(&sender) {
        return Err(Problem::NotAMember);
    }
    Ok(others(members, sender))
}

/// The member devices of `group` once the server follows it from its first GroupInfo; before it,
/// the channel's envelopes other than a Commit are refused, so the creator uploads it first.
fn followed(group: Option<&Group>) -> Result<&[DeviceId], Problem> {
    match group {
        Some(Group {
            view: Some(_),
            members,
            ..
        }) => Ok(members),
        _ => Err(Problem::NotAMember),
    }
}

/// `members` but `sender`.
fn others(members: &[DeviceId], sender: DeviceId) -> Vec<DeviceId> {
    members
        .iter()
        .copied()
        .filter(|device| *device != sender)
        .collect()
}

/// A Commit's recipients: each member device of the epoch it commits from but `sender`. The first
/// Commit of epoch 0 starts the channel's group at epoch 1 on `tx`; a later one must be of the
/// current epoch, otherwise `epoch-mismatch`, and one the view follows, sent by the device of its
/// sender leaf or its joiner, otherwise `not-a-member`; the view follows it on `tx`, and the presence
/// values of the devices it removes go.
async fn commit(
    tx: &mut PgConnection,
    sender: &Sender,
    channel: ChannelId,
    group: Option<Group>,
    epoch: u64,
    payload: &[u8],
) -> Result<Result<Vec<DeviceId>, Problem>, sqlx::Error> {
    let group = match group {
        None if epoch == 0 => return create(tx, sender.device, channel).await,
        None => return Ok(Err(Problem::EpochMismatch)),
        Some(group) => group,
    };
    if epoch != group.epoch {
        return Ok(Err(Problem::EpochMismatch));
    }
    let Some(saved) = &group.view else {
        // Before the first GroupInfo the server cannot tell a member's Commit.
        return Ok(Err(Problem::NotAMember));
    };
    let Some(next) = followed_by(channel, saved, payload, sender.device) else {
        return Ok(Err(Problem::NotAMember));
    };
    sqlx::query(
        "UPDATE channel_groups
         SET epoch = $2, view = $3, members = $4::text[]::uuid[], group_info = NULL
         WHERE channel_id = $1::uuid",
    )
    .bind(channel.to_string())
    .bind(i64::try_from(next.epoch).expect("an epoch below 2^63"))
    .bind(&next.view)
    .bind(texts(&next.members))
    .execute(&mut *tx)
    .await?;
    keep_epoch(tx, channel, &next).await?;
    presence::keep_members(tx, channel, &texts(&next.members)).await?;
    Ok(Ok(others(&group.members, sender.device)))
}

/// Keeps `creator`'s Commit of epoch 0 as `channel`'s first on `tx`: the group at epoch 1, which
/// goes to no one, as the epoch it commits from has no member but its sender. Refused with
/// `epoch-mismatch` if another one was kept meanwhile.
async fn create(
    tx: &mut PgConnection,
    creator: DeviceId,
    channel: ChannelId,
) -> Result<Result<Vec<DeviceId>, Problem>, sqlx::Error> {
    // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
    let inserted = sqlx::query(
        "INSERT INTO channel_groups (channel_id, creator_id, epoch) VALUES ($1::uuid, $2::uuid, 1)
         ON CONFLICT DO NOTHING",
    )
    .bind(channel.to_string())
    .bind(creator.to_string())
    .execute(tx)
    .await?;
    Ok(if inserted.rows_affected() == 1 {
        Ok(Vec::new())
    } else {
        Err(Problem::EpochMismatch)
    })
}

/// Keeps the member devices of `view`'s epoch of `channel` on `tx`.
async fn keep_epoch(
    tx: &mut PgConnection,
    channel: ChannelId,
    view: &Saved,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO channel_epochs (channel_id, epoch, members)
         VALUES ($1::uuid, $2, $3::text[]::uuid[]) ON CONFLICT DO NOTHING",
    )
    .bind(channel.to_string())
    .bind(i64::try_from(view.epoch).expect("an epoch below 2^63"))
    .bind(texts(&view.members))
    .execute(tx)
    .await?;
    Ok(())
}

/// A view as the database keeps it: saved, with its epoch and member devices.
struct Saved {
    epoch: u64,
    view: Vec<u8>,
    members: Vec<DeviceId>,
}

impl Saved {
    fn of(view: &ChannelView) -> Self {
        Saved {
            epoch: view.epoch(),
            view: view.save().expect("a view saves"),
            members: view.members().iter().map(|member| member.device).collect(),
        }
    }
}

/// The view of `channel` after `commit`, from the view `saved`, if the view follows it and
/// `sender` is the device of its sender leaf or its joiner.
fn followed_by(channel: ChannelId, saved: &[u8], commit: &[u8], sender: DeviceId) -> Option<Saved> {
    let view = ChannelView::load(channel, saved).expect("a saved view loads");
    let next = view.follow(commit).ok()?;
    (next.signer().device == sender).then(|| Saved::of(&next))
}

/// `devices` as text, as the queries take them.
fn texts(devices: &[DeviceId]) -> Vec<String> {
    devices.iter().map(ToString::to_string).collect()
}

/// `GroupInfoRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupInfoRequest {
    group_info: String,
}

/// `ChannelGroup` in the spec.
#[derive(Serialize)]
struct ChannelGroup {
    group: bool,
    group_info: Option<String>,
}

/// The channel `channel` of the space `space`, path parameters, that the latest revision lists,
/// for `caller`: a session of an account that gets the space, or a service token of the space that
/// lists the channel; with the channel's members.
async fn channel_of(
    state: &AppState,
    caller: &Caller,
    space: &str,
    channel: &str,
) -> Result<(ChannelId, Vec<ChannelMember>), Refusal> {
    let space = readable(&state.db, caller, space).await?;
    let channel: ChannelId = channel.parse().map_err(|_| Refusal::NoChannel)?;
    if caller
        .token()
        .is_some_and(|token| !token.lists(space, channel))
    {
        return Err(Refusal::NotAllowed(
            "the service token does not list the channel".to_owned(),
        ));
    }
    let roster: Option<Roster> = latest_revision(&state.db, space)
        .await
        .map_err(|err| Refusal::Database("read the latest revision", err))?;
    let listed = roster.and_then(|roster| {
        let body = roster.body();
        let listed = body.channels.iter().find(|listed| listed.id == channel)?;
        Some(listed.members.clone())
    });
    Ok((channel, listed.ok_or(Refusal::NoChannel)?))
}

/// The channel `channel` of the space `space`, path parameters, for `caller`: a session of an
/// account the latest revision lists as one of the channel's members, or a service token that
/// lists the channel; with whether the revision lists the account, the token's device's for a
/// token, with `history`. Otherwise refused.
pub(crate) async fn members_channel(
    state: &AppState,
    caller: &Caller,
    space: &str,
    channel: &str,
) -> Result<(ChannelId, bool), Refusal> {
    let (channel, members) = channel_of(state, caller, space, channel).await?;
    let id: AccountId = caller.account().parse().expect("a stored account ID");
    let member = members.iter().find(|member| member.account == id);
    match (member, caller) {
        (Some(member), _) => Ok((channel, member.history)),
        (None, Caller::Token(_)) => Ok((channel, false)),
        (None, Caller::Session(_)) => Err(Refusal::NotAllowed(
            "only the channel's members in the latest revision get it".to_owned(),
        )),
    }
}

/// `GET /spaces/<space>/channels/<channel>/group-info`: whether the channel has a group, and its
/// GroupInfo of the current epoch if the server has one; for sessions of the channel's members in
/// the latest revision and service tokens that list the channel.
pub(crate) async fn latest(
    State(state): State<AppState>,
    caller: Caller,
    Path((space, channel)): Path<(String, String)>,
) -> Answer {
    let (channel, _) = members_channel(&state, &caller, &space, &channel).await?;
    let found: Option<(Option<Vec<u8>>,)> =
        sqlx::query_as("SELECT group_info FROM channel_groups WHERE channel_id = $1::uuid")
            .bind(channel.to_string())
            .fetch_optional(&state.db)
            .await
            .map_err(|err| Refusal::Database("read a channel's group", err))?;
    Ok(Json(ChannelGroup {
        group: found.is_some(),
        group_info: found
            .and_then(|(group_info,)| group_info)
            .map(|info| base64(&info)),
    })
    .into_response())
}

/// `PUT /spaces/<space>/channels/<channel>/group-info`: keeps a GroupInfo of the channel's current
/// epoch, from a session of an account with a member device that it lists, or a service token that
/// lists the channel, whose device it lists, with `ratchet_tree` and `external_pub`, that verifies
/// and has the GroupContext of the group the server follows. The first one, of epoch 1, only from
/// the device of the channel's first Commit; the server follows the group from it.
pub(crate) async fn upload(
    State(state): State<AppState>,
    caller: Caller,
    Path((space, channel)): Path<(String, String)>,
    body: Bytes,
) -> Answer {
    let (channel, _) = channel_of(&state, &caller, &space, &channel).await?;
    let account = caller.account();
    // A token uploads as its device alone.
    let only = caller.token().map(|token| token.device.to_string());
    let request: GroupInfoRequest = parse(&body)?;
    let group_info = bytes(&request.group_info, "group_info")?;
    let uploaded = ChannelView::observe_joinable(channel, &group_info).map_err(|err| {
        Refusal::GroupInfo(format!(
            "not a GroupInfo of the channel with ratchet_tree and external_pub that verifies: {err}"
        ))
    })?;
    let uploaded = Saved::of(&uploaded);
    let failed = |err| Refusal::Database("keep a GroupInfo", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    let group = locked(&mut tx, channel)
        .await
        .map_err(failed)?
        .ok_or_else(|| Refusal::GroupInfoConflict("the channel has no group yet".to_owned()))?;
    // The uploader's devices that the GroupInfo lists, with a certificate: a revoked one has none.
    let uploaders: Vec<(String,)> = sqlx::query_as(
        "SELECT id::text FROM devices
         WHERE account_id = $1::uuid AND id = ANY($2::text[]::uuid[]) AND cert IS NOT NULL
           AND NOT revoked AND ($3::uuid IS NULL OR id = $3::uuid)",
    )
    .bind(&account)
    .bind(texts(&uploaded.members))
    .bind(only)
    .fetch_all(&mut *tx)
    .await
    .map_err(failed)?;
    let uploaders: Vec<DeviceId> = (uploaders.iter())
        .map(|(device,)| device.parse().expect("a stored device ID"))
        .collect();
    let starts = match &group.view {
        None => {
            if uploaded.epoch != 1 {
                return Err(Refusal::GroupInfoConflict(
                    "the channel's first GroupInfo is of epoch 1".to_owned(),
                ));
            }
            if !group
                .creator
                .is_some_and(|creator| uploaders.contains(&creator))
            {
                return Err(Refusal::NotAllowed(
                    "only the device of the channel's first Commit uploads its first GroupInfo"
                        .to_owned(),
                ));
            }
            true
        }
        Some(saved) => {
            let view = ChannelView::load(channel, saved).expect("a saved view loads");
            if !view.has_context(&group_info) {
                return Err(Refusal::GroupInfoConflict(
                    "not of the group the server follows, in its current epoch".to_owned(),
                ));
            }
            if uploaders.is_empty() {
                return Err(Refusal::NotAllowed(
                    "the GroupInfo lists no device of the session's account that is not revoked"
                        .to_owned(),
                ));
            }
            false
        }
    };
    sqlx::query(
        "UPDATE channel_groups SET group_info = $2,
             view = CASE WHEN $3 THEN $4 ELSE view END,
             members = CASE WHEN $3 THEN $5::text[]::uuid[] ELSE members END
         WHERE channel_id = $1::uuid",
    )
    .bind(channel.to_string())
    .bind(&group_info)
    .bind(starts)
    .bind(&uploaded.view)
    .bind(texts(&uploaded.members))
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    if starts {
        keep_epoch(&mut tx, channel, &uploaded)
            .await
            .map_err(failed)?;
    }
    tx.commit().await.map_err(failed)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use skald_proto::{MessageId, NonEmpty, PROTOCOL_VERSION, Scheme, SpaceId, Unknown};

    use super::*;

    /// The header of an `mls` envelope of `channel` to `recipients`.
    fn header(channel: ChannelId, recipients: &[DeviceId]) -> Header {
        Header {
            v: PROTOCOL_VERSION,
            space: SpaceId::generate(),
            channel,
            msg_id: MessageId::generate(),
            scheme: Scheme::Mls,
            recipients: NonEmpty::from_vec(recipients.to_vec()),
            ttl: None,
            unknown: Unknown::default(),
        }
    }

    /// A message of `group`.
    fn message(group: &[u8], content: MlsContent) -> MlsFraming {
        MlsFraming::Message {
            group: group.to_vec(),
            epoch: 1,
            public: content != MlsContent::Application,
            content,
        }
    }

    #[test]
    fn a_welcome_goes_to_recipients_and_a_message_of_the_channels_group_to_the_channel() {
        let channel = ChannelId::generate();
        let device = DeviceId::from_signing_key(&[1; 32]);
        let own = channel.as_bytes();
        let other = ChannelId::generate();
        let cases = [
            (MlsFraming::Welcome, vec![device], Ok(())),
            (MlsFraming::Welcome, vec![], Err(Problem::UnknownRecipient)),
            (MlsFraming::Other, vec![], Ok(())),
        ]
        .into_iter()
        .chain(
            [
                MlsContent::Application,
                MlsContent::Proposal,
                MlsContent::Commit,
            ]
            .into_iter()
            .flat_map(|content| {
                [
                    (message(own, content), vec![], Ok(())),
                    (
                        message(own, content),
                        vec![device],
                        Err(Problem::UnknownRecipient),
                    ),
                    (
                        message(other.as_bytes(), content),
                        vec![],
                        Err(Problem::NotAMember),
                    ),
                    (
                        message(&own[..15], content),
                        vec![],
                        Err(Problem::NotAMember),
                    ),
                ]
            }),
        );
        for (framing, recipients, expected) in cases {
            let found = addressed(&header(channel, &recipients), Some(framing.clone()));
            assert_eq!(found.map(|_| ()), expected, "{framing:?} to {recipients:?}");
        }
        let none = addressed(&header(channel, &[]), None);
        assert_eq!(none, Err(Problem::NotAMember), "no MLS message");
    }
}
