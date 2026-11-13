//! A CBOR writer for the fixtures, apart from the crates under test: values encode with the core
//! deterministic encoding of `docs/protocol/README.md`, "Encoding".

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Uint(u64),
    Bytes(Vec<u8>),
    Text(String),
    Bool(bool),
    Array(Vec<Value>),
    /// Integer keys; encoding sorts them bytewise by their encodings.
    Map(Vec<(i64, Value)>),
}

pub fn bytes(value: &[u8]) -> Value {
    Value::Bytes(value.to_vec())
}

pub fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

impl Value {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Uint(value) => head(0, *value),
            Self::Bytes(value) => [head(2, value.len() as u64), value.clone()].concat(),
            Self::Text(value) => [head(3, value.len() as u64), value.as_bytes().to_vec()].concat(),
            Self::Bool(value) => vec![if *value { 0xf5 } else { 0xf4 }],
            Self::Array(items) => {
                let mut out = head(4, items.len() as u64);
                out.extend(items.iter().flat_map(Value::encode));
                out
            }
            Self::Map(fields) => {
                // Key encodings are prefix-free, so sorting whole entries sorts them by key.
                let mut entries: Vec<Vec<u8>> = fields
                    .iter()
                    .map(|(key, value)| [int_key(*key), value.encode()].concat())
                    .collect();
                entries.sort();
                [head(5, entries.len() as u64), entries.concat()].concat()
            }
        }
    }
}

fn int_key(key: i64) -> Vec<u8> {
    match u64::try_from(key) {
        Ok(key) => head(0, key),
        Err(_) => head(1, (-1 - key).unsigned_abs()),
    }
}

/// The shortest head of major type `major` with argument `value`.
fn head(major: u8, value: u64) -> Vec<u8> {
    let major = major << 5;
    let (info, size) = match value {
        0..=23 => return vec![major | value as u8],
        24..=0xff => (24, 1),
        0x100..=0xffff => (25, 2),
        0x1_0000..=0xffff_ffff => (26, 4),
        _ => (27, 8),
    };
    let mut out = vec![major | info];
    out.extend(&value.to_be_bytes()[8 - size..]);
    out
}
