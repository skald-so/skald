//! A wrap the server refuses is not uploaded again, and the run's warning names its browser.

use crate::front::Answers;
use crate::kit::{RECV, received};
use crate::talk::Talk;
use crate::world::run;
use crate::wraps::{held_epochs, sent_to, valid_browser};

/// The test sets up a browser of Ada's account, and the front turns the server's answers to Ada's
/// uploads into `history-refused` while her next run wraps epoch 1 to it: the run's warning names
/// the browser. With the front forwarding again and the wraps deleted from the server's database,
/// Ada's next two runs send the browser no wrap.
#[test]
fn refused_wrap_is_not_uploaded_again() {
    let talk = Talk::new(
        &[7_300_014_001, 7_300_014_002],
        &["ada-wrap-refused", "bob-sees-wrap-refused"],
        false,
    );
    let browser = valid_browser(&talk.world.cloud, &talk.ada, "ada-browser-refused");
    let from = talk.front.uploads().len();
    talk.front
        .answer_history(Answers::Refuse("not-a-member".to_owned()));
    let ran = run(&talk.ada.machine, &[RECV]);
    talk.front.answer_history(Answers::Forward);
    let sent = sent_to(&talk.front, talk.channel, from, browser.id);
    assert!(
        sent.iter().any(|one| one.epoch == 1 && one.forwarded),
        "no wrap of epoch 1 sent: {sent:?}"
    );
    assert!(
        (ran.stderr.lines())
            .any(|line| line.contains("refused") && line.contains(&browser.id.to_string())),
        "no warning names the browser of the refused wrap: {ran:?}"
    );
    talk.cloud().execute(&format!(
        "DELETE FROM history_wraps WHERE channel_id = '{}'::uuid",
        talk.channel
    ));
    let later = talk.front.uploads().len();

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let sent = sent_to(&talk.front, talk.channel, later, browser.id);
    assert!(sent.is_empty(), "wraps sent again: {sent:?}");
    assert!(
        held_epochs(&talk, browser.id).is_empty(),
        "the server holds a wrap"
    );
}
