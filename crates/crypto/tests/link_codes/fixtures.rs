//! What the tests share: the draft's test vectors, the inputs of a link run and what "Link code"
//! makes of them, apart from the crate under test.

use skald_crypto::{Isk, LinkCode};
use skald_proto::AccountId;

pub use crate::common::{hex, hex32};

/// draft-irtf-cfrg-cpace-21, Appendix B.3: CPACE-RISTR255-SHA512, as hex digits.
pub mod draft {
    pub const PRS: &[u8] = b"Password";
    pub const CI: &str = "0b415f696e69746961746f720b425f726573706f6e646572";
    pub const SID: &str = "7e4b4791d6a8ef019b936c79fb7f2c57";
    pub const AD_A: &[u8] = b"ADa";
    pub const AD_B: &[u8] = b"ADb";
    /// The scalars, little endian.
    pub const YA: &str = "da3d23700a9e5699258aef94dc060dfda5ebb61f02a5ea77fad53f4ff0976d08";
    pub const YB: &str = "d2316b454718c35362d83d69df6320f38578ed5984651435e2949762d900b80d";
    /// The shares.
    pub const SHARE_A: &str = "d6bac480f2c386c394efc7c47adb9925dcd2630b64f240c50f8d0eec482b9157";
    pub const SHARE_B: &str = "3ea7e0b19560d7c0b0f5734f63b955286dfa8232b5ebe63324e2d9e7433f7258";
    /// The ISK of the initiator-responder transcript, B.3.5.
    pub const ISK: &str = "b69effbf61b51d56401c0f65601abe428de8206feaaf0e32198896dcae7b35cd\
                           2b38950a39dfd5d4a79164614c2984f7daa460b588c1e80c3fa2068af7900447";
    /// The ISK of the parallel transcript, B.3.6, which initiator-responder runs never give.
    pub const ISK_PARALLEL: &str = "544199d71f62f8d9a1fee55727e24fe4a45844593c2b6013c4fa3969d0e5debb\
                                    2244675c0b43397cbb68d342b01fc0f98fc961469a25134de9f0f813c1a57476";
    /// B.3.11: a share that does not decode, and the identity.
    pub const UNDECODABLE: &str =
        "2b3c6b8c4f3800e7aef6864025b4ed79bd599117e427c41bd47d93d654b4a51c";
    pub const IDENTITY: [u8; 32] = [0; 32];
}

/// Shares a side must refuse: B.3.11's, and one not below the field's order.
pub fn bad_shares() -> [(&'static str, [u8; 32]); 3] {
    [
        ("a share that does not decode", hex32(draft::UNDECODABLE)),
        ("the identity", draft::IDENTITY),
        ("a non-canonical encoding", [0xff; 32]),
    ]
}

/// The account of the tests' runs, and another.
pub const ACCOUNT: AccountId = account(0xe9);
pub const OTHER_ACCOUNT: AccountId = account(0xea);

/// An account ID ending in `last`.
const fn account(last: u8) -> AccountId {
    let mut id = [
        0x01, 0x9a, 0x6f, 0x1e, 0x2b, 0x3c, 0x7d, 0x4e, 0x8f, 0x50, 0x61, 0xa2, 0xb3, 0xc4, 0xd5, 0,
    ];
    id[15] = last;
    AccountId::from_bytes(id)
}

/// The `session` of the tests' runs, and another.
pub const SESSION: [u8; 16] = [0x5e; 16];
pub const OTHER_SESSION: [u8; 16] = [0x5f; 16];

/// The code of most runs, as shown.
pub const SHOWN: &str = "K7Q-M2X";
/// Its PRS: its 6 characters in uppercase ASCII.
pub const SHOWN_PRS: &[u8] = b"K7QM2X";

/// The code `entry`, which must be one.
#[track_caller]
pub fn code(entry: &str) -> LinkCode {
    LinkCode::from_entry(entry).unwrap_or_else(|_| panic!("{entry:?} is a link code"))
}

/// The CI of a run on `account`: the encoded `["skald link code", account]`, the label a text
/// string and the account ID a byte string.
pub fn ci(account: AccountId) -> Vec<u8> {
    let label = b"skald link code";
    let mut ci = vec![0x82, 0x60 | u8::try_from(label.len()).unwrap()];
    ci.extend_from_slice(label);
    ci.push(0x50);
    ci.extend_from_slice(account.as_bytes());
    ci
}

/// The 64 bytes of `isk`.
pub fn bytes(isk: &Isk) -> Vec<u8> {
    isk.to_bytes().as_slice().to_vec()
}

/// A full link run with `code` on A's side and `entered` on B's: both ISKs.
pub fn run(code: &LinkCode, entered: &LinkCode) -> (Isk, Isk) {
    let (a, ya) = code.initiate(ACCOUNT, &SESSION);
    let (yb, isk_b) = entered
        .respond(ACCOUNT, &SESSION, &ya)
        .expect("B takes A's share");
    let isk_a = a.finish(&yb, &[]).expect("A takes B's share");
    (isk_a, isk_b)
}

/// The section `heading` of `docs/protocol/test-vectors.md`, which must have it: its text without
/// whitespace, in lowercase, so that it holds each hex value it gives.
#[track_caller]
pub fn vector_section(heading: &str) -> String {
    const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");
    let mut lines = DOC.lines().skip_while(|line| *line != heading);
    assert!(
        lines.next().is_some(),
        "test-vectors.md has no section {heading:?}"
    );
    lines
        .take_while(|line| !line.starts_with("## "))
        .flat_map(str::split_whitespace)
        .collect::<String>()
        .to_lowercase()
}
