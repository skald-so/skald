//! Deterministic CBOR: `docs/protocol/README.md`, "Encoding".
//!
//! Decoding checks the input against the spec's rules before any field is read; encoding always
//! produces them. A decoded value keeps the map keys it does not know, so encoding it again gives
//! back the bytes it was decoded from.

use std::cmp::Ordering;
use std::fmt;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::str::FromStr;

use ciborium_ll::{Decoder, Encoder, Header};

use crate::address::{Address, AddressError, Name, NameError};
use crate::id::{Id, Kind};

/// The deepest nesting of arrays and maps the spec allows, the outermost included: "Encoding".
pub const MAX_DEPTH: usize = 64;

/// A type with one deterministic CBOR form, as the spec defines it.
pub trait Cbor: Sized {
    /// The deterministic CBOR encoding.
    fn to_cbor(&self) -> Vec<u8>;

    /// Decodes exactly one CBOR data item. Input that breaks the spec's encoding rules, or does
    /// not match the type's CDDL, is an error. Unknown map keys are kept, so
    /// `T::from_cbor(bytes)?.to_cbor() == bytes`. The exceptions: a frame of a type this version
    /// does not know decodes as [`Frame::Unknown`](crate::Frame::Unknown), which keeps only its
    /// type, and a `batch` entry of such a type as [`Push::Unknown`](crate::Push::Unknown), which
    /// keeps its bytes.
    ///
    /// # Errors
    /// Why the bytes are not a `Self`.
    fn from_cbor(bytes: &[u8]) -> Result<Self, DecodeError>;
}

/// Encoding and decoding of one type; [`Cbor`] is implemented on top of it.
pub(crate) trait Codec: Sized {
    fn encode(&self) -> Vec<u8>;
    fn decode(item: &Item<'_>) -> Result<Self, DecodeError>;
}

macro_rules! cbor {
    ($($type:ty),* $(,)?) => {$(
        impl $crate::cbor::Cbor for $type {
            fn to_cbor(&self) -> Vec<u8> {
                $crate::cbor::Codec::encode(self)
            }

            fn from_cbor(bytes: &[u8]) -> Result<Self, $crate::cbor::DecodeError> {
                $crate::cbor::Codec::decode(&$crate::cbor::parse(bytes)?)
            }
        }
    )*};
}
pub(crate) use cbor;

/// Why bytes do not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ends inside an item.
    Truncated,
    /// Bytes follow the item; holds how many.
    TrailingBytes(usize),
    /// The item breaks the spec's encoding rules at this byte offset.
    Encoding { at: usize, problem: EncodingProblem },
    /// The item is well-formed, but `field` is not what the spec defines.
    Field {
        field: &'static str,
        problem: FieldProblem,
    },
}

/// How an item breaks the spec's encoding rules: `docs/protocol/README.md`, "Encoding".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingProblem {
    /// Not well-formed CBOR: a reserved head or a break code outside an indefinite-length item.
    Malformed,
    /// A head that is not in its shortest form.
    NotShortest,
    /// An indefinite-length string, array or map.
    IndefiniteLength,
    /// A map key that does not follow the previous one in bytewise order.
    UnsortedKeys,
    /// A map key that repeats the previous one.
    DuplicateKey,
    /// A tag; the spec uses none.
    Tag,
    /// A floating-point value; the spec uses none.
    Float,
    /// A simple value other than `false`, `true` and `null`.
    SimpleValue,
    /// A text string that is not UTF-8.
    InvalidUtf8,
    /// Arrays and maps nested deeper than [`MAX_DEPTH`].
    TooDeep,
}

/// How a field differs from the spec's CDDL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldProblem {
    /// A required field is absent.
    Missing,
    /// The field has another CBOR type; holds the expected one.
    Type(&'static str),
    /// A byte string has another length.
    Length { expected: usize, actual: usize },
    /// A string's length in bytes is outside `min..=max`.
    Size {
        min: usize,
        max: usize,
        actual: usize,
    },
    /// An array has another number of items.
    Items { expected: usize, actual: usize },
    /// A list that needs at least one item is empty.
    Empty,
    /// A number that is at least 1 is 0.
    Zero,
    /// A number is below its least value.
    Below { min: u64, actual: u64 },
    /// A number is none of the values the spec defines for the field.
    Value(u64),
    /// A map key is not an integer.
    KeyType,
    /// Content carries no body.
    NoBody,
    /// Content carries more than one body.
    TwoBodies,
    /// A field is present where the spec does not allow it.
    NotAllowed,
    /// The field differs from another one it must equal; holds the other's name.
    Mismatch(&'static str),
    /// The field equals another one it must differ from; holds the other's name.
    Same(&'static str),
    /// An agent address is invalid.
    Address(AddressError),
    /// A name of an agent address is invalid.
    Name(NameError),
    /// A byte string that holds a CBOR item does not decode.
    Embedded(Box<DecodeError>),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("the CBOR item is truncated"),
            Self::TrailingBytes(count) => write!(f, "{count} bytes follow the CBOR item"),
            Self::Encoding { at, problem } => write!(f, "at byte {at}: {problem}"),
            Self::Field { field, problem } => write!(f, "{field}: {problem}"),
        }
    }
}

