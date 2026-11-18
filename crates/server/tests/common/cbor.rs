//! CBOR items as the tests write them, byte by byte, apart from the encoder under test.

use skald_crypto::SigningKey;

/// The head of a CBOR item of major type `major` whose argument is `arg`, in the shortest form.
pub fn head(major: u8, arg: u64) -> Vec<u8> {
    let top = major << 5;
    match arg {
        0..=23 => vec![top | arg as u8],
        24..=0xff => vec![top | 24, arg as u8],
        0x100..=0xffff => [vec![top | 25], (arg as u16).to_be_bytes().to_vec()].concat(),
        0x1_0000..=0xffff_ffff => [vec![top | 26], (arg as u32).to_be_bytes().to_vec()].concat(),
        _ => [vec![top | 27], arg.to_be_bytes().to_vec()].concat(),
    }
}

/// `bytes` as a CBOR byte string.
pub fn byte_item(bytes: &[u8]) -> Vec<u8> {
    let mut item = head(2, u64::try_from(bytes.len()).expect("a length of 64 bits"));
    item.extend_from_slice(bytes);
    item
}

/// The CBOR array of the encoded `items`.
pub fn array_of(items: &[Vec<u8>]) -> Vec<u8> {
    let mut array = head(4, u64::try_from(items.len()).expect("a length of 64 bits"));
    for item in items {
        array.extend_from_slice(item);
    }
    array
}

/// The signed object `[body, signature]` of the encoded `body`, signed by `key` over
/// `[label, body]`.
pub fn signed(body: &[u8], key: &SigningKey, label: &str) -> Vec<u8> {
    array_of(&[byte_item(body), byte_item(&key.sign(label, &[body]))])
}

/// The CBOR unsigned integer `n`.
pub fn uint(n: u64) -> Vec<u8> {
    head(0, n)
}
