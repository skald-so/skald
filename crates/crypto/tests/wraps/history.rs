//! A reader outside the group reads a channel's history from its wrap: `docs/protocol/
//! membership.md`, "Key chain", "Wraps" and "History entry". It opens the wrap of an epoch, follows
//! the key chain back from that epoch's key and reads the entries of that epoch and earlier ones,
//! as the server hands them out, encoded.

use std::collections::BTreeMap;

use skald_crypto::{HistoryKey, follow_links};
use skald_proto::{Cbor, Content, HistoryBody, HistoryEntry, HistoryLink, HistoryWrap, MessageId};

use crate::cbor::{Value, bytes, text};
use crate::fixtures::{Channel, Reader, cert, device_id, read, readers, signing_key};

/// The time a history body states, in seconds since the Unix epoch.
const SENT: u64 = 1_800_000_123;

/// Content with an A2A message, as in `docs/protocol/test-vectors.md`, "Content with an A2A
/// message and an attachment", without the attachment, its text `said`.
fn content(said: &str) -> Vec<u8> {
    let message = format!(
        r#"{{"messageId":"019a6f20-1a00-7c00-8a00-000000000004","contextId":"019a6f20-0000-7000-8000-00000000c0de","role":"ROLE_USER","parts":[{{"text":"{said}"}}]}}"#
    );
    Value::Map(vec![
        (1, text("acme/alice/claude-api@skald.example")),
        (2, text("acme/bob/codex@skald.example")),
        (3, Value::Uint(7)),
        (4, Value::Uint(1)),
        (6, text(&message)),
    ])
    .encode()
}

/// The body of a message device `seed` sends in `channel` with the text `said`.
fn body(seed: u8, channel: &Channel, said: &str) -> HistoryBody {
    let encoded = Value::Map(vec![
        (1, bytes(device_id(seed).as_bytes())),
        (2, bytes(channel.id.as_bytes())),
        (3, bytes(MessageId::generate().as_bytes())),
        (4, Value::Uint(SENT)),
        (5, bytes(&content(said))),
    ])
    .encode();
    let body = HistoryBody::from_cbor(&encoded).expect("a history body");
    assert_eq!(body.content, Content::from_cbor(&content(said)).unwrap());
    body
}

/// What the server hands out: an object as its bytes.
fn served<T: Cbor>(object: &T) -> T {
    T::from_cbor(&object.to_cbor()).expect("a served object decodes")
}

/// A channel's history as members of its group wrote it.
struct History {
    channel: Channel,
    /// Each entry, its epoch and the body its sender wrote.
    entries: Vec<(HistoryEntry, HistoryBody)>,
    links: Vec<HistoryLink>,
}

/// Members 30 and 31 of a channel write an entry in each of epochs 1 to 3, and member 30 uploads
/// the links of epochs 2 and 3.
fn history() -> History {
    let mut channel = Channel::new(&[30, 31]);
    let mut entries = Vec::new();
    for epoch in 1..=3 {
        if epoch > 1 {
            channel.next_epoch();
        }
        assert_eq!(channel.epoch(), epoch);
        let at = usize::try_from(epoch % 2).unwrap();
        let seed = channel.seeds[at];
        let sent = body(seed, &channel, &format!("epoch {epoch}"));
        let entry = channel
            .key(at, epoch)
            .seal_entry(epoch, &sent, &signing_key(seed))
            .expect("an entry");
        entries.push((entry, sent));
    }
    let links = (2..=3)
        .map(|epoch| {
            channel
                .key(0, epoch)
                .seal_link(channel.id, epoch, epoch - 1, channel.key(0, epoch - 1))
                .expect("a link")
        })
        .collect();
    History {
        channel,
        entries,
        links,
    }
}

/// The body of `entry`, as a reader with `key` gets it: decrypted and verified with the
/// certificate of its `sender`.
fn read_entry(key: &HistoryKey, entry: &HistoryEntry) -> Option<HistoryBody> {
    let message = key.open_entry(entry).ok()?;
    let sender = message.sender();
    let cert = [30, 31]
        .map(cert)
        .into_iter()
        .find(|cert| cert.body().device == sender)?;
    message.verify(&cert).ok()
}

/// What `reader` reads of `history` from `wrap`, a wrap of `epoch`: the body of each entry it
/// reads with a key it holds, by the entry's epoch.
fn reads(
    reader: &Reader,
    wrap: &HistoryWrap,
    epoch: u64,
    history: &History,
) -> BTreeMap<u64, HistoryBody> {
    let key = HistoryKey::from_bytes(&read(&served(wrap), &reader.key).expect("the wrap opens"));
    let links: Vec<HistoryLink> = history.links.iter().map(served).collect();
    let mut keys = follow_links(history.channel.id, epoch, &key, &links);
    assert!(
        !keys.contains_key(&epoch),
        "follow_links gives earlier epochs only"
    );
    keys.insert(epoch, key);
    let mut read = BTreeMap::new();
    for (entry, _) in &history.entries {
        let entry = served(entry);
        // Every key the reader holds is tried: one of another epoch opens nothing.
        if let Some(body) = keys.values().find_map(|key| read_entry(key, &entry)) {
            read.insert(entry.epoch, body);
        }
    }
    read
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_browser_or_the_recovery_key_reads_the_history_of_its_wrap_through_the_key_chain() {
    let history = history();
    for reader in readers() {
        for (epoch, signer) in [(3, 0), (2, 1)] {
            let wrap = history.channel.wrap(signer, epoch, &reader);
            let expected: BTreeMap<u64, HistoryBody> = history
                .entries
                .iter()
                .filter(|(entry, _)| entry.epoch <= epoch)
                .map(|(entry, sent)| (entry.epoch, sent.clone()))
                .collect();
            assert_eq!(
                expected.keys().copied().collect::<Vec<_>>(),
                (1..=epoch).collect::<Vec<_>>()
            );
            assert_eq!(
                reads(&reader, &wrap, epoch, &history),
                expected,
                "the wrap of epoch {epoch} gives that epoch's entries and earlier ones"
            );
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_of_one_epoch_reads_no_entry_of_a_later_one() {
    let mut history = history();
    history.channel.next_epoch();
    let sent = body(31, &history.channel, "epoch 4");
    let entry = history
        .channel
        .key(1, 4)
        .seal_entry(4, &sent, &signing_key(31))
        .expect("an entry of epoch 4");
    history.entries.push((entry, sent));
    let link = history
        .channel
        .key(1, 4)
        .seal_link(history.channel.id, 4, 3, history.channel.key(1, 3))
        .expect("the link of epoch 4");
    history.links.push(link);
    for reader in readers() {
        let wrap = history.channel.wrap(0, 3, &reader);
        let read = reads(&reader, &wrap, 3, &history);
        assert_eq!(read.keys().copied().collect::<Vec<_>>(), [1, 2, 3]);
        let wrap = history.channel.wrap(1, 4, &reader);
        assert_eq!(
            reads(&reader, &wrap, 4, &history)
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }
}
