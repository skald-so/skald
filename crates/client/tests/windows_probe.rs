//! TEMPORARY: probes of the `Client on Windows` flakes. Removed before merge.
#![cfg(windows)]

#[allow(dead_code)]
mod common;
#[allow(dead_code)]
#[path = "sign_in/relay.rs"]
mod relay;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sqlx::{AssertSqlSafe, Connection, PgConnection};

type Stats = Arc<Mutex<BTreeMap<String, usize>>>;

fn count(stats: &Stats, outcome: String) {
    *stats.lock().unwrap().entry(outcome).or_default() += 1;
}

fn report(stats: &Stats) -> String {
    format!("{:?}", stats.lock().unwrap())
}

fn server() -> String {
    std::env::var("SKALD_TEST_DATABASE_URL").expect("SKALD_TEST_DATABASE_URL")
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Busy threads until the flag drops.
fn load(stop: &Arc<AtomicBool>, threads: usize) -> Vec<thread::JoinHandle<()>> {
    (0..threads)
        .map(|_| {
            let stop = Arc::clone(stop);
            thread::spawn(move || {
                let mut x: u64 = 1;
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..10_000 {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    std::hint::black_box(x);
                }
            })
        })
        .collect()
}

fn err_text(err: &sqlx::Error) -> String {
    match err {
        sqlx::Error::Io(io) => format!("io {:?} {:?}", io.kind(), io.raw_os_error()),
        other => {
            let text = other.to_string();
            text.chars().take(80).collect()
        }
    }
}

/// The server's settings that say where its log goes.
fn settings() -> String {
    rt().block_on(async {
        let mut conn = PgConnection::connect(&format!("{}/postgres", server()))
            .await
            .unwrap();
        let mut out = Vec::new();
        for name in [
            "server_version",
            "logging_collector",
            "log_destination",
            "log_directory",
            "log_min_messages",
            "data_directory",
            "max_connections",
        ] {
            let (value,): (String,) = sqlx::query_as(AssertSqlSafe(format!("SHOW {name}")))
                .fetch_one(&mut conn)
                .await
                .unwrap();
            out.push(format!("{name}={value}"));
        }
        let _ = conn.close().await;
        out.join(" ")
    })
}

/// A connection whose backend another connection terminates, then closed or queried.
fn terminated(then_query: bool, rounds: usize) -> String {
    let stats: Stats = Arc::default();
    let runtime = rt();
    for _ in 0..rounds {
        let outcome = runtime.block_on(async {
            let url = format!("{}/postgres", server());
            let mut a = PgConnection::connect(&url).await.unwrap();
            let mut b = PgConnection::connect(&url).await.unwrap();
            let (pid,): (i32,) = sqlx::query_as("SELECT pg_backend_pid()")
                .fetch_one(&mut a)
                .await
                .unwrap();
            sqlx::query("SELECT pg_terminate_backend($1)")
                .bind(pid)
                .execute(&mut b)
                .await
                .unwrap();
            loop {
                let (n,): (i64,) =
                    sqlx::query_as("SELECT count(*) FROM pg_stat_activity WHERE pid = $1")
                        .bind(pid)
                        .fetch_one(&mut b)
                        .await
                        .unwrap();
                if n == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = b.close().await;
            let result = if then_query {
                sqlx::raw_sql("SELECT 1").execute(&mut a).await.map(|_| ())
            } else {
                a.close().await
            };
            match result {
                Ok(()) => "ok".to_owned(),
                Err(err) => err_text(&err),
            }
        });
        count(&stats, outcome);
    }
    report(&stats)
}

/// Connect, a statement, close, on `workers` threads, under `spinners` busy threads.
fn churn(workers: usize, spinners: usize, rounds: usize, create: bool) -> String {
    let stats: Stats = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let busy = load(&stop, spinners);
    let started = Instant::now();
    let threads: Vec<_> = (0..workers)
        .map(|w| {
            let stats = Arc::clone(&stats);
            thread::spawn(move || {
                let runtime = rt();
                for round in 0..rounds {
                    let name = format!("probe_{}_{w}_{round}", std::process::id());
                    let outcome = runtime.block_on(async {
                        let url = format!("{}/postgres", server());
                        let mut conn = match PgConnection::connect(&url).await {
                            Ok(conn) => conn,
                            Err(err) => return format!("connect {}", err_text(&err)),
                        };
                        let statements = if create {
                            vec![
                                format!("DROP DATABASE IF EXISTS {name}"),
                                format!("CREATE DATABASE {name}"),
                                format!("DROP DATABASE {name} WITH (FORCE)"),
                            ]
                        } else {
                            vec!["SELECT 1".to_owned()]
                        };
                        for sql in statements {
                            if let Err(err) =
                                sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await
                            {
                                return format!("statement {}", err_text(&err));
                            }
                        }
                        match conn.close().await {
                            Ok(()) => "ok".to_owned(),
                            Err(err) => format!("close {}", err_text(&err)),
                        }
                    });
                    count(&stats, outcome);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for t in busy {
        t.join().unwrap();
    }
    format!("{:?} {}", started.elapsed(), report(&stats))
}

/// Requests through the relay to the server, under load.
fn relayed(workers: usize, spinners: usize, rounds: usize) -> String {
    let server = common::Server::start(None);
    let relay = relay::Relay::start(server.url.trim_start_matches("http://"));
    let stats: Stats = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let busy = load(&stop, spinners);
    let started = Instant::now();
    let threads: Vec<_> = (0..workers)
        .map(|_| {
            let stats = Arc::clone(&stats);
            let url = format!("{}/health", relay.url);
            thread::spawn(move || {
                for _ in 0..rounds {
                    let agent = ureq::Agent::config_builder()
                        .proxy(None)
                        .http_status_as_error(false)
                        .build()
                        .new_agent();
                    let outcome = match agent.get(&url).call() {
                        Ok(response) => format!("status {}", response.status().as_u16()),
                        Err(err) => format!("error {err}"),
                    };
                    count(&stats, outcome);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for t in busy {
        t.join().unwrap();
    }
    format!("{:?} {}", started.elapsed(), report(&stats))
}

/// A fresh copy of skald, then a rename of it at once, or after a run, or a staged file renamed.
fn renames(rounds: usize, how: &str) -> String {
    let stats: Stats = Arc::default();
    let dir = std::env::temp_dir().join(format!("skald-client-probe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let skald = fs::read(env!("CARGO_BIN_EXE_skald")).unwrap();
    for round in 0..rounds {
        let exe = dir.join(format!("skald-{how}-{round}.exe"));
        let outcome = match how {
            "copy" => {
                fs::copy(env!("CARGO_BIN_EXE_skald"), &exe).unwrap();
                fs::rename(&exe, exe.with_extension("old"))
            }
            "run" => {
                fs::copy(env!("CARGO_BIN_EXE_skald"), &exe).unwrap();
                let mut child = std::process::Command::new(&exe)
                    .arg("--version")
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                let moved = fs::rename(&exe, exe.with_extension("old"));
                let _ = child.wait();
                moved
            }
            _ => {
                let staged = exe.with_extension("new");
                let mut file = fs::File::create_new(&staged).unwrap();
                file.write_all(&skald).unwrap();
                file.sync_all().unwrap();
                drop(file);
                fs::rename(&staged, &exe)
            }
        };
        count(
            &stats,
            match outcome {
                Ok(()) => "ok".to_owned(),
                Err(err) => format!("{:?}", err.raw_os_error()),
            },
        );
    }
    let _ = fs::remove_dir_all(&dir);
    report(&stats)
}

/// A plain std server behind the relay, to tell the relay from hyper.
fn relayed_plain(workers: usize, spinners: usize, rounds: usize) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let mut buf = [0; 4096];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                );
            });
        }
    });
    let relay = relay::Relay::start(&addr.to_string());
    let stats: Stats = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let busy = load(&stop, spinners);
    let threads: Vec<_> = (0..workers)
        .map(|_| {
            let stats = Arc::clone(&stats);
            let url = relay.url.trim_start_matches("http://").to_owned();
            thread::spawn(move || {
                for _ in 0..rounds {
                    let outcome = (|| {
                        let mut s = TcpStream::connect(&url).map_err(|e| format!("connect {e}"))?;
                        s.write_all(b"GET / HTTP/1.1\r\nhost: x\r\n\r\n")
                            .map_err(|e| format!("write {:?}", e.raw_os_error()))?;
                        let mut out = Vec::new();
                        s.read_to_end(&mut out)
                            .map_err(|e| format!("read {:?}", e.raw_os_error()))?;
                        Ok::<_, String>(if out.ends_with(b"ok") {
                            "ok".into()
                        } else {
                            format!("short {}", out.len())
                        })
                    })()
                    .unwrap_or_else(|e| e);
                    count(&stats, outcome);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for t in busy {
        t.join().unwrap();
    }
    report(&stats)
}

/// PostgreSQL's log files, their sizes and the lines that name failures.
fn pg_log() -> String {
    let Ok(data) = std::env::var("PGDATA") else {
        return "no PGDATA".to_owned();
    };
    let dir = std::path::Path::new(&data).join("log");
    let Ok(entries) = fs::read_dir(&dir) else {
        return format!("no {}", dir.display());
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let bytes = fs::read(entry.path()).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        let failing: Vec<&str> = text
            .lines()
            .filter(|line| line.contains("could not") || line.contains("FATAL"))
            .collect();
        out.push(format!(
            "{:?} {} bytes, {} failing, last: {:?}",
            entry.file_name(),
            bytes.len(),
            failing.len(),
            failing.iter().rev().take(3).collect::<Vec<_>>()
        ));
    }
    out.join("; ")
}

#[test]
fn probe() {
    let cores = thread::available_parallelism().map_or(4, |n| n.get());
    let mut out = vec![format!("cores {cores}"), settings(), pg_log()];
    out.push(format!("terminated+close {}", terminated(false, 10)));
    out.push(format!("terminated+query {}", terminated(true, 10)));
    out.push(format!("renames copy {}", renames(40, "copy")));
    out.push(format!("renames run {}", renames(40, "run")));
    out.push(format!("renames staged {}", renames(40, "staged")));
    out.push(format!("churn select no load {}", churn(8, 0, 100, false)));
    out.push(format!(
        "churn select load {}",
        churn(8, cores * 2, 100, false)
    ));
    out.push(format!(
        "churn create load {}",
        churn(4, cores * 2, 30, true)
    ));
    out.push(format!(
        "relayed plain load {}",
        relayed_plain(8, cores * 2, 300)
    ));
    out.push(format!(
        "relayed server load {}",
        relayed(8, cores * 2, 200)
    ));
    panic!("PROBE\n{}", out.join("\n"));
}
