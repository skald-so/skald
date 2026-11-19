//! The grant device authorization gives a run of `skald sign-in` at a terminal.

use crate::cloud::{Cloud, Grant};
use crate::terminal::Terminal;

/// Waits until device authorization has given `run` a grant after the `before` it gave earlier,
/// for up to [`WAIT`](crate::cloud::WAIT), and `run` shows its code; gives the grant.
#[track_caller]
pub fn shown_grant(cloud: &Cloud, run: &mut Terminal, before: usize) -> Grant {
    let grant = cloud.next_grant(before, || {
        assert!(!run.exited(), "skald exited: {}", run.shown());
        run.shown()
    });
    run.wait_to_show(&grant.user_code);
    grant
}
