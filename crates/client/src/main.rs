//! The `skald` client binary.

mod connection;

use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use connection::{Connection, Versions, WebSocket};
use tungstenite::stream::MaybeTlsStream;

/// Message broker and task queue for AI agents.
#[derive(Parser)]
#[command(name = "skald", version, arg_required_else_help = true)]
struct Cli {
    /// Protocol versions to speak, for tests: <n> or <oldest>-<latest>
    /// [default: the current version and the previous one].
    #[arg(
        long,
        env = "SKALD_PROTOCOL_VERSIONS",
        value_name = "VERSIONS",
        global = true,
        hide = true
    )]
    protocol_versions: Option<Versions>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Shows whether the server is reachable and speaks a compatible protocol version.
    Status {
        /// The server's URL, such as http://localhost:8080.
        #[arg(long, value_name = "URL")]
        server: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let speaks = cli.protocol_versions.unwrap_or_default();
    match cli.command {
        Command::Status { server } => status(&server, speaks),
    }
}

/// `skald status`: requests the protocol versions and opens a WebSocket, then reports.
fn status(server: &str, speaks: Versions) -> ExitCode {
    let mut connection = match Connection::new(server, speaks) {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("skald: {err}");
            return ExitCode::FAILURE;
        }
    };
    let result = connection
        .get("/protocol")
        .and_then(|_| connection.open_websocket("/ws"))
        .map(close);
    match result {
        Ok(()) => {
            println!("Server {server}: reachable");
            println!("Protocol: version {}, compatible", connection.version());
            if let Some(latest) = connection.update_available() {
                println!(
                    "Update available: the server speaks protocol version {latest}; update skald"
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            let reached = if err.reached() {
                "reachable"
            } else {
                "unreachable"
            };
            println!("Server {server}: {reached}");
            if err.incompatible() {
                println!("Protocol: incompatible");
            }
            eprintln!("skald: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Closes `socket`, waiting a few seconds for the server to close it too.
fn close(mut socket: WebSocket) {
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    }
    if socket.close(None).is_ok() {
        while socket.read().is_ok() {}
    }
}
