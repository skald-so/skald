//! With the server's answer to storing the invite lost, the run resends the same bytes and leaves
//! one invite, the one its link names.

use crate::relay::Lose;
use crate::secret::assert_secret_nowhere;
use crate::{
    Admin, TIMEOUT_SECONDS, TIMEOUT_SETTING, invite_at_terminal, shown_link, stored_since,
};

/// The relay loses, as `how` says, the server's answer to the first request that stores an invite
/// of `acme`; with [`Lose::Withhold`], the run's requests time out after a few seconds. `skald
/// invite acme` at a terminal succeeds and shows a link; the server took the request more than
/// once, holds one invite under the IDs it issued in the run, the one the link names, and the
/// secret is nowhere but in what the run showed.
fn one_invite_after_a_lost_answer(how: Lose, ada: (u64, &str)) {
    let mut admin = Admin::new(ada);
    admin.ada.machine = admin.ada.machine.with_env(TIMEOUT_SETTING, TIMEOUT_SECONDS);
    let cloud = admin.cloud();
    let request = format!("PUT /spaces/{}/invites/", admin.space);
    let since = cloud.relay.mark();
    cloud.relay.lose_answer(&request, how);

    let shown = invite_at_terminal(&admin.ada.machine, &[]);

    assert!(shown.success, "{}", shown.text);
    assert_eq!(cloud.relay.lost(), 1, "the server never answered {request}");
    assert!(
        cloud.count_sent(since, &request) >= 2,
        "the invite was not sent again: {:?}",
        cloud.relay.sent_since(since)
    );
    let (_, link) = shown_link(&shown.text, &cloud.public);
    let stored: Vec<_> = stored_since(cloud, &admin.ada.token, since)
        .into_iter()
        .map(|invite| invite.body().invite)
        .collect();
    assert_eq!(stored, [link.invite], "the invites the server holds");
    assert_secret_nowhere(&admin, &link, &shown.text);
}

#[test]
fn answer_dropped_by_the_proxy_leaves_one_invite() {
    one_invite_after_a_lost_answer(Lose::Close, (6_418_303_001, "ada-invite-answer-dropped"));
}

#[test]
fn answer_withheld_by_the_proxy_leaves_one_invite() {
    one_invite_after_a_lost_answer(
        Lose::Withhold,
        (6_418_303_101, "ada-invite-answer-withheld"),
    );
}
