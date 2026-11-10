//! Deterministic CBOR of `docs/protocol/README.md`, "Encoding", for the objects the scenarios
//! carry: integers, byte and text strings, arrays, maps and the simple values. Tags and floats are
//! not used.

use crate::Result;

/// A CBOR data item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Uint(u64),
    /// The negative integer `-1 - n`.
    Nint(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    /// Entries in the order they are encoded.
    Map(Vec<(Value, Value)>),
    /// `false`, `true`, `null` and `undefined`: 20 to 23.
    Simple(u8),
}

impl Value {
    /// The item's encoding: shortest forms and definite lengths; map entries as given.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Value::Uint(n) => head(out, 0, *n),
            Value::Nint(n) => head(out, 1, *n),
            Value::Bytes(bytes) => {
                head(out, 2, bytes.len() as u64);
                out.extend_from_slice(bytes);
            }
            Value::Text(text) => {
                head(out, 3, text.len() as u64);
                out.extend_from_slice(text.as_bytes());
            }
            Value::Array(items) => {
                head(out, 4, items.len() as u64);
                for item in items {
                    item.write(out);
                }
            }
            Value::Map(entries) => {
                head(out, 5, entries.len() as u64);
                for (key, value) in entries {
                    key.write(out);
                    value.write(out);
                }
            }
            Value::Simple(value) => out.push(0xe0 | value),
        }
    }

    /// Decodes one item that is the whole input. Rejects what the deterministic encoding forbids:
    /// longer forms than needed, indefinite lengths, map keys out of bytewise order or repeated.
    pub fn decode(input: &[u8]) -> Result<Value> {
        let mut reader = Reader(input);
        let value = reader.value()?;
        if reader.0.is_empty() {
            Ok(value)
        } else {
            Err("CBOR: trailing bytes".into())
        }
    }
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let major = major << 5;
    match n {
        0..24 => out.push(major | n as u8),
        24..0x100 => out.extend_from_slice(&[major | 24, n as u8]),
        0x100..0x1_0000 => {
            out.push(major | 25);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        0x1_0000..0x1_0000_0000 => {
            out.push(major | 26);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
        _ => {
            out.push(major | 27);
            out.extend_from_slice(&n.to_be_bytes());
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.0.len() {
            return Err("CBOR: truncated".into());
        }
        let (value, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(value)
    }

    /// The major type and the argument of the next item's head, in its shortest form.
    fn head(&mut self) -> Result<(u8, u64)> {
        let first = self.take(1)?[0];
        let (major, info) = (first >> 5, first & 0x1f);
        let (n, min) = match info {
            0..24 => return Ok((major, info.into())),
            24 => (self.take(1)?[0].into(), 24),
            25 => (u16::from_be_bytes(self.take(2)?.try_into()?).into(), 0x100),
            26 => (
                u32::from_be_bytes(self.take(4)?.try_into()?).into(),
                0x1_0000,
            ),
            27 => (u64::from_be_bytes(self.take(8)?.try_into()?), 0x1_0000_0000),
            _ => return Err("CBOR: indefinite length or reserved head".into()),
        };
        if n < min {
            return Err("CBOR: not the shortest form".into());
        }
        Ok((major, n))
    }

    fn len(&mut self, n: u64) -> Result<usize> {
        let len = usize::try_from(n)?;
        if len > self.0.len() {
            return Err("CBOR: truncated".into());
        }
        Ok(len)
    }

    fn value(&mut self) -> Result<Value> {
        let (major, n) = self.head()?;
        Ok(match major {
            0 => Value::Uint(n),
            1 => Value::Nint(n),
            2 => {
                let len = self.len(n)?;
                Value::Bytes(self.take(len)?.to_vec())
            }
            3 => {
                let len = self.len(n)?;
                Value::Text(String::from_utf8(self.take(len)?.to_vec())?)
            }
            4 => {
                // Each item takes at least one byte.
                let len = self.len(n)?;
                Value::Array((0..len).map(|_| self.value()).collect::<Result<_>>()?)
            }
            5 => {
                let len = self.len(n)?;
                let mut entries = Vec::with_capacity(len);
                let mut last: Option<&[u8]> = None;
                for _ in 0..len {
                    let start = self.0;
                    let key = self.value()?;
                    let encoded = &start[..start.len() - self.0.len()];
                    if last.is_some_and(|last| last >= encoded) {
                        return Err("CBOR: map keys out of order or repeated".into());
                    }
                    last = Some(encoded);
                    entries.push((key, self.value()?));
                }
                Value::Map(entries)
            }
            7 if (20..24).contains(&n) => Value::Simple(n as u8),
            _ => return Err(format!("CBOR: major type {major} with {n} is not used").into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: Value, encoded: &[u8]) {
        assert_eq!(value.encode(), encoded, "{value:?}");
        assert_eq!(Value::decode(encoded).unwrap(), value, "{encoded:02x?}");
    }

    #[test]
    fn encodes_integers_in_their_shortest_form() {
        round_trip(Value::Uint(0), &[0x00]);
        round_trip(Value::Uint(23), &[0x17]);
        round_trip(Value::Uint(24), &[0x18, 24]);
        round_trip(Value::Uint(255), &[0x18, 0xff]);
        round_trip(Value::Uint(256), &[0x19, 0x01, 0x00]);
        round_trip(Value::Uint(65535), &[0x19, 0xff, 0xff]);
        round_trip(Value::Uint(65536), &[0x1a, 0, 1, 0, 0]);
        round_trip(Value::Uint(0xffff_ffff), &[0x1a, 0xff, 0xff, 0xff, 0xff]);
        round_trip(Value::Uint(1 << 32), &[0x1b, 0, 0, 0, 1, 0, 0, 0, 0]);
        round_trip(Value::Nint(0), &[0x20]);
        round_trip(Value::Nint(500), &[0x39, 0x01, 0xf4]);
    }

    #[test]
    fn encodes_strings_arrays_maps_and_simple_values() {
        round_trip(Value::Bytes(vec![1, 2]), &[0x42, 1, 2]);
        round_trip(Value::Text("ab".into()), &[0x62, b'a', b'b']);
        round_trip(
            Value::Array(vec![Value::Uint(1), Value::Simple(21)]),
            &[0x82, 0x01, 0xf5],
        );
        round_trip(
            Value::Map(vec![
                (Value::Uint(1), Value::Simple(20)),
                (Value::Nint(0), Value::Simple(22)),
            ]),
            &[0xa2, 0x01, 0xf4, 0x20, 0xf6],
        );
        let long = vec![7; 300];
        round_trip(
            Value::Bytes(long.clone()),
            &[&[0x59, 0x01, 0x2c][..], &long].concat(),
        );
    }

    #[test]
    fn rejects_what_the_deterministic_encoding_forbids() {
        for (encoded, why) in [
            (&[0x18, 23][..], "a longer form than needed"),
            (&[0x19, 0x00, 0xff], "a longer form than needed"),
            (&[0x1a, 0, 0, 0xff, 0xff], "a longer form than needed"),
            (
                &[0x1b, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff],
                "a longer form than needed",
            ),
            (&[0x5f, 0x41, 0, 0xff], "an indefinite length"),
            (&[0xa2, 0x02, 0x00, 0x01, 0x00], "map keys out of order"),
            (&[0xa2, 0x01, 0x00, 0x01, 0x00], "a repeated map key"),
            (
                &[0xa2, 0x20, 0x00, 0x01, 0x00],
                "map keys out of bytewise order",
            ),
            (&[0xc1, 0x00], "a tag"),
            (&[0xf9, 0x3c, 0x00], "a float"),
            (&[0xf3], "an unused simple value"),
            (&[0xf8, 0x20], "an unused simple value"),
            (&[0x42, 0x01], "a truncated string"),
            (&[0x82, 0x01], "a truncated array"),
            (&[0x81, 0x01, 0x01], "trailing bytes"),
            (&[0x62, 0xff, 0xff], "a text string that is not UTF-8"),
            (&[0x9b, 0, 0, 0, 1, 0, 0, 0, 0], "more items than bytes"),
        ] {
            assert!(Value::decode(encoded).is_err(), "{why}: {encoded:02x?}");
        }
    }
}
