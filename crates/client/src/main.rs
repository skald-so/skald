//! The `skald` client binary.

mod channels;
mod connection;
mod device;
mod devices;
mod folders;
mod follow;
mod plain_path;
mod secrets;
mod set_username;
mod sign_in;
mod sign_out;
mod spaces;
mod state;
#[cfg(test)]
mod test_dir;
#[cfg(test)]
mod test_dir_tests;
mod tls;
mod update;

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

    /// The directory of the local state and of the secrets file
    #[cfg_attr(windows, doc = r"[default: %LOCALAPPDATA%\skald].")]
    #[cfg_attr(
        target_os = "macos",
        doc = "[default: ~/Library/Application Support/skald]."
    )]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        doc = "[default: $XDG_STATE_HOME/skald or ~/.local/state/skald]."
    )]
    #[arg(long, env = "SKALD_STATE_DIR", value_name = "DIR", global = true)]
    state_dir: Option<PathBuf>,

    /// Where secrets, such as the session token, go: auto keeps them in the system's key storage,
    /// or in a file in the state directory when the system has none; file keeps them only in the
    /// file.
    #[cfg_attr(unix, doc = "The file has 0600 permissions.")]
    #[cfg_attr(
        windows,
        doc = "The file has the access rights it inherits from the state directory."
    )]
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
        /// The server's URL, such as https://api.skald.so; plain http:// only to this machine, such as
        /// http://localhost:8080.
        #[arg(long, value_name = "URL")]
        server: String,
    },
    /// Signs this machine in to the server: its person approves a code in a browser on any device.
    /// Run again, checks the session with the server and says which account it is signed in to.
    ///
    /// At its first sign-in, the machine generates its device's keys, which never leave it, and
    /// registers the device. The account's first device creates the account's master key, issues
    /// its own certificate with it and signs the account's personal space as its admin; any other
    /// device waits for linking.
    SignIn {
        /// The server's URL, such as https://api.skald.so; plain http:// only to this machine, such as
        /// http://localhost:8080.
        #[arg(long, value_name = "URL")]
        server: String,
        /// Whether this is the account's first device, for scripts [default: ask].
        #[arg(long, value_name = "ANSWER")]
        first_device: Option<Answer>,
        /// The device's name, 1 to 64 bytes, set at its first sign-in [default: the host name].
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// Lists the devices of the account this machine signed in to, from the server of its session.
    ///
    /// Each device is checked against the account's master key this machine pinned: a device with
    /// a valid certificate shows by its certificate's name, kind and date issued; any other, such
    /// as one only the server signed, is listed apart as not linked, by its ID only.
    Devices,
    /// Signs this machine out: ends its session on the server it signed in to and deletes the
    /// session token. The device's keys and certificate stay, so the next sign-in to the same
    /// account resumes the same device.
    SignOut,
    /// Sets the username of the account this machine signed in to, on the server of its session.
    /// The server refuses a username that is invalid, reserved or another account's, and the
    /// account keeps its old one.
    SetUsername {
        /// The new username, such as ada-lovelace.
        #[arg(value_name = "USERNAME", allow_hyphen_values = true)]
        username: String,
    },
    /// Creates a team space on the server of this machine's session: this device signs the space's
    /// first roster revision as its first admin, with the account's username as its handle. Run
    /// again after a lost answer, it finishes the same space.
    CreateSpace {
        /// The space's name, such as acme: a-z, 0-9 and '-' inside, 1 to 63 characters.
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        name: String,
    },
    /// Lists the spaces this device follows, as their last accepted roster revisions give them:
    /// each one's name, the account's role and handle in it, and its ID; the device's personal
    /// space is marked personal. Signed in, it first accepts the revisions the server has of each,
    /// and warns of each one that has no effect.
    Spaces,
    /// Shows the members of a space this device follows, as its last accepted roster revision gives
    /// them: each one's handle and role, and its devices with a valid certificate. It first accepts
    /// the revisions the server has, and warns of each one that has no effect.
    Members {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Adds a channel to a space this device follows, on a device of an admin of it: the device
    /// signs the space's next roster revision with the channel, its account the channel's member.
    /// It first accepts the revisions the server has; when another revision comes first, it signs
    /// its change into the new one. Says the channel's ID.
    CreateChannel {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Lists the channels of a space this device follows, as its last accepted roster revision gives
    /// them: each one's ID and its members' handles. It first accepts the revisions the server has.
    Channels {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Binds a folder and everything below it to a space, by a rule in the local state, in place
    /// of the folder's earlier rule. The space may be one this device is not in yet; until it is,
    /// the folder is in no space. At the same folder the rule wins over a .skald.toml.
    AddRule {
        /// The folder, such as ~/work/acme; a symlink stands for its target.
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Lists the rules: each one's folder and space.
    Rules,
    /// Removes a folder's rule.
    RemoveRule {
        /// The folder, such as ~/work/acme.
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
    },
    /// Approves the .skald.toml in a folder as it is now: it binds the folder and everything below
    /// it to the space it names, space = "<name>", until it changes. An unapproved or changed one
    /// binds nothing.
    ApproveBinding {
        /// The folder, such as ~/work/acme.
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
    },
    /// Shows the space of a folder and what binds it to it: a rule or an approved .skald.toml of
    /// the folder or of the nearest folder above it with one, the rule first; with neither, the
    /// personal space this device pinned. A binding to a space this device is not in leaves the
    /// folder in no space. A session's folder is its working directory.
    Folder {
        /// The folder [default: the working directory].
        #[arg(value_name = "FOLDER")]
        folder: Option<PathBuf>,
    },
    /// Updates skald to the latest stable release on GitHub Releases: downloads the release's
    /// archive for this machine, checks it against the release's checksum and replaces this skald
    /// with it. A beta or an older release is never installed; a skald installed from npm is left
    /// to npm. A token in GITHUB_TOKEN, or else GH_TOKEN, goes to GitHub's API only, for its higher
    /// rate limit.
    Update {
        /// The base URL of GitHub's REST API, for tests.
        #[arg(
            long,
            env = "SKALD_GITHUB_API_URL",
            value_name = "URL",
            hide = true,
            default_value = update::GITHUB_API_URL
        )]
        github_api_url: String,
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
                sign_in::run(&mut connection, &dir, &options).map_err(|err| err.to_string())?;
                spaces::take_personal(&dir, options.storage, speaks, timeout);
                Ok(())
            });
            exit(result)
        }
        Command::Devices => {
            let result = state::dir(cli.state_dir)
                .and_then(|dir| devices::run(&dir, cli.key_storage, speaks, timeout));
            exit(result)
        }
        Command::SignOut => {
            let result = state::dir(cli.state_dir)
                .and_then(|dir| sign_out::run(&dir, cli.key_storage, speaks, timeout));
            exit(result)
        }
        Command::SetUsername { username } => {
            let result = state::dir(cli.state_dir).and_then(|dir| {
                set_username::run(&dir, cli.key_storage, speaks, timeout, &username)
            });
            exit(result)
        }
        Command::CreateSpace { name } => {
            let result = state::dir(cli.state_dir)
                .and_then(|dir| spaces::create(&dir, cli.key_storage, speaks, timeout, &name));
            exit(result)
        }
        Command::Spaces => {
            let result = state::dir(cli.state_dir)
                .and_then(|dir| spaces::list(&dir, cli.key_storage, speaks, timeout));
            exit(result)
        }
        Command::Members { space } => exit(
            state::dir(cli.state_dir)
                .and_then(|dir| spaces::synced(&dir, cli.key_storage, speaks, timeout, &space))
                .and_then(spaces::members),
        ),
        Command::CreateChannel { space } => exit(
            state::dir(cli.state_dir)
                .and_then(|dir| spaces::synced(&dir, cli.key_storage, speaks, timeout, &space))
                .and_then(channels::create),
        ),
        Command::Channels { space } => exit(
            state::dir(cli.state_dir)
                .and_then(|dir| spaces::synced(&dir, cli.key_storage, speaks, timeout, &space))
                .and_then(channels::list),
        ),
        Command::AddRule { folder, space } => {
            exit(state::dir(cli.state_dir).and_then(|dir| folders::add_rule(&dir, &folder, &space)))
        }
        Command::Rules => exit(state::dir(cli.state_dir).and_then(|dir| folders::rules(&dir))),
        Command::RemoveRule { folder } => {
            exit(state::dir(cli.state_dir).and_then(|dir| folders::remove_rule(&dir, &folder)))
        }
        Command::ApproveBinding { folder } => {
            exit(state::dir(cli.state_dir).and_then(|dir| folders::approve(&dir, &folder)))
        }
        Command::Folder { folder } => {
            exit(state::dir(cli.state_dir).and_then(|dir| folders::show(&dir, folder.as_deref())))
        }
        Command::Update { github_api_url } => exit(update::run(&github_api_url, timeout)),
    }
}

/// Exits as `result` says, printing its error.
fn exit(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("skald: {err}");
            ExitCode::FAILURE
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
