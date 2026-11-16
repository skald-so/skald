//! The `skald` client binary.

mod admin;
mod agents;
mod channels;
mod clock;
mod connection;
mod device;
mod devices;
mod folders;
mod follow;
mod gaps;
mod groups;
mod key_packages;
mod lock;
mod plain_path;
mod recv;
mod revocations;
mod secrets;
mod send;
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
use spaces::Scope;

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

    /// How long a request to the server, and the wait for the local state's lock, may take, in
    /// seconds, for tests [default: 30].
    #[arg(
        long,
        env = "SKALD_TIMEOUT",
        value_name = "SECONDS",
        global = true,
        hide = true
    )]
    timeout: Option<NonZeroU64>,

    /// Seconds to add to the system's clock when counting a channel epoch's age, for tests
    /// [default: 0].
    #[arg(
        long,
        env = "SKALD_CLOCK_OFFSET",
        value_name = "SECONDS",
        global = true,
        hide = true
    )]
    clock_offset: Option<u64>,

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

    /// Acts as the agent of this name on this device, such as claude-api: a-z, 0-9 and '-' inside,
    /// 1 to 63 characters; user is reserved. The name never leaves this machine.
    #[arg(long, value_name = "NAME", global = true)]
    agent: Option<String>,

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
    /// device waits for linking. A device that holds its certificate keeps its KeyPackages, with
    /// which others add it to channels, on the server, here and on each skald send and skald recv.
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
    /// Each device is checked against the account's master key this machine pinned and the
    /// account's revocations: a device with a valid certificate shows by its certificate's name,
    /// kind and date issued, and a revoked one so too, marked revoked; any other, such as one only
    /// the server signed, is listed apart as not linked, by its ID only. Warns of each device
    /// revoked, or valid again, since this device last checked it.
    Devices,
    /// Revokes a device of the account this machine signed in to, this one included, on a device
    /// with a valid certificate: this device signs the revocation, and the server stores it. A
    /// revocation the server has not taken waits in the local state, and the next skald command
    /// that connects sends it again. The other member devices remove the revoked device from each
    /// channel, and it reads nothing sent after; a revoked device sends and receives nothing.
    RevokeDevice {
        /// The device's ID, as skald devices shows it.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
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
    /// first roster revision as its first admin, with the account's username as its handle, then
    /// the next with the space's default channel, and creates the channel's group. Run again after
    /// a lost answer, it finishes the same space.
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
    /// them: each one's handle and role, and its devices with a valid certificate, then its revoked
    /// ones. It first accepts the revisions the server has, and warns of each one that has no
    /// effect and of each device revoked, or valid again, since this device last checked it.
    Members {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Adds a channel to a space this device follows, on a device of an admin of it: the device
    /// signs the space's next roster revision with the channel, its account the channel's member.
    /// It first accepts the revisions the server has; when another revision comes first, it signs
    /// its change into the new one. Says the channel's ID, and creates the channel's group.
    CreateChannel {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Removes a member from a space this device follows and from every channel of it, on a device
    /// of an admin of it, by the space's next roster revision, which the device signs as skald
    /// create-channel does. Member devices then remove the member's devices from the channels.
    RemoveMember {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The member's handle, as skald members shows it.
        #[arg(value_name = "HANDLE", allow_hyphen_values = true)]
        handle: String,
    },
    /// Bars a member's device from a space this device follows, on a device of an admin of it, by
    /// the space's next roster revision, which the device signs as skald create-channel does.
    /// Member devices then remove the device from the channels.
    ExcludeDevice {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The device's ID.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
    /// Adds a member of a space this device follows to a channel of it, on a device of an admin of
    /// it, by the space's next roster revision, which the device signs as skald create-channel
    /// does. This device adds the member's devices to the channel's group if it is in it;
    /// otherwise they join by themselves.
    AddToChannel {
        #[command(flatten)]
        member: InChannel,
        /// Whether the member reads the channel's history from before it joined.
        #[arg(long, value_name = "ANSWER")]
        history: Answer,
    },
    /// Removes a member from a channel of a space this device follows, on a device of an admin of
    /// it, by the space's next roster revision, which the device signs as skald create-channel
    /// does; the default channel keeps every member. Member devices then remove the member's
    /// devices from the channel.
    RemoveFromChannel {
        #[command(flatten)]
        member: InChannel,
    },
    /// Sets how long each channel of a space this device follows keeps one history key, on a device
    /// of an admin of it, by the space's next roster revision, which the device signs as skald
    /// create-channel does. Member devices then rotate the key of each channel whose current epoch
    /// is older.
    SetHistoryPeriod {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// Whole days, such as 7d, or whole hours, such as 36h; at least a day.
        #[arg(value_name = "PERIOD", value_parser = history_period)]
        period: u64,
    },
    /// Lists the channels of a space this device follows, as its last accepted roster revision gives
    /// them: each one's ID and its members' handles, and the devices in its group as this device
    /// holds it, or that this device is not in it. It first accepts the revisions the server has.
    Channels {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Sends a text to a channel of a space this device follows; as an agent, only in the space of
    /// the agent's folder. It first accepts the space's latest roster revisions and goes through the
    /// mailbox as skald recv does, showing nothing, then encrypts the message in the channel's MLS
    /// group and has the server store it as one envelope to the group. Sends nothing, and says why,
    /// while the group holds a device the channel does not admit or no other device, or while this
    /// device's Commit there waits for the server.
    /// Says the message's ID and whether the server stored it. What the server has not stored
    /// within the request timeout waits in the local state, and the next skald send or skald recv
    /// sends it first.
    Send {
        /// The channel's ID, as skald channels shows it.
        #[arg(value_name = "CHANNEL")]
        channel: String,
        /// The text.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
    },
    /// Receives this device's messages: first sends what waits in the outbox, then gets every entry
    /// of the device's mailbox and exits, accepting meanwhile the new revisions of each space this
    /// device follows. It joins and follows its channels' groups and keeps them to the devices each
    /// channel admits, warning of the devices another device added that it does not, and starts a
    /// new epoch, with a new history key, where this device entered the current one longer ago
    /// than the space's history period. Shows each
    /// message that passes the protocol's checks, once, and each delivered and expired of this
    /// device's messages; warns of each message it drops, and of the messages missing from a
    /// sending device once a later one arrives, which it marks. As an agent, only those of the
    /// space of the agent's folder: the rest wait for a later run.
    Recv,
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
    /// Shows the address of the agent --agent names in the space of the working directory,
    /// space/owner/agent@server: the space's name and the account's handle in its last accepted
    /// roster revision, and the server's name. A folder in no space, or a server without a valid
    /// name, gives no address.
    Address,
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

impl Command {
    /// The CLI's name of this command if it is one of the person's, which an agent cannot run.
    fn person_only(&self) -> Option<&'static str> {
        match self {
            Command::CreateSpace { .. } => Some("create-space"),
            Command::CreateChannel { .. } => Some("create-channel"),
            Command::RemoveMember { .. } => Some("remove-member"),
            Command::ExcludeDevice { .. } => Some("exclude-device"),
            Command::AddToChannel { .. } => Some("add-to-channel"),
            Command::RemoveFromChannel { .. } => Some("remove-from-channel"),
            Command::SetHistoryPeriod { .. } => Some("set-history-period"),
            Command::AddRule { .. } => Some("add-rule"),
            Command::Rules => Some("rules"),
            Command::RemoveRule { .. } => Some("remove-rule"),
            Command::ApproveBinding { .. } => Some("approve-binding"),
            _ => None,
        }
    }

    /// Whether the run takes the local state's lock before the command, for the whole run, as one
    /// that may change the local state does: `None` if not; otherwise whether it creates a missing
    /// state directory.
    fn locks(&self) -> Option<bool> {
        match self {
            Command::Status { .. }
            | Command::Rules
            | Command::Folder { .. }
            | Command::Update { .. } => None,
            // It takes the lock once its server's URL passes: a refused one leaves no file.
            Command::SignIn { .. } => None,
            Command::AddRule { .. } | Command::ApproveBinding { .. } => Some(true),
            _ => Some(false),
        }
    }
}

/// A member of a space in a channel of it, as a command names them.
#[derive(clap::Args)]
struct InChannel {
    /// The space's name, such as acme.
    #[arg(value_name = "SPACE", allow_hyphen_values = true)]
    space: String,
    /// The channel's ID, as skald channels shows it.
    #[arg(value_name = "CHANNEL")]
    channel: String,
    /// The member's handle, as skald members shows it.
    #[arg(value_name = "HANDLE", allow_hyphen_values = true)]
    handle: String,
}

impl InChannel {
    /// The change to the space its member makes `change` of it.
    fn edit(self, change: impl FnOnce(String, String) -> admin::Edit) -> (String, admin::Edit) {
        (self.space, change(self.channel, self.handle))
    }
}

/// An answer to a question.
#[derive(Clone, Copy, clap::ValueEnum)]
enum Answer {
    Yes,
    No,
}

/// A policy's `history` in seconds, given as whole days, `<n>d`, or whole hours, `<n>h`; refused
/// under a day (`docs/protocol/membership.md`, "Policy").
fn history_period(given: &str) -> Result<u64, String> {
    let unit = match given.chars().last() {
        Some('d') => 86_400,
        Some('h') => 3_600,
        _ => return Err("give whole days, such as 7d, or whole hours, such as 36h".to_owned()),
    };
    let seconds = given[..given.len() - 1]
        .parse::<u64>()
        .ok()
        .and_then(|count| count.checked_mul(unit))
        .ok_or_else(|| "give whole days, such as 7d, or whole hours, such as 36h".to_owned())?;
    if seconds < skald_proto::HISTORY_MIN {
        return Err("the period is under a day, the shortest a space's policy allows".to_owned());
    }
    Ok(seconds)
}

/// The setting of the host name the client reads, for tests; empty, the host name is empty.
const HOST_NAME: &str = "SKALD_HOST_NAME";

fn main() -> ExitCode {
    let cli = Cli::parse();
    let speaks = cli.protocol_versions.unwrap_or_default();
    clock::set_offset(cli.clock_offset.unwrap_or(0));
    let timeout = cli
        .timeout
        .map_or(TIMEOUT, |seconds| Duration::from_secs(seconds.get()));
    let agent = match cli.agent.as_deref().map(agents::name).transpose() {
        Ok(agent) => agent,
        Err(err) => return exit(Err(err)),
    };
    if agent.is_some()
        && let Some(command) = cli.command.person_only()
    {
        return exit(Err(agents::person_only(command)));
    }
    // Held until the run exits.
    let _lock = match cli.command.locks().map(|create| {
        state::dir(cli.state_dir.clone()).and_then(|dir| lock::take(&dir, create, timeout))
    }) {
        Some(Err(err)) => return exit(Err(err)),
        taken => taken,
    };
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
                let _lock = lock::take(&dir, true, timeout)?;
                sign_in::run(&mut connection, &dir, &options).map_err(|err| err.to_string())?;
                // An agent's run takes nothing of another space than its folder's.
                if agent.is_none() {
                    spaces::take_personal(&dir, options.storage, speaks, timeout);
                }
                Ok(())
            });
            exit(result)
        }
        Command::Devices => {
            let result = state::dir(cli.state_dir)
                .and_then(|dir| devices::run(&dir, cli.key_storage, speaks, timeout));
            exit(result)
        }
        Command::RevokeDevice { device } => exit(
            state::dir(cli.state_dir)
                .and_then(|dir| revocations::run(&dir, cli.key_storage, speaks, timeout, &device)),
        ),
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
            let result = state::dir(cli.state_dir).and_then(|dir| match agent {
                Some(_) => agents::spaces(&dir, cli.key_storage, speaks, timeout),
                None => spaces::list(&dir, cli.key_storage, speaks, timeout, Scope::All),
            });
            exit(result)
        }
        Command::Members { space } => exit(
            synced(
                cli.state_dir,
                cli.key_storage,
                speaks,
                timeout,
                &space,
                agent.is_some(),
            )
            .and_then(spaces::members),
        ),
        Command::CreateChannel { space } => exit(
            synced(
                cli.state_dir,
                cli.key_storage,
                speaks,
                timeout,
                &space,
                agent.is_some(),
            )
            .and_then(channels::create),
        ),
        Command::RemoveMember { space, handle } => edit(
            (cli.state_dir, cli.key_storage, speaks, timeout),
            &space,
            admin::Edit::RemoveMember(handle),
        ),
        Command::ExcludeDevice { space, device } => edit(
            (cli.state_dir, cli.key_storage, speaks, timeout),
            &space,
            admin::Edit::Exclude(device),
        ),
        Command::AddToChannel { member, history } => {
            let (space, change) = member.edit(|channel, handle| admin::Edit::AddToChannel {
                channel,
                handle,
                history: matches!(history, Answer::Yes),
            });
            edit(
                (cli.state_dir, cli.key_storage, speaks, timeout),
                &space,
                change,
            )
        }
        Command::RemoveFromChannel { member } => {
            let (space, change) =
                member.edit(|channel, handle| admin::Edit::RemoveFromChannel { channel, handle });
            edit(
                (cli.state_dir, cli.key_storage, speaks, timeout),
                &space,
                change,
            )
        }
        Command::SetHistoryPeriod { space, period } => edit(
            (cli.state_dir, cli.key_storage, speaks, timeout),
            &space,
            admin::Edit::HistoryPeriod(period),
        ),
        Command::Channels { space } => exit(
            synced(
                cli.state_dir,
                cli.key_storage,
                speaks,
                timeout,
                &space,
                agent.is_some(),
            )
            .and_then(channels::list),
        ),
        Command::Send { channel, text } => exit(state::dir(cli.state_dir).and_then(|dir| {
            send::run(
                &dir,
                cli.key_storage,
                speaks,
                timeout,
                &channel,
                &text,
                agent.as_ref(),
            )
        })),
        Command::Recv => exit(
            state::dir(cli.state_dir)
                .and_then(|dir| recv::run(&dir, cli.key_storage, speaks, timeout, agent.is_some())),
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
        Command::Folder { folder } => exit(
            state::dir(cli.state_dir)
                .and_then(|dir| folders::show(&dir, folder.as_deref(), agent.is_some())),
        ),
        Command::Address => exit(
            agent
                .ok_or_else(|| "name the agent: skald --agent <name> address".to_owned())
                .and_then(|agent| {
                    state::dir(cli.state_dir).and_then(|dir| {
                        agents::address(&dir, cli.key_storage, &agent, speaks, timeout)
                    })
                }),
        ),
        Command::Update { github_api_url } => exit(update::run(&github_api_url, timeout)),
    }
}

