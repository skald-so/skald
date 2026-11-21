//! The invite link (`membership.md`, "Invite", "Link"): `https://<server>/invite/<invite>#<fragment>`,
//! the server its web address, the fragment base64url without padding of the secret followed by
//! the hash of a roster revision. A link reads back into what made it; any other form is refused.

use skald_crypto::{
    InviteLink, InviteSecret, check_acceptance, check_invite, sign_acceptance, sign_invite,
};
use skald_proto::{Cbor, Invite, InviteId};

use crate::fixtures::{LAPTOP, key, public};
use crate::objects::{Acc, Inv, SECRET};

/// The tests' server's web address.
const WEB: &str = "https://skald.example";
/// The invite of the tests' links.
const INVITE: &str = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5f1";
/// The hash of the roster revision the tests' links name.
const REVISION: [u8; 32] = [0xa1; 32];

/// The secret of the bytes 00 to 1f.
fn counting() -> [u8; 32] {
    std::array::from_fn(|at| u8::try_from(at).unwrap())
}

/// The link of [`WEB`], [`INVITE`], [`counting`] and [`REVISION`], computed apart from the crates
/// with Python's `base64.urlsafe_b64encode`, its padding dropped.
const COUNTING_LINK: &str = "https://skald.example/invite/019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5f1#\
                             AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh-hoaGhoaGhoaGhoaGhoaGhoaGh\
                             oaGhoaGhoaGhoaGhoQ";

/// Base64url without padding, RFC 4648, §5, apart from the crates.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (at, byte)| {
            bits | (u32::from(*byte) << (16 - 8 * at))
        });
        for at in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(bits >> (18 - 6 * at)) as usize & 63]));
        }
    }
    out
}

/// The link the spec gives for these parts.
fn expected(web: &str, invite: &str, secret: &[u8; 32], revision: &[u8; 32]) -> String {
    format!(
        "{web}/invite/{invite}#{}",
        base64url(&[secret.as_slice(), revision.as_slice()].concat())
    )
}

fn invite_id() -> InviteId {
    INVITE.parse().unwrap()
}

fn link(web: &str, secret: &[u8; 32], revision: [u8; 32]) -> InviteLink {
    InviteLink {
        web: web.to_owned(),
        invite: invite_id(),
        secret: InviteSecret::from_bytes(secret),
        revision,
    }
}

