//! The anchors objects the server holds, read byte by byte apart from the code under test, and what
//! the tests expect of them.

use std::collections::BTreeMap;

use skald_proto::{AccountId, DeviceId, Id, Kind, SpaceId};

use crate::cloud::{Cloud, unb64};
use crate::world::Person;

/// The request that stores an anchors object, as its line starts.
pub const ANCHORS_REQUEST: &str = "POST /account/anchors ";
/// The account's anchors objects: `GET`.
const ANCHORS: &str = "/account/anchors";
/// The label of an anchors object's signature.
const LABEL: &str = "skald anchors";

/// A CBOR item of the forms an anchors object holds.
#[derive(Debug)]
enum Item {
    Uint(u64),
    Bytes(Vec<u8>),
    Array(Vec<Item>),
    Map(Vec<(u64, Item)>),
}

/// The item at the start of `input`, and what follows it; fails on any other form.
fn read(input: &[u8]) -> (Item, &[u8]) {
    let (&initial, mut rest) = input.split_first().expect("a CBOR item, not the end");
    let info = initial & 0x1f;
    let argument = if info < 24 {
        u64::from(info)
    } else {
        assert!(info < 28, "a head with additional information {info}");
        let (be, after) = rest.split_at(1 << (info - 24));
        rest = after;
        be.iter()
            .fold(0, |value, byte| value << 8 | u64::from(*byte))
    };
    let count = usize::try_from(argument).expect("a length that fits");
    let item = match initial >> 5 {
        0 => Item::Uint(argument),
        2 => {
            let (content, after) = rest.split_at(count);
            rest = after;
            Item::Bytes(content.to_vec())
        }
        4 => Item::Array(
            (0..count)
                .map(|_| {
                    let (one, after) = read(rest);
                    rest = after;
                    one
                })
                .collect(),
        ),
        5 => Item::Map(
            (0..count)
                .map(|_| {
                    let (key, after) = read(rest);
                    let (value, after) = read(after);
                    rest = after;
                    match key {
                        Item::Uint(key) => (key, value),
                        other => panic!("a map key that is no unsigned integer: {other:?}"),
                    }
                })
                .collect(),
        ),
        major => panic!("an item of major type {major}"),
    };
    (item, rest)
}

/// The one item `input` holds.
fn whole(input: &[u8]) -> Item {
    let (item, rest) = read(input);
    assert!(rest.is_empty(), "bytes after the item: {rest:?}");
    item
}

fn byte_string(item: &Item) -> &[u8] {
    match item {
        Item::Bytes(bytes) => bytes,
        other => panic!("not a byte string: {other:?}"),
    }
}

fn id<K: Kind>(item: &Item) -> Id<K> {
    Id::from_bytes(byte_string(item).try_into().expect("an ID of 16 bytes"))
}

/// An anchors object as the server holds it, decoded.
#[derive(Debug)]
pub struct Held {
    pub account: AccountId,
    pub device: DeviceId,
    /// The hash of the anchored revision of each space.
    pub anchors: BTreeMap<SpaceId, [u8; 32]>,
    pub personal: Option<SpaceId>,
    body: Vec<u8>,
    signature: [u8; 64],
}

