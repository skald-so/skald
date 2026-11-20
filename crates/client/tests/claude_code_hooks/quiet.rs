//! The hooks print nothing while nothing waits for their agent, in a folder in no space, and in a
//! folder of another space, where the agent of that name is another agent.

use crate::hooks::{Event, Web, assert_own_line, hook_in};
use crate::lines::assert_handed_line;
use crate::solo::WEB;
use crate::world::dir_of;

/// web's session runs and nothing waits for it: its `PostToolUse` and `Stop` hooks, `Stop` also in
/// a turn a hook kept going, print nothing and exit with success.
#[test]
fn hooks_print_nothing_while_nothing_waits() {
    let web = Web::new(4_913_005_001, "ada-hooks-an-idle-web");

    for event in [Event::PostToolUse, Event::Stop, Event::StopActive] {
        web.hook(event).assert_silent(event.name());
    }
}

/// What api says to web; it is nowhere else.
const WAITS: &str = "hook test: api leaves web a note on the release";

/// api sends web a message. The hooks of an agent web in `~/work/elsewhere/web`, a folder in no
/// space, print nothing on each event; web's `inbox` in `~/work/clientA/web` then shows the message.
#[test]
fn hooks_in_a_folder_in_no_space_print_nothing() {
    let mut web = Web::new(4_913_006_001, "ada-hooks-outside-spaces");
    let msg_id = web.own(WAITS);
    let elsewhere = dir_of(web.machine(), "work/elsewhere/web");

    for event in [Event::PostToolUse, Event::Stop] {
        hook_in(web.machine(), &elsewhere, Some(WEB), event).assert_silent(event.name());
    }

    assert_handed_line(&web.session.inbox(), WAITS, msg_id);
}

/// What api says to web in `clienta`; it is nowhere else.
const FOR_CLIENT_A: &str = "hook test: api asks clienta's web to bump the version";

/// Ada also has `clientb`, bound to `~/work/clientB`. api sends web in `clienta` a message. The hooks
/// of the agent web in `~/work/clientB/web` print nothing on each event; web's `PostToolUse` hook in
/// `~/work/clientA/web` then gives the message.
#[test]
fn hooks_of_an_agent_of_that_name_in_another_space_get_nothing() {
    let web = Web::new(4_913_007_001, "ada-hooks-in-two-spaces");
    let client_b = web.solo.with_client_b();
    let other = dir_of(web.machine(), "work/clientB/web");
    assert!(other.starts_with(&client_b), "{}", other.display());
    let msg_id = web.own(FOR_CLIENT_A);

    for event in [Event::PostToolUse, Event::Stop] {
        hook_in(web.machine(), &other, Some(WEB), event).assert_silent(event.name());
    }

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);
    assert_own_line(&context, FOR_CLIENT_A, msg_id, &web.api());
}
