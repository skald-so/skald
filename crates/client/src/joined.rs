//! Spaces another device of the account created or joined (`docs/protocol/membership.md`,
//! "Anchors"). A person's `skald recv`, once caught up, and a run that gets a `space-notice` of a
//! space the device does not follow, take the account's spaces list; when it names a space the
//! device does not follow, they take the account's anchors objects. They start each such space the
//! anchors objects of the account's other `client` devices with a valid certificate name and agree
//! on ([`anchors::start`]), and warn of one those disagree on. A `service` device skips this. When
//! the server does not give what this needs, the device follows nothing new and the run goes on.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use skald_crypto::{check_anchors, check_valid_device};
use skald_proto::{Anchors, Cbor as _, DeviceKind, SpaceId};

use crate::anchors;
use crate::devices::{self, text};
use crate::follow::SpaceDevices;
use crate::spaces::Follower;

/// The account's spaces list, `GET`.
const SPACES_PATH: &str = "/spaces";

/// `Spaces` in the API spec.
#[derive(Deserialize)]
struct Spaces {
    spaces: Vec<Listed>,
}

/// `ListedSpace` in the API spec, as far as this step reads it.
#[derive(Deserialize)]
struct Listed {
    space: String,
}

/// Whether a space of `noticed`, the spaces of `space-notice`s, is one the device does not follow.
pub fn names_unfollowed(follower: &Follower, noticed: &BTreeSet<SpaceId>) -> bool {
    match follower.state.spaces() {
        Ok(spaces) => noticed
            .iter()
            .any(|id| !spaces.iter().any(|space| space.id == *id)),
        Err(_) => false,
    }
}

/// Starts each space the account's spaces list names that the device does not follow and the
/// anchors objects of the account's other valid `client` devices name and agree on; warns of each
/// they disagree on, and of what fails. Gives the spaces it started.
pub fn follow(follower: &mut Follower) -> Vec<SpaceId> {
    if follower.service.is_some() {
        return Vec::new();
    }
    match started(follower) {
        Ok(started) => started,
        Err(err) => {
            eprintln!(
                "skald: warning: {err}; a later skald recv follows the spaces the account's other \
                 devices joined"
            );
            Vec::new()
        }
    }
}

/// As [`follow`], failing on what the server does not give.
fn started(follower: &mut Follower) -> Result<Vec<SpaceId>, String> {
    let unfollowed = unfollowed(follower)?;
    if unfollowed.is_empty() {
        return Ok(Vec::new());
    }
    let body = (follower.connection)
        .get_as(anchors::PATH, &follower.token)
        .map_err(|err| format!("the account's anchors objects are not read: {}", text(err)))?;
    let objects = anchors::objects(&body)?;
    let devices = devices::own(
        &mut follower.connection,
        &follower.token,
        &follower.state,
        &follower.own.master,
    )?;
    let mut by_space: BTreeMap<SpaceId, Vec<[u8; 32]>> = BTreeMap::new();
    for object in objects
        .iter()
        .filter(|object| valid(follower, &devices, object))
    {
        for (space, hash) in &object.body().anchors {
            if unfollowed.contains(space) {
                by_space.entry(*space).or_default().push(*hash);
            }
        }
    }
    let mut started = Vec::new();
    for (space, hashes) in &by_space {
        let start = anchors::start(
            (&mut follower.connection, &follower.token, &follower.state),
            *space,
            hashes,
            &follower.own,
        )?;
        match start {
            Ok(start) => {
                (follower.state)
                    .add_linked_space(*space, start.body().name.as_str(), &start.to_cbor(), false)
                    .map_err(text)?;
                started.push(*space);
            }
            Err(warning) => eprintln!("skald: warning: {warning}"),
        }
    }
    Ok(started)
}

/// The spaces the account's spaces list names that the device does not follow.
fn unfollowed(follower: &mut Follower) -> Result<BTreeSet<SpaceId>, String> {
    let body = (follower.connection)
        .get_as(SPACES_PATH, &follower.token)
        .map_err(|err| format!("the account's spaces are not read: {}", text(err)))?;
    let listed: Spaces = serde_json::from_slice(&body)
        .map_err(|_| "unexpected answer from the server: a space list without its spaces")?;
    let followed: BTreeSet<SpaceId> = (follower.state.spaces().map_err(text)?.into_iter())
        .map(|space| space.id)
        .collect();
    Ok((listed.spaces.iter())
        .filter_map(|listed| listed.space.parse().ok())
        .filter(|space| !followed.contains(space))
        .collect())
}

/// Whether `object` counts for a linked device: of its account, signed by another `client` device
/// of it whose certificate among `devices` is valid against the pinned master key ("Device
/// checks", checks 1–4).
fn valid(follower: &Follower, devices: &SpaceDevices, object: &Anchors) -> bool {
    let body = object.body();
    let own = &follower.own;
    body.account == own.account
        && body.device != follower.this
        && check_valid_device(
            &own.master,
            &devices.certs,
            &devices.revocations,
            body.device,
        )
        .is_ok_and(|cert| {
            let signer = cert.body();
            signer.account == own.account
                && signer.kind == DeviceKind::Client
                && check_anchors(object, &signer.signing).is_ok()
        })
}
