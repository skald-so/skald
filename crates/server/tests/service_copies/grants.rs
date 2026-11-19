//! The space's members get the grants of its live tokens that have the receive permission: those
//! of tokens that only send or claim from queues, or that ended, are not given, and an account
//! outside the space gets none.

use crate::api::assert_refused;
use crate::copies::{
    drop_entries, expire, grant_for, grants_of, grants_response, sorted, token_with,
};
use crate::tokens::{QUEUES, RECEIVE, SEND, Stage, bearer};

/// Of six tokens of the bot, each with a grant of its own for `general`, alice, an admin, bob, a
/// member of `general`, and dave, a member outside it, each get the grants of the two that
/// receive and live: not those of the tokens that only send or only claim from queues, nor of the
/// revoked one, nor of the one that expired, which the test moves back in the database.
#[test]
fn members_get_the_grants_of_live_tokens_that_receive() {
    let stage = Stage::new(3_555_200_001);
    let general = [stage.f.general];
    let both = grant_for(&stage, &general, 0);
    let receives = grant_for(&stage, &general, 1);
    token_with(&stage, &both, &[SEND, RECEIVE]);
    token_with(&stage, &receives, &[RECEIVE]);
    token_with(&stage, &grant_for(&stage, &general, 2), &[SEND]);
    token_with(&stage, &grant_for(&stage, &general, 3), &[QUEUES]);
    let revoked = token_with(&stage, &grant_for(&stage, &general, 4), &[RECEIVE]);
    let expired = token_with(&stage, &grant_for(&stage, &general, 5), &[SEND, RECEIVE]);
    let team = &stage.f.team;
    let response = stage.revoke_as(&team.alice, &revoked.id);
    assert_eq!(response.status, 204, "revoked");
    expire(&stage, &expired.id);

    let expected = sorted(&[&both, &receives]);
    for (who, person) in [
        ("alice", &team.alice),
        ("bob", &team.bob),
        ("dave", &team.dave),
    ] {
        assert_eq!(grants_of(&stage, person), expected, "the grants {who} gets");
    }
}

/// Once a revision drops the bot's entry, its tokens end, and the members get none of their grants.
#[test]
fn members_get_no_grant_once_the_service_agent_s_entry_is_dropped() {
    let mut stage = Stage::new(3_555_200_011);
    let general = [stage.f.general];
    let grant = grant_for(&stage, &general, 0);
    token_with(&stage, &grant, &[SEND, RECEIVE]);
    let bob = &stage.f.team.bob;
    assert_eq!(grants_of(&stage, bob), sorted(&[&grant]), "before");

    drop_entries(&mut stage);

    let bob = &stage.f.team.bob;
    assert_eq!(grants_of(&stage, bob), Vec::<Vec<u8>>::new(), "after");
}

/// eve, who signed in but is no member of `acme`, and a request without a session are refused
/// the space's grants, though the bot has a token that receives; bob, a member, gets its grant.
#[test]
fn an_account_outside_the_space_gets_no_grants() {
    let stage = Stage::new(3_555_200_021);
    let grant = grant_for(&stage, &[stage.f.general], 0);
    token_with(&stage, &grant, &[SEND, RECEIVE]);
    assert_eq!(
        grants_of(&stage, &stage.f.team.bob),
        sorted(&[&grant]),
        "bob's"
    );

    let eve = bearer(stage.f.eve.session.token());
    let response = grants_response(&stage, &[("authorization", &eve)]);
    assert_refused(&response, "eve");
    let response = grants_response(&stage, &[]);
    assert_refused(&response, "no session");
}
