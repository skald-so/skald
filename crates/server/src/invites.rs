//! Invites and their acceptances (`docs/protocol/membership.md`, "Invite"): the server issues
//! invite IDs and stores invites for the space's admins, gives an invite by its ID to any session
//! until it has an acceptance and then to that acceptance's account only, keeps the first
//! acceptance, and gives the waiting acceptances to the space's admins. An invite ends when it
//! expires or a stored revision lists a member with it: the server then deletes it with its
//! acceptance.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use skald_proto::{Acceptance, Cbor as _, Invite, InviteId, SpaceId};
use sqlx::{PgConnection, PgPool};

use crate::devices::{Devices, base64, bytes, taken};
use crate::instances::{self, Event};
use crate::refusal::{Answer, Refusal, SignedIn, parse};
use crate::spaces::{self, Found};
use crate::sweeps::{self, PAGE, PAUSE};
use crate::{AppState, mailboxes, revocations};

/// The latest `expires` of an invite, after the server stores it, and how long an issued ID waits
/// for its invite.
const LONGEST: Duration = Duration::from_secs(30 * 24 * 3_600);

/// `NewInvite` in the spec.
#[derive(Serialize)]
struct NewInvite {
    invite: String,
    /// The server's web address, which the invite's link starts with.
    #[serde(skip_serializing_if = "Option::is_none")]
    web: Option<String>,
}

/// `InviteRequest` in the spec, and `StoredInvite`.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InviteBytes {
    invite: String,
}

/// `AcceptanceRequest` in the spec, and `StoredAcceptance`.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceBytes {
    acceptance: String,
}

/// `InviteWithSpace` in the spec.
#[derive(Serialize)]
struct WithSpace {
    invite: String,
    revisions: Vec<String>,
    #[serde(flatten)]
    devices: Devices,
}

/// `WaitingAcceptance` in the spec.
#[derive(Serialize)]
struct Waiting {
    acceptance: String,
    #[serde(flatten)]
    devices: Devices,
}

/// `WaitingAcceptances` in the spec.
#[derive(Serialize)]
struct WaitingAcceptances {
    acceptances: Vec<Waiting>,
}

/// The space of the request's path, which the session's account gets as an admin in its latest
/// revision, who `does` what it asks.
async fn as_admin(db: &PgPool, space: &str, account: &str, does: &str) -> Result<Found, Refusal> {
    let found = spaces::find(db, space, account).await?;
    if !found.gets {
        return Err(Refusal::NoSpace);
    }
    if !found.admin {
        return Err(Refusal::NotAllowed(format!(
            "only an admin in the space's latest revision {does}"
        )));
    }
    Ok(found)
}

/// `POST /spaces/<space>/invites`: a new invite ID for the space, for an admin's session, with the
/// server's web address.
pub(crate) async fn issue(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(space): Path<String>,
) -> Answer {
    let found = as_admin(&state.db, &space, &account, "gets an invite ID").await?;
    let invite = InviteId::generate();
    sqlx::query(
        "INSERT INTO invites (id, space_id, expires_at)
         VALUES ($1::uuid, $2::uuid, now() + make_interval(secs => $3))",
    )
    .bind(invite.to_string())
    .bind(found.id.to_string())
    .bind(LONGEST.as_secs_f64())
    .execute(&state.db)
    .await
    .map_err(|err| Refusal::Database("issue an invite ID", err))?;
    taken(
        true,
        NewInvite {
            invite: invite.to_string(),
            web: (state.sign_in.as_deref()).map(|settings| settings.web().origin()),
        },
    )
}

/// The answer to `sent` under an ID that holds `stored`: taken if it is the same bytes, a retry
/// after a lost answer.
fn invite_again(sent: &[u8], stored: &[u8]) -> Answer {
    if sent != stored {
        return Err(Refusal::InviteTaken);
    }
    taken(
        false,
        InviteBytes {
            invite: base64(stored),
        },
    )
}

/// The invite stored under `id` in `space` while it has not ended: `Some(None)` for an issued ID
/// with no invite yet, `None` for no such ID.
async fn stored_invite(
    db: &PgPool,
    space: SpaceId,
    id: InviteId,
) -> Result<Option<Option<Vec<u8>>>, Refusal> {
    let found: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
        "SELECT invite FROM invites
         WHERE id = $1::uuid AND space_id = $2::uuid AND expires_at > now()",
    )
    .bind(id.to_string())
    .bind(space.to_string())
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read an invite", err))?;
    Ok(found.map(|(invite,)| invite))
}

