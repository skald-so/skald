//! A grant fails once its signer's device is revoked, as a revoked device is no admin's device.

use skald_proto::Role;

use crate::revocations::revoked_by;
use crate::world::now;
use crate::{Bot, GRANTED, Note, grant};

const BEFORE: &str = "deploy-bot: bob's grant still holds";
const AFTER: &str = "deploy-bot: bob's grant outlived his device";
const STILL: &str = "deploy-bot: ada's grant holds";

/// Bob, an admin, signs the agent's grant on his device, and Ada's `skald recv` shows its message.
/// Bob's device then revokes itself: the next message with Bob's grant is dropped with a warning
/// naming the agent's device, while one with Ada's grant is shown.
#[test]
fn grant_fails_once_its_signers_device_is_revoked() {
    let bot = Bot::new(
        [4_235_061_001, 4_235_061_002],
        [
            "ada-sees-a-revoked-signer-standin",
            "bob-revokes-his-box-standin",
        ],
        Role::Admin,
        true,
    );
    let bob = &bot.talk.played[0].person;
    let bobs = grant(
        (bob.device, &bob.key),
        bot.device.id,
        &[bot.talk.channel],
        now() + GRANTED,
    );
    let by_bob = |text| Note::new(&bot.address, Some(bobs.clone()), text);
    let first = bot.ada_receives(vec![bot.sealed(&by_bob(BEFORE))]);
    bot.assert_shown(&first, BEFORE);
    revoked_by(bot.cloud(), bob, bob.device);

    let after = by_bob(AFTER);
    let next = bot.ada_receives(vec![bot.sealed(&after), bot.sealed(&bot.note(STILL))]);

    bot.assert_dropped(&next, &after, bot.device.id);
    bot.assert_shown(&next, STILL);
}
