//! The device's anchors object (`docs/protocol/membership.md`, "Anchors"): the latest revision it
//! accepted of each of its spaces, by hash, and its pinned personal space, signed with its key. A
//! run signs a new one when the state holds other revisions or another personal space than the last
//! one names, a state older than anchors objects included, writes it to the state before it sends it
//! and sends it again while the server has not taken it. A revoked device runs no follower, so it
//! signs none.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_proto::{AccountId, Anchors, AnchorsBody, Cbor as _, DeviceId, Unknown};

use crate::connection::{Connection, Method};
use crate::device;
use crate::devices::text;
use crate::follow::{self, ATTEMPTS};
use crate::state::State;

/// Who signs the anchors object: the account, the device and its keys.
pub struct Signer<'k> {
    pub account: AccountId,
    pub device: DeviceId,
    pub keys: &'k device::Keys,
}

/// Signs the device's anchors object if `state` holds other revisions or another personal space
/// than its last one names, and sends it, as the session `token`, while the server has not taken
/// it; warns of what fails. Once a send fails, `unsent` is set and the run only signs: it sends
/// and warns once.
pub fn keep(
    connection: &mut Connection,
    token: &str,
    state: &State,
    signer: &Signer<'_>,
    unsent: &mut bool,
) {
    let kept = signed(state, signer).and_then(|waiting| match waiting {
        Some(anchors) if !*unsent => {
            store(connection, token, state, &anchors).inspect_err(|_| *unsent = true)
        }
        _ => Ok(()),
    });
    if let Err(err) = kept {
        eprintln!("skald: warning: {err}");
    }
}

/// The anchors object that waits for the server, signing it first if `state` holds other
/// revisions or another personal space than the last one names; `None` if none waits.
fn signed(state: &State, signer: &Signer<'_>) -> Result<Option<Vec<u8>>, String> {
    for _ in 0..ATTEMPTS {
        let body = current(state, signer)?;
        let held = state.anchors().map_err(text)?;
        let names = |(bytes, _): &(Vec<u8>, bool)| {
            Anchors::from_cbor(bytes).is_ok_and(|held| *held.body() == body)
        };
        match &held {
            Some(held) if names(held) => return Ok(held.1.then(|| held.0.clone())),
            // Nothing accepted, nothing to anchor.
            None if body.anchors.is_empty() => return Ok(None),
            _ => {}
        }
        let anchors = skald_crypto::sign_anchors(&body, signer.keys.signing())
            .map_err(|err| format!("the anchors object does not encode: {err}"))?
            .to_cbor();
        let replaced = held.as_ref().map(|(bytes, _)| bytes.as_slice());
        if state.set_anchors(replaced, &anchors).map_err(text)? {
            return Ok(Some(anchors));
        }
    }
    Err("other skald runs kept signing anchors objects; the next skald command signs it".to_owned())
}

/// The body of the anchors object of what `state` holds: the hash of the last accepted revision
/// of each space, by space ID, and the pinned personal space.
fn current(state: &State, signer: &Signer<'_>) -> Result<AnchorsBody, String> {
    let mut spaces = state.spaces().map_err(text)?;
    spaces.sort_by_key(|space| space.id);
    let anchors = spaces
        .iter()
        .map(|space| {
            Ok((
                space.id,
                follow::decode(&space.accepted, &space.name)?.hash(),
            ))
        })
        .collect::<Result<_, String>>()?;
    Ok(AnchorsBody {
        account: signer.account,
        device: signer.device,
        anchors,
        personal: spaces
            .iter()
            .find(|space| space.personal)
            .map(|space| space.id),
        unknown: Unknown::default(),
    })
}

