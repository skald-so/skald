//! A member agent's run uploads the history links of its channel that the server lacks, as a
//! member's device does.

use crate::chain::{commits_empty, pairs};
use crate::member::Bot;

/// With the agent in the group, Bob's device commits an empty Commit and uploads no link of its
/// epoch. Only the agent runs next: the server then holds that epoch's link, whose `prev` is the
/// epoch before.
#[test]
fn a_member_agent_uploads_a_link_the_server_lacks() {
    let mut bot = Bot::created(
        [3_135_760_001, 3_135_760_002],
        ["ada-member-links-standin", "bob-member-links-standin"],
    );
    bot.brought_in();
    let at = bot.talk.at();
    let epoch = commits_empty(&bot.talk.world.cloud, &mut bot.talk.played[0], at);
    let held = |bot: &Bot| {
        pairs(&crate::chain::links(
            bot.talk.cloud(),
            &bot.talk.ada.token,
            at,
        ))
    };
    assert!(
        !held(&bot).contains(&(epoch, epoch - 1)),
        "before the agent's run"
    );

    bot.runner.received();

    assert!(
        held(&bot).contains(&(epoch, epoch - 1)),
        "the links the server holds: {:?}",
        held(&bot)
    );
}
