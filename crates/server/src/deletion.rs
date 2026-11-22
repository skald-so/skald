//! Account deletion: a session whose sign-in is recent deletes its account, never while the account
//! is the only admin of a space with other members. A space's other members are the accounts of its
//! latest revision but the account itself and those already deleted. At once, on every instance,
//! the account loses its sessions, its service tokens, its and its devices' link runs, approved
//! device codes and sign-in methods, and its username; each of its devices is served as a revoked
//! one, as is a device a certificate names on it later; each space it alone is in is deleted; each
//! space it shares keeps it in its revisions, and its members' connections get a `space-notice`.
//! Nothing undoes a deletion.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::Serialize;
use skald_proto::{AccountId, DeviceId, SpaceId};
use sqlx::PgConnection;

use crate::instances::{Event, latest_members};
use crate::refusal::{Answer, Refusal, SignedIn};
use crate::sign_in::{bearer, signed_in};
use crate::{AppState, revocations, sessions};

/// A row lock on an account that a request takes before it acts for the account.
#[derive(Clone, Copy)]
pub(crate) enum Lock {
    Update,
    NoKeyUpdate,
    KeyShare,
}

/// Takes `lock` on the row of `account` on `conn`, in its transaction, and gives whether the
/// account is deleted then. A deletion holds the row's lock until it commits, so a request that
/// takes it first is one the deletion waits for. An account the server does not hold is no deleted
/// one.
pub(crate) async fn deleted(
    conn: &mut PgConnection,
    account: &str,
    lock: Lock,
) -> Result<bool, sqlx::Error> {
    let sql = match lock {
        Lock::Update => {
            "SELECT deleted_at IS NOT NULL FROM accounts WHERE id = $1::uuid FOR UPDATE"
        }
        Lock::NoKeyUpdate => {
            "SELECT deleted_at IS NOT NULL FROM accounts WHERE id = $1::uuid FOR NO KEY UPDATE"
        }
        Lock::KeyShare => {
            "SELECT deleted_at IS NOT NULL FROM accounts WHERE id = $1::uuid FOR KEY SHARE"
        }
    };
    let found: Option<(bool,)> = sqlx::query_as(sql)
        .bind(account)
        .fetch_optional(conn)
        .await?;
    Ok(found.is_some_and(|(deleted,)| deleted))
}

/// A space a deletion deletes or that blocks it: `DeletionSpace` in the spec.
#[derive(Serialize)]
pub(crate) struct Listed {
    space: String,
    name: String,
}

/// `AccountDeletion` in the spec.
#[derive(Serialize)]
struct Preview {
    deletes: Vec<Listed>,
    blocks: Vec<Listed>,
}

/// What a deletion of an account does to the spaces it gets.
struct Spaces {
    /// Those where it is the only member, which it deletes.
    deletes: Vec<Listed>,
    /// Those where it is the only admin and that have other members, which block it.
    blocks: Vec<Listed>,
    /// Those with other members that do not block it, which stay.
    stay: Vec<SpaceId>,
}

/// What a deletion of `account` does to the spaces it gets, as `conn` reads them now.
async fn spaces(conn: &mut PgConnection, account: &str) -> Result<Spaces, sqlx::Error> {
    // An account this server does not hold is no deleted one.
    let found: Vec<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT s.id::text, s.name,
             EXISTS (SELECT 1 FROM space_access o LEFT JOIN accounts x ON x.id = o.account_id
                     WHERE o.space_id = s.id AND o.account_id <> $1::uuid
                         AND x.deleted_at IS NULL),
             a.admin AND NOT EXISTS (
                 SELECT 1 FROM space_access o LEFT JOIN accounts x ON x.id = o.account_id
                 WHERE o.space_id = s.id AND o.account_id <> $1::uuid AND o.admin
                     AND x.deleted_at IS NULL)
         FROM space_access a JOIN spaces s ON s.id = a.space_id
         WHERE a.account_id = $1::uuid ORDER BY s.created_at, s.id",
    )
    .bind(account)
    .fetch_all(conn)
    .await?;
    let mut spaces = Spaces {
        deletes: Vec::new(),
        blocks: Vec::new(),
        stay: Vec::new(),
    };
    for (space, name, shared, only_admin) in found {
        match (shared, only_admin) {
            (false, _) => spaces.deletes.push(Listed { space, name }),
            (true, true) => spaces.blocks.push(Listed { space, name }),
            (true, false) => spaces.stay.push(space.parse().expect("a stored space ID")),
        }
    }
    Ok(spaces)
}

