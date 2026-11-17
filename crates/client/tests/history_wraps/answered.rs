//! A wrap the server stored, its answer lost, is not uploaded again: a later run finds it held.

use crate::front::Answers;
use crate::kit::received;
use crate::talk::Talk;
use crate::wraps::{assert_wrapped, sent_to, valid_browser};

/// The test sets up a browser of Ada's account, and the front loses the server's `history-stored`
/// answers while Ada's next run wraps epoch 1 to it. With the front forwarding again, Ada's next
/// two runs send the browser no wrap, and the server holds its one wrap of epoch 1.
#[test]
fn wrap_whose_answer_is_lost_is_not_uploaded_again() {
    let mut talk = Talk::new(
        &[7_300_011_001, 7_300_011_002],
        &["ada-loses-wrap-answer", "bob-sees-wrap-answer-lost"],
        false,
    );
    let browser = valid_browser(&talk.world.cloud, &talk.ada, "ada-browser-unanswered");
    let from = talk.front.uploads().len();
    talk.front.answer_history(Answers::Lose);
    received(&talk.ada.machine);
    talk.front.answer_history(Answers::Forward);
    let sent = sent_to(&talk.front, talk.channel, from, browser.id);
    assert!(
        sent.iter().any(|one| one.epoch == 1 && one.forwarded),
        "no wrap of epoch 1 sent: {sent:?}"
    );
    let later = talk.front.uploads().len();

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let sent = sent_to(&talk.front, talk.channel, later, browser.id);
    assert!(sent.is_empty(), "wraps sent again: {sent:?}");
    assert_wrapped(&mut talk, &browser, 1);
}