/// `PUT /spaces/<space>/invites/<invite>`: stores the invite under that ID, issued for the space,
/// from an admin's session; the same bytes again are taken as stored.
pub(crate) async fn store(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path((space, invite)): Path<(String, String)>,
    body: Bytes,
) -> Answer {
    let db = &state.db;
    let found = as_admin(db, &space, &account, "stores an invite").await?;
    let unknown = || {
        Refusal::Invite(
            "the server issued no such invite ID for this space, or it ended".to_owned(),
        )
    };
    let id: InviteId = invite.parse().map_err(|_| unknown())?;
    let request: InviteBytes = parse(&body)?;
    let sent = bytes(&request.invite, "invite")?;
    match stored_invite(db, found.id, id).await? {
        None => return Err(unknown()),
        Some(Some(stored)) => return invite_again(&sent, &stored),
        Some(None) => {}
    }
    let invite =
        Invite::from_cbor(&sent).map_err(|err| Refusal::Invite(format!("not an invite: {err}")))?;
    check_invite(db, &found, id, &invite, &account).await?;
    // Of two stores at once, the second waits for the first to commit, and updates nothing.
    let updated = sqlx::query(
        "UPDATE invites SET invite = $1, expires_at = to_timestamp($2::bigint)
         WHERE id = $3::uuid AND invite IS NULL AND expires_at > now()",
    )
    .bind(&sent)
    .bind(i64::try_from(invite.body().expires).expect("an expiry within 30 days fits"))
    .bind(id.to_string())
    .execute(db)
    .await
    .map_err(|err| Refusal::Database("store an invite", err))?;
    if updated.rows_affected() == 0 {
        return match stored_invite(db, found.id, id).await? {
            Some(Some(stored)) => invite_again(&sent, &stored),
            _ => Err(unknown()),
        };
    }
    taken(
        true,
        InviteBytes {
            invite: base64(&sent),
        },
    )
}

/// Checks `invite`, to be stored under `id` in the space `found` from a session of `account`: of
/// that ID and space, signed by a `client` device of the account with a certificate, not revoked,
/// expiring within [`LONGEST`], naming channels of the latest revision with its default channel
/// first.
async fn check_invite(
    db: &PgPool,
    found: &Found,
    id: InviteId,
    invite: &Invite,
    account: &str,
) -> Result<(), Refusal> {
    let refused = |detail: &str| Refusal::Invite(detail.to_owned());
    let body = invite.body();
    if body.invite != id {
        return Err(refused("the invite's ID is not the one it is stored under"));
    }
    if body.space != found.id {
        return Err(refused("the invite is of another space"));
    }
    let signer: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT signing FROM devices
         WHERE id = $1::uuid AND account_id = $2::uuid AND kind = 'client' AND cert IS NOT NULL
           AND NOT revoked",
    )
    .bind(body.signer.to_string())
    .bind(account)
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a device", err))?;
    let signing: [u8; 32] = signer
        .and_then(|(signing,)| signing.try_into().ok())
        .ok_or_else(|| {
            refused(
                "the signer is no client device of the session's account with a certificate, or \
                 revoked",
            )
        })?;
    skald_crypto::check_invite(invite, &signing)
        .map_err(|_| refused("the signature does not verify with the signer's key"))?;
    // The database's clock decides, as it does when the invite ends.
    let (now,): (i64,) = sqlx::query_as("SELECT floor(extract(epoch FROM now()))::bigint")
        .fetch_one(db)
        .await
        .map_err(|err| Refusal::Database("read the time", err))?;
    let now = u64::try_from(now).expect("the clock is past 1970");
    if body.expires <= now {
        return Err(refused("the invite has expired"));
    }
    if body.expires > now + LONGEST.as_secs() {
        return Err(refused("the invite expires more than 30 days from now"));
    }
    let latest = mailboxes::latest(db, found.id)
        .await
        .map_err(|err| Refusal::Database("read the latest revision", err))?
        .ok_or_else(|| refused("the space has no revision"))?;
    let channels = &latest.body().channels;
    let default = channels.first().map(|channel| channel.id);
    if body.channels.first().map(|(channel, _)| *channel) != default {
        return Err(refused(
            "the invite does not name the latest revision's default channel first",
        ));
    }
    let listed = |id| channels.iter().any(|channel| channel.id == id);
    if !body.channels.iter().all(|(channel, _)| listed(*channel)) {
        return Err(refused(
            "the invite names a channel the latest revision lacks",
        ));
    }
    Ok(())
}

