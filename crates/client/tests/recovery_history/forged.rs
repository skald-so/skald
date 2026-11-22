//! Wraps to the recovery key a reader MUST drop leave the entries they would open counted, not
//! shown: one signed by a device whose certificate chains to no member's master key, and one whose
//! signature does not verify.

use serde_json::Value;
use skald_crypto::HistoryKey;
use skald_proto::{Cbor as _, HistoryWrap, WrapRecipient};

use crate::cloud::unb64;
use crate::fixtures::{Issuer, add_device};
use crate::kit::b64;
use crate::machine::Machine;
use crate::reading::{assert_history, shown_history, without_key};
use crate::recovering::{LOST, NEW_HOST, recover};
use crate::talk::Talk;
use crate::{assert_wrapped_to_recovery, person_on, recovery_hpke, talked, wraps_request};

/// The epoch whose wrap to Ada's recovery key the relay replaces.
const FORGED: u64 = 2;

/// The wrap of epoch [`FORGED`] of the talk's channel to Ada's recovery key, `recovery`, giving
/// `key`, signed by the device of `signing`.
fn wrap_of(
    talk: &Talk,
    (key, recovery): (&HistoryKey, &[u8; 32]),
    signing: &skald_crypto::SigningKey,
) -> HistoryWrap {
    let recipient = WrapRecipient::Recovery(talk.ada.account);
    key.wrap(talk.channel, FORGED, recipient, recovery, signing)
        .expect("a wrap")
}

/// `wrap` with its signature broken.
fn broken(wrap: &HistoryWrap) -> HistoryWrap {
    let mut signature = *wrap.signature();
    signature[0] ^= 1;
    HistoryWrap::from_parts(wrap.body_bytes().to_vec(), signature).expect("a wrap")
}

/// `answer`, the server's answer with a channel's wraps, with `forged` in place of those of epoch
/// [`FORGED`] to the recovery key of `recipient`.
fn with_forged(answer: &str, recipient: WrapRecipient, forged: &[Vec<u8>]) -> String {
    let mut body: Value = serde_json::from_str(answer).expect("the wraps' answer in JSON");
    let replaced = |item: &Value| {
        let wrap = HistoryWrap::from_cbor(&unb64(item.as_str().unwrap_or_default())).ok();
        wrap.is_some_and(|wrap| (wrap.body().epoch, wrap.body().recipient) == (FORGED, recipient))
    };
    let given = body["wraps"].as_array().cloned().unwrap_or_default();
    let kept = given.into_iter().filter(|item| !replaced(item));
    let added = forged.iter().map(|wrap| Value::String(b64(wrap)));
    body["wraps"] = Value::Array(kept.chain(added).collect());
    body.to_string()
}

/// In the talk of [`talked`], the relay then gives the machines, in place of the wrap of the
/// channel's epoch 2 to Ada's recovery key, two of that epoch's key to it: one signed by a device
/// of her account the server made, whose certificate chains to no master key of a member, and one
/// Ada's device signed, its signature broken. Ada's first machine is wiped, and her new machine,
/// linked with the recovery key naming that device lost, warns of each wrap it drops: of the
/// server's device by its ID, of the other by its signature. It shows with `skald history` the two
/// lines of epoch 1, not Bob's line of epoch 2, and says it holds no key for one entry.
#[test]
fn wraps_to_the_recovery_key_a_reader_must_drop_leave_their_entries_counted_not_shown() {
    let (mut talk, key, said) = talked(
        &[4_177_708_001, 4_177_708_002],
        &["ada-wraps-forged", "bob-closes-the-books"],
    );
    let recovery = recovery_hpke(&key, talk.ada.account);
    let account = talk.ada.account;
    let epoch_key = assert_wrapped_to_recovery(&mut talk, (account, &recovery), FORGED);
    let cloud = talk.cloud();
    let server_made = add_device(cloud, account, "ada-server-device", &Issuer::Server);
    let sealed = (&epoch_key, &recovery.public_key());
    let forged = [
        wrap_of(&talk, sealed, &server_made.signing).to_cbor(),
        broken(&wrap_of(&talk, sealed, &talk.ada.key)).to_cbor(),
    ];
    let recipient = WrapRecipient::Recovery(account);
    (cloud.relay).rewrite(&wraps_request(talk.at()), move |answer| {
        with_forged(answer, recipient, &forged)
    });

    let machine = Machine::new().with_host_name(NEW_HOST);
    let ran = recover(
        &talk.world,
        &talk.ada.user,
        &machine,
        &[(LOST, talk.ada.device)],
        Some(key.as_str()),
    );
    assert!(ran.success, "linking with the recovery key failed: {ran:?}");
    let warned: Vec<&str> = (ran.text.lines())
        .filter(|line| line.contains("warning"))
        .collect();
    let of_wrap =
        |part: &str| (warned.iter()).any(|line| line.contains("wrap") && line.contains(part));
    assert!(
        of_wrap(&server_made.id.to_string()),
        "no warning of the wrap the server's device signed: {warned:?}"
    );
    assert!(
        of_wrap("signature"),
        "no warning of the wrap with a broken signature: {warned:?}"
    );
    let new = person_on(
        &talk.world,
        &talk.ada,
        (machine, NEW_HOST),
        &[talk.ada.device, server_made.id],
    );
    let shown = shown_history(&new.machine, talk.channel);

    let under_no_key = without_key(&shown);
    assert_eq!(under_no_key, Some(1), "the entries under no key: {shown:?}");
    assert_history(&shown, talk.channel, &said[..2], &[&said[2].text]);
}
