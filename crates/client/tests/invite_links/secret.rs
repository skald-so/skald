//! An invite's secret reaches neither the server nor the disk: no request the recording relay
//! sees, no row of the server's database and no file of the machine, its state directory and its
//! file key storage included, holds the secret or the invite private key derived from it, in any
//! encoding (`device_keys/keys.rs`).

use skald_crypto::InviteLink;

use crate::keys::holds;
use crate::{Admin, invite_at_terminal, shown_link};

/// Fails unless `link`'s secret and its invite private key are in `shown`, what the run showed,
/// and nowhere the server or Ada's machine keeps or was sent.
#[track_caller]
pub fn assert_secret_nowhere(admin: &Admin, link: &InviteLink, shown: &str) {
    let secret = link.secret.to_bytes();
    let private = link.secret.invite_key().to_bytes();
    assert!(
        holds(shown.as_bytes(), secret.as_ref()),
        "the check finds no secret in the link itself: {shown}"
    );
    let cloud = admin.cloud();
    let mut kept = vec![
        ("the requests".to_owned(), cloud.relay.sent_bytes()),
        (
            "the database".to_owned(),
            cloud.rows().join("\n").into_bytes(),
        ),
    ];
    for file in admin.ada.machine.files() {
        if let Ok(bytes) = std::fs::read(&file) {
            kept.push((file.display().to_string(), bytes));
        }
    }
    for (what, bytes) in kept {
        assert!(!holds(&bytes, secret.as_ref()), "{what} holds the secret");
        assert!(
            !holds(&bytes, private.as_ref()),
            "{what} holds the invite private key"
        );
    }
}

/// `skald invite acme` at a terminal on Ada's machine shows a link; its secret and invite private
/// key are in no request, database row or file of her machine.
#[test]
fn secret_reaches_neither_the_server_nor_the_disk() {
    let admin = Admin::new((6_418_304_001, "ada-keeps-the-secret"));

    let shown = invite_at_terminal(&admin.ada.machine, &[]);

    assert!(shown.success, "{}", shown.text);
    let (_, link) = shown_link(&shown.text, &admin.cloud().public);
    assert_secret_nowhere(&admin, &link, &shown.text);
}
