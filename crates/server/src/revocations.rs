//! Revocations (`docs/protocol/membership.md`, "Revocation"): the server stores a revocation from a
//! session of its account only if it counts against the account's master key, the certificates the
//! server holds and the revocations it stored before, and gives each account's revocations with
//! its devices. A device is revoked when checks 3 and 4 of "Device checks" fail with the stored
//! revocations; checks 1 and 2 were made when the server took each certificate. At revocation, on
//! every instance, the server expires each mailbox entry of the device ("Mailbox"), closes its
//! connections, deletes its presence values and adds it no later entry.

use axum::body::Bytes;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use skald_proto::{AccountId, Cbor as _, DeviceId, Revocation, ServerStatus, SpaceId};
use sqlx::{PgConnection, PgPool};

use crate::delivery::{Statuses, remove_on, removing};
use crate::devices::{base64, bytes, taken};
use crate::instances::Event;
use crate::refusal::{Answer, Refusal, SignedIn, parse};
use crate::spaces::certs;
use crate::{AppState, presence};

/// `RevocationRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevocationRequest {
    revocation: String,
}

/// `StoredRevocation` in the spec.
#[derive(Serialize)]
struct StoredRevocation {
    revocation: String,
}

/// The stored revocations of `account`, as the server got them, in the order it stored them.
pub(crate) async fn of_account(db: &PgPool, account: &str) -> Result<Vec<(Vec<u8>,)>, Refusal> {
    sqlx::query_as("SELECT revocation FROM revocations WHERE account_id = $1::uuid ORDER BY id")
        .bind(account)
        .fetch_all(db)
        .await
        .map_err(|err| Refusal::Database("read the revocations", err))
}

/// The stored revocations of the accounts the revisions of `space` list, as the server got them.
pub(crate) async fn of_space(db: &PgPool, space: SpaceId) -> Result<Vec<(Vec<u8>,)>, Refusal> {
    sqlx::query_as(
        "SELECT revocation FROM revocations
         WHERE account_id IN (SELECT account_id FROM roster_members WHERE space_id = $1::uuid)
         ORDER BY id",
    )
    .bind(space.to_string())
    .fetch_all(db)
    .await
    .map_err(|err| Refusal::Database("read the space's revocations", err))
}

/// `POST /account/revocations`: stores a revocation of the session's account that counts; the same
/// bytes again are taken as stored. Each device it revokes has its mailbox entries expired and its
/// connections closed, on every instance.
pub(crate) async fn store(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let request: RevocationRequest = parse(&body)?;
    let sent = bytes(&request.revocation, "revocation")?;
    let refused = |detail: &str| Refusal::Revocation(detail.to_owned());
    let revocation = Revocation::from_cbor(&sent)
        .map_err(|err| Refusal::Revocation(format!("not a revocation: {err}")))?;
    let answer = StoredRevocation {
        revocation: base64(&sent),
    };
    let failed = |err| Refusal::Database("store a revocation", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    // The account's lock makes its revocations wait for each other, so that each counts against
    // those stored before it.
    let (master,): (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT master FROM accounts WHERE id = $1::uuid FOR NO KEY UPDATE")
            .bind(&account)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failed)?
            .ok_or(Refusal::NotSignedIn)?;
    let stored: Vec<(Vec<u8>,)> =
        sqlx::query_as("SELECT revocation FROM revocations WHERE account_id = $1::uuid")
            .bind(&account)
            .fetch_all(&mut *tx)
            .await
            .map_err(failed)?;
    if stored.iter().any(|(bytes,)| *bytes == sent) {
        return taken(false, answer);
    }
    let id: AccountId = account.parse().expect("a stored account ID");
    if revocation.body().account != id {
        return Err(refused("the revocation is of another account"));
    }
    let master: [u8; 32] = master
        .and_then(|master| master.try_into().ok())
        .ok_or_else(|| refused("the account has no master key"))?;
    let certs = certs(&mut *tx, std::slice::from_ref(&account)).await?;
    let mut revocations: Vec<Revocation> = stored
        .iter()
        .map(|(bytes,)| Revocation::from_cbor(bytes).expect("a stored revocation decodes"))
        .collect();
    if !skald_crypto::counts(&master, &certs, &revocations, &revocation) {
        return Err(refused(
            "the revocation does not count: its signature does not verify with the key `signer` \
             names, `signer` is no `client` or `browser` device of the account with a valid \
             certificate, a device's revocation has `also`, or a master-key revocation voids it",
        ));
    }
    sqlx::query(
        "INSERT INTO revocations (account_id, revocation, digest) VALUES ($1::uuid, $2, $3)",
    )
    .bind(&account)
    .bind(&sent)
    .bind(Sha256::digest(&sent).to_vec())
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    revocations.push(revocation);
    let revoked = skald_crypto::revoked_devices(&master, &certs, &revocations, id);
    let (newly, statuses) = mark(&mut tx, &state, &account, &revoked)
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    statuses.woken(&state.wakers);
    for device in newly {
        state.connections.end_device(device);
    }
    taken(true, answer)
}

/// Marks the devices of `account` in `revoked` as revoked and its others as not, on `conn`, in its
/// transaction. Takes from each device it newly revokes what [`withdraw`] takes, and tells the
/// other instances to close its connections once the transaction commits. Gives those devices, and
/// the statuses the expiry added.
async fn mark(
    conn: &mut PgConnection,
    state: &AppState,
    account: &str,
    revoked: &[DeviceId],
) -> Result<(Vec<DeviceId>, Statuses), sqlx::Error> {
    let revoked: Vec<String> = revoked.iter().map(ToString::to_string).collect();
    // The rows' locks make a store to the mailbox of a device it revokes wait for this transaction,
    // or this one wait for the store, whose entry it then expires.
    let changed: Vec<(String, bool)> = sqlx::query_as(
        "UPDATE devices SET revoked = (id = ANY($2::text[]::uuid[]))
         WHERE account_id = $1::uuid AND revoked <> (id = ANY($2::text[]::uuid[]))
         RETURNING id::text, revoked",
    )
    .bind(account)
    .bind(&revoked)
    .fetch_all(&mut *conn)
    .await?;
    let newly: Vec<String> = changed
        .into_iter()
        .filter_map(|(device, revoked)| revoked.then_some(device))
        .collect();
    let statuses = withdraw(conn, state, &newly).await?;
    let newly: Vec<DeviceId> = newly
        .iter()
        .map(|device| device.parse().expect("a stored device ID"))
        .collect();
    let events: Vec<Event> = newly.iter().copied().map(Event::Revoked).collect();
    state.instance.tell(conn, &events).await?;
    Ok((newly, statuses))
}

/// Takes from `devices`, newly revoked, what the server serves only to devices that are not, on
/// `conn`, in its transaction: expires every mailbox entry of each, and deletes its KeyPackages and
/// presence values. Gives the statuses the expiry added.
pub(crate) async fn withdraw(
    conn: &mut PgConnection,
    state: &AppState,
    devices: &[String],
) -> Result<Statuses, sqlx::Error> {
    let query = sqlx::query_as(removing!(
        "DELETE FROM entries WHERE device_id = ANY($1::text[]::uuid[])
         RETURNING device_id, seq, envelope_id, now() AS at"
    ))
    .bind(devices);
    let statuses = remove_on(conn, &state.wakers, query, ServerStatus::Expired).await?;
    sqlx::query("DELETE FROM key_packages WHERE device_id = ANY($1::text[]::uuid[])")
        .bind(devices)
        .execute(&mut *conn)
        .await?;
    presence::revoked(conn, devices).await?;
    Ok(statuses)
}
