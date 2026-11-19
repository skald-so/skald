//! Content from the service agent with a valid grant is shown: without `counter` under `hpke`, once
//! per sending device and `msg_id`, and with one too.

use skald_proto::Role;

use crate::Bot;

/// What the service agent says; none is anywhere else.
const DEPLOYED: &str = "deploy-bot: release 4.2.0 is live on staging";
const COUNTED: &str = "deploy-bot: the smoke tests passed, 31 of 31";

/// The service agent's message without `counter` comes three times in Ada's catch-up: the same
/// envelope twice and sealed again with the same `msg_id`, as a stateless run that retries sends
/// it. Ada's `skald recv` shows it once, with its channel, the agent's address and its device, and
/// warns of none of them; a message of the agent with `counter` 1 is shown too. The same envelope
/// in a later catch-up is not shown again.
#[test]
fn content_from_a_service_agent_with_a_valid_grant_and_no_counter_is_shown_once() {
    let bot = Bot::new(
        [4_235_011_001, 4_235_011_002],
        ["ada-runs-ci-standin", "bob-watches-ci-standin"],
        Role::Member,
        true,
    );
    let note = bot.note(DEPLOYED);
    let envelope = bot.sealed(&note);
    let mut counted = bot.note(COUNTED);
    counted.counter = Some(1);

    let first = bot.ada_receives(vec![
        envelope.clone(),
        envelope.clone(),
        bot.sealed(&note),
        bot.sealed(&counted),
    ]);

    bot.assert_shown(&first, DEPLOYED);
    bot.assert_shown(&first, COUNTED);
    let warned = crate::kit::warnings_naming(&first, bot.device.id);
    assert!(
        warned.is_empty(),
        "warnings of the agent's messages: {first:?}"
    );

    let again = bot.ada_receives(vec![envelope]);

    assert!(!again.shown().contains(DEPLOYED), "shown again: {again:?}");
}
