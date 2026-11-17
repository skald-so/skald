//! Each limit is its server setting's value; without the settings, the counts are given and no
//! limit applies, as on a self-hosted server.

use skald_proto::{Header, MessageId};

use crate::api::ACME;
use crate::limits::{DEVICES, MONTHLY, SIZE, assert_stored, clear_of_the_month_end};
use crate::usage::{Web, account_usage, names, of, space_usage, unlimited};
use crate::wire::{Socket, envelope};

/// The cloud's values (`docs/dev/architecture.md`, "Cloud limits"): 2 owned spaces, 50 devices,
/// about 10M messages a month, a message text of 64 KB and 7 days for an undelivered message.
const OWNED_2: u64 = 2;
const DEVICES_50: u64 = 50;
const MESSAGES_10M: u64 = 10_000_000;
const KB_64: u64 = 64 * 1024;
const DAYS_7: u64 = 7 * 86_400;

/// With the cloud's settings, alice's, bob's and carol's web sessions get each limit as its
/// setting's value, with the counts of a fresh `acme`: alice owns `acme` and her personal space to
/// come, and each of them has a laptop with a certificate.
#[test]
fn with_the_settings_each_limit_is_its_settings_value() {
    let args = [
        ("--owned-spaces-limit", OWNED_2),
        (DEVICES, DEVICES_50),
        (MONTHLY, MESSAGES_10M),
        (SIZE, KB_64),
        ("--mailbox-retention", DAYS_7),
    ]
    .map(|(setting, value)| [setting.to_owned(), value.to_string()]);
    let args: Vec<&str> = args.iter().flatten().map(String::as_str).collect();
    let web = Web::with(3_226_400_001, &args);
    let (team, api) = (&web.fixture.team, web.api());

    let usage = account_usage(&api, &web.alice);
    assert_eq!(usage.owned, of(2, OWNED_2), "alice's owned spaces");
    assert_eq!(usage.devices, of(1, DEVICES_50), "alice's devices");
    let usage = account_usage(&api, &web.bob);
    assert_eq!(usage.owned, of(1, OWNED_2), "bob's owned spaces");
    for (cookie, who) in [
        (&web.alice, "alice"),
        (&web.bob, "bob"),
        (&web.carol, "carol"),
    ] {
        let usage = space_usage(&api, team.space, cookie);
        assert_eq!(usage.messages, of(0, MESSAGES_10M), "{who}");
        assert_eq!(usage.message_size, Some(KB_64), "{who}");
        assert_eq!(usage.retention, Some(DAYS_7), "{who}");
    }
}

/// Without the settings: alice's two messages, one of them to bob and carol, count 2, and no limit
/// applies to them; alice creates `globex` and `initech` beside `acme` and owns 4 with her personal
/// space to come, without a limit, all three listed; her laptop counts 1, without a limit; no
/// message size and no retention.
#[test]
fn without_the_settings_the_counts_are_given_and_no_limit_applies() {
    let web = Web::with(3_226_500_001, &[]);
    let (fixture, api) = (&web.fixture, web.api());
    let (team, db) = (&fixture.team, fixture.db());
    clear_of_the_month_end(db);
    let [_, bob_laptop, carol_laptop, ..] = fixture.laptops();

    let mut alice = Socket::of(&team.flow, &team.alice);
    let msg_id = MessageId::generate();
    for recipient in [bob_laptop, carol_laptop] {
        let header = Header {
            msg_id,
            ..fixture.to(recipient)
        };
        assert_stored(&mut alice, &header, &envelope(&header));
    }
    let other = fixture.to(bob_laptop);
    assert_stored(&mut alice, &other, &envelope(&other));
    let usage = space_usage(&api, team.space, &web.carol);
    assert_eq!(usage.messages, unlimited(2), "two messages");
    assert_eq!((usage.message_size, usage.retention), (None, None));

    let globex = team.alice.create_space("globex");
    let initech = team.alice.create_space("initech");
    let usage = account_usage(&api, &web.alice);
    assert_eq!(usage.owned, unlimited(4), "alice's owned spaces");
    let three = names(&[(team.space, ACME), (globex, "globex"), (initech, "initech")]);
    assert_eq!(usage.owned_spaces, three, "alice's owned spaces");
    assert_eq!(usage.devices, unlimited(1), "alice's devices");
}
