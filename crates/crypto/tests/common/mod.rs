//! Helpers the test targets share.

/// Bytes from hex digits; whitespace between them is skipped.
pub fn hex(digits: &str) -> Vec<u8> {
    let digits: String = digits.split_whitespace().collect();
    assert_eq!(digits.len() % 2, 0, "{digits}");
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).unwrap())
        .collect()
}

pub fn hex32(digits: &str) -> [u8; 32] {
    hex(digits).try_into().unwrap()
}
