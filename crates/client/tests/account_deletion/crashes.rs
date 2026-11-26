//! Killed once its request to delete the account was sent, the command keeps the local state until
//! its next run, which finds the account deleted and finishes.

use crate::deletion::{
    AtGrant, DELETE_ACCOUNT, DELETE_REQUEST, Held, assert_deleted, assert_keeps_nothing,
    delete_at_terminal, deleted_lines, drive,
};
use crate::fixtures::fronted;
use crate::relay::Lose;
use crate::talk::ADA_HOST;
use crate::terminal::Terminal;

/// The relay withholds the server's answer to the request of Ada's `skald delete-account` at a
/// terminal that deletes her account, and the test kills the run once the server has answered:
/// the server has deleted the account, and her machine still holds its session token. Her next
/// `skald delete-account` at a terminal asks to sign in to nothing, exits with success, says the
/// account is deleted, and her machine's state directory then keeps nothing of it.
#[test]
fn killed_after_sending_its_next_run_deletes_the_local_state() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_005_001, "ada-killed-mid-deletion", ADA_HOST);
    let held = Held::of(cloud, &ada);
    cloud.relay.lose_answer(DELETE_REQUEST, Lose::Withhold);

    let before = cloud.grants().len();
    let mut run = Terminal::start(&ada.machine, &[DELETE_ACCOUNT]);
    let typed = (ada.handle.as_str(), AtGrant::Approve(&ada.user), before);
    drive(&world, &mut run, typed, |run| {
        assert!(!run.exited(), "it exited: {}", run.shown());
        cloud.relay.lost() > 0
    });
    let killed = run.kill();

    assert_deleted(cloud, &ada);
    assert!(
        !cloud.token_files(&ada.machine).is_empty(),
        "the local state went before the server's answer: {}",
        killed.text
    );

    let next = delete_at_terminal(&world, &ada.machine, ada.handle.as_str(), AtGrant::Refuse);

    let text = &next.shown.text;
    assert!(next.shown.success, "{text}");
    assert!(
        !deleted_lines(text).is_empty(),
        "it does not say the account is deleted: {text}"
    );
    assert_keeps_nothing(cloud, &ada, &held);
}
