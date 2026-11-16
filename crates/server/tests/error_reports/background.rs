//! The server's background tasks report their failures too.

use crate::common::{Server, TestDb};
use crate::sentry::Sentry;
use crate::{DSN_ENV, WITHIN, reported};

const KEY: &str = "backgroundkey7e8f9a0b";

/// While the database is down, the mailbox expiry and the listening to other instances fail.
#[test]
fn a_background_failure_is_reported() {
    let sentry = Sentry::start();
    let dsn = sentry.dsn(KEY, 13);
    let db = TestDb::create();
    let _server = Server::launch(&db, "127.0.0.1:0", &[], &[(DSN_ENV, &dsn)]);
    db.set_reachable(false);

    sentry.wait_until(WITHIN, |reports| {
        [
            "cannot expire mailbox entries",
            "cannot listen to the other instances",
        ]
        .iter()
        .all(|failure| reported(reports, KEY, 13, failure))
    });
}
