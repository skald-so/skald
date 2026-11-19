//! How often a run fetches the space's revisions for content from a `service` device: once more for
//! a device no entry lists, however many of its messages come, and never for one an entry lists.

use skald_proto::Role;

use crate::world::{now, revisions_request};
use crate::{Bot, GRANTED, Note, grant, service_device};

const LISTED: &str = "deploy-bot: the build is green";
const LISTED_AGAIN: &str = "deploy-bot: the deploy is done";
const UNLISTED: &str = "stray-bot: knock knock";
const UNLISTED_AGAIN: &str = "stray-bot: anyone there";

/// Ada's `skald recv` with nothing new gives how many times a run fetches the space's revisions.
/// A run with two messages of the listed agent fetches them as often; a run with two messages of
/// a `service` device of Ada's account that no revision lists fetches them once more, and drops
/// both.
#[test]
fn run_fetches_the_revisions_again_once_for_an_unlisted_device_and_never_for_a_listed_one() {
    let bot = Bot::new(
        [4_235_051_001, 4_235_051_002],
        ["ada-counts-fetches-standin", "bob-near-fetches-standin"],
        Role::Member,
        true,
    );
    let (cloud, ada) = (bot.cloud(), &bot.talk.ada);
    let unlisted = service_device(cloud, ada, "stray-bot-runner");
    let request = revisions_request(bot.talk.space.id);
    let fetches = |envelopes| {
        let since = cloud.relay.mark();
        let finished = bot.ada_receives(envelopes);
        (cloud.count_sent(since, &request), finished)
    };
    let (usual, _) = fetches(Vec::new());

    let listed = vec![
        bot.sealed(&bot.note(LISTED)),
        bot.sealed(&bot.note(LISTED_AGAIN)),
    ];
    let (after_listed, shown) = fetches(listed);
    bot.assert_shown(&shown, LISTED);
    bot.assert_shown(&shown, LISTED_AGAIN);
    assert_eq!(after_listed, usual, "fetches for a listed agent");

    let at = (ada.device, &ada.key);
    let stray = grant(at, unlisted.id, &[bot.talk.channel], now() + GRANTED);
    let from = format!(
        "{}/ci/stray-bot@{}",
        bot.talk.space.name,
        crate::world::HOST
    );
    let notes = [
        Note::new(&from, Some(stray.clone()), UNLISTED),
        Note::new(&from, Some(stray), UNLISTED_AGAIN),
    ];
    let envelopes = notes
        .iter()
        .map(|note| bot.sealed_by(note, &unlisted.signing))
        .collect();
    let (after_unlisted, dropped) = fetches(envelopes);

    for note in &notes {
        bot.assert_dropped(&dropped, note, unlisted.id);
    }
    assert_eq!(after_unlisted, usual + 1, "fetches for an unlisted device");
}