/// The space and the bytes of the invite `id` that the server holds, and the account of its
/// acceptance, if it has one; `None` once it has ended.
async fn held(
    db: impl sqlx::PgExecutor<'_>,
    id: InviteId,
) -> Result<Option<(SpaceId, Vec<u8>, Option<String>)>, Refusal> {
    let found: Option<(String, Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT i.space_id::text, i.invite, a.account_id::text
         FROM invites i LEFT JOIN acceptances a ON a.invite_id = i.id
         WHERE i.id = $1::uuid AND i.invite IS NOT NULL AND i.expires_at > now()",
    )
    .bind(id.to_string())
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read an invite", err))?;
    Ok(found.map(|(space, invite, accepted)| {
        let space = space.parse().expect("a stored space ID");
        (space, invite, accepted)
    }))
}

/// `GET /invites/<invite>`: the invite as the server got it, with its space's revisions and devices
/// as a member gets them; once it has an acceptance, only to that acceptance's account.
pub(crate) async fn given(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(invite): Path<String>,
) -> Answer {
    let db = &state.db;
    let id: InviteId = invite.parse().map_err(|_| Refusal::NoInvite)?;
    let (space, invite, accepted) = held(db, id).await?.ok_or(Refusal::NoInvite)?;
    if accepted.is_some_and(|accepted| accepted != account) {
        return Err(Refusal::NoInvite);
    }
    taken(
        false,
        WithSpace {
            invite: base64(&invite),
            revisions: spaces::stored_revisions(db, space).await?,
            devices: spaces::space_devices(db, space).await?,
        },
    )
}

/// The answer to `sent` for an invite whose acceptance is `stored`: taken if it is the same bytes,
/// a retry after a lost answer.
fn acceptance_again(sent: &[u8], stored: &[u8]) -> Answer {
    if sent != stored {
        return Err(Refusal::AcceptanceTaken);
    }
    taken(
        false,
        AcceptanceBytes {
            acceptance: base64(stored),
        },
    )
}

