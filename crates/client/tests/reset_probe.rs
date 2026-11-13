//! TEMPORARY: where PostgreSQL on the Windows runner resets new connections. Removed before merge.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use sqlx::{AssertSqlSafe, Connection, PgConnection};

type Stats = Arc<Mutex<BTreeMap<String, usize>>>;

fn count(stats: &Stats, outcome: String) {
    *stats.lock().unwrap().entry(outcome).or_default() += 1;
}

/// Connects to `url`, runs `SELECT 1` and closes, until `stop`; counts each outcome.
fn connections(url: String, stats: Stats, stop: Arc<AtomicBool>, label: &'static str) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    while !stop.load(Ordering::Relaxed) {
        let outcome = runtime.block_on(async {
            let mut conn = match PgConnection::connect(&url).await {
                Ok(conn) => conn,
                Err(err) => return format!("{label} connect: {err}"),
            };
            if let Err(err) = sqlx::raw_sql("SELECT 1").execute(&mut conn).await {
                return format!("{label} query: {err}");
            }
            match conn.close().await {
                Ok(()) => format!("{label} ok"),
                Err(err) => format!("{label} close: {err}"),
            }
        });
        count(&stats, outcome);
    }
}

/// Creates a database, holds a connection in it, drops it WITH (FORCE), `rounds` times.
fn churn(server: String, stats: Stats, id: usize, rounds: usize) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for round in 0..rounds {
        let name = format!("probe_{}_{id}_{round}", std::process::id());
        let outcome = runtime.block_on(async {
            let admin = format!("{server}/postgres");
            let step = |what: &'static str| move |err: sqlx::Error| format!("churn {what}: {err}");
            let mut conn = PgConnection::connect(&admin)
                .await
                .map_err(step("connect"))?;
            sqlx::raw_sql(AssertSqlSafe(format!("CREATE DATABASE {name}")))
                .execute(&mut conn)
                .await
                .map_err(step("create"))?;
            let mut inside = PgConnection::connect(&format!("{server}/{name}"))
                .await
                .map_err(step("connect inside"))?;
            sqlx::raw_sql("SELECT 1")
                .execute(&mut inside)
                .await
                .map_err(step("query inside"))?;
            sqlx::raw_sql(AssertSqlSafe(format!("DROP DATABASE {name} WITH (FORCE)")))
                .execute(&mut conn)
                .await
                .map_err(step("drop"))?;
            drop(inside);
            conn.close().await.map_err(step("close"))?;
            Ok::<_, String>("churn ok".to_owned())
        });
        count(&stats, outcome.unwrap_or_else(|err| err));
    }
}

#[test]
fn probe_resets() {
    let server = std::env::var("SKALD_TEST_DATABASE_URL").expect("SKALD_TEST_DATABASE_URL");
    let mut report = Vec::new();
    for (churners, connectors) in [(2, 4), (4, 4), (4, 0)] {
        let stats: Stats = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let connecting: Vec<_> = (0..connectors)
            .map(|_| {
                let (url, stats, stop) = (
                    format!("{server}/postgres"),
                    Arc::clone(&stats),
                    Arc::clone(&stop),
                );
                thread::spawn(move || connections(url, stats, stop, "postgres"))
            })
            .collect();
        let churning: Vec<_> = (0..churners)
            .map(|id| {
                let (server, stats) = (server.clone(), Arc::clone(&stats));
                thread::spawn(move || churn(server, stats, id + 10 * churners, 25))
            })
            .collect();
        for thread in churning {
            thread.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        for thread in connecting {
            thread.join().unwrap();
        }
        report.push(format!(
            "{churners} churning, {connectors} connecting, {:?}: {:?}",
            started.elapsed(),
            stats.lock().unwrap()
        ));
    }
    panic!("PROBE\n{}", report.join("\n"));
}
