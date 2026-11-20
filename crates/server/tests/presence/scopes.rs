//! What else bounds who reads and uploads a channel's values: the space's latest revision for a
//! member device's reads, as for its sends, and a service token's channels for its uploads.

use crate::bot::{ops, token};
use crate::fixture::{channel_members, exclude};
use crate::kit::{
    assert_all_kept, assert_given, assert_not_held, assert_refused_with, callers, formed, sorted,
};
use crate::services::{bot_caller, with_bot};
use crate::tokens::{SEND, grant};
use crate::wire::{NOT_A_MEMBER, now};

/// carol's laptop uploads a value, which bob's gets. Once a revision drops bob's account from
/// `general`, bob's laptop, still in the group, gets nothing; once another excludes carol's laptop,
/// neither does carol's. alice's still gets carol's value.
#[test]
fn a_member_device_that_may_not_send_in_the_channel_gets_nothing() {
    let (mut s, general) = formed(3_621_600_001);
    let [alice, bob, carol, ..] = callers(&s);
    let uploaded = general.value(s.carol.id);
    assert_all_kept(&general, &[(&carol, &uploaded)]);
    assert_eq!(bob.values(&general), sorted(&[&uploaded]), "bob's before");

    let t = &s.fixture.team;
    let (others, carol_account) = ([t.alice.account, t.carol.account], t.carol.account);
    s.fixture
        .revise(|body| channel_members(body, general.id, &others));
    let response = bob.get(&general, true);
    assert_refused_with(&response, Some(NOT_A_MEMBER), "bob out of the channel");

    let carol_laptop = s.carol.id;
    s.fixture
        .revise(|body| exclude(body, carol_account, &[carol_laptop]));
    let response = carol.get(&general, true);
    assert_refused_with(&response, Some(NOT_A_MEMBER), "carol's laptop excluded");
    assert_given(&[&alice], &general, &[&uploaded], "alice after");
}

/// The bot, a member of `general`'s group, uploads nothing there on a sending token whose grant
/// lists only `ops`.
#[test]
fn a_token_that_does_not_list_the_channel_uploads_nothing() {
    let (mut stage, _laptop, general) = with_bot(3_621_600_011);
    let (channels, bot) = ([ops(&mut stage)], stage.bot.id);
    let person = &stage.f.team.alice;
    let expires = now() + 86_400;
    let listing_ops = grant(bot, &channels, &person.laptop, expires);
    let created = stage.create_as(person, bot, &listing_ops, &[SEND]);
    let of_bot = general.value(bot);
    let response = bot_caller(&stage, &created.token).put(&general, &of_bot);
    assert_refused_with(&response, Some(NOT_A_MEMBER), "a token for ops");
    assert_not_held(stage.f.db(), &of_bot, "a token for ops");

    let sending = bot_caller(&stage, &token(&stage, &[SEND]).token);
    assert_given(&[&sending], &general, &[], "nothing kept");
}
