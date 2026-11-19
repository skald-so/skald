//! Spaces and their roster revisions (`docs/protocol/membership.md`, "Roster"): a signed-in account
//! creates a space by name, the server stores its revisions in order and only from its admins'
//! sessions, and gives a space only to its members' sessions.

use std::collections::HashMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use skald_proto::{
    AccountId, Cbor as _, Cert, ChannelId, Name, ROSTER_LABEL, Role, Roster, SpaceId,
};
use sqlx::PgPool;

use crate::accounts::GITHUB;
use crate::devices::{Devices, base64, bytes, taken};
use crate::instances::Event;
use crate::refusal::{Answer, Caller, Refusal, SignedIn, parse};
use crate::{AppState, history, mailboxes, revocations, sessions};

/// `SpaceRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceRequest {
    name: String,
}

/// `Space` in the spec.
#[derive(Serialize)]
pub(crate) struct Space {
    pub(crate) space: String,
    pub(crate) name: String,
}

/// `ListedSpace` in the spec.
#[derive(Serialize)]
struct ListedSpace {
    space: String,
    name: String,
    personal: bool,
}

/// `Spaces` in the spec.
#[derive(Serialize)]
struct Spaces {
    spaces: Vec<ListedSpace>,
}

/// `RosterRevisionRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionRequest {
    revision: String,
}

/// `RosterRevisions` in the spec.
#[derive(Serialize)]
struct Revisions {
    revisions: Vec<String>,
}

/// `StoredRosterRevision` in the spec.
#[derive(Serialize)]
struct StoredRevision {
    space: String,
    revision: u64,
}

/// `ListedMember` in the spec.
#[derive(Serialize)]
struct ListedMember {
    handle: String,
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    github_login: Option<String>,
    linked_devices: i64,
    joined_at: String,
}

/// `ListedMembers` in the spec.
#[derive(Serialize)]
struct ListedMembers {
    members: Vec<ListedMember>,
}

/// `NewChannel` in the spec.
#[derive(Serialize)]
struct NewChannel {
    channel: String,
}

/// A space as a session's account finds it.
pub(crate) struct Found {
    pub(crate) id: SpaceId,
    name: Name,
    creator: String,
    /// Whether the account gets the space.
    pub(crate) gets: bool,
    /// Whether the account is an admin in the latest revision.
    pub(crate) admin: bool,
}

/// The space `space`, a path parameter, as the account `account` finds it.
pub(crate) async fn find(db: &PgPool, space: &str, account: &str) -> Result<Found, Refusal> {
    let id: SpaceId = space.parse().map_err(|_| Refusal::NoSpace)?;
    let found: Option<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT s.name, s.creator_id::text, a.account_id IS NOT NULL, COALESCE(a.admin, false)
         FROM spaces s LEFT JOIN space_access a ON a.space_id = s.id AND a.account_id = $1::uuid
         WHERE s.id = $2::uuid",
    )
    .bind(account)
    .bind(id.to_string())
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a space", err))?;
    let (name, creator, gets, admin) = found.ok_or(Refusal::NoSpace)?;
    Ok(Found {
        id,
        name: name.parse().expect("a stored space name is a valid name"),
        creator,
        gets,
        admin,
    })
}

/// The space of the request's path, which the session's account gets, as a handler's argument.
pub(crate) struct Gotten(pub(crate) Found);

impl FromRequestParts<AppState> for Gotten {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Refusal> {
        let SignedIn(account) = SignedIn::from_request_parts(parts, state).await?;
        let Path(space) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| Refusal::NoSpace)?;
        let found = find(&state.db, &space, &account).await?;
        if !found.gets {
            return Err(Refusal::NoSpace);
        }
        Ok(Gotten(found))
    }
}

/// The space of the request's path, which the session's account gets or the request's service
/// token is of, as a handler's argument.
pub(crate) struct Readable(pub(crate) SpaceId);

impl FromRequestParts<AppState> for Readable {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Refusal> {
        let caller = Caller::from_request_parts(parts, state).await?;
        let Path(space) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| Refusal::NoSpace)?;
        let found = match caller {
            Caller::Session(account) => {
                let found = find(&state.db, &space, &account).await?;
                found.gets.then_some(found.id)
            }
            Caller::Token(token) => space.parse().ok().filter(|space| *space == token.space),
        };
        Ok(Readable(found.ok_or(Refusal::NoSpace)?))
    }
}