/// The space named `space`, synced, by the state in `state_dir`, its secrets in `storage`; for an
/// `agent`, only the space of the working directory ([`agents::scope`]).
fn synced(
    state_dir: Option<PathBuf>,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    space: &str,
    agent: bool,
) -> Result<spaces::Synced, String> {
    let dir = state::dir(state_dir)?;
    let scope = agents::scope(&dir, space, agent)?;
    spaces::synced(&dir, storage, speaks, timeout, space, scope)
}

/// Makes `change` in the space named `space` ([`admin::run`]), by the state in `state_dir`, its
/// secrets in `storage`.
fn edit(
    (state_dir, storage, speaks, timeout): (
        Option<PathBuf>,
        secrets::Storage,
        VersionRange,
        Duration,
    ),
    space: &str,
    change: admin::Edit,
) -> ExitCode {
    // Agents run none of the admin commands.
    exit(
        synced(state_dir, storage, speaks, timeout, space, false)
            .and_then(|synced| admin::run(synced, &change)),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A period is whole days or hours from a day on; under a day, or of another form, it is
    /// refused.
    #[test]
    fn history_period_is_whole_days_or_hours_from_a_day_on() {
        for (given, seconds) in [
            ("1d", 86_400),
            ("24h", 86_400),
            ("36h", 129_600),
            ("7d", 604_800),
        ] {
            assert_eq!(history_period(given), Ok(seconds), "{given}");
        }
        for given in [
            "23h",
            "0d",
            "",
            "d",
            "7",
            "7w",
            "1.5d",
            "-1d",
            "99999999999999999d",
        ] {
            assert!(history_period(given).is_err(), "{given}");
        }
        assert!(history_period("23h").unwrap_err().contains("day"));
    }
}
