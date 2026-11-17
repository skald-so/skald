//! Entries that stay back: that of a message whose envelope the server refused is never uploaded,
//! and an agent's run uploads none of another space's.

use crate::entries::{ada_s_message, assert_one_entry, entries_of, talk_entries};
use crate::front::Answers;
use crate::kit::{
    RECV, assert_names_none, bind_client_a, create_space, received, received_as_agent,
};
use crate::talk::Talk;
use crate::world::{CLIENT_A, CLIENT_B, run};

/// The agent the tests run as.
const CLAUDE: &str = "claude-h5e2q8";

/// The front turns the server's `stored` for Ada's message into a `send-refused`. Neither her
/// `skald send` nor her next run, with the answers forwarded, uploads an entry of the message.
#[test]
fn entry_of_a_refused_message_is_never_uploaded() {
    let talk = Talk::new(
        &[4_165_007_001, 4_165_007_002],
        &["ada-message-refused", "bob-gets-no-entry"],
        true,
    );
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());
    talk.front
        .answer(Answers::Refuse("quota-exceeded".to_owned()));

    let said = talk.send("a line the server refuses");

    let msg_id = ada_s_message(&talk, sends);
    let sent = entries_of(&talk.front, talk.channel, uploads, msg_id);
    assert!(
        sent.is_empty(),
        "an entry of {msg_id} sent: {sent:?} {said:?}"
    );
    talk.front.answer(Answers::Forward);

    drop(run(&talk.ada.machine, &[RECV]));

    let sent = entries_of(&talk.front, talk.channel, uploads, msg_id);
    assert!(sent.is_empty(), "an entry of {msg_id} sent later: {sent:?}");
    let held = talk_entries(&talk);
    assert!(held.is_empty(), "the server holds an entry: {held:?}");
}

/// Ada's machine follows `clientb`, where she sends a message whose entry's answer the front
/// loses, so the entry waits, and `clienta`, to which `~/work/clientA` is bound. As an agent there,
/// `skald recv` uploads no entry of the message and names nothing of it; the person's next run
/// uploads it, leaving one entry.
#[test]
fn agent_s_run_uploads_no_entry_of_another_space_and_the_person_s_run_does() {
    let mut talk = Talk::in_space(
        CLIENT_B,
        &[4_165_008_001, 4_165_008_002],
        &["ada-desk-history", "bob-client-history"],
        true,
    );
    create_space(&talk.world.cloud, &talk.ada, CLIENT_A);
    let folder = bind_client_a(&talk.ada.machine);
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());
    talk.front.answer_history(Answers::Lose);
    drop(talk.send("a line of client b for the history"));
    let msg_id = ada_s_message(&talk, sends);
    assert!(
        !entries_of(&talk.front, talk.channel, uploads, msg_id).is_empty(),
        "no entry of {msg_id} sent"
    );
    talk.front.answer_history(Answers::Forward);
    let later = talk.front.uploads().len();
    let names = [
        CLIENT_B.to_owned(),
        talk.space.id.to_string(),
        talk.channel.to_string(),
        msg_id.to_string(),
    ];

    let agent = received_as_agent(&talk.ada.machine, &folder, CLAUDE);

    let again = entries_of(&talk.front, talk.channel, later, msg_id);
    assert!(again.is_empty(), "the agent's run sent an entry: {again:?}");
    assert_names_none(&agent, &names);

    received(&talk.ada.machine);

    let again = entries_of(&talk.front, talk.channel, later, msg_id);
    assert!(
        again.iter().any(|one| one.forwarded),
        "the person's run did not send the entry: {again:?}"
    );
    assert_one_entry(&mut talk, uploads, (msg_id, 1));
}