impl fmt::Display for EncodingProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed CBOR",
            Self::NotShortest => "a head not in its shortest form",
            Self::IndefiniteLength => "an indefinite length",
            Self::UnsortedKeys => "a map key out of bytewise order",
            Self::DuplicateKey => "a repeated map key",
            Self::Tag => "a tag",
            Self::Float => "a floating-point value",
            Self::SimpleValue => "a simple value other than false, true and null",
            Self::InvalidUtf8 => "a text string that is not UTF-8",
            Self::TooDeep => "arrays and maps nested too deep",
        })
    }
}

impl fmt::Display for FieldProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("missing"),
            Self::Type(expected) => write!(f, "not {expected}"),
            Self::Length { expected, actual } => {
                write!(f, "{actual} bytes, not {expected}")
            }
            Self::Size { min, max, actual } => {
                write!(f, "{actual} bytes, not {min} to {max}")
            }
            Self::Items { expected, actual } => write!(f, "{actual} items, not {expected}"),
            Self::Empty => f.write_str("empty; at least one item is needed"),
            Self::Zero => f.write_str("0; at least 1 is needed"),
            Self::Below { min, actual } => write!(f, "{actual}; at least {min} is needed"),
            Self::Value(value) => write!(f, "{value} is not a defined value"),
            Self::KeyType => f.write_str("a map key that is not an integer"),
            Self::NoBody => f.write_str("no body"),
            Self::TwoBodies => f.write_str("more than one body"),
            Self::NotAllowed => f.write_str("not allowed here"),
            Self::Mismatch(other) => write!(f, "differs from {other}"),
            Self::Same(other) => write!(f, "the same as {other}"),
            Self::Address(error) => write!(f, "{error}"),
            Self::Name(error) => write!(f, "{error}"),
            Self::Embedded(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub(crate) fn field_error(field: &'static str, problem: FieldProblem) -> DecodeError {
    DecodeError::Field { field, problem }
}

/// A list with at least one item, for CDDL `[+ T]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NonEmpty<T>(Vec<T>);

impl<T> NonEmpty<T> {
    /// A list of one item.
    pub fn new(first: T) -> Self {
        Self(vec![first])
    }

    /// The list of `items`, or `None` if it is empty.
    pub fn from_vec(items: Vec<T>) -> Option<Self> {
        (!items.is_empty()).then_some(Self(items))
    }

    /// Adds an item at the end.
    pub fn push(&mut self, item: T) {
        self.0.push(item);
    }

    /// The items.
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T> Deref for NonEmpty<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

/// The fields of a decoded map that this version does not know, kept so that encoding gives
/// back the decoded bytes. Only decoding adds fields, so they never collide with known ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Unknown(Vec<UnknownField>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UnknownField {
    key: i128,
    raw_key: Vec<u8>,
    raw_value: Vec<u8>,
}

impl Unknown {
    /// Whether there are no unknown fields.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The keys of the unknown fields, in encoding order.
    pub fn keys(&self) -> impl Iterator<Item = i128> + '_ {
        self.0.iter().map(|field| field.key)
    }

    /// The encoded value of the field with this key.
    pub fn get(&self, key: i128) -> Option<&[u8]> {
        self.0
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.raw_value.as_slice())
    }
}

/// A decoded item that follows the spec's encoding rules.
pub(crate) struct Item<'b> {
    raw: &'b [u8],
    node: Node<'b>,
}

