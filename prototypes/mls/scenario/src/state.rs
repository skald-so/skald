//! A length-prefixed encoding for the prototypes' saved state.

use crate::Result;

/// Appends fields to a saved state.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// A byte string, prefixed with its length.
    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.u64(value.len() as u64);
        self.0.extend_from_slice(value);
        self
    }

    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// Reads the fields of a saved state in the order a [`Writer`] wrote them.
pub struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    pub fn new(state: &'a [u8]) -> Self {
        Self(state)
    }

    pub fn u64(&mut self) -> Result<u64> {
        let Some((value, rest)) = self.0.split_first_chunk::<8>() else {
            return Err("saved state: truncated".into());
        };
        self.0 = rest;
        Ok(u64::from_be_bytes(*value))
    }

    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = usize::try_from(self.u64()?)?;
        if len > self.0.len() {
            return Err("saved state: truncated".into());
        }
        let (value, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(value)
    }

    /// Fails if anything is left unread.
    pub fn finish(self) -> Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err("saved state: trailing bytes".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_was_written() {
        let mut writer = Writer::default();
        writer.u64(3).bytes(b"key").bytes(b"");
        let state = writer.finish();

        let mut reader = Reader::new(&state);
        assert_eq!(reader.u64().unwrap(), 3);
        assert_eq!(reader.bytes().unwrap(), b"key");
        assert_eq!(reader.bytes().unwrap(), b"");
        reader.finish().unwrap();
    }

    #[test]
    fn rejects_truncated_and_trailing_bytes() {
        let mut writer = Writer::default();
        writer.bytes(b"value");
        let state = writer.finish();

        assert!(Reader::new(&state[..state.len() - 1]).bytes().is_err());
        assert!(Reader::new(&state[..4]).u64().is_err());

        let mut reader = Reader::new(&state);
        reader.u64().unwrap();
        assert!(reader.finish().is_err());
    }
}