/// `GET /account/deletion`: the spaces a deletion of the session's account deletes and those that
/// block it.
pub(crate) async fn preview(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    let failed = |err| Refusal::Database("read what a deletion deletes", err);
    let mut conn = state.db.acquire().await.map_err(failed)?;
    let Spaces {
        deletes, blocks, ..
    } = spaces(&mut conn, &account).await.map_err(failed)?;
    Ok(Json(Preview { deletes, blocks }).into_response())
}

/// `DELETE /account`: deletes the account of the request's session: a device session, or a web
/// session from a page of the web address, whose sign-in is more recent than the server allows,
/// unless a space blocks it.
pub(crate) async fn delete(State(state): State<AppState>, headers: HeaderMap) -> Answer {
    let session = match bearer(&headers) {
        Some(_) => signed_in(&state.db, &headers)
            .await
            .map_err(|err| Refusal::Database("read a session", err))?
            .ok_or(Refusal::NotSignedIn)?,
        None => sessions::web_session_from_page(&state, &headers, "deletes the account").await?,
    };
    let settings = state.sign_in.as_deref().expect("a session signs in");
    let failed = |err| Refusal::Database("delete an account", err);
    let account = session.account.as_str();
    let mut tx = state.db.begin().await.map_err(failed)?;
    // The spaces' locks make a revision of one of them wait for the deletion, or the deletion
    // for the revision, which it then sees; a revision takes the space's lock first, then its
    // members' accounts', as the deletion does.
    let mut locked: Vec<String> = Vec::new();
    loop {
        let gotten: Vec<(String,)> =
            sqlx::query_as("SELECT space_id::text FROM space_access WHERE account_id = $1::uuid")
                .bind(account)
                .fetch_all(&mut *tx)
                .await
                .map_err(failed)?;
        let new: Vec<String> = (gotten.into_iter())
            .map(|(space,)| space)
            .filter(|space| !locked.contains(space))
            .collect();
        if new.is_empty() {
            break;
        }
        sqlx::query(
            "SELECT 1 FROM spaces WHERE id = ANY($1::text[]::uuid[]) ORDER BY id
             FOR NO KEY UPDATE",
        )
        .bind(&new)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
        locked.extend(new);
    }
    let live: Option<(bool,)> =
        sqlx::query_as("SELECT deleted_at IS NULL FROM accounts WHERE id = $1::uuid FOR UPDATE")
            .bind(account)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failed)?;
    if live != Some((true,)) {
        return Err(Refusal::NotSignedIn);
    }
    sessions::recent(&mut tx, &session, settings.link_age, "delete an account").await?;
    let spaces = spaces(&mut tx, account).await.map_err(failed)?;
    if !spaces.blocks.is_empty() {
        return Err(Refusal::DeletionBlocked(spaces.blocks));
    }
    let deleted = erase(&mut tx, &state, account, &spaces)
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    deleted.statuses.woken(&state.wakers);
    for device in deleted.devices {
        state.connections.end_deleted(device);
    }
    for (space, deletions, members) in deleted.noticed {
        state.instance.record_deletions(space, deletions);
        state.notices.notify(members, space);
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// What a deletion leaves to do once it commits, on this instance.
struct Deleted {
    /// The statuses the expiry of its devices' mailbox entries added.
    statuses: crate::delivery::Statuses,
    /// Its devices, whose connections close.
    devices: Vec<DeviceId>,
    /// The spaces it shared, each with its count of member deletions and the accounts of its latest
    /// revision, whose connections get a notice of it.
    noticed: Vec<(SpaceId, i64, Vec<AccountId>)>,
}

/// Deletes `account`, whose `spaces` none blocks, on `conn`, in its transaction, and tells the
/// other instances to close its devices' connections and send the `space-notice` of each space it
/// shared once the transaction commits.
async fn erase(
    conn: &mut PgConnection,
    state: &AppState,
    account: &str,
    spaces: &Spaces,
) -> Result<Deleted, sqlx::Error> {
    // A poll of an approved code then refuses, or has given a session that ends below.
    sqlx::query(
        "UPDATE device_codes SET state = 'denied'
         WHERE account_id = $1::uuid AND state = 'approved'",
    )
    .bind(account)
    .execute(&mut *conn)
    .await?;
    // The methods' locks make a sign-in with one of them wait, then sign up anew.
    sqlx::query("SELECT 1 FROM sign_in_methods WHERE account_id = $1::uuid FOR UPDATE")
        .bind(account)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE account_id = $1::uuid")
        .bind(account)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM sign_in_methods WHERE account_id = $1::uuid")
        .bind(account)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM service_tokens
         WHERE device_id IN (SELECT id FROM devices WHERE account_id = $1::uuid)",
    )
    .bind(account)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE accounts SET deleted_at = now(), username = NULL WHERE id = $1::uuid")
        .bind(account)
        .execute(&mut *conn)
        .await?;
    // The rows' locks make a store to the mailbox of one of its devices wait for the deletion, or
    // the deletion wait for the store, whose entry it then expires.
    let newly: Vec<(String,)> = sqlx::query_as(
        "UPDATE devices SET revoked = true WHERE account_id = $1::uuid AND NOT revoked
         RETURNING id::text",
    )
    .bind(account)
    .fetch_all(&mut *conn)
    .await?;
    let newly: Vec<String> = newly.into_iter().map(|(device,)| device).collect();
    // Its runs and those of its devices, after the rows' locks: a run without a session that a
    // device opened meanwhile is then seen, and its devices, revoked, open none later.
    sqlx::query(
        "DELETE FROM link_runs
         WHERE account_id = $1::uuid
             OR device_id IN (SELECT id FROM devices WHERE account_id = $1::uuid)",
    )
    .bind(account)
    .execute(&mut *conn)
    .await?;
    let statuses = revocations::withdraw(conn, state, &newly).await?;
    let deleted: Vec<String> = (spaces.deletes.iter())
        .map(|listed| listed.space.clone())
        .collect();
    sqlx::query("DELETE FROM spaces WHERE id = ANY($1::text[]::uuid[])")
        .bind(&deleted)
        .execute(&mut *conn)
        .await?;
    // A long-poll then answers a notice of each space it shared.
    let shared: Vec<String> = spaces.stay.iter().map(ToString::to_string).collect();
    sqlx::query("DELETE FROM long_poll_revisions WHERE space_id = ANY($1::text[]::uuid[])")
        .bind(&shared)
        .execute(&mut *conn)
        .await?;
    let counted: Vec<(String, i64)> = sqlx::query_as(
        "UPDATE spaces SET member_deletions = member_deletions + 1
         WHERE id = ANY($1::text[]::uuid[]) RETURNING id::text, member_deletions",
    )
    .bind(&shared)
    .fetch_all(&mut *conn)
    .await?;
    let mut noticed = Vec::new();
    for (space, deletions) in counted {
        let space: SpaceId = space.parse().expect("a stored space ID");
        noticed.push((
            space,
            deletions,
            latest_members(&mut *conn, space, false).await?,
        ));
    }
    let devices: Vec<(String,)> =
        sqlx::query_as("SELECT id::text FROM devices WHERE account_id = $1::uuid")
            .bind(account)
            .fetch_all(&mut *conn)
            .await?;
    let devices: Vec<DeviceId> = devices
        .iter()
        .map(|(device,)| device.parse().expect("a stored device ID"))
        .collect();
    let events: Vec<Event> = (devices.iter().copied().map(Event::Deleted))
        .chain(
            noticed
                .iter()
                .map(|(space, deletions, _)| Event::Deletion(*space, *deletions)),
        )
        .collect();
    state.instance.tell(conn, &events).await?;
    Ok(Deleted {
        statuses,
        devices,
        noticed,
    })
}