/// `POST /spaces`: creates a space the session's account owns, unless it owns as many as it may.
/// The same name again gives the account's space while it has no revision, also at the limit.
pub(crate) async fn create(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let request: SpaceRequest = parse(&body)?;
    let name: Name = request
        .name
        .parse()
        .map_err(|err| Refusal::Invalid(format!("name: {err}")))?;
    let failed = |err| Refusal::Database("create a space", err);
    let space = |space| Space {
        space,
        name: name.to_string(),
    };
    let mut tx = state.db.begin().await.map_err(failed)?;
    if let Some(limit) = state.owned_spaces_limit {
        // The account's lock makes its creations wait for each other, so none passes the limit.
        // It lets a request for the personal space insert it meanwhile, which changes no count.
        sqlx::query("SELECT 1 FROM accounts WHERE id = $1::uuid FOR NO KEY UPDATE")
            .bind(&account)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        if let Some(id) = unrevised(&mut tx, &name, &account).await? {
            return taken(false, space(id));
        }
        // The personal space counts whether or not it is created yet.
        let (team,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM spaces WHERE creator_id = $1::uuid AND NOT personal",
        )
        .bind(&account)
        .fetch_one(&mut *tx)
        .await
        .map_err(failed)?;
        if team + 1 >= i64::from(limit) {
            return Err(Refusal::OwnedSpacesLimit(limit));
        }
    }
    // Of two creations with one name at once, the second waits for the first to commit.
    let created: Option<(String,)> = sqlx::query_as(
        "INSERT INTO spaces (id, name, creator_id) VALUES ($1::uuid, $2, $3::uuid)
         ON CONFLICT (name) DO NOTHING RETURNING id::text",
    )
    .bind(SpaceId::generate().to_string())
    .bind(name.as_str())
    .bind(&account)
    .fetch_optional(&mut *tx)
    .await
    .map_err(failed)?;
    if let Some((id,)) = created {
        tx.commit().await.map_err(failed)?;
        return taken(true, space(id));
    }
    let id = unrevised(&mut tx, &name, &account)
        .await?
        .ok_or(Refusal::SpaceNameTaken)?;
    taken(false, space(id))
}

/// The ID of the space named `name` that `account` created and that has no revision yet, if any.
async fn unrevised(
    tx: &mut sqlx::PgConnection,
    name: &Name,
    account: &str,
) -> Result<Option<String>, Refusal> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM spaces s WHERE name = $1 AND creator_id = $2::uuid AND NOT personal
           AND NOT EXISTS (SELECT 1 FROM roster_revisions r WHERE r.space_id = s.id)",
    )
    .bind(name.as_str())
    .bind(account)
    .fetch_optional(tx)
    .await
    .map_err(|err| Refusal::Database("read a space", err))?;
    Ok(found.map(|(id,)| id))
}

/// `POST /account/personal-space`: the personal space of the session's account, which the first
/// request creates, named from the account's username.
pub(crate) async fn personal(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    // Each round inserts the space, finds the one an earlier request inserted, or finds the name
    // taken by a space that another request inserted meanwhile, which the next round sees.
    loop {
        // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
        let created: Option<(String, String)> = sqlx::query_as(
            "INSERT INTO spaces (id, name, creator_id, personal)
             SELECT $1::uuid, free_space_name(username), id, true FROM accounts WHERE id = $2::uuid
             ON CONFLICT DO NOTHING RETURNING id::text, name",
        )
        .bind(SpaceId::generate().to_string())
        .bind(&account)
        .fetch_optional(&state.db)
        .await
        .map_err(|err| Refusal::Database("create the personal space", err))?;
        if let Some((space, name)) = created {
            return taken(true, Space { space, name });
        }
        // No row once the account is gone.
        let found: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT s.id::text, s.name FROM accounts a
             LEFT JOIN spaces s ON s.creator_id = a.id AND s.personal WHERE a.id = $1::uuid",
        )
        .bind(&account)
        .fetch_optional(&state.db)
        .await
        .map_err(|err| Refusal::Database("read the personal space", err))?;
        if let (Some(space), Some(name)) = found.ok_or(Refusal::NotSignedIn)? {
            return taken(false, Space { space, name });
        }
    }
}

