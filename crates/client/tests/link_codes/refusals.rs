//! A link code is shown and typed only by a person at a terminal: a new device signing in through a
//! pipe shows no code and sends no hello; `skald link` through a pipe, or as an agent at a
//! terminal, refuses and sends nothing to the link code relay.

use std::time::Instant;

use crate::cloud::Cloud;
use crate::linking::{HELLO, LINK, RELAYED, codes_in, says};
use crate::machine::{FIRST_DEVICE, Machine, NO};
use crate::terminal::Terminal;
use crate::world::{AGENT, World};

/// What `skald link` says when no person types at its terminal, and as an agent.
const TERMINAL: &str = "terminal";
const AS_AGENT: &str = "agent";

/// The requests to the link code relay the relay saw since `since`.
fn relayed_since(cloud: &Cloud, since: Instant) -> Vec<String> {
    let path = RELAYED.to_lowercase();
    (cloud.relay.sent_since(since).into_iter())
        .map(|sent| sent.head)
        .filter(|head| head.contains(&path))
        .collect()
}

/// Ada's new machine signs in through a pipe as no first device: it shows no code and sends no
/// hello. `skald link` on her first machine through a pipe, given a code and `yes` on its input,
/// fails, says it needs a terminal, and sends no request to the link code relay.
#[test]
fn through_a_pipe_no_code_is_shown_or_taken() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_901_001, "ada-pipes-the-code", "ada-workstation");
    let since = cloud.relay.mark();

    let machine = Machine::new().with_host_name("ada-laptop");
    let signed_in = cloud.signed_in(
        &world.browser.page(),
        &machine,
        &ada.user,
        &[FIRST_DEVICE, NO],
    );
    assert!(signed_in.success, "{signed_in:?}");
    assert_eq!(
        codes_in(&signed_in.shown()),
        Vec::<String>::new(),
        "{signed_in:?}"
    );
    assert_eq!(cloud.count_sent(since, HELLO), 0, "a hello through a pipe");

    let mut piped = ada.machine.start_asking(&[LINK]);
    piped.answer("ABC-DEF\nyes\n");
    let piped = piped.finish();
    assert!(!piped.success, "{piped:?}");
    assert!(says(&piped.shown(), &[TERMINAL]), "{piped:?}");
    assert_eq!(relayed_since(cloud, since), Vec::<String>::new());
}

/// `skald --agent builder link` at a terminal on Ada's first machine fails, says it does not run
/// as an agent, and sends no request to the link code relay.
#[test]
fn an_agent_cannot_link_a_device() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_901_011, "ada-agent-links", "ada-workstation");
    let since = cloud.relay.mark();

    let refused = Terminal::start(&ada.machine, &[AGENT, "builder", LINK]).finish();
    assert!(!refused.success, "{}", refused.text);
    assert!(says(&refused.text, &[AS_AGENT]), "{}", refused.text);
    assert!(
        !refused.text.to_lowercase().contains("own device"),
        "{}",
        refused.text
    );
    assert_eq!(relayed_since(cloud, since), Vec::<String>::new());
}
