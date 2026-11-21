//! What a deletion leaves no row of, and how a space's other members hear of it: the account's
//! service tokens and link runs go; a space it alone was in goes with its history and tokens; a
//! long-poll, on any instance, gets the `space-notice` of a space it shared.

use std::time::Instant;

use skald_proto::DeviceKind;

use crate::common::TestDb;
use crate::common::devices::assert_taken;
use crate::deletion::{assert_holds_space, assert_space_gone, deletes};
use crate::fixture::Fixture;
use crate::history::{entry, formed, stores};
use crate::https::{Batch, Https, LONG, WAKE, answered_within};
use crate::relay::{fresh_session, hello};
use crate::rosters::service_agent;
use crate::sessions::Api;
use crate::tokens::Stage;

/// The rows of `table` in `db` that hold `text`.
fn rows_of(db: &TestDb, table: &str, text: &str) -> Vec<String> {
    let prefix = format!("{table}: ");
    let mut rows = db.rows();
    rows.retain(|row| row.starts_with(&prefix) && row.contains(text));
    rows
}

/// alice's `service` device has a token in `acme`, which stays, and her uncertified device has an
/// open link run. Once she deletes her account, no row of the database holds either.
#[test]
fn no_service_token_or_link_run_row_of_a_deleted_account_stays() {
    let stage = Stage::new(3_303_060_001);
    let alice = &stage.f.team.alice;
    stage.create();
    let opened = alice
        .session
        .send_hello(alice.uncertified.id, &hello(&fresh_session()));
    assert_taken(&opened);
    let db = stage.db();
    let (bot, account) = (stage.bot.id.to_string(), alice.account.to_string());
    assert_eq!(
        rows_of(db, "service_tokens", &bot).len(),
        1,
        "the bot's token"
    );
    assert_eq!(rows_of(db, "link_runs", &account).len(), 1, "the link run");

    deletes(&Api::of(&stage.f.team.flow), alice);

    assert_eq!(rows_of(db, "service_tokens", &bot), Vec::<String>::new());
    assert_eq!(rows_of(db, "link_runs", &account), Vec::<String>::new());
    assert_holds_space(db, stage.space(), "acme");
}

/// In `acme`, bob's entry of epoch 1 is in the channel's history and alice's `service` device has
/// a token. Once bob, carol and dave delete their accounts, then alice hers, no row of the database
/// holds `acme`, its channel, the entry or the token.
#[test]
fn space_of_only_the_account_is_deleted_with_its_history_and_tokens() {
    let (mut s, _) = formed(3_303_060_011);
    let general = s.fixture.general;
    let item = entry(general, 1);
    stores(&mut s.bob, &s.fixture, &item, "bob's entry");
    let alice = &s.fixture.team.alice;
    let bot = alice.certified_by(s.fixture.db(), &alice.laptop, DeviceKind::Service, "Bot");
    s.fixture
        .revise(|body| body.service_agents.push(service_agent(&bot)));
    let stage = Stage { f: s.fixture, bot };
    let token = stage.create().token;
    let db = stage.db();
    let hexed = crate::common::hex(item.bytes());
    assert!(
        db.rows().iter().any(|row| row.contains(&hexed)),
        "bob's entry"
    );
    let t = &stage.f.team;
    let api = Api::of(&t.flow);

    for person in [&t.bob, &t.carol, &t.dave, &t.alice] {
        deletes(&api, person);
    }

    assert_space_gone(db, t.space, &[general], "acme");
    assert!(
        !db.rows().iter().any(|row| row.contains(&hexed)),
        "bob's entry"
    );
    assert!(!stage.works(&token), "the bot's token");
}

/// bob's laptop long-polls the server that deletes, and dave's laptop waits in a long-poll on
/// another instance. Once carol deletes her account, dave's long-poll answers a notice of `acme`
/// within a second, bob's next long-poll answers one and the one after none, and eve's, outside
/// `acme`, none.
#[test]
fn long_polls_on_any_instance_get_the_notice_of_a_members_deletion() {
    let f = Fixture::new(3_303_060_021);
    let t = &f.team;
    let eve = Https::of(&t.flow, &f.eve);
    let bob = Https::of(&t.flow, &t.bob);
    let other = t.flow.stand.instance(&[], &[]);
    let dave = Https::to(&other, &t.flow, &t.dave, &t.dave.laptop);
    for https in [&bob, &dave, &eve] {
        https.poll_now(0);
    }
    let waiting = dave.start_poll(0, LONG);
    waiting.assert_waiting();

    deletes(&Api::of(&t.flow), &t.carol);

    let notice = Batch {
        entries: Vec::new(),
        notices: vec![t.space],
    };
    let answered = answered_within(waiting, Instant::now(), WAKE);
    assert_eq!(answered, notice, "dave's long-poll on the other instance");
    assert_eq!(bob.poll_now(0), notice, "bob's next long-poll");
    assert_eq!(bob.poll_now(0), Batch::default(), "bob's long-poll after");
    assert_eq!(eve.poll_now(0), Batch::default(), "eve's long-poll");
}
