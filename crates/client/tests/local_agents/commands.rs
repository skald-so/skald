//! `skald send` and `skald reply` between agents of one device, and between an agent and its
//! person, in a channel whose group holds the device alone.

use crate::kit::received_as_agent;
use crate::lines::{agent_line_of, assert_says_sent};
use crate::receipts::read_lines;
use crate::replying::{REPLY, replied_lines};
use crate::solo::{API, OWN, SEND, Solo, TO, WEB, assert_got, assert_not_shown, replies};
use crate::talk::{ADA_HOST, one_message_sent};
use crate::world::AGENT;

/// What api asks web and web answers with the commands, and what the person asks web and web
/// answers with the tool; none is anywhere else.
const ASKED: &str = "local commands test: api asks web whether the tests pass";
const ANSWERED: &str = "local commands test: web says every test passes";
const PERSON_ASKS: &str = "local commands test: ada asks web for the deploy log";
const WEB_ANSWERS: &str = "local commands test: web sends ada the deploy log";

/// Ada's machine, alone in `clienta`'s groups, has run as web in `~/work/clientA/web`. api's
/// `skald send --to ada/web` in `~/work/clientA/api` says `sent #<ref>` of one message in
/// `clienta`'s default channel; web's `skald recv` shows it from `ada/api` at `own`. web's
/// `skald reply` to its `#<ref>` says `sent #<ref>` of one message there, which api's
/// `skald recv` shows from `ada/web` at `own`, and web's next one does not. The person's next
/// `skald recv` shows `read` and `replied` of api's message once each.
#[test]
fn send_and_reply_commands_between_agents_of_one_device_give_read_and_replied() {
    let solo = Solo::new(4_657_009_001, "ada-agents-use-commands");
    let (api_dir, web_dir) = (solo.dir(API), solo.dir(WEB));
    received_as_agent(solo.machine(), &web_dir, WEB);
    let mark = solo.mark();

    let sent = (solo.machine())
        .start_in(&api_dir, &[AGENT, API, SEND, TO, &solo.short(WEB), ASKED])
        .finish();

    assert!(sent.success, "{sent:?}");
    let asked = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    assert_says_sent(&sent.stdout, asked);
    let got = received_as_agent(solo.machine(), &web_dir, WEB).stdout;
    assert_got(&got, ASKED, asked, (&solo.short(API), ADA_HOST, OWN));
    let reference = format!("#{}", agent_line_of(&got, ASKED).reference);
    let mark = solo.mark();
    let replied = (solo.machine())
        .start_in(&web_dir, &[AGENT, WEB, REPLY, &reference, ANSWERED])
        .finish();
    assert!(replied.success, "{replied:?}");
    let answered = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    assert_says_sent(&replied.stdout, answered);
    let back = received_as_agent(solo.machine(), &api_dir, API).stdout;
    assert_got(&back, ANSWERED, answered, (&solo.short(WEB), ADA_HOST, OWN));
    let again = received_as_agent(solo.machine(), &web_dir, WEB).stdout;
    assert_not_shown(&again, ANSWERED, WEB);
    let person = solo.person_reads();
    assert_eq!(read_lines(&person, asked), 1, "not read once: {person}");
    let shown = replied_lines(&person, asked, &[ANSWERED]);
    assert_eq!(shown.len(), 1, "not replied once: {person}");
}

/// A session web runs in `~/work/clientA/web`. The person's `skald send --to ada/web` in
/// `~/work/clientA` succeeds; web's `inbox` shows it from the person's full address at `own`, and
/// web's reply tool answers it. The person's next `skald recv` shows the answer once and `replied`
/// of the person's message once; web's next `inbox` does not show the answer.
#[test]
fn reply_to_the_person_s_message_reaches_the_person_on_the_same_device() {
    let solo = Solo::new(4_657_010_001, "ada-asks-her-own-agent");
    let mut web = solo.session(WEB);
    let mark = solo.mark();

    let sent = solo.person_runs(&[SEND, TO, &solo.short(WEB), PERSON_ASKS]);

    assert!(sent.success, "{sent:?}");
    let asked = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    let got = web.inbox();
    assert_got(&got, PERSON_ASKS, asked, (&solo.person(), ADA_HOST, OWN));
    replies(&mut web, &got, PERSON_ASKS, WEB_ANSWERS);
    let person = solo.person_reads();
    assert_eq!(person.matches(WEB_ANSWERS).count(), 1, "{person}");
    let shown = replied_lines(&person, asked, &[WEB_ANSWERS]);
    assert_eq!(shown.len(), 1, "not replied once: {person}");
    assert_not_shown(&web.inbox(), WEB_ANSWERS, WEB);
}
