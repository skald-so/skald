//! TEMPORARY: where PostgreSQL on the Windows runner resets new connections. Removed before merge.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use sqlx::{Connection, PgConnection};

#[test]
fn probe_resets() {
    let server = std::env::var("SKALD_TEST_DATABASE_URL").expect("SKALD_TEST_DATABASE_URL");
    let url = format!("{server}/postgres");
    let mut report = Vec::new();
    for threads in [1, 4, 8] {
        let stats: Arc<Mutex<BTreeMap<String, usize>>> = Arc::default();
        let started = Instant::now();
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let (url, stats) = (url.clone(), Arc::clone(&stats));
                thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    for _ in 0..(600 / threads) {
                        let outcome = runtime.block_on(async {
                            let mut conn = match PgConnection::connect(&url).await {
                                Ok(conn) => conn,
                                Err(err) => return format!("connect: {err}"),
                            };
                            if let Err(err) = sqlx::raw_sql("SELECT 1").execute(&mut conn).await {
                                return format!("query: {err}");
                            }
                            match conn.close().await {
                                Ok(()) => "ok".to_owned(),
                                Err(err) => format!("close: {err}"),
                            }
                        });
                        *stats.lock().unwrap().entry(outcome).or_default() += 1;
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        report.push(format!(
            "{threads} threads, {:?}: {:?}",
            started.elapsed(),
            stats.lock().unwrap()
        ));
    }
    panic!("PROBE\n{}", report.join("\n"));
}
