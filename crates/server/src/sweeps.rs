//! The server's sweeps: background tasks that look by themselves for rows that are due and delete
//! them, a page at a time.

use std::future::Future;
use std::time::Duration;

use tokio::time::MissedTickBehavior;

use crate::error_reports;

/// How often a sweep looks.
pub(crate) const SWEEP: Duration = Duration::from_secs(5);
/// How many rows a sweep deletes at a time.
pub(crate) const PAGE: i64 = 100;
/// How long a sweep waits between two pages.
pub(crate) const PAUSE: Duration = Duration::from_millis(10);

/// Runs the sweep `sweep` gives every [`SWEEP`], for good. A failure goes to the server's output and
/// its error reports as one to do `doing`, and the next look tries again.
pub(crate) async fn every<F>(doing: &str, mut sweep: impl FnMut() -> F)
where
    F: Future<Output = Result<(), sqlx::Error>>,
{
    let mut looks = tokio::time::interval(SWEEP);
    looks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        looks.tick().await;
        if let Err(err) = sweep().await {
            error_reports::background_failed(doing, &err);
        }
    }
}