/// `PUT /invites/<invite>/acceptance`: stores the invite's acceptance from a session of the account
/// it names, with the master key the server holds for that account, signed by the invite's key,
/// before the invite expires; the same bytes again are taken as stored. Of two at once, one is
/// kept. Once stored, the space's admins get a notice of the space, on every instance.
pub(crate) async fn accept(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(invite): Path<String>,
    body: Bytes,
) -> Answer {
    let db = &state.db;
    let id: InviteId = invite.parse().map_err(|_| Refusal::NoInvite)?;
    let request: AcceptanceBytes = parse(&body)?;
    let sent = bytes(&request.acceptance, "acceptance")?;
    let refused = |detail: &str| Refusal::Acceptance(detail.to_owned());
    let acceptance = Acceptance::from_cbor(&sent)
        .map_err(|err| Refusal::Acceptance(format!("not an acceptance: {err}")))?;
    let body = acceptance.body();
    if body.invite != id {
        return Err(refused("the acceptance is of another invite"));
    }
    if body.account.to_string() != account {
        return Err(Refusal::NotAllowed(
            "an acceptance comes only from a session of the account it names".to_owned(),
        ));
    }
    let failed = |err| Refusal::Database("store an acceptance", err);
    let mut tx = db.begin().await.map_err(failed)?;
    // The lock makes the invite's deletion wait for the acceptance, or the acceptance for it.
    let found: Option<(String, Vec<u8>, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT i.space_id::text, i.invite, a.acceptance
         FROM invites i LEFT JOIN acceptances a ON a.invite_id = i.id
         WHERE i.id = $1::uuid AND i.invite IS NOT NULL AND i.expires_at > now()
         FOR KEY SHARE OF i",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *tx)
    .await
    .map_err(failed)?;
    let (space, invite, stored) = found.ok_or(Refusal::NoInvite)?;
    if let Some(stored) = stored {
        return acceptance_again(&sent, &stored);
    }
    let invite = Invite::from_cbor(&invite).expect("a stored invite decodes");
    let master: Option<(Option<Vec<u8>>,)> =
        sqlx::query_as("SELECT master FROM accounts WHERE id = $1::uuid")
            .bind(&account)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failed)?;
    if master.and_then(|(master,)| master).as_deref() != Some(&body.master[..]) {
        return Err(refused(
            "the master key is not the one the server holds for the account",
        ));
    }
    skald_crypto::check_acceptance(&acceptance, &invite.body().key)
        .map_err(|_| refused("the signature does not verify with the invite's key"))?;
    // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
    let inserted = sqlx::query(
        "INSERT INTO acceptances (invite_id, account_id, acceptance)
         VALUES ($1::uuid, $2::uuid, $3) ON CONFLICT DO NOTHING",
    )
    .bind(id.to_string())
    .bind(&account)
    .bind(&sent)
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    if inserted.rows_affected() == 0 {
        drop(tx);
        let stored: Option<(Vec<u8>,)> =
            sqlx::query_as("SELECT acceptance FROM acceptances WHERE invite_id = $1::uuid")
                .bind(id.to_string())
                .fetch_optional(db)
                .await
                .map_err(failed)?;
        return match stored {
            Some((stored,)) => acceptance_again(&sent, &stored),
            None => Err(Refusal::NoInvite),
        };
    }
    let space: SpaceId = space.parse().expect("a stored space ID");
    let (count,): (i64,) = sqlx::query_as(
        "UPDATE spaces SET acceptances = acceptances + 1 WHERE id = $1::uuid
         RETURNING acceptances",
    )
    .bind(space.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(failed)?;
    state
        .instance
        .tell(&mut tx, &[Event::Acceptance(space, count)])
        .await
        .map_err(|err| Refusal::Database("tell the other instances of an acceptance", err))?;
    tx.commit().await.map_err(failed)?;
    state.instance.record_acceptances(space, count);
    let admins = instances::latest_members(db, space, true)
        .await
        .map_err(|err| Refusal::Database("read the space's admins", err))?;
    state.notices.notify(admins, space);
    taken(
        true,
        AcceptanceBytes {
            acceptance: base64(&sent),
        },
    )
}

/// `GET /spaces/<space>/acceptances`: the space's waiting acceptances, oldest first, each with its
/// account's devices and revocations, for an admin's session.
pub(crate) async fn waiting(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(space): Path<String>,
) -> Answer {
    let db = &state.db;
    let found = as_admin(db, &space, &account, "gets the waiting acceptances").await?;
    let stored: Vec<(Vec<u8>, String)> = sqlx::query_as(
        "SELECT a.acceptance, a.account_id::text
         FROM acceptances a JOIN invites i ON i.id = a.invite_id
         WHERE i.space_id = $1::uuid AND i.expires_at > now()
         ORDER BY a.stored_at, a.invite_id",
    )
    .bind(found.id.to_string())
    .fetch_all(db)
    .await
    .map_err(|err| Refusal::Database("read the waiting acceptances", err))?;
    let mut acceptances = Vec::with_capacity(stored.len());
    for (acceptance, account) in stored {
        let devices = sqlx::query_as(
            "SELECT id::text, cert FROM devices WHERE account_id = $1::uuid
             ORDER BY registered_at, id",
        )
        .bind(&account)
        .fetch_all(db)
        .await
        .map_err(|err| Refusal::Database("read the devices", err))?;
        let revocations = revocations::of_account(db, &account).await?;
        acceptances.push(Waiting {
            acceptance: base64(&acceptance),
            devices: Devices::of(devices, &revocations),
        });
    }
    taken(false, WaitingAcceptances { acceptances })
}

/// Whether `account` has the acceptance of an invite of the space `space`, a path parameter, that
/// has not ended: it then gets the space's revisions and devices as a member does.
pub(crate) async fn accepted(
    db: &PgPool,
    space: &str,
    account: &str,
) -> Result<Option<SpaceId>, Refusal> {
    let Ok(space) = space.parse::<SpaceId>() else {
        return Ok(None);
    };
    let (found,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (
             SELECT 1 FROM acceptances a JOIN invites i ON i.id = a.invite_id
             WHERE i.space_id = $1::uuid AND a.account_id = $2::uuid AND i.expires_at > now())",
    )
    .bind(space.to_string())
    .bind(account)
    .fetch_one(db)
    .await
    .map_err(|err| Refusal::Database("read the account's acceptances", err))?;
    Ok(found.then_some(space))
}

/// Deletes the invites of `space` that `invites`, the invites a stored revision lists its members
/// with, name, with their acceptances, on `conn`.
pub(crate) async fn admitted(
    conn: &mut PgConnection,
    space: SpaceId,
    invites: &[String],
) -> Result<(), sqlx::Error> {
    if invites.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM invites WHERE space_id = $1::uuid AND id = ANY($2::text[]::uuid[])")
        .bind(space.to_string())
        .bind(invites)
        .execute(conn)
        .await?;
    Ok(())
}

/// Looks for invites past their expiry, as [`sweeps::every`] says.
pub(crate) async fn sweep(db: PgPool) {
    sweeps::every("delete expired invites", || {
        let db = db.clone();
        async move { delete_expired(&db).await }
    })
    .await;
}

/// Deletes the expired invites with their acceptances, [`PAGE`] at a time.
async fn delete_expired(db: &PgPool) -> Result<(), sqlx::Error> {
    loop {
        let deleted = sqlx::query(
            "DELETE FROM invites WHERE id IN (
                 SELECT id FROM invites WHERE expires_at <= now() LIMIT $1)",
        )
        .bind(PAGE)
        .execute(db)
        .await?;
        if deleted.rows_affected() == 0 {
            return Ok(());
        }
        // Leaves the database to other work between pages.
        tokio::time::sleep(PAUSE).await;
    }
}