impl Held {
    /// The encoded anchors object `bytes`, decoded; fails unless it holds only the spec's fields,
    /// each once.
    fn of(bytes: &[u8]) -> Self {
        let Item::Array(parts) = whole(bytes) else {
            panic!("an anchors object that is no array: {bytes:?}");
        };
        let [body, signature] = parts.as_slice() else {
            panic!("an anchors object of {} items", parts.len());
        };
        let body = byte_string(body).to_vec();
        let Item::Map(fields) = whole(&body) else {
            panic!("an anchors body that is no map");
        };
        let keys: Vec<u64> = fields.iter().map(|(key, _)| *key).collect();
        assert!(
            keys == [1, 2, 3] || keys == [1, 2, 3, 4],
            "an anchors body of the fields {keys:?}"
        );
        let field = |key| &fields.iter().find(|(k, _)| *k == key).expect("a field").1;
        let Item::Array(listed) = field(3) else {
            panic!("anchors that are no array");
        };
        let mut anchors = BTreeMap::new();
        for anchor in listed {
            let Item::Array(pair) = anchor else {
                panic!("an anchor that is no array: {anchor:?}");
            };
            let [space, revision] = pair.as_slice() else {
                panic!("an anchor of {} items", pair.len());
            };
            let revision = byte_string(revision)
                .try_into()
                .expect("a hash of 32 bytes");
            let space = id(space);
            assert!(
                anchors.insert(space, revision).is_none(),
                "{space} anchored twice"
            );
        }
        Held {
            account: id(field(1)),
            device: id(field(2)),
            anchors,
            personal: (keys.len() == 4).then(|| id(field(4))),
            signature: byte_string(signature)
                .try_into()
                .expect("a signature of 64 bytes"),
            body,
        }
    }
}

/// The hash of the revision of `space` that the anchors object the server holds of `person`'s
/// device names, if the server holds one that names `space`.
pub fn anchored(cloud: &Cloud, person: &Person, space: SpaceId) -> Option<[u8; 32]> {
    (held(cloud, &person.token).into_iter())
        .find(|object| object.device == person.device)
        .and_then(|object| object.anchors.get(&space).copied())
}

/// The anchors objects the server gives the session `token`, straight from the server, decoded.
pub fn held(cloud: &Cloud, token: &str) -> Vec<Held> {
    let (status, body) = cloud.get_as(ANCHORS, token);
    assert_eq!(status, 200, "GET {ANCHORS}: {body}");
    body["anchors"]
        .as_array()
        .unwrap_or_else(|| panic!("GET {ANCHORS}: {body}"))
        .iter()
        .map(|object| {
            Held::of(&unb64(
                object.as_str().expect("an anchors object in base64url"),
            ))
        })
        .collect()
}

/// The ID of `person`'s personal space, the space of the account named its username.
pub fn personal_space(cloud: &Cloud, person: &Person) -> SpaceId {
    let spaces = cloud.spaces(&person.token);
    spaces
        .iter()
        .find_map(|(id, name)| (name == person.handle.as_str()).then_some(*id))
        .unwrap_or_else(|| panic!("no personal space: {spaces:?}"))
}

/// The hash of the latest revision the server holds of each of `person`'s spaces.
pub fn latest(cloud: &Cloud, person: &Person) -> BTreeMap<SpaceId, [u8; 32]> {
    cloud
        .spaces(&person.token)
        .into_iter()
        .map(|(space, _)| {
            let (_, last) = cloud
                .revisions(&person.token, space)
                .pop()
                .unwrap_or_else(|| panic!("no revision of {space}"));
            (space, last.hash())
        })
        .collect()
}

/// Fails unless the server holds, of `person`'s account, one anchors object, that of the device of
/// `person`'s machine, signed by it over `["skald anchors", body]`, which anchors each space of
/// `anchors` at its revision and names `personal` as the personal space.
#[track_caller]
pub fn assert_anchors(
    cloud: &Cloud,
    person: &Person,
    anchors: &BTreeMap<SpaceId, [u8; 32]>,
    personal: SpaceId,
) {
    let mut objects = held(cloud, &person.token);
    assert_eq!(objects.len(), 1, "not one anchors object: {objects:?}");
    let object = objects.remove(0);
    assert_eq!(
        (object.account, object.device),
        (person.account, person.device),
        "not the machine's anchors object"
    );
    skald_crypto::verify(
        &person.key.public_key(),
        LABEL,
        &[&object.body],
        &object.signature,
    )
    .expect("the machine's signature over the anchors label");
    assert_eq!(&object.anchors, anchors, "not the anchored revisions");
    assert_eq!(object.personal, Some(personal), "not the personal space");
}
