//! HMAC-SHA256 fingerprints of a repository's paths, hunks and symbols, computed with the space
//! key.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::symbols::Symbol;

/// The space key: a secret every device of the space holds and the server does not.
#[derive(Clone)]
pub struct SpaceKey([u8; 32]);

impl SpaceKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// The repository a fingerprint belongs to: the same on every clone of it, so that only its
/// clones' fingerprints match. It goes into the HMAC only; the server never sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope(String);

impl Scope {
    pub fn new(scope: impl Into<String>) -> Self {
        Self(scope.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An HMAC fingerprint, all the server learns of a path, a hunk or a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub [u8; 32]);

/// The label that separates path fingerprints from other HMACs of the space key.
pub const PATH_LABEL: &str = "skald edit path";

/// The fingerprint of `path`, relative to the repository root, in the repository `scope`:
/// HMAC-SHA256 with the space key over the label, the scope and the path, each prefixed with its
/// length as a big-endian `u64`.
pub fn path(key: &SpaceKey, scope: &Scope, path: &str) -> Fingerprint {
    hmac(
        key,
        &[
            PATH_LABEL.as_bytes(),
            scope.as_str().as_bytes(),
            path.as_bytes(),
        ],
    )
}

/// The label that separates hunk fingerprints from other HMACs of the space key.
pub const HUNK_LABEL: &str = "skald edit hunk";

/// The fingerprint of a boundary a hunk touches between two lines of the file at `path` in the
/// merge base: `before` and `after`, each a line with its line ending, or empty at the start or
/// the end of the file. HMAC-SHA256 with the space key over the label, the scope, the path and
/// the two lines, each prefixed with its length as a big-endian `u64`.
pub fn hunk(key: &SpaceKey, scope: &Scope, path: &str, before: &[u8], after: &[u8]) -> Fingerprint {
    let fields = [
        HUNK_LABEL.as_bytes(),
        scope.as_str().as_bytes(),
        path.as_bytes(),
        before,
        after,
    ];
    hmac(key, &fields)
}

/// The label that separates symbol fingerprints from other HMACs of the space key.
pub const SYMBOL_LABEL: &str = "skald edit symbol";

/// The fingerprint of `symbol` of the file at `path`: HMAC-SHA256 with the space key over the
/// label, the scope, the path, and the kind and the name of each symbol, outermost first, each
/// prefixed with its length as a big-endian `u64`.
pub fn symbol(key: &SpaceKey, scope: &Scope, path: &str, symbol: &Symbol) -> Fingerprint {
    let mut fields = vec![
        SYMBOL_LABEL.as_bytes(),
        scope.as_str().as_bytes(),
        path.as_bytes(),
    ];
    for (kind, name) in &symbol.0 {
        fields.extend([kind.as_bytes(), name.as_bytes()]);
    }
    hmac(key, &fields)
}

/// HMAC-SHA256 with the space key over `fields`, each prefixed with its length.
fn hmac(key: &SpaceKey, fields: &[&[u8]]) -> Fingerprint {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key.0).expect("HMAC takes any key");
    for field in fields {
        mac.update(&(field.len() as u64).to_be_bytes());
        mac.update(field);
    }
    Fingerprint(mac.finalize().into_bytes().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scope `0123abcd` and the path `src/lib.rs`, each after its length.
    const SCOPE_AND_PATH: &[u8] = b"\0\0\0\0\0\0\0\x080123abcd\0\0\0\0\0\0\0\x0asrc/lib.rs";

    /// HMAC-SHA256 with the key `[7; 32]` over `input`, written out byte by byte.
    fn want(input: &[&[u8]]) -> Fingerprint {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&[7; 32]).unwrap();
        for bytes in input {
            mac.update(bytes);
        }
        Fingerprint(mac.finalize().into_bytes().into())
    }

    fn key() -> SpaceKey {
        SpaceKey::from_bytes([7; 32])
    }

    fn scope() -> Scope {
        Scope::new("0123abcd")
    }

    #[test]
    fn path_fingerprint_is_hmac_sha256_over_length_prefixed_fields() {
        let label = b"\0\0\0\0\0\0\0\x0fskald edit path";
        assert_eq!(
            path(&key(), &scope(), "src/lib.rs"),
            want(&[label, SCOPE_AND_PATH])
        );
    }

    #[test]
    fn hunk_fingerprint_is_hmac_sha256_over_length_prefixed_fields() {
        let label = b"\0\0\0\0\0\0\0\x0fskald edit hunk";
        let lines = b"\0\0\0\0\0\0\0\x02}\n\0\0\0\0\0\0\0\0";
        assert_eq!(
            hunk(&key(), &scope(), "src/lib.rs", b"}\n", b""),
            want(&[label, SCOPE_AND_PATH, lines])
        );
    }

    #[test]
    fn symbol_fingerprint_is_hmac_sha256_over_length_prefixed_fields() {
        let label = b"\0\0\0\0\0\0\0\x11skald edit symbol";
        let outer = b"\0\0\0\0\0\0\0\x09impl_item\0\0\0\0\0\0\0\x05Store";
        let inner = b"\0\0\0\0\0\0\0\x0dfunction_item\0\0\0\0\0\0\0\x07reserve";
        let reserve = Symbol(vec![
            ("impl_item".into(), "Store".into()),
            ("function_item".into(), "reserve".into()),
        ]);
        assert_eq!(
            symbol(&key(), &scope(), "src/lib.rs", &reserve),
            want(&[label, SCOPE_AND_PATH, outer, inner])
        );
    }

    #[test]
    fn fields_do_not_run_into_each_other() {
        let key = SpaceKey::from_bytes([7; 32]);
        assert_ne!(
            path(&key, &Scope::new("ab"), "c"),
            path(&key, &Scope::new("a"), "bc")
        );
        let scope = Scope::new("ab");
        assert_ne!(
            hunk(&key, &scope, "f", b"a\n", b"b\n"),
            hunk(&key, &scope, "f", b"a\nb\n", b"")
        );
    }
}