/// `POST /account/anchors`: has the server store `anchors`, in CBOR, which `state` holds as
/// waiting, as the session `token`. Taken, it no longer waits; otherwise it waits for a later run.
fn store(
    connection: &mut Connection,
    token: &str,
    state: &State,
    anchors: &[u8],
) -> Result<(), String> {
    let request = serde_json::json!({ "anchors": URL_SAFE_NO_PAD.encode(anchors) }).to_string();
    connection
        .send_json_as(Method::Post, "/account/anchors", token, request.as_bytes())
        .map_err(|err| {
            format!(
                "{err}; this device's anchors object waits for the server to take it: the next \
                 skald command that connects sends it again"
            )
        })?;
    state.anchors_taken(anchors).map_err(text)
}

#[cfg(test)]
mod tests {
    use skald_proto::SpaceId;

    use super::*;
    use crate::spaces::tests::first;
    use crate::test_dir::TestDir;

    /// The anchors object `bytes`, checked with the key of `keys`.
    fn checked(bytes: &[u8], keys: &device::Keys) -> AnchorsBody {
        let anchors = Anchors::from_cbor(bytes).expect("an anchors object");
        skald_crypto::check_anchors(&anchors, &keys.signing().public_key())
            .expect("signed by the device");
        anchors.body().clone()
    }

    /// The signer with `keys` on a new account.
    fn signer(keys: &device::Keys) -> Signer<'_> {
        Signer {
            account: AccountId::generate(),
            device: keys.id(),
            keys,
        }
    }

    #[test]
    fn device_signs_its_anchors_object_once_per_change_and_until_the_server_takes_it() {
        let dir = TestDir::new("anchors");
        let state = State::open(&dir).unwrap();
        let keys = device::Keys::generate();
        let signer = signer(&keys);
        assert_eq!(signed(&state, &signer), Ok(None), "nothing accepted");
        assert_eq!(state.anchors().unwrap(), None);

        let (acme, personal) = (first(signer.account), first(signer.account));
        let (acme_id, personal_id) = (acme.body().space, personal.body().space);
        state.add_space(acme_id, "acme", &acme.to_cbor()).unwrap();
        state
            .add_personal_space(personal_id, "ada", &personal.to_cbor())
            .unwrap();
        let sent = signed(&state, &signer).unwrap().expect("an anchors object");
        let mut anchors = vec![(acme_id, acme.hash()), (personal_id, personal.hash())];
        anchors.sort_unstable();
        let expected = AnchorsBody {
            account: signer.account,
            device: signer.device,
            anchors,
            personal: Some(personal_id),
            unknown: Unknown::default(),
        };
        assert_eq!(checked(&sent, &keys), expected);
        assert_eq!(state.anchors().unwrap(), Some((sent.clone(), true)));
        assert_eq!(
            signed(&state, &signer),
            Ok(Some(sent.clone())),
            "the same one, while it waits"
        );

        state.anchors_taken(&sent).unwrap();
        assert_eq!(signed(&state, &signer), Ok(None), "taken");

        let next = first(signer.account);
        assert!(
            state
                .accept(
                    acme_id,
                    &acme.to_cbor(),
                    &[(1, acme.hash())],
                    &next.to_cbor()
                )
                .unwrap()
        );
        let resent = signed(&state, &signer).unwrap().expect("a new one");
        let body = checked(&resent, &keys);
        assert!(body.anchors.contains(&(acme_id, next.hash())), "{body:?}");
        assert_eq!(state.anchors().unwrap(), Some((resent, true)));
    }

    #[test]
    fn device_without_a_personal_space_names_none() {
        let dir = TestDir::new("anchors-personal");
        let state = State::open(&dir).unwrap();
        let keys = device::Keys::generate();
        let signer = signer(&keys);
        let acme = first(signer.account);
        let id: SpaceId = acme.body().space;
        state.add_space(id, "acme", &acme.to_cbor()).unwrap();
        let sent = signed(&state, &signer).unwrap().expect("an anchors object");
        let body = checked(&sent, &keys);
        assert_eq!(
            (body.anchors, body.personal),
            (vec![(id, acme.hash())], None)
        );
    }
}
