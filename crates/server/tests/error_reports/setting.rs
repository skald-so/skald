//! The server reports its errors only when the setting gives a DSN.

use std::thread;

use crate::common::{Server, TestDb};
use crate::sentry::Sentry;
use crate::{AFTER, DSN_ENV, DSN_FLAG, USERNAME, WITHIN, fail_a_request, reported, secret};

/// Checks that a server started with `args` and `envs`, given the DSN of `sentry`'s project
/// `project` with the key `key`, reports a request's error there.
fn assert_reports(sentry: &Sentry, key: &str, project: u32, args: &[&str], envs: &[(&str, &str)]) {
    let db = TestDb::create();
    let server = Server::launch(&db, "127.0.0.1:0", args, envs);
    db.set_reachable(false);

    fail_a_request(&server, &secret(), br#"{"username":"ada"}"#);

    sentry.wait_until(WITHIN, |reports| reported(reports, key, project, USERNAME));
}

#[test]
fn an_error_is_reported_to_the_dsn_of_the_environment_variable() {
    let sentry = Sentry::start();
    let dsn = sentry.dsn("envkey0a1b2c3d4e", 7);
    assert_reports(&sentry, "envkey0a1b2c3d4e", 7, &[], &[(DSN_ENV, &dsn)]);
}

#[test]
fn an_error_is_reported_to_the_dsn_of_the_flag() {
    let sentry = Sentry::start();
    let dsn = sentry.dsn("flagkey5f6a7b8c9", 8);
    assert_reports(&sentry, "flagkey5f6a7b8c9", 8, &[DSN_FLAG, &dsn], &[]);
}

/// Unset, the default and the self-hosted server's, the server sends nothing, also with
/// `SENTRY_DSN`, which the Sentry SDKs read by default, set. Another server on the same database,
/// with the setting, shows how soon a report comes.
#[test]
fn without_the_setting_nothing_is_sent() {
    let sentry = Sentry::start();
    let db = TestDb::create();
    let sdk_dsn = sentry.dsn("sdkkey1d2e3f4a5b", 1);
    let unset = Server::launch(&db, "127.0.0.1:0", &[], &[("SENTRY_DSN", &sdk_dsn)]);
    let set_dsn = sentry.dsn("setkey6c7d8e9f0a", 2);
    let set = Server::launch(&db, "127.0.0.1:0", &[], &[(DSN_ENV, &set_dsn)]);
    db.set_reachable(false);

    fail_a_request(&unset, &secret(), br#"{"username":"ada"}"#);
    fail_a_request(&set, &secret(), br#"{"username":"ada"}"#);
    sentry.wait_until(WITHIN, |reports| {
        reported(reports, "setkey6c7d8e9f0a", 2, USERNAME)
    });
    thread::sleep(AFTER);

    let unexpected: Vec<_> = sentry
        .reports()
        .into_iter()
        .filter(|report| !report.is_for("setkey6c7d8e9f0a", 2))
        .map(|report| format!("{} {}", report.method, report.target))
        .collect();
    assert_eq!(unexpected, Vec::<String>::new(), "sent without the setting");
}