enum Node<'b> {
    Uint(u64),
    /// A negative integer, -1 - n.
    Negative(u64),
    Bytes(&'b [u8]),
    Text(&'b str),
    Array(Vec<Item<'b>>),
    Map(Vec<(Item<'b>, Item<'b>)>),
    /// `false`, `true` or `null`: simple value 20, 21 or 22.
    Simple(u64),
}

/// Decodes exactly one item and checks it against the spec's encoding rules.
pub(crate) fn parse(bytes: &[u8]) -> Result<Item<'_>, DecodeError> {
    let mut at = 0;
    let item = parse_item(bytes, &mut at, 0)?;
    match bytes.len() - at {
        0 => Ok(item),
        trailing => Err(DecodeError::TrailingBytes(trailing)),
    }
}

fn parse_item<'b>(bytes: &'b [u8], at: &mut usize, depth: usize) -> Result<Item<'b>, DecodeError> {
    let start = *at;
    let fail = |problem| DecodeError::Encoding { at: start, problem };
    let mut decoder = Decoder::from(&bytes[start..]);
    let header = decoder.pull().map_err(|error| match error {
        ciborium_ll::Error::Io(_) => DecodeError::Truncated,
        ciborium_ll::Error::Syntax(_) => fail(EncodingProblem::Malformed),
    })?;
    let head_len = decoder.offset();
    *at += head_len;
    let argument = match header {
        Header::Positive(value) | Header::Negative(value) => value,
        Header::Bytes(Some(len))
        | Header::Text(Some(len))
        | Header::Array(Some(len))
        | Header::Map(Some(len)) => len as u64,
        Header::Bytes(None) | Header::Text(None) | Header::Array(None) | Header::Map(None) => {
            return Err(fail(EncodingProblem::IndefiniteLength));
        }
        Header::Simple(value @ 20..=22) => u64::from(value),
        Header::Simple(_) => return Err(fail(EncodingProblem::SimpleValue)),
        Header::Tag(_) => return Err(fail(EncodingProblem::Tag)),
        Header::Float(_) => return Err(fail(EncodingProblem::Float)),
        Header::Break => return Err(fail(EncodingProblem::Malformed)),
    };
    if head_len != shortest_head_len(argument) {
        return Err(fail(EncodingProblem::NotShortest));
    }
    let node = match header {
        Header::Positive(value) => Node::Uint(value),
        Header::Negative(value) => Node::Negative(value),
        Header::Bytes(Some(len)) => Node::Bytes(take(bytes, at, len)?),
        Header::Text(Some(len)) => Node::Text(
            std::str::from_utf8(take(bytes, at, len)?)
                .map_err(|_| fail(EncodingProblem::InvalidUtf8))?,
        ),
        Header::Array(Some(len)) => {
            if depth == MAX_DEPTH {
                return Err(fail(EncodingProblem::TooDeep));
            }
            // Every item takes at least one byte, so a false length ends at `Truncated`.
            let mut items = Vec::new();
            for _ in 0..len {
                items.push(parse_item(bytes, at, depth + 1)?);
            }
            Node::Array(items)
        }
        Header::Map(Some(len)) => {
            if depth == MAX_DEPTH {
                return Err(fail(EncodingProblem::TooDeep));
            }
            let mut entries: Vec<(Item<'b>, Item<'b>)> = Vec::new();
            for _ in 0..len {
                let key_at = *at;
                let key = parse_item(bytes, at, depth + 1)?;
                if let Some((previous, _)) = entries.last() {
                    let problem = match previous.raw.cmp(key.raw) {
                        Ordering::Less => None,
                        Ordering::Equal => Some(EncodingProblem::DuplicateKey),
                        Ordering::Greater => Some(EncodingProblem::UnsortedKeys),
                    };
                    if let Some(problem) = problem {
                        return Err(DecodeError::Encoding {
                            at: key_at,
                            problem,
                        });
                    }
                }
                let value = parse_item(bytes, at, depth + 1)?;
                entries.push((key, value));
            }
            Node::Map(entries)
        }
        _ => Node::Simple(argument),
    };
    Ok(Item {
        raw: &bytes[start..*at],
        node,
    })
}

/// The length of the shortest head for this argument.
fn shortest_head_len(argument: u64) -> usize {
    match argument {
        0..24 => 1,
        24..0x100 => 2,
        0x100..0x1_0000 => 3,
        0x1_0000..0x1_0000_0000 => 5,
        _ => 9,
    }
}

fn take<'b>(bytes: &'b [u8], at: &mut usize, len: usize) -> Result<&'b [u8], DecodeError> {
    let end = at
        .checked_add(len)
        .filter(|&end| end <= bytes.len())
        .ok_or(DecodeError::Truncated)?;
    let taken = &bytes[*at..end];
    *at = end;
    Ok(taken)
}

