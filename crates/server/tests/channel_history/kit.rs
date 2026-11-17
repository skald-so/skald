//! What the tests upload and read: history frames on a device's connection and their answers, the
//! channel's links, wraps and entries over HTTPS, items to upload, and the scene with every channel
//! member reading the whole history.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use skald_crypto::{HistoryKey, HpkeKey, SigningKey};
use skald_proto::{
    Cbor, ChannelId, ChannelMember, HistoryEntry, HistoryLink, MessageId, SpaceId, WrapRecipient,
};
use tungstenite::Message;

use crate::api::{Person, Session, assert_refused};
use crate::common::Response;
use crate::common::cbor::{array_of, byte_item, uint};
use crate::common::device_flow::{json, text};
use crate::common::devices::{random, unb64};
use crate::fixture::Fixture;
use crate::scene::{Formed, Peer, Scene, empty};
use crate::wire::{Socket, assert_nowhere};

/// How long a test waits for an answer, or for the server to close a connection.
const WAIT: Duration = Duration::from_secs(20);

/// The kinds of a channel's history the server gives, each its path's last segment and its
/// response's field.
pub const LINKS: &str = "links";
pub const WRAPS: &str = "wraps";
pub const ENTRIES: &str = "entries";

/// What a test uploads: an item of `history`, with its CBOR bytes.
#[derive(Debug, Clone)]
pub enum Item {
    Link(Vec<u8>),
    Wrap(Vec<u8>),
    /// An entry and the `msg_id` its upload names.
    Entry(MessageId, Vec<u8>),
}

impl Item {
    /// The item's CBOR bytes.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Link(bytes) | Self::Wrap(bytes) | Self::Entry(_, bytes) => bytes,
        }
    }

    fn encoded(&self) -> Vec<u8> {
        match self {
            Self::Link(bytes) => array_of(&[uint(1), byte_item(bytes)]),
            Self::Wrap(bytes) => array_of(&[uint(2), byte_item(bytes)]),
            Self::Entry(msg_id, bytes) => {
                array_of(&[uint(3), byte_item(msg_id.as_bytes()), byte_item(bytes)])
            }
        }
    }
}

/// The server's answer to an upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Stored,
    Refused(String),
}

/// The next upload number of this test binary.
fn next_upload() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The `history` frame of upload `upload` of `item` in `channel` of `space`.
pub fn history_frame(upload: u64, space: SpaceId, channel: ChannelId, item: &Item) -> Vec<u8> {
    array_of(&[
        uint(11),
        uint(upload),
        byte_item(space.as_bytes()),
        byte_item(channel.as_bytes()),
        item.encoded(),
    ])
}

/// The head of a CBOR item of major type `major` at the start of `bytes`: its argument and the
/// bytes after the head.
fn read_head(bytes: &[u8], major: u8) -> Option<(u64, &[u8])> {
    let (&first, rest) = bytes.split_first()?;
    if first >> 5 != major {
        return None;
    }
    let size = match first & 0x1f {
        small @ 0..=23 => return Some((u64::from(small), rest)),
        24 => 1,
        25 => 2,
        26 => 4,
        27 => 8,
        _ => return None,
    };
    let (argument, rest) = rest.split_at_checked(size)?;
    let argument = argument
        .iter()
        .fold(0, |value, &byte| (value << 8) | u64::from(byte));
    Some((argument, rest))
}

/// Fails the test on `bytes`, an answer to an upload that does not decode.
fn malformed(bytes: &[u8]) -> ! {
    panic!("a malformed answer to an upload: {bytes:02x?}")
}

/// The upload an answer frame names and the answer; `None` for a frame of another type.
fn answer_in(bytes: &[u8]) -> Option<(u64, Answer)> {
    match bytes {
        [0x82, 0x0c, rest @ ..] => match read_head(rest, 0) {
            Some((upload, [])) => Some((upload, Answer::Stored)),
            _ => malformed(bytes),
        },
        [0x83, 0x0d, rest @ ..] => {
            let Some((upload, rest)) = read_head(rest, 0) else {
                malformed(bytes)
            };
            match read_head(rest, 3) {
                Some((len, text)) if text.len() as u64 == len => {
                    let problem = String::from_utf8(text.to_vec()).expect("a UTF-8 problem");
                    Some((upload, Answer::Refused(problem)))
                }
                _ => malformed(bytes),
            }
        }
        _ => None,
    }
}

/// Uploads `item` in `channel` of `space` on `socket`, a connection the server may close; gives
/// its answer, or `None` if the connection ends first.
pub fn attempt(
    socket: &mut Socket,
    space: SpaceId,
    channel: ChannelId,
    item: &Item,
) -> Option<Answer> {
    let upload = next_upload();
    let frame = history_frame(upload, space, channel, item);
    // A connection the server closed can fail the send.
    if catch_unwind(AssertUnwindSafe(|| socket.frame(frame))).is_err() {
        return None;
    }
    let deadline = Instant::now() + WAIT;
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Binary(bytes))) => {
                if let Some((named, answer)) = answer_in(&bytes) {
                    assert_eq!(named, upload, "the upload the answer names");
                    return Some(answer);
                }
            }
            Ok(Some(Message::Close(_))) | Err(_) => return None,
            Ok(Some(_)) => {}
            Ok(None) => panic!("neither an answer nor the connection's end within {WAIT:?}"),
        }
    }
}