/// `GET /spaces`: the spaces the session's account gets, its own personal space marked. It creates
/// none.
pub(crate) async fn list(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    let found: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT s.id::text, s.name, s.personal AND s.creator_id = a.account_id
         FROM space_access a JOIN spaces s ON s.id = a.space_id
         WHERE a.account_id = $1::uuid ORDER BY s.created_at, s.id",
    )
    .bind(&account)
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the spaces", err))?;
    let spaces = found
        .into_iter()
        .map(|(space, name, personal)| ListedSpace {
            space,
            name,
            personal,
        })
        .collect();
    Ok(Json(Spaces { spaces }).into_response())
}

/// `GET /spaces/<space>/revisions`: the space's revisions as stored, in order.
pub(crate) async fn revisions(State(state): State<AppState>, Readable(space): Readable) -> Answer {
    let stored: Vec<(Vec<u8>,)> = sqlx::query_as(
        "SELECT roster FROM roster_revisions WHERE space_id = $1::uuid ORDER BY revision",
    )
    .bind(space.to_string())
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the revisions", err))?;
    let revisions = stored.iter().map(|(roster,)| base64(roster)).collect();
    Ok(Json(Revisions { revisions }).into_response())
}

/// `GET /spaces/<space>/devices`: the devices of the accounts the space's revisions list, and those
/// accounts' revocations.
pub(crate) async fn devices(State(state): State<AppState>, Readable(space): Readable) -> Answer {
    let listed = sqlx::query_as(
        "SELECT id::text, cert FROM devices
         WHERE account_id IN (SELECT account_id FROM roster_members WHERE space_id = $1::uuid)
         ORDER BY registered_at, id",
    )
    .bind(space.to_string())
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the space's devices", err))?;
    let revocations = revocations::of_space(&state.db, space).await?;
    Ok(Json(Devices::of(listed, &revocations)).into_response())
}

/// `GET /spaces/<space>/members`: the members of the space's latest stored revision as the web app
/// shows them, in its order. A member joined when the server stored the first revision of the run
/// up to the latest that lists it.
pub(crate) async fn members(State(state): State<AppState>, Gotten(found): Gotten) -> Answer {
    let space = found.id.to_string();
    let latest = mailboxes::latest(&state.db, found.id)
        .await
        .map_err(|err| Refusal::Database("read the latest revision", err))?;
    let Some(roster) = latest else {
        return Ok(Json(ListedMembers { members: vec![] }).into_response());
    };
    let revision =
        i64::try_from(roster.body().revision).expect("a stored revision's number fits its column");
    // `to_char` gives RFC 3339 in UTC; an account this server does not hold has no login or device.
    let rows: Vec<(String, Option<String>, i64, String)> = sqlx::query_as(
        "SELECT m.account_id::text,
             (SELECT login FROM sign_in_methods
              WHERE account_id = m.account_id AND provider = $3 ORDER BY created_at LIMIT 1),
             (SELECT count(*) FROM devices
              WHERE account_id = m.account_id AND cert IS NOT NULL AND NOT revoked),
             to_char(r.stored_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')
         FROM roster_members m
         JOIN roster_revisions r ON r.space_id = m.space_id AND r.revision = 1 + COALESCE(
             (SELECT max(o.revision) FROM roster_revisions o
              WHERE o.space_id = m.space_id AND o.revision < m.revision
                  AND NOT EXISTS (SELECT 1 FROM roster_members x
                      WHERE x.space_id = o.space_id AND x.revision = o.revision
                          AND x.account_id = m.account_id)),
             0)
         WHERE m.space_id = $1::uuid AND m.revision = $2",
    )
    .bind(&space)
    .bind(revision)
    .bind(GITHUB)
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the space's members", err))?;
    let mut shown: HashMap<String, (Option<String>, i64, String)> = rows
        .into_iter()
        .map(|(account, login, linked, joined)| (account, (login, linked, joined)))
        .collect();
    let members = roster
        .body()
        .members
        .iter()
        .filter_map(|member| {
            let (github_login, linked_devices, joined_at) =
                shown.remove(&member.account.to_string())?;
            Some(ListedMember {
                handle: member.handle.to_string(),
                role: match member.role {
                    Role::Admin => "admin",
                    Role::Member => "member",
                },
                github_login,
                linked_devices,
                joined_at,
            })
        })
        .collect();
    Ok(Json(ListedMembers { members }).into_response())
}

/// `POST /spaces/<space>/channels`: a new channel ID for the space, for an admin's session.
pub(crate) async fn channel(State(state): State<AppState>, Gotten(found): Gotten) -> Answer {
    if !found.admin {
        return Err(Refusal::NotAllowed(
            "only an admin in the space's latest revision gets a channel ID".to_owned(),
        ));
    }
    let channel = ChannelId::generate().to_string();
    sqlx::query("INSERT INTO channels (id, space_id) VALUES ($1::uuid, $2::uuid)")
        .bind(&channel)
        .bind(found.id.to_string())
        .execute(&state.db)
        .await
        .map_err(|err| Refusal::Database("issue a channel ID", err))?;
    Ok((StatusCode::CREATED, Json(NewChannel { channel })).into_response())
}

/// The stored bytes of revision `number` of `space`, if it has one.
async fn stored(db: &PgPool, space: SpaceId, number: u64) -> Result<Option<Vec<u8>>, Refusal> {
    // A number past `i64::MAX` follows no stored revision.
    let Ok(number) = i64::try_from(number) else {
        return Ok(None);
    };
    let found: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT roster FROM roster_revisions WHERE space_id = $1::uuid AND revision = $2",
    )
    .bind(space.to_string())
    .bind(number)
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a revision", err))?;
    Ok(found.map(|(roster,)| roster))
}

