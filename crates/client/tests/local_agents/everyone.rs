//! A message to everyone in a channel whose group holds the device alone reaches each of its agents
//! in the space but its sender, and its person unless the person sent it.

use crate::fixtures::{ALL, SEND};
use crate::senders::OWN;
use crate::solo::{API, EVERYONE, Solo, WEB, assert_got, assert_not_shown, session_in};
use crate::talk::{ADA_HOST, one_message_sent};

/// What api says to everyone, and what the person says to everyone; neither is anywhere else.
const FROM_API: &str = "local agents test: api tells everyone the schema changed";
const FROM_PERSON: &str = "local agents test: ada tells her agents to stop for lunch";

/// Ada's machine follows `clienta` and `clientb`, each with groups of her device alone. Sessions
/// api and web run in `~/work/clientA`, and a session `--agent web` in `~/work/clientB`. api's send
/// tool `to` `*` sends one message in `clienta`'s default channel: web's `inbox` shows it from
/// `ada/api` at `own`; api's and the session's in `~/work/clientB` show nothing of it; the person's
/// next `skald recv` shows it once.
#[test]
fn everyone_from_api_reaches_web_and_the_person_not_api_or_another_space() {
    let solo = Solo::new(4_657_002_001, "ada-tells-every-agent");
    let in_b = solo.with_client_b();
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let mut elsewhere = session_in(solo.machine(), &in_b, WEB);

    let said = solo.sends(&mut api, EVERYONE, FROM_API);

    assert_got(
        &web.inbox(),
        FROM_API,
        said,
        (&solo.short(API), ADA_HOST, OWN),
    );
    assert_not_shown(&api.inbox(), FROM_API, API);
    assert_not_shown(&elsewhere.inbox(), FROM_API, "web in clientb");
    let person = solo.person_reads();
    assert_eq!(person.matches(FROM_API).count(), 1, "{person}");
}

/// Sessions api and web run in `~/work/clientA`. The person's `skald send --all` there, with no
/// channel, succeeds and sends one message in `clienta`'s default channel: each session's `inbox`
/// shows it from the person's full address, Ada's machine, at `own`; the person's next
/// `skald recv` does not show it.
#[test]
fn person_s_send_to_all_reaches_both_sessions_and_not_the_person() {
    let solo = Solo::new(4_657_003_001, "ada-talks-to-her-agents");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let mark = solo.mark();

    let sent = solo.person_runs(&[SEND, ALL, FROM_PERSON]);

    assert!(sent.success, "{sent:?}");
    let said = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    let person = solo.person();
    for session in [&mut api, &mut web] {
        let shown = session.inbox();
        assert_got(&shown, FROM_PERSON, said, (&person, ADA_HOST, OWN));
    }
    assert_not_shown(&solo.person_reads(), FROM_PERSON, "the person");
}
