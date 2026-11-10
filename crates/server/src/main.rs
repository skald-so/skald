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
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.listen).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("skald-server: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(listen: SocketAddr) -> Result<(), String> {
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| format!("cannot listen on {listen}: {err}"))?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    eprintln!("skald-server: listening on http://{addr}");
    axum::serve(listener, skald_server::app())
        .await
        .map_err(|err| err.to_string())
}