/// Checks that `link` reads back into `web`, [`INVITE`], `secret` and `revision`.
fn assert_reads_into(link: &str, web: &str, secret: &[u8; 32], revision: &[u8; 32]) {
    let read = InviteLink::parse(link).unwrap_or_else(|error| panic!("{link}: {error:?}"));
    assert_eq!(read.web, web, "{link}: the web address");
    assert_eq!(read.invite, invite_id(), "{link}: the invite");
    assert_eq!(*read.secret.to_bytes(), *secret, "{link}: the secret");
    assert_eq!(read.revision, *revision, "{link}: the revision");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_is_the_web_address_the_invite_and_base64url_of_the_secret_and_revision() {
    let shown = link(WEB, &counting(), REVISION).shown();
    assert_eq!(shown.as_str(), COUNTING_LINK);
    assert_eq!(
        COUNTING_LINK,
        expected(WEB, INVITE, &counting(), &REVISION),
        "the tests' encoder"
    );
    let fragment = COUNTING_LINK.split_once('#').unwrap().1;
    assert_eq!(fragment.len(), 86, "64 bytes, without padding");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_reads_back_into_what_made_it() {
    let cases = [
        (WEB, counting(), REVISION),
        (WEB, SECRET, [0x77; 32]),
        // Bytes whose base64 has `+` and `/`, which base64url writes `-` and `_`.
        (WEB, [0xfb; 32], [0xff; 32]),
        ("https://skald.example:8443", [0; 32], [0; 32]),
        ("https://app.skald.example", [0xff; 32], REVISION),
    ];
    for (web, secret, revision) in cases {
        let shown = link(web, &secret, revision).shown();
        assert_eq!(
            shown.as_str(),
            expected(web, INVITE, &secret, &revision),
            "{web}"
        );
        assert_reads_into(&shown, web, &secret, &revision);
    }
    assert_reads_into(COUNTING_LINK, WEB, &counting(), &REVISION);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_links_are_refused() {
    let fragment = base64url(&[counting().as_slice(), REVISION.as_slice()].concat());
    let path = format!("/invite/{INVITE}");
    let at_path = |path: &str| format!("{WEB}{path}#{fragment}");
    let with_fragment = |fragment: &str| format!("{WEB}{path}#{fragment}");
    // Each replaces a character of the fragment, keeping its length.
    let replaced = |at: usize, with: char| {
        let mut chars: Vec<char> = fragment.chars().collect();
        chars[at] = with;
        with_fragment(&chars.into_iter().collect::<String>())
    };
    let padded = format!(
        "{}==",
        base64url(&[counting().as_slice(), REVISION.as_slice()].concat())
    );
    let cases = [
        ("an empty link", String::new()),
        ("only the fragment", format!("#{fragment}")),
        ("no scheme", format!("skald.example{path}#{fragment}")),
        ("no host", format!("https://{path}#{fragment}")),
        (
            "the path /invites/<invite>",
            at_path(&format!("/invites/{INVITE}")),
        ),
        ("a path with a trailing /", at_path(&format!("{path}/"))),
        ("a path with a prefix", at_path(&format!("/app{path}"))),
        (
            "a path with another segment",
            at_path(&format!("{path}/accept")),
        ),
        ("no invite in the path", at_path("/invite/")),
        ("the root path", at_path("/")),
        (
            "an invite in uppercase",
            at_path(&format!("/invite/{}", INVITE.to_uppercase())),
        ),
        (
            "an invite of 35 characters",
            at_path(&format!("/invite/{}", &INVITE[1..])),
        ),
        (
            "an invite without hyphens",
            at_path(&format!("/invite/{}", INVITE.replace('-', ""))),
        ),
        ("a query", format!("{WEB}{path}?from=mail#{fragment}")),
        ("no fragment", format!("{WEB}{path}")),
        ("an empty fragment", with_fragment("")),
        (
            "a fragment of 85 characters",
            with_fragment(&fragment[..85]),
        ),
        (
            "a fragment of 87 characters",
            with_fragment(&format!("{fragment}A")),
        ),
        (
            "a fragment of only the secret",
            with_fragment(&base64url(&counting())),
        ),
        ("a fragment with padding", with_fragment(&padded)),
        ("a fragment with = in it", replaced(85, '=')),
        ("a fragment with +", replaced(10, '+')),
        ("a fragment with /", replaced(10, '/')),
        ("a fragment with .", replaced(10, '.')),
        ("a fragment with a space", replaced(10, ' ')),
        ("a fragment with a second #", replaced(10, '#')),
        ("a fragment with a letter outside ASCII", replaced(10, 'é')),
        (
            "a fragment percent-encoded",
            with_fragment(&format!("%41{}", &fragment[1..])),
        ),
    ];
    for (what, link) in &cases {
        assert!(InviteLink::parse(link).is_err(), "{what}: {link}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_newcomer_proves_the_secret_of_a_link_to_the_invite_it_names() {
    // The admin's device: a secret, the invite of its key, signed by the laptop, and the link.
    let secret = InviteSecret::generate();
    let body = skald_proto::InviteBody {
        key: secret.invite_key().public_key(),
        ..Inv::of_member().proto_body()
    };
    let invite = sign_invite(&body, &key(LAPTOP)).expect("an invite");
    let shown = InviteLink {
        web: WEB.to_owned(),
        invite: body.invite,
        secret,
        revision: REVISION,
    }
    .shown();

    // The newcomer's device: the invite as the server gives it, checked, and the acceptance.
    let read = InviteLink::parse(&shown).expect("the link reads back");
    let invite = Invite::from_cbor(&invite.to_cbor()).expect("the invite decodes");
    assert_eq!(invite.body().invite, read.invite);
    assert!(check_invite(&invite, &public(LAPTOP)).is_ok());
    let invite_key = read.secret.invite_key();
    assert_eq!(invite_key.public_key(), invite.body().key);
    let acceptance =
        sign_acceptance(&Acc::of_newcomer().proto_body(), &invite_key).expect("an acceptance");

    // The admin's device checks the acceptance with the invite's key.
    assert!(check_acceptance(&acceptance, &invite.body().key).is_ok());
}
