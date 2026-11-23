//! An invite's secret reaches neither the server nor the newcomer's disk: no request the recording
//! relay sees, no row of the server's database and no file of Bob's machine, his state directory
//! and his file key storage included, holds the secret or the invite private key derived from it,
//! in any encoding (`device_keys/keys.rs`).

use skald_crypto::InviteSecret;

use crate::Scene;
use crate::keys::holds;
use crate::kit::received;
use crate::world::assert_secret_nowhere;

/// Bob accepts the invite at a terminal, and his machine runs `skald recv`. The link's secret and
/// its invite private key are in no request, database row or file of his machine.
#[test]
fn secret_reaches_neither_the_server_nor_the_newcomer_s_disk() {
    let scene = Scene::new(
        (7_305_408_001, "ada-shares-a-secret"),
        (7_305_408_002, "bob-keeps-it-in-memory"),
    );

    let made = scene.bob_accepts();
    received(&scene.bob.machine);

    let secret = made.secret;
    let private = InviteSecret::from_bytes(&secret).invite_key().to_bytes();
    assert!(
        holds(made.link.as_bytes(), &secret),
        "the check finds no secret in the link itself"
    );
    assert_secret_nowhere(scene.cloud(), &scene.bob.machine, &secret, private.as_ref());
}
