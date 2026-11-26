//! Starts an owned fixture process and writes its delivery report to stdout.

use std::io::{self, BufRead, BufReader};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use clap::Parser;
use skald_github_stand_in::{App, GitHub};
use skald_load_delivery::{
    Delivery, FIXTURE_SEND_LIMIT, FIXTURE_SERVER_NAME, Report, Result, Settings,
};
use url::Url;

const CLIENT: &str = "skald-delivery-fixture";
const SECRET: &str = "not-a-real-secret";

/// Measure delivery in a short run against a new, locally owned server process.
#[derive(Parser)]
#[command(
    after_help = "Set SKALD_LOAD_DATABASE_URL to an empty disposable skald_load_* database on 127.0.0.1 with an explicit port. The tool starts and stops its own server and GitHub stand-in. Dispose of the database after the run."
)]
struct Cli {
    /// Path to a locally built skald-server.
    #[arg(long)]
    server_binary: PathBuf,
    /// Strictly increasing message rates per second.
    #[arg(long, value_delimiter = ',', default_value = "1,2,4")]
    rates: Vec<u64>,
    /// Seconds offered at each rate.
    #[arg(long, default_value_t = 2)]
    seconds: u64,
    /// Synthetic device count.
    #[arg(long, default_value_t = 4)]
    devices: usize,
    /// Bytes of synthetic text in each message.
    #[arg(long, default_value_t = 64)]
    text_bytes: usize,
    /// Largest permitted p99 delay from observing Stored to observing the push.
    #[arg(long, default_value_t = 500)]
    max_p99_push_ms: u64,
    /// Smallest permitted rate of confirmed and read-back history writes.
    #[arg(long, default_value_t = 0.0)]
    min_history_per_second: f64,
}

fn main() -> ExitCode {
    let run = execute(Cli::parse()).and_then(|report| {
        let mut output = io::stdout().lock();
        serde_json::to_writer_pretty(&mut output, &report)?;
        use io::Write;
        writeln!(&mut output)?;
        Ok(if report.first_failed_check.is_none() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        })
    });
    match run {
        Ok(status) => status,
        Err(error) => {
            eprintln!("skald-load-delivery: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(cli: Cli) -> Result<Report> {
    let settings = Settings {
        devices: cli.devices,
        rates: cli.rates,
        seconds: cli.seconds,
        text_bytes: cli.text_bytes,
        max_p99_push: Duration::from_millis(cli.max_p99_push_ms),
        min_history_per_second: cli.min_history_per_second,
    };
    settings.validate()?;
    let database = std::env::var("SKALD_LOAD_DATABASE_URL")
        .map_err(|_| "set SKALD_LOAD_DATABASE_URL to a new local skald_load_* fixture database")?;
    check_database(&database)?;
    let path = cli.server_binary.canonicalize()?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr()?.port());
    let github = GitHub::start(App {
        client_id: CLIENT.to_owned(),
        client_secret: SECRET.to_owned(),
        callback: Some(format!("http://{address}/auth/github/callback")),
    });
    drop(listener);
    let _server = Server::start(&path, address, &database, &github)?;
    Delivery::prepare(address, &github, settings)?.run()
}

fn check_database(database: &str) -> Result<()> {
    let database = Url::parse(database).map_err(|_| "invalid fixture database URL")?;
    let name = database.path().strip_prefix("/skald_load_");
    if !matches!(database.scheme(), "postgres" | "postgresql")
        || database.host_str() != Some("127.0.0.1")
        || database.port().is_none()
        || database.query().is_some()
        || database.fragment().is_some()
        || !name.is_some_and(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
    {
        return Err("use a new skald_load_* database on 127.0.0.1 with an explicit port".into());
    }
    Ok(())
}

struct Server(Child);

impl Server {
    fn start(path: &Path, address: SocketAddrV4, database: &str, github: &GitHub) -> Result<Self> {
        let address = address.to_string();
        let public = format!("http://{address}");
        let mut server = Self(
            Command::new(path)
                .env_clear()
                .env("SKALD_DATABASE_URL", database)
                .env("SKALD_GITHUB_CLIENT_SECRET", SECRET)
                .args([
                    "--listen",
                    &address,
                    "--public-url",
                    &public,
                    "--server-name",
                    FIXTURE_SERVER_NAME,
                    "--github-client-id",
                    CLIENT,
                    "--github-url",
                    github.url(),
                    "--github-api-url",
                    github.url(),
                    "--sign-up",
                    "open",
                    "--device-poll-interval",
                    "1",
                    "--send-rate-limit",
                    &FIXTURE_SEND_LIMIT.to_string(),
                    "--send-rate-window",
                    "1",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        let stderr = server.0.stderr.take().expect("piped stderr");
        let ready = format!("skald-server: listening on {public}");
        let (started, result) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if line == ready {
                    let _ = started.send(());
                }
            }
        });
        result
            .recv_timeout(Duration::from_secs(15))
            .map_err(|_| "the local fixture server did not start")?;
        Ok(server)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::check_database;

    #[test]
    fn only_explicit_local_fixture_databases_are_taken() {
        assert!(check_database("postgres://local@127.0.0.1:5432/skald_load_run1").is_ok());
        for url in [
            "postgres://local@127.0.0.1:5432/skald",
            "postgres://local@127.0.0.1:5432/skald_load_run?host=example.test",
            "postgres://local@example.test:5432/skald_load_run",
            "postgres://local@127.0.0.1/skald_load_run",
        ] {
            assert!(check_database(url).is_err(), "{url}");
        }
    }
}
