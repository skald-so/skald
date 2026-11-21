//! The vectors of `docs/protocol/test-vectors.md`, each value a list item `- <item>: `<value>`` of
//! its section:
//! - "Invite key": `Secret`, and the `Private key` and `Public key` of its invite key pair, in hex;
//! - "Invite": an encoded `invite` in its `hex` block, its `key` the `Public key` of "Invite key";
//! - "Acceptance": an encoded `acceptance` in its `hex` block, of the invite of "Invite";
//! - "Invite link": `Web address`, `Invite`, the ID of the invite of "Invite", `Secret`, that of
//!   "Invite key", and `Revision`, a hash in hex, and the `Link` they make.
//!
//! The invite key and the link derive as the spec says apart from the crates, and the crates derive
//! them, decode the objects and encode them back to the same bytes. `skald-proto`'s test of the
//! vectors checks the diagnostic notation of "Invite" and "Acceptance".

use skald_crypto::{InviteLink, InviteSecret};
use skald_proto::{Acceptance, Cbor, Invite, InviteId};

use crate::cbor::Value;
use crate::common::hex;
use crate::objects::{acceptance_fields, invite_dalek, invite_fields, invite_public};

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The text of the section `## <name>`, up to the next section.
fn section(name: &str) -> &'static str {
    let heading = format!("\n## {name}\n");
    let at = DOC
        .find(&heading)
        .unwrap_or_else(|| panic!("test-vectors.md has no section {name:?}"));
    let rest = &DOC[at + heading.len()..];
    rest.find("\n## ").map_or(rest, |end| &rest[..end])
}

/// The value of the list item `- <item>: `<value>`` of the section `## <name>`.
fn item(name: &str, item: &str) -> &'static str {
    let prefix = format!("- {item}: `");
    section(name)
        .lines()
        .find_map(|line| line.trim_end().strip_prefix(&prefix)?.strip_suffix('`'))
        .unwrap_or_else(|| panic!("{name:?} has no item {item:?}"))
}

/// The value of the list item `item` of the section `name`, 32 bytes in hex.
fn item32(name: &str, item_name: &str) -> [u8; 32] {
    let value = hex(item(name, item_name));
    <[u8; 32]>::try_from(value.as_slice())
        .unwrap_or_else(|_| panic!("{name:?}: {item_name:?} is not 32 bytes"))
}

/// The bytes of the `hex` block of the section `## <name>`.
fn block(name: &str) -> Vec<u8> {
    let (_, after) = section(name)
        .split_once("```hex\n")
        .unwrap_or_else(|| panic!("{name:?} has no hex block"));
    let (digits, _) = after
        .split_once("```")
        .unwrap_or_else(|| panic!("{name:?}: the hex block does not end"));
    hex(digits)
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_invite_key_vector_derives_from_its_secret() {
    let secret = item32("Invite key", "Secret");
    let private = item32("Invite key", "Private key");
    let public = item32("Invite key", "Public key");
    assert_eq!(
        invite_dalek(&secret).to_bytes(),
        private,
        "HKDF-SHA256 of the secret"
    );
    assert_eq!(
        invite_public(&secret),
        public,
        "the private key's public key"
    );

    let key = InviteSecret::from_bytes(&secret).invite_key();
    assert_eq!(*key.to_bytes(), private);
    assert_eq!(key.public_key(), public);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_invite_vector_round_trips_byte_for_byte_with_the_vector_key() {
    let encoded = block("Invite");
    let invite = Invite::from_cbor(&encoded).unwrap_or_else(|error| panic!("Invite: {error}"));
    assert_eq!(invite.to_cbor(), encoded, "Invite, byte for byte");
    let body = invite.body();
    assert_eq!(body.to_cbor(), invite.body_bytes(), "Invite: the body");
    let fields = Value::Map(invite_fields(body));
    assert_eq!(
        fields.encode(),
        invite.body_bytes(),
        "Invite: the body holds only its spec fields, in the deterministic encoding"
    );
    assert!(!body.channels.is_empty(), "Invite names no channel");
    assert_eq!(
        body.key,
        item32("Invite key", "Public key"),
        "Invite: its key is that of \"Invite key\""
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_acceptance_vector_round_trips_byte_for_byte_and_names_the_vector_invite() {
    let encoded = block("Acceptance");
    let acceptance =
        Acceptance::from_cbor(&encoded).unwrap_or_else(|error| panic!("Acceptance: {error}"));
    assert_eq!(acceptance.to_cbor(), encoded, "Acceptance, byte for byte");
    let body = acceptance.body();
    assert_eq!(
        body.to_cbor(),
        acceptance.body_bytes(),
        "Acceptance: the body"
    );
    let fields = Value::Map(acceptance_fields(
        body.invite,
        body.account,
        &body.master,
        body.handle.as_str(),
    ));
    assert_eq!(
        fields.encode(),
        acceptance.body_bytes(),
        "Acceptance: the body holds only its spec fields, in the deterministic encoding"
    );
    let invite = Invite::from_cbor(&block("Invite")).expect("the Invite vector decodes");
    assert_eq!(
        body.invite,
        invite.body().invite,
        "Acceptance: of the invite of \"Invite\""
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_invite_link_vector_is_made_of_its_parts_and_reads_back_into_them() {
    const NAME: &str = "Invite link";
    let web = item(NAME, "Web address");
    let invite: InviteId = item(NAME, "Invite")
        .parse()
        .unwrap_or_else(|error| panic!("{NAME}: Invite: {error}"));
    let secret = item32(NAME, "Secret");
    let revision = item32(NAME, "Revision");
    let published = item(NAME, "Link");

    assert_eq!(
        secret,
        item32("Invite key", "Secret"),
        "{NAME}: the secret of \"Invite key\""
    );
    let vector_invite = Invite::from_cbor(&block("Invite")).expect("the Invite vector decodes");
    assert_eq!(
        invite,
        vector_invite.body().invite,
        "{NAME}: the invite of \"Invite\""
    );

    let (head, fragment) = published
        .split_once('#')
        .unwrap_or_else(|| panic!("{NAME}: no fragment in {published}"));
    assert_eq!(
        head,
        format!("{web}/invite/{invite}"),
        "{NAME}: before the fragment"
    );
    assert_eq!(fragment.len(), 86, "{NAME}: 64 bytes without padding");
    let decoded = base64url_decode(fragment);
    assert_eq!(decoded[..32], secret, "{NAME}: the fragment's secret");
    assert_eq!(decoded[32..], revision, "{NAME}: the fragment's revision");

    let made = InviteLink {
        web: web.to_owned(),
        invite,
        secret: InviteSecret::from_bytes(&secret),
        revision,
    }
    .shown();
    assert_eq!(made.as_str(), published, "{NAME}: the crate's link");
    let read = InviteLink::parse(published).unwrap_or_else(|error| panic!("{NAME}: {error:?}"));
    assert_eq!(read.web, web);
    assert_eq!(read.invite, invite);
    assert_eq!(*read.secret.to_bytes(), secret);
    assert_eq!(read.revision, revision);
}

/// The bytes of base64url without padding, RFC 4648, §5, apart from the crates.
fn base64url_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0);
    for char in text.bytes() {
        let value = ALPHABET
            .iter()
            .position(|digit| *digit == char)
            .unwrap_or_else(|| panic!("{char:?} is not base64url"));
        bits = (bits << 6) | u32::try_from(value).unwrap();
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push(u8::try_from((bits >> count) & 0xff).unwrap());
        }
    }
    out
}
