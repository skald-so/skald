//! A wrap the server stored, its answer lost, is not uploaded again: a later run finds it held.

use crate::fixtures::Keyed;
use crate::front::Answers;
use crate::kit::{RECV, received};
use crate::machine::Finished;
use crate::talk::Talk;
use crate::world::run;
use crate::wraps::{assert_wrapped, sent_to, valid_browser};

/// Ada's next `skald recv`, while the front does `answers` with the server's answers to her
/// uploads: fails unless it sends the wrap of epoch 1 to `browser`, which the front forwards. The
/// front forwards the answers again after it. Gives the run.
#[track_caller]
pub fn wraps_first_epoch_answered(talk: &Talk, browser: &Keyed, answers: Answers) -> Finished {
    let from = talk.front.uploads().len();
    talk.front.answer_history(answers);
    let ran = run(&talk.ada.machine, &[RECV]);
    talk.front.answer_history(Answers::Forward);
    let sent = sent_to(&talk.front, talk.channel, from, browser.id);
    assert!(
        sent.iter().any(|one| one.epoch == 1 && one.forwarded),
        "no wrap of epoch 1 sent: {sent:?}"
    );
    ran
}

/// Ada's next two runs: fails if they send `browser` a wrap.
#[track_caller]
pub fn assert_none_sent_again(talk: &Talk, browser: &Keyed) {
    let later = talk.front.uploads().len();
    received(&talk.ada.machine);
    received(&talk.ada.machine);
    let sent = sent_to(&talk.front, talk.channel, later, browser.id);
    assert!(sent.is_empty(), "wraps sent again: {sent:?}");
}

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
    let ran = wraps_first_epoch_answered(&talk, &browser, Answers::Lose);
    assert!(ran.success, "{ran:?}");

    assert_none_sent_again(&talk, &browser);

    assert_wrapped(&mut talk, &browser, 1);
}