/// The answer to `sent`, revision `number` of `space`, when the space has `stored` with that
/// number: taken if it is the same bytes, a retry after a lost answer.
fn again(space: SpaceId, number: u64, sent: &[u8], stored: &[u8]) -> Answer {
    if sent != stored {
        return Err(Refusal::RevisionConflict(format!(
            "the space has another revision {number}"
        )));
    }
    taken(
        false,
        StoredRevision {
            space: space.to_string(),
            revision: number,
        },
    )
}

/// `POST /spaces/<space>/revisions`: stores a revision of the space, from its creator's session for
/// revision 1 and from a session of an admin in revision n for n+1.
pub(crate) async fn store(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(space): Path<String>,
    body: Bytes,
) -> Answer {
    let db = &state.db;
    let found = find(db, &space, &account).await?;
    // An account that does not get the space learns nothing of it, unless it may store the
    // revision and sends it again: an admin's retry of a revision that took its account out.
    let hide = |refusal| {
        if found.gets {
            refusal
        } else {
            Refusal::NoSpace
        }
    };
    let request: RevisionRequest = parse(&body).map_err(hide)?;
    let sent = bytes(&request.revision, "revision").map_err(hide)?;
    let roster = Roster::from_cbor(&sent)
        .map_err(|err| hide(Refusal::Roster(format!("not a roster revision: {err}"))))?;
    let number = roster.body().revision;
    let previous = match number {
        1 => None,
        _ => {
            let bytes = stored(db, found.id, number - 1).await?.ok_or_else(|| {
                hide(Refusal::RevisionConflict(format!(
                    "the space has no revision {} for revision {number} to follow",
                    number - 1
                )))
            })?;
            Some(Roster::from_cbor(&bytes).expect("a stored revision decodes"))
        }
    };
    let id: AccountId = account.parse().expect("a stored account ID");
    let allowed = match &previous {
        None => found.creator == account,
        Some(previous) => admin(previous, id),
    };
    if !allowed {
        return Err(hide(Refusal::NotAllowed(match previous {
            None => "only the space's creator stores revision 1".to_owned(),
            Some(_) => format!(
                "only an admin in revision {} stores revision {number}",
                number - 1
            ),
        })));
    }
    if let Some(stored) = stored(db, found.id, number).await? {
        return again(found.id, number, &sent, &stored).map_err(hide);
    }
    check(db, &found, &roster, previous.as_ref(), id).await?;
    insert(&state, found.id, &roster, previous.as_ref(), &sent).await
}

