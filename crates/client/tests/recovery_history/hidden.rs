//! Wraps a compromised server hides leave the entries they would open counted, not shown.

use serde_json::Value;
use skald_proto::{Cbor as _, HistoryWrap};

use crate::cloud::unb64;
use crate::entries::talk_entries;
use crate::reading::{assert_history, shown_history, without_key};
use crate::{recovers, talked, wraps_request};

/// The epoch whose wraps the relay hides.
const HIDDEN: u64 = 2;

/// `answer`, the server's answer with a channel's wraps, without those of epoch [`HIDDEN`].
fn without_hidden(answer: &str) -> String {
    let mut body: Value = serde_json::from_str(answer).expect("the wraps' answer in JSON");
    if let Some(wraps) = body["wraps"].as_array_mut() {
        wraps.retain(|item| {
            let bytes = unb64(item.as_str().expect("a wrap in base64url"));
            HistoryWrap::from_cbor(&bytes).map_or(true, |wrap| wrap.body().epoch != HIDDEN)
        });
    }
    body.to_string()
}

/// In the talk of [`talked`], the relay then gives the machines no wrap of the channel's epoch 2.
/// Ada's first machine is wiped, and her new machine, linked with the recovery key naming that
/// device lost, shows with `skald history` the two lines of epoch 1, not Bob's line of epoch 2, and
/// says it holds no key for one entry.
#[test]
fn wraps_the_proxy_hides_leave_their_entries_counted_not_shown() {
    let (talk, key, said) = talked(
        &[4_177_706_001, 4_177_706_002],
        &["ada-wraps-withheld", "bob-audits-the-books"],
    );
    assert_eq!(talk_entries(&talk).len(), said.len(), "the entries held");
    (talk.cloud().relay).rewrite(&wraps_request(talk.at()), without_hidden);

    let new = recovers(&talk, &key, &[talk.ada.device]);
    let shown = shown_history(&new.machine, talk.channel);

    assert_history(&shown, talk.channel, &said[..2], &[said[2].text.as_str()]);
    assert_eq!(
        without_key(&shown),
        Some(1),
        "not one entry counted as under no key: {shown:?}"
    );
}