impl<'b> Item<'b> {
    /// The bytes it was decoded from.
    pub(crate) fn raw(&self) -> &'b [u8] {
        self.raw
    }

    pub(crate) fn uint(&self, field: &'static str) -> Result<u64, DecodeError> {
        match self.node {
            Node::Uint(value) => Ok(value),
            _ => Err(field_error(
                field,
                FieldProblem::Type("an unsigned integer"),
            )),
        }
    }

    /// An unsigned integer of at least 1, for CDDL `uint .ge 1`.
    pub(crate) fn positive(&self, field: &'static str) -> Result<NonZeroU64, DecodeError> {
        NonZeroU64::new(self.uint(field)?).ok_or(field_error(field, FieldProblem::Zero))
    }

    pub(crate) fn bytes(&self, field: &'static str) -> Result<&'b [u8], DecodeError> {
        match self.node {
            Node::Bytes(bytes) => Ok(bytes),
            _ => Err(field_error(field, FieldProblem::Type("a byte string"))),
        }
    }

    /// A byte string of exactly `N` bytes, for CDDL `bstr .size N`.
    pub(crate) fn fixed<const N: usize>(
        &self,
        field: &'static str,
    ) -> Result<[u8; N], DecodeError> {
        let bytes = self.bytes(field)?;
        bytes.try_into().map_err(|_| {
            field_error(
                field,
                FieldProblem::Length {
                    expected: N,
                    actual: bytes.len(),
                },
            )
        })
    }

    pub(crate) fn text(&self, field: &'static str) -> Result<&'b str, DecodeError> {
        match self.node {
            Node::Text(text) => Ok(text),
            _ => Err(field_error(field, FieldProblem::Type("a text string"))),
        }
    }

    pub(crate) fn boolean(&self, field: &'static str) -> Result<bool, DecodeError> {
        match self.node {
            Node::Simple(20) => Ok(false),
            Node::Simple(21) => Ok(true),
            _ => Err(field_error(field, FieldProblem::Type("a boolean"))),
        }
    }

    /// A text string that is a name of an agent address.
    pub(crate) fn name(&self, field: &'static str) -> Result<Name, DecodeError> {
        Name::from_str(self.text(field)?)
            .map_err(|error| field_error(field, FieldProblem::Name(error)))
    }

    pub(crate) fn array(&self, field: &'static str) -> Result<&[Item<'b>], DecodeError> {
        match &self.node {
            Node::Array(items) => Ok(items),
            _ => Err(field_error(field, FieldProblem::Type("an array"))),
        }
    }

    /// An array of exactly `N` items.
    pub(crate) fn tuple<const N: usize>(
        &self,
        field: &'static str,
    ) -> Result<&[Item<'b>; N], DecodeError> {
        let items = self.array(field)?;
        items.try_into().map_err(|_| {
            field_error(
                field,
                FieldProblem::Items {
                    expected: N,
                    actual: items.len(),
                },
            )
        })
    }

    /// A non-empty array, each item decoded by `decode`, for CDDL `[+ T]`.
    pub(crate) fn non_empty<T>(
        &self,
        field: &'static str,
        decode: impl Fn(&Item<'b>) -> Result<T, DecodeError>,
    ) -> Result<NonEmpty<T>, DecodeError> {
        let items = self
            .array(field)?
            .iter()
            .map(decode)
            .collect::<Result<Vec<_>, _>>()?;
        NonEmpty::from_vec(items).ok_or(field_error(field, FieldProblem::Empty))
    }

    /// A map with integer keys, for CDDL `{ ... * int => any }`.
    pub(crate) fn map(&self, field: &'static str) -> Result<Fields<'_, 'b>, DecodeError> {
        let Node::Map(entries) = &self.node else {
            return Err(field_error(field, FieldProblem::Type("a map")));
        };
        let entries = entries
            .iter()
            .map(|(key, value)| {
                let key_value = match key.node {
                    Node::Uint(value) => i128::from(value),
                    Node::Negative(value) => -1 - i128::from(value),
                    _ => return Err(field_error(field, FieldProblem::KeyType)),
                };
                Ok(Some((key_value, key, value)))
            })
            .collect::<Result<_, _>>()?;
        Ok(Fields { entries })
    }

    pub(crate) fn id<K: Kind>(&self, field: &'static str) -> Result<Id<K>, DecodeError> {
        self.fixed::<16>(field).map(Id::from_bytes)
    }

    pub(crate) fn address(&self, field: &'static str) -> Result<Address, DecodeError> {
        Address::from_str(self.text(field)?)
            .map_err(|error| field_error(field, FieldProblem::Address(error)))
    }

    /// A byte string that holds one item of `T`, for CDDL `bstr .cbor T`.
    pub(crate) fn embedded<T: Codec>(&self, field: &'static str) -> Result<T, DecodeError> {
        decode_embedded(self.bytes(field)?, field)
    }
}

/// `bytes` as one item of `T`, the bytes of `field`, for CDDL `bstr .cbor T`.
pub(crate) fn decode_embedded<T: Codec>(
    bytes: &[u8],
    field: &'static str,
) -> Result<T, DecodeError> {
    parse(bytes)
        .and_then(|item| T::decode(&item))
        .map_err(|error| field_error(field, FieldProblem::Embedded(Box::new(error))))
}

/// The fields of a decoded map, taken one by one; the rest are [`Unknown`].
pub(crate) struct Fields<'i, 'b> {
    entries: Vec<Option<(i128, &'i Item<'b>, &'i Item<'b>)>>,
}

impl<'i, 'b> Fields<'i, 'b> {
    pub(crate) fn take(&mut self, key: u64) -> Option<&'i Item<'b>> {
        let key = i128::from(key);
        self.entries
            .iter_mut()
            .find(|entry| matches!(entry, Some((found, _, _)) if *found == key))
            .and_then(Option::take)
            .map(|(_, _, value)| value)
    }

    pub(crate) fn required(
        &mut self,
        key: u64,
        field: &'static str,
    ) -> Result<&'i Item<'b>, DecodeError> {
        self.take(key)
            .ok_or(field_error(field, FieldProblem::Missing))
    }

    /// Whether a field with this key is left.
    pub(crate) fn has(&self, key: u64) -> bool {
        let key = i128::from(key);
        self.entries
            .iter()
            .any(|entry| matches!(entry, Some((found, _, _)) if *found == key))
    }

    /// Whether every field has been taken.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.iter().all(Option::is_none)
    }

    /// The fields not taken.
    pub(crate) fn unknown(self) -> Unknown {
        Unknown(
            self.entries
                .into_iter()
                .flatten()
                .map(|(key, raw_key, raw_value)| UnknownField {
                    key,
                    raw_key: raw_key.raw.to_vec(),
                    raw_value: raw_value.raw.to_vec(),
                })
                .collect(),
        )
    }
}

fn head(out: &mut Vec<u8>, header: Header) {
    let Ok(()) = Encoder::from(out).push(header);
}

pub(crate) fn uint(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    head(&mut out, Header::Positive(value));
    out
}

pub(crate) fn bytes(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    head(&mut out, Header::Bytes(Some(value.len())));
    out.extend_from_slice(value);
    out
}

pub(crate) fn text(value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    head(&mut out, Header::Text(Some(value.len())));
    out.extend_from_slice(value.as_bytes());
    out
}

pub(crate) fn boolean(value: bool) -> Vec<u8> {
    vec![if value { 0xf5 } else { 0xf4 }]
}

pub(crate) fn id<K: Kind>(value: &Id<K>) -> Vec<u8> {
    bytes(value.as_bytes())
}

pub(crate) fn array(items: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    let items: Vec<_> = items.into_iter().collect();
    let mut out = Vec::new();
    head(&mut out, Header::Array(Some(items.len())));
    out.extend(items.into_iter().flatten());
    out
}

/// What a signature with `label` covers: the encoded CBOR array of `label` and `parts` as byte
/// strings (`docs/protocol/README.md`, "Conventions").
pub fn signed_data(label: &str, parts: &[&[u8]]) -> Vec<u8> {
    array(std::iter::once(text(label)).chain(parts.iter().map(|part| bytes(part))))
}

/// A map being encoded: known fields and the [`Unknown`] ones, written in bytewise key order.
pub(crate) struct MapOut {
    entries: Vec<(Vec<u8>, Vec<u8>)>,
}

impl MapOut {
    pub(crate) fn new(unknown: &Unknown) -> Self {
        Self {
            entries: unknown
                .0
                .iter()
                .map(|field| (field.raw_key.clone(), field.raw_value.clone()))
                .collect(),
        }
    }

    pub(crate) fn put(mut self, key: u64, value: Vec<u8>) -> Self {
        self.entries.push((uint(key), value));
        self
    }

    pub(crate) fn put_some(self, key: u64, value: Option<Vec<u8>>) -> Self {
        match value {
            Some(value) => self.put(key, value),
            None => self,
        }
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut out = Vec::new();
        head(&mut out, Header::Map(Some(self.entries.len())));
        for (key, value) in self.entries {
            out.extend(key);
            out.extend(value);
        }
        out
    }
}
