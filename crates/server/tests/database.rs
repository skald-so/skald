//! Runs `skald-server` against PostgreSQL: connection and migrations at start.

mod common;

use std::net::TcpListener;
use std::process::Command;

use common::{SERVER, Server, TestDb, start_failure};

/// The embedded migrations as `_sqlx_migrations` records them once applied: version, checksum.
fn embedded_migrations() -> Vec<(i64, Vec<u8>)> {
    skald_server::MIGRATOR
        .iter()
        .map(|migration| (migration.version, migration.checksum.to_vec()))
        .collect()
}

#[test]
fn start_applies_migrations_to_empty_database() {
    let db = TestDb::create();
    let _server = Server::start(&db);

    let applied = db.applied_migrations();
    assert!(!applied.is_empty(), "no migrations applied");
    assert!(
        applied.iter().all(|migration| migration.success),
        "failed migrations: {applied:?}"
    );
    assert_eq!(
        applied
            .into_iter()
            .map(|migration| (migration.version, migration.checksum))
            .collect::<Vec<_>>(),
        embedded_migrations()
    );
}

#[test]
fn restart_does_not_reapply_migrations() {
    let db = TestDb::create();
    drop(Server::start(&db));
    let applied = db.applied_migrations();

    let server = Server::start(&db);

    assert_eq!(server.get("/health").status, 200);
    assert_eq!(db.applied_migrations(), applied);
}

#[test]
fn exits_with_error_when_migrations_fail() {
    let db = TestDb::create();
    drop(Server::start(&db));
    // An applied migration whose file has changed since.
    db.execute(
        "UPDATE _sqlx_migrations SET checksum = '\\x00'
         WHERE version = (SELECT max(version) FROM _sqlx_migrations)",
    );

    let stderr = start_failure(&["--listen", "127.0.0.1:0", "--database-url", &db.url()]);

    assert!(
        stderr.starts_with("skald-server: cannot apply migrations: "),
        "stderr: {stderr:?}"
    );
}

#[test]
fn exits_with_error_when_database_is_unreachable() {
    let closed = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let addr = closed.local_addr().expect("local address");
    drop(closed);

    let stderr = start_failure(&[
        "--listen",
        "127.0.0.1:0",
        "--database-url",
        &format!("postgres://skald@{addr}/skald"),
    ]);

    assert!(
        stderr.starts_with("skald-server: cannot connect to PostgreSQL: "),
        "stderr: {stderr:?}"
    );
}

#[test]
fn exits_with_error_without_database_url() {
    let output = Command::new(SERVER)
        .env_remove("SKALD_DATABASE_URL")
        .output()
        .expect("run skald-server");

    assert!(!output.status.success(), "exit status: {}", output.status);
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("--database-url"), "stderr: {stderr:?}");
}
