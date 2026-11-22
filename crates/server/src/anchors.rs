//! Anchors objects (`docs/protocol/membership.md`, "Anchors"): the server keeps the last one of each
//! `client` device of an account that it took from a session of the account, also once the device
//! is revoked, and gives them, as the bytes it got, only to the account's sessions and to its
//! service tokens with their device's proof.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use serde::{Deserialize, Serialize};
use skald_proto::{AccountId, Anchors, Cbor as _, DeviceKind, Revocation};

use crate::AppState;
use crate::devices::{base64, bytes, taken};
use crate::refusal::{Answer, Caller, Refusal, SignedIn, answer, parse};
use crate::requests;
use crate::revocations;
use crate::spaces::certs;

/// `AnchorsRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorsRequest {
    anchors: String,
}

/// `StoredAnchors` in the spec.
#[derive(Serialize)]
struct StoredAnchors {
    anchors: String,
}

/// `AccountAnchors` in the spec.
#[derive(Serialize)]
struct AccountAnchors {
    anchors: Vec<String>,
}

/// `POST /account/anchors`: stores an anchors object of the session's account in place of the last
/// one of its device, a `client` device of the account with a valid certificate that is not
/// revoked, whose key its signature verifies with; the same bytes again are taken as stored.
pub(crate) async fn store(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let request: AnchorsRequest = parse(&body)?;
    let sent = bytes(&request.anchors, "anchors")?;
    let refused = |detail: &str| Refusal::Anchors(detail.to_owned());
    let anchors = Anchors::from_cbor(&sent)
        .map_err(|err| Refusal::Anchors(format!("not an anchors object: {err}")))?;
    let id: AccountId = account.parse().expect("a stored account ID");
    if anchors.body().account != id {
        return Err(refused("the anchors object is of another account"));
    }
    let (master,): (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT master FROM accounts WHERE id = $1::uuid")
            .bind(&account)
            .fetch_optional(&state.db)
            .await
            .map_err(|err| Refusal::Database("read the master key", err))?
            .ok_or(Refusal::NotSignedIn)?;
    let master: [u8; 32] = master
        .and_then(|master| master.try_into().ok())
        .ok_or_else(|| refused("the account has no master key"))?;
    let certs = certs(&state.db, std::slice::from_ref(&account)).await?;
    let revocations: Vec<Revocation> = revocations::of_account(&state.db, &account)
        .await?
        .iter()
        .map(|(bytes,)| Revocation::from_cbor(bytes).expect("a stored revocation decodes"))
        .collect();
    let device = anchors.body().device;
    let cert = skald_crypto::check_valid_device(&master, &certs, &revocations, device)
        .ok()
        .filter(|cert| cert.body().kind == DeviceKind::Client)
        .ok_or_else(|| {
            refused(
                "`device` is no `client` device of the account with a valid certificate that is \
                 not revoked",
            )
        })?;
    skald_crypto::check_anchors(&anchors, &cert.body().signing).map_err(|_| {
        refused("the signature does not verify with the key of the certificate of `device`")
    })?;
    let stored = sqlx::query(
        "UPDATE devices SET anchors = $3
         WHERE id = $1::uuid AND account_id = $2::uuid AND anchors IS DISTINCT FROM $3",
    )
    .bind(device.to_string())
    .bind(&account)
    .bind(&sent)
    .execute(&state.db)
    .await
    .map_err(|err| Refusal::Database("store an anchors object", err))?;
    taken(
        stored.rows_affected() > 0,
        StoredAnchors {
            anchors: base64(&sent),
        },
    )
}

/// `GET /account/anchors`: the last anchors object of each device of the account, as the bytes the
/// server got, to a session of the account, or to a service token of its with its device's proof.
pub(crate) async fn list(
    State(state): State<AppState>,
    caller: Caller,
    headers: HeaderMap,
) -> Response {
    let account = match caller {
        Caller::Session(account) => account,
        Caller::Token(_) => match requests::prove(&state, &headers).await {
            Ok(proved) => proved.account.to_string(),
            Err(refused) => return refused,
        },
    };
    answer(async {
        let held: Vec<(Vec<u8>,)> = sqlx::query_as(
            "SELECT anchors FROM devices WHERE account_id = $1::uuid AND anchors IS NOT NULL
             ORDER BY id",
        )
        .bind(&account)
        .fetch_all(&state.db)
        .await
        .map_err(|err| Refusal::Database("read the anchors objects", err))?;
        let anchors = held.iter().map(|(bytes,)| base64(bytes)).collect();
        Ok(Json(AccountAnchors { anchors }).into_response())
    })
    .await
}