/// `peer` uploads `item` in the fixture's channel; gives the answer.
#[track_caller]
pub fn upload(peer: &mut Peer, f: &Fixture, item: &Item) -> Answer {
    upload_to(peer, f.team.space, f.general, item)
}

/// `peer` uploads `item` in `channel` of `space`; gives the answer.
#[track_caller]
pub fn upload_to(peer: &mut Peer, space: SpaceId, channel: ChannelId, item: &Item) -> Answer {
    attempt(&mut peer.socket, space, channel, item)
        .unwrap_or_else(|| panic!("the connection ended instead of an answer to an upload"))
}

/// Checks that the server stores `item` from `peer`.
#[track_caller]
pub fn stores(peer: &mut Peer, f: &Fixture, item: &Item, what: &str) {
    let answer = upload(peer, f, item);
    assert_eq!(answer, Answer::Stored, "{what}: history-stored");
}

/// Checks that the server answers `peer`'s upload of `item`, whatever the answer: a later item
/// that changes nothing.
#[track_caller]
pub fn answers(peer: &mut Peer, f: &Fixture, item: &Item) {
    upload(peer, f, item);
}

/// Checks that `answer` refuses an upload of `item` and that the server stored nothing of it.
#[track_caller]
pub fn assert_refused_upload(answer: &Answer, f: &Fixture, item: &Item, what: &str) {
    assert!(
        matches!(answer, Answer::Refused(_)),
        "{what}: history-refused, not {answer:?}"
    );
    assert_nowhere(f.db(), item.bytes());
}

/// Checks that the server refuses `item` from `peer` in the fixture's channel and stores nothing
/// of it.
#[track_caller]
pub fn refuses(peer: &mut Peer, f: &Fixture, item: &Item, what: &str) {
    let answer = upload(peer, f, item);
    assert_refused_upload(&answer, f, item, what);
}

/// The path of the fixture channel's history of `kind`.
fn path(f: &Fixture, kind: &str) -> String {
    format!(
        "/spaces/{}/channels/{}/history/{kind}",
        f.team.space, f.general
    )
}

/// `GET` of the fixture channel's history of `kind`.
pub fn fetch(session: &Session, f: &Fixture, kind: &str) -> Response {
    session.call("get", &path(f, kind), None)
}

/// What `session` gets of the fixture channel's history of `kind`: its items' bytes, sorted.
#[track_caller]
pub fn given(session: &Session, f: &Fixture, kind: &str) -> Vec<Vec<u8>> {
    let response = fetch(session, f, kind);
    assert_eq!(response.status, 200, "{kind}: {}", text(&response));
    let body = json(&response);
    let items = body[kind]
        .as_array()
        .unwrap_or_else(|| panic!("no {kind} in {body}"));
    let mut items: Vec<Vec<u8>> = (items.iter())
        .map(|item| {
            unb64(
                item.as_str()
                    .unwrap_or_else(|| panic!("bytes, not {item}, in {kind}")),
            )
        })
        .collect();
    items.sort();
    items
}

/// Checks that `session` gets exactly `expected` of the fixture channel's history of `kind`, byte
/// for byte, each once.
#[track_caller]
pub fn assert_given(session: &Session, f: &Fixture, kind: &str, expected: &[&Item], what: &str) {
    let mut expected: Vec<Vec<u8>> = expected.iter().map(|item| item.bytes().to_vec()).collect();
    expected.sort();
    let found = given(session, f, kind);
    assert!(
        found == expected,
        "{what}: {kind} given are not those expected: {} given, {} expected",
        found.len(),
        expected.len()
    );
}

/// Checks that `session` gets exactly `links`, `wraps` and `entries` of the fixture channel's
/// history, as [`assert_given`] does.
#[track_caller]
pub fn assert_history(
    session: &Session,
    f: &Fixture,
    [links, wraps, entries]: [&[&Item]; 3],
    what: &str,
) {
    assert_given(session, f, LINKS, links, what);
    assert_given(session, f, WRAPS, wraps, what);
    assert_given(session, f, ENTRIES, entries, what);
}

/// Checks that the server refuses `session` the fixture channel's history of every kind.
#[track_caller]
pub fn assert_none_given(session: &Session, f: &Fixture, what: &str) {
    for kind in [LINKS, WRAPS, ENTRIES] {
        assert_refused(&fetch(session, f, kind), &format!("{what}: {kind}"));
    }
}

/// `n` random bytes.
fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    for chunk in bytes.chunks_mut(32) {
        chunk.copy_from_slice(&random()[..chunk.len()]);
    }
    bytes
}

/// A link of `channel`'s epoch `epoch` to `prev` with a key of its own, as the server sees one: it
/// cannot open it.
pub fn link(channel: ChannelId, epoch: u64, prev: u64) -> Item {
    Item::Link(link_bytes(channel, epoch, prev))
}

