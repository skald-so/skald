//! A `skald recv --wait` that has run since before the revision that lists a service agent takes the
//! agent's first message.

use skald_proto::Role;

use crate::Bot;
use crate::fixtures::await_running;
use crate::kit::{RECV, received};
use crate::talk::{Talk, stored};
use crate::world::ACME;

/// The flag of `skald recv` that waits for what is to come, and the request that opens a
/// WebSocket, as its line starts.
const WAIT_FLAG: &str = "--wait";
const WEBSOCKET: &str = "GET /ws ";

/// The service agent's first message; it is nowhere else.
const HELLO: &str = "deploy-bot: first run on this space, hello";

/// Ada's `skald recv --wait` in one-shot mode connects while no revision lists the service agent,
/// whose device is on Ada's account. A revision Ada's device signs then lists it, and the agent's
/// first message reaches Ada's device through Bob's device's connection, as the server cannot tell
/// who sealed it: the wait shows it once, from the agent's address and device, and exits.
#[test]
fn waiting_recv_shows_the_first_message_of_an_agent_a_later_revision_lists() {
    let talk = Talk::with(
        ACME,
        &[4_235_041_001, 4_235_041_002],
        &["ada-waits-for-a-bot-standin", "bob-relays-a-bot-standin"],
        false,
        (Role::Member, |_| {}),
    );
    let bot = Bot::beside(talk);
    received(&bot.talk.ada.machine);
    let since = bot.cloud().relay.mark();
    let mut waiting = bot.talk.ada.machine.start(&[RECV, WAIT_FLAG]);
    await_running(&mut waiting, "connection to the server", || {
        bot.cloud().count_sent(since, WEBSOCKET) >= 1
    });

    bot.list();
    stored(
        bot.cloud(),
        &bot.talk.played[0],
        &bot.sealed(&bot.note(HELLO)),
    );

    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    bot.assert_shown(&got, HELLO);
}
