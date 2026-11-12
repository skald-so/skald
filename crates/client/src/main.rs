//! The `skald` client binary.

mod connection;
mod device;
mod secrets;
mod sign_in;
mod state;
#[cfg(test)]
mod test_dir;

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use connection::{Connection, TIMEOUT};
use device::First;
use skald_proto::VersionRange;

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
    protocol_versions: Option<VersionRange>,

    /// How long a request to the server may take, in seconds, for tests [default: 30].
    #[arg(
        long,
        env = "SKALD_TIMEOUT",
        value_name = "SECONDS",
        global = true,
        hide = true
    )]
    timeout: Option<NonZeroU64>,

    /// The directory of the local state and of the secrets file [default: the platform's directory
    /// for application state, such as ~/.local/state/skald].
    #[arg(long, env = "SKALD_STATE_DIR", value_name = "DIR", global = true)]
    state_dir: Option<PathBuf>,

    /// Where secrets, such as the session token, go: auto keeps them in the system's key storage,
    /// or in a 0600 file in the state directory when the system has none; file keeps them only in
    /// the file.
    #[arg(
        long,
        env = "SKALD_KEY_STORAGE",
        value_name = "STORAGE",
        global = true,
        default_value = "auto"
    )]
    key_storage: secrets::Storage,

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
    /// Signs this machine in to the server: its person approves a code in a browser on any device.
    /// Run again, checks the session with the server and says which account it is signed in to.
    ///
    /// At its first sign-in, the machine generates its device's keys, which never leave it, and
    /// registers the device. The account's first device creates the account's master key and
    /// issues its own certificate with it; any other device waits for linking.
    SignIn {
        /// The server's URL, such as http://localhost:8080.
        #[arg(long, value_name = "URL")]
        server: String,
        /// Whether this is the account's first device, for scripts [default: ask].
        #[arg(long, value_name = "ANSWER")]
        first_device: Option<Answer>,
        /// The device's name, 1 to 64 bytes, set at its first sign-in [default: the host name].
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

/// An answer to a question.
#[derive(Clone, Copy, clap::ValueEnum)]
enum Answer {
    Yes,
    No,
}

/// The setting of the host name the client reads, for tests; empty, the host name is empty.
const HOST_NAME: &str = "SKALD_HOST_NAME";

fn main() -> ExitCode {
    let cli = Cli::parse();
    let speaks = cli.protocol_versions.unwrap_or_default();
    let timeout = cli
        .timeout
        .map_or(TIMEOUT, |seconds| Duration::from_secs(seconds.get()));
    match cli.command {
        Command::Status { server } => status(&server, speaks, timeout),
        Command::SignIn {
            server,
            first_device,
            name,
        } => {
            let options = sign_in::Options {
                storage: cli.key_storage,
                first: match first_device {
                    Some(Answer::Yes) => First::Yes,
                    Some(Answer::No) => First::No,
                    None => First::Ask,
                },
                name,
                host_name: std::env::var_os(HOST_NAME)
                    .unwrap_or_else(gethostname::gethostname)
                    .to_string_lossy()
                    .into_owned(),
            };
            let result = state::dir(cli.state_dir).and_then(|dir| {
                let mut connection =
                    Connection::new(&server, speaks, timeout).map_err(|err| err.to_string())?;
                sign_in::run(&mut connection, &dir, &options).map_err(|err| err.to_string())
            });
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("skald: {err}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// `skald status`: requests the protocol versions and opens a WebSocket, then reports.
fn status(server: &str, speaks: VersionRange, timeout: Duration) -> ExitCode {
    let mut connection = match Connection::new(server, speaks, timeout) {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("skald: {err}");
            return ExitCode::FAILURE;
        }
    };
    let result = connection
        .get("/protocol")
        .and_then(|_| connection.open_websocket("/ws"))
        // The WebSocket carries nothing yet: it closes on drop.
        .map(drop);
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