/// Checks `roster`, a revision the account `account` may store in the space `found` after
/// `previous`, or revision 1 if `None`: its space and name, its signer and signature, checks 2 to 4
/// of "Roster" with the certificates the server holds, a revoked service agent having none
/// (revision 1: checks 3 and 4, and its creator an admin), and its channels.
async fn check(
    db: &PgPool,
    found: &Found,
    roster: &Roster,
    previous: Option<&Roster>,
    account: AccountId,
) -> Result<(), Refusal> {
    let refused = |detail: &str| Refusal::Roster(detail.to_owned());
    let body = roster.body();
    if body.space != found.id {
        return Err(refused("the revision is of another space"));
    }
    if body.name != found.name {
        return Err(refused("the revision has another name than the space's"));
    }

    // The signer: a device of the session's account with a certificate, not revoked, and its key's
    // signature.
    let signer: Option<(Vec<u8>, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT d.signing, a.master FROM devices d JOIN accounts a ON a.id = d.account_id
         WHERE d.id = $1::uuid AND d.account_id = $2::uuid AND d.cert IS NOT NULL
           AND NOT d.revoked",
    )
    .bind(body.signer.to_string())
    .bind(account.to_string())
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a device", err))?;
    let keys = signer.and_then(|(signing, master)| {
        let signing: [u8; 32] = signing.try_into().ok()?;
        let master: [u8; 32] = master?.try_into().ok()?;
        Some((signing, master))
    });
    let (signing, master) = keys.ok_or_else(|| {
        refused("the signer is no device of the session's account with a certificate, or revoked")
    })?;
    skald_crypto::verify(
        &signing,
        ROSTER_LABEL,
        &[roster.body_bytes()],
        roster.signature(),
    )
    .map_err(|_| refused("the signature does not verify with the signer's key"))?;

    let mut accounts: Vec<String> = body
        .members
        .iter()
        .chain(
            previous
                .iter()
                .flat_map(|previous| previous.body().members.iter()),
        )
        .map(|member| member.account.to_string())
        .collect();
    accounts.sort();
    accounts.dedup();
    let certs = certs(db, &accounts).await?;
    let checked = match previous {
        None => {
            if !admin(roster, account) {
                return Err(refused("revision 1 does not list its creator as an admin"));
            }
            skald_crypto::check_roster(roster, &certs)
        }
        Some(previous) => skald_crypto::accept_next(previous, roster, &certs, account, &master),
    };
    checked.map_err(|err| Refusal::Roster(err.to_string()))?;
    // Check 4 took revoked devices' certificates, which the chains of the devices they keep need;
    // a revoked service agent has none.
    let agents: Vec<String> = body
        .service_agents
        .iter()
        .map(|agent| agent.device.to_string())
        .collect();
    let revoked: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM devices WHERE id = ANY($1::text[]::uuid[]) AND revoked LIMIT 1",
    )
    .bind(&agents)
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read the service agents", err))?;
    if let Some((device,)) = revoked {
        return Err(Refusal::Roster(format!(
            "the service agent {device} is revoked"
        )));
    }

    let channels: Vec<String> = body.channels.iter().map(|c| c.id.to_string()).collect();
    let (issued,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM channels WHERE space_id = $1::uuid AND id = ANY($2::text[]::uuid[])",
    )
    .bind(found.id.to_string())
    .bind(&channels)
    .fetch_one(db)
    .await
    .map_err(|err| Refusal::Database("read the channels", err))?;
    // Check 3 has refused a repeated channel.
    if usize::try_from(issued).ok() != Some(channels.len()) {
        return Err(refused(
            "a channel's ID is not one the server issued for this space",
        ));
    }
    Ok(())
}

/// Whether `roster` lists `account` as an admin.
pub(crate) fn admin(roster: &Roster, account: AccountId) -> bool {
    roster
        .body()
        .members
        .iter()
        .any(|member| member.account == account && member.role == Role::Admin)
}