/// The bytes of a link as [`link`] makes it, of any epoch and `prev`, below their bounds too.
pub fn link_bytes(channel: ChannelId, epoch: u64, prev: u64) -> Vec<u8> {
    HistoryLink {
        channel,
        epoch,
        prev,
        nonce: random_bytes(),
        ciphertext: random_bytes::<32>().to_vec(),
        unknown: Default::default(),
    }
    .to_cbor()
}

/// The link of `channel`'s epoch `epoch` to `prev`, sealed under `key`, `epoch`'s key, holding
/// `prev_key`.
pub fn sealed_link(
    key: &HistoryKey,
    channel: ChannelId,
    epoch: u64,
    prev: u64,
    prev_key: &HistoryKey,
) -> Item {
    let link = (key.seal_link(channel, epoch, prev, prev_key)).expect("a link");
    Item::Link(link.to_cbor())
}

/// A wrap of a key of its own of `channel`'s epoch `epoch` to `recipient`'s recovery key, signed
/// by `signer`'s laptop.
pub fn wrap(channel: ChannelId, epoch: u64, recipient: &Person, signer: &Person) -> Item {
    let key = HistoryKey::from_bytes(&random_bytes());
    sealed_wrap(&key, channel, epoch, recipient, signer)
}

/// The wrap of `key`, the key of `channel`'s epoch `epoch`, to `recipient`'s recovery key, a key
/// the test makes for it, signed by `signer`'s laptop.
pub fn sealed_wrap(
    key: &HistoryKey,
    channel: ChannelId,
    epoch: u64,
    recipient: &Person,
    signer: &Person,
) -> Item {
    let to = HpkeKey::generate().public_key();
    let signing = SigningKey::from_bytes(&signer.laptop.signing.secret);
    let wrap = key.wrap(
        channel,
        epoch,
        WrapRecipient::Recovery(recipient.account),
        &to,
        &signing,
    );
    Item::Wrap(wrap.expect("a wrap").to_cbor())
}

/// An entry of `channel`'s epoch `epoch` under a key of its own, as the server sees one: it cannot
/// open it; its upload names `msg_id`.
pub fn entry_of(msg_id: MessageId, channel: ChannelId, epoch: u64) -> Item {
    let entry = HistoryEntry {
        channel,
        epoch,
        nonce: random_bytes(),
        ciphertext: random_bytes::<64>().to_vec(),
        unknown: Default::default(),
    };
    Item::Entry(msg_id, entry.to_cbor())
}

/// A link of the fixture channel's epoch 2 to 1, a wrap of epoch 2 to `recipient` signed by
/// `signer`'s laptop and an entry of epoch 2, as [`link`], [`wrap`] and [`entry`] make them.
pub fn of_epoch_2(f: &Fixture, recipient: &Person, signer: &Person) -> [Item; 3] {
    let g = f.general;
    [link(g, 2, 1), wrap(g, 2, recipient, signer), entry(g, 2)]
}

/// As [`entry_of`], with a new `msg_id`.
pub fn entry(channel: ChannelId, epoch: u64) -> Item {
    entry_of(MessageId::generate(), channel, epoch)
}

/// The scene of the Delivery Service's tests, with a revision in which every channel member reads
/// the history from before it joined, and the channel's group formed at epoch 1: alice, bob and
/// carol are its members, dave is a channel member but not in the group, eve is outside the space.
/// GitHub user IDs from `github` on.
pub fn formed(github: u64) -> (Scene, Formed) {
    formed_with(github, &[])
}

/// As [`formed`], on a server with the extra arguments `args`.
pub fn formed_with(github: u64, args: &[&str]) -> (Scene, Formed) {
    let mut s = Scene::with(github, args);
    let t = &s.fixture.team;
    let accounts = [
        t.alice.account,
        t.bob.account,
        t.carol.account,
        t.dave.account,
    ];
    revise_members(&mut s, |members| {
        *members = (accounts.iter())
            .map(|&account| ChannelMember {
                account,
                history: true,
            })
            .collect();
    });
    let formed = s.form();
    (s, formed)
}

/// Stores the next revision, signed by alice's laptop, with `change` applied to the members of the
/// scene's channel.
pub fn revise_members(s: &mut Scene, change: impl FnOnce(&mut Vec<ChannelMember>)) {
    let general = s.fixture.general;
    s.fixture.revise(|body| {
        let channel = (body.channels.iter_mut())
            .find(|channel| channel.id == general)
            .expect("the channel");
        change(&mut channel.members);
    });
}

/// bob commits an empty Commit, which the server stores, and uploads the GroupInfo of the new
/// epoch; alice and carol process it. Gives the new epoch.
#[track_caller]
pub fn bob_rotates(s: &mut Scene) -> u64 {
    let f = &s.fixture;
    let commit = s.bob.commits(f, empty);
    for peer in [&mut s.alice, &mut s.carol] {
        peer.process(f, &commit);
    }
    (s.bob.mls.epoch(f.general)).expect("bob's epoch")
}
