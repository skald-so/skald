//! The `skald-server` binary.

use std::net::SocketAddr;
use std::process::ExitCode;

use clap::Parser;
use tokio::net::TcpListener;

/// Skald server.
#[derive(Parser)]
#[command(name = "skald-server", version)]
struct Cli {
    /// Address to listen on.
    #[arg(long, env = "SKALD_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,

    /// PostgreSQL connection URL, e.g. postgres://skald:password@localhost/skald.
    #[arg(long, env = "SKALD_DATABASE_URL", hide_env_values = true)]
    database_url: String,

    /// Protocol versions to accept: <n>, or <oldest>-<latest> for two adjacent versions
    /// [default: the current version and the previous one].
    #[arg(long, env = "SKALD_PROTOCOL_VERSIONS", value_name = "VERSIONS")]
    protocol_versions: Option<skald_server::Versions>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("skald-server: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Connects to PostgreSQL, applies pending migrations, then serves the API.
async fn run(cli: Cli) -> Result<(), String> {
    let db = skald_server::connect(&cli.database_url)
        .await
        .map_err(|err| format!("cannot connect to PostgreSQL: {err}"))?;
    skald_server::MIGRATOR
        .run(&db)
        .await
        .map_err(|err| format!("cannot apply migrations: {err}"))?;

    let listen = cli.listen;
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| format!("cannot listen on {listen}: {err}"))?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    eprintln!("skald-server: listening on http://{addr}");
    axum::serve(
        listener,
        skald_server::app(db, cli.protocol_versions.unwrap_or_default()),
    )
    .await
    .map_err(|err| err.to_string())
}