/// The certificates of the devices of `accounts`, as the devices sent them, revoked or not.
pub(crate) async fn certs(
    db: impl sqlx::PgExecutor<'_>,
    accounts: &[String],
) -> Result<Vec<Cert>, Refusal> {
    let found: Vec<(Vec<u8>,)> = sqlx::query_as(
        "SELECT cert FROM devices
         WHERE account_id = ANY($1::text[]::uuid[]) AND cert IS NOT NULL",
    )
    .bind(accounts)
    .fetch_all(db)
    .await
    .map_err(|err| Refusal::Database("read the certificates", err))?;
    // The server checked each certificate when it took it.
    Ok(found
        .iter()
        .filter_map(|(cert,)| Cert::from_cbor(cert).ok())
        .collect())
}

/// Stores `roster`, received as `sent`, as the next revision of `space`, with its members, and
/// gives their accounts' connections a notice of `space`, on each instance; with the accounts it
/// adds to channels after `previous`. Of two revisions with one number at once, the first is stored
/// and the other refused.
async fn insert(
    state: &AppState,
    space: SpaceId,
    roster: &Roster,
    previous: Option<&Roster>,
    sent: &[u8],
) -> Answer {
    let db = &state.db;
    let body = roster.body();
    let number = body.revision;
    let revision = i64::try_from(number).map_err(|_| {
        Refusal::RevisionConflict(format!("the server stores no revision {number}"))
    })?;
    let mut tx = db
        .begin()
        .await
        .map_err(|err| Refusal::Database("store a revision", err))?;
    // A service token's creation waits for the revision, and the revision for the creation
    // (`service_tokens::create`).
    sqlx::query("SELECT 1 FROM spaces WHERE id = $1::uuid FOR NO KEY UPDATE")
        .bind(space.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|err| Refusal::Database("store a revision", err))?;
    // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
    let inserted = sqlx::query(
        "INSERT INTO roster_revisions (space_id, revision, roster) VALUES ($1::uuid, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(space.to_string())
    .bind(revision)
    .bind(sent)
    .execute(&mut *tx)
    .await
    .map_err(|err| Refusal::Database("store a revision", err))?;
    if inserted.rows_affected() == 0 {
        drop(tx);
        // The insert it waited for has committed.
        let stored = stored(db, space, number).await?.unwrap_or_default();
        return again(space, number, sent, &stored);
    }
    let (accounts, admins): (Vec<String>, Vec<bool>) = body
        .members
        .iter()
        .map(|member| (member.account.to_string(), member.role == Role::Admin))
        .unzip();
    sqlx::query(
        "INSERT INTO roster_members (space_id, revision, account_id, admin)
         SELECT $1::uuid, $2, account::uuid, admin FROM unnest($3::text[], $4::bool[])
             AS listed (account, admin)",
    )
    .bind(space.to_string())
    .bind(revision)
    .bind(&accounts)
    .bind(&admins)
    .execute(&mut *tx)
    .await
    .map_err(|err| Refusal::Database("store a revision's members", err))?;
    history::added(&mut tx, roster, previous)
        .await
        .map_err(|err| Refusal::Database("store a revision's channel additions", err))?;
    // A token ends once the latest revision lists its device in no service agent entry.
    let agents: Vec<String> = body
        .service_agents
        .iter()
        .map(|agent| agent.device.to_string())
        .collect();
    let ending = |err| Refusal::Database("end the service tokens of removed agents", err);
    let ended: Vec<(Vec<u8>,)> = sqlx::query_as(
        "DELETE FROM service_tokens
         WHERE space_id = $1::uuid AND device_id <> ALL($2::text[]::uuid[])
         RETURNING token_hash",
    )
    .bind(space.to_string())
    .bind(&agents)
    .fetch_all(&mut *tx)
    .await
    .map_err(ending)?;
    sessions::tell_ended(state, &mut tx, &ended)
        .await
        .map_err(ending)?;
    state
        .instance
        .tell(&mut tx, &[Event::Revision(space, revision)])
        .await
        .map_err(|err| Refusal::Database("tell the other instances of a revision", err))?;
    tx.commit()
        .await
        .map_err(|err| Refusal::Database("store a revision", err))?;
    state.instance.record(space, revision);
    sessions::close_ended(state, &ended);
    // An account the revision removes gets no notice.
    state
        .notices
        .notify(body.members.iter().map(|member| member.account), space);
    taken(
        true,
        StoredRevision {
            space: space.to_string(),
            revision: number,
        },
    )
}
