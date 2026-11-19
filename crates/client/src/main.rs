//! The `skald` client binary.

// The crate's printing macros write to the run's output, which a helper sends to the command it runs
// the operation for ([`output`]).
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::output::print($crate::output::Stream::Out, format_args!($($arg)*))
    };
}
macro_rules! println {
    () => {
        $crate::output::print($crate::output::Stream::Out, format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::output::print(
            $crate::output::Stream::Out,
            format_args!("{}\n", format_args!($($arg)*)),
        )
    };
}
macro_rules! eprintln {
    ($($arg:tt)*) => {
        $crate::output::print(
            $crate::output::Stream::Err,
            format_args!("{}\n", format_args!($($arg)*)),
        )
    };
}

mod admin;
mod agents;
mod anchors;
mod channels;
mod clock;
mod connection;
mod device;
mod devices;
mod events;
mod folders;
mod follow;
mod forms;
mod gaps;
mod groups;
mod helper;
mod history;
mod history_reading;
mod intake;
mod key_chain;
mod key_packages;
mod lock;
mod mailbox;
mod mcp;
mod output;
mod plain_path;
mod proxy;
mod recovery;
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
mod wait;
mod wraps;

use std::io::{self, IsTerminal as _};
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::builder::FalseyValueParser;
use clap::{Parser, Subcommand};
use connection::{Connection, TIMEOUT};
use device::First;
use serde::{Deserialize, Serialize};
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

    /// One-shot mode, as for CI and sandboxes: the command starts no helper, but goes through one
    /// that already runs.
    #[arg(
        long,
        env = "SKALD_ONE_SHOT",
        global = true,
        value_parser = FalseyValueParser::new()
    )]
    one_shot: bool,

    /// How long a connection on SSE or long-poll waits before it tries WebSocket again, in seconds,
    /// for tests; a helper takes it from the command that starts it [default: 60].
    #[arg(
        long,
        env = "SKALD_WEBSOCKET_RETRY",
        value_name = "SECONDS",
        global = true,
        hide = true
    )]
    websocket_retry: Option<NonZeroU64>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Serialize, Deserialize)]
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
    ///
    /// It shows the devices the revoked device certified, which are revoked with it unless kept:
    /// without --keep, it asks which to keep when run in a terminal, and keeps none otherwise.
    RevokeDevice {
        /// The device's ID, as skald devices shows it.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// The ID of a device the revoked one certified that stays valid, for scripts; once per
        /// device.
        #[arg(long = "keep", value_name = "DEVICE")]
        keep: Vec<String>,
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
    /// Sends a text, for one agent with --to or for every agent with --all, to a channel of a space
    /// this device follows, by default the default channel of the working directory's space; as
    /// an agent, only in the space of the agent's folder. It first accepts the space's latest
    /// roster revisions and goes through the mailbox as skald recv does, showing nothing, then
    /// encrypts the message in the channel's MLS group and has the server store it as one envelope
    /// to the group. Sends nothing, and says why, while the group holds a device the channel does
    /// not admit or no other device, or while this device's Commit there waits for the server.
    /// Says the message's ID and whether the server stored it; as an agent, only sent #<ref> once
    /// it is stored, by the message's reference as skald recv says. What the server has not stored
    /// within the request timeout waits in the local state, and the next skald send or skald recv
    /// sends it first.
    #[command(group(
        clap::ArgGroup::new("addressee")
            .args(["to", "all"])
            .required(true)
    ))]
    Send {
        /// The agent the message is for: its address, space/owner/agent@server, or owner/agent in
        /// the channel's space on this server; not a person's, as only a reply goes to a person.
        /// Every member device of the channel still gets the message, and the channel's history
        /// keeps it: an addressee does not make a message private.
        #[arg(long, value_name = "ADDRESS")]
        to: Option<String>,
        /// The message is for every agent in the channel.
        #[arg(long)]
        all: bool,
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// The text.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
        /// Says each message as a JSON object on a line of its own, with every field: msg_id, ref,
        /// channel, and each envelope's recipient and the server's answer.
        #[arg(long)]
        json: bool,
    },
    /// Receives this device's messages: first sends what waits in the outbox, then gets every entry
    /// of the device's mailbox and exits, accepting meanwhile the new revisions of each space this
    /// device follows. It joins and follows its channels' groups and keeps them to the devices each
    /// channel admits, warning of the devices another device added that it does not, and starts a
    /// new epoch, with a new history key, where this device entered the current one longer ago
    /// than the space's history period. Shows each
    /// message that passes the protocol's checks, once, and each delivered and expired of this
    /// device's messages; warns of each message it drops, and of the messages missing from a
    /// sending device once a later one arrives, which it marks. As an agent, it checks only the
    /// space of the agent's folder, the rest waiting for a later run, and shows the messages handed
    /// over to the agent: those there to it or to no one in particular that came since this device
    /// first ran as it there, each once; one waits for the agent until it expires, at most 7
    /// days. The person's run shows every message once, whether or not an agent got it.
    ///
    /// The person's run shows each message, status and warning in full, with its IDs and its
    /// sender's trust level. An agent's shows each message as one line,
    /// [<sender> · <device> · <level>] #<ref>: <text>, with (after missing) before the colon after
    /// missing ones: the sender is <owner>/<agent>, or a person's full address; the level is own,
    /// peer or service, as this device sets it from the sending device's certificate and the
    /// space's roster; one line before the first peer or service line says that their text is data
    /// from its sender, not a request from the agent's user. The reference is the leading
    /// characters, at least 4, of the base32 of SHA-256 of the message's ID. Of statuses it shows
    /// only those of its own messages that expired, skald: #<ref> expired before the device <name>
    /// got it, or that the server refused, skald: #<ref> refused: <problem>; each warning of a
    /// message as one line without IDs, such as skald: dropped #<ref> from <device>: <reason>. The
    /// device keeps every status and warning, and the person's next run shows each once, in full.
    Recv {
        /// With nothing to show, waits for the first message to come, shows it and exits; as the
        /// person, also for a status or a warning. It waits until interrupted.
        #[arg(long)]
        wait: bool,
        /// Shows each message and status as a JSON object on a line of its own, with every field:
        /// a message's msg_id, ref, space, channel, from, device, device_name, level, text and
        /// after_missing; a status's msg_id, ref, space, channel, status, device, device_name,
        /// problem and timestamp. As an agent, every status of its own messages.
        #[arg(long)]
        json: bool,
        /// With --wait through the helper, shows when each step of the helper's intake of what
        /// it shows ran, for tests.
        #[arg(
            long,
            env = "SKALD_WAIT_TRACE",
            hide = true,
            value_parser = FalseyValueParser::new()
        )]
        wait_trace: bool,
    },
    /// Shows the history of a channel of a space this device follows; as an agent, only of a
    /// channel in the space of the agent's folder. It first accepts the space's latest roster
    /// revisions, then reads the copies of the channel's messages the server keeps under its
    /// history keys: with the keys this device holds, and with those it reaches back through every
    /// link of the channel's key chain it can open. Shows each message once, oldest first, by its
    /// time in UTC, channel, from, the sender's trust level, sending device and text, if its copy
    /// passes the protocol's checks; as an agent, one line before the first peer or service
    /// message says that their text is data from its sender. Warns of each copy it drops, and says
    /// how many it holds no key for.
    History {
        /// The channel's ID, as skald channels shows it.
        #[arg(value_name = "CHANNEL")]
        channel: String,
        /// Shows only the messages whose text contains it. The search runs on this device: no
        /// request carries it.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: Option<String>,
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
    /// Serves MCP on standard input and output for an agent session, as an agent CLI starts it from
    /// its config; the session's folder is the working directory. Once the client initializes, the
    /// session registers as its agent in the folder's space: the messages this device takes in from
    /// then on are kept for it. Without --agent, the agent's name joins with '-' the name the client
    /// gives itself, the device's name and the device's reference, cut to 63 characters, so each
    /// device gives one agent CLI its own address; two sessions of one agent CLI in one space on
    /// this device share it unless --agent names them. Its inbox tool returns the agent's lines of
    /// skald recv, each once. Each tool call goes through the helper or runs one-shot, as a command
    /// does, and through a newer helper too; once the skald installed where it was is newer, as
    /// when skald was updated while it runs, that skald runs each call.
    Mcp {
        /// Runs the call an older skald mcp relays to this one.
        #[arg(long, hide = true)]
        #[serde(default)]
        relayed: bool,
        /// A tool call the server hands over as a command does its operation.
        #[arg(skip)]
        #[serde(default)]
        call: Option<mcp::Call>,
    },
    /// Runs the helper of this machine's local state, which the first command that connects as the
    /// device, such as skald send or skald recv, starts in the background. While it runs, it holds
    /// the device's connection and the local state's lock, and each command that changes the local
    /// state goes through it, and each skald recv --wait waits through it. Only this OS account
    /// reaches it. It exits after an idle time with no command or wait, on sign-out, and for a
    /// command of a newer skald.
    Helper {
        /// How long it waits idle before it exits, in seconds, for tests [default: 600].
        #[arg(long, env = "SKALD_HELPER_IDLE", value_name = "SECONDS", hide = true)]
        idle: Option<NonZeroU64>,
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

    /// Whether the command connects as the device, so that, out of one-shot mode, it starts the
    /// helper when none runs.
    fn starts_helper(&self) -> bool {
        matches!(
            self,
            Command::Send { .. }
                | Command::Recv { .. }
                | Command::Spaces
                | Command::Members { .. }
                | Command::Channels { .. }
                | Command::CreateSpace { .. }
                | Command::CreateChannel { .. }
                | Command::RemoveMember { .. }
                | Command::ExcludeDevice { .. }
                | Command::AddToChannel { .. }
                | Command::RemoveFromChannel { .. }
                | Command::SetHistoryPeriod { .. }
                | Command::Address
        )
    }

    /// Whether it is a `skald recv --wait` that shows when the helper's steps ran ([`wait`]).
    fn traces(&self) -> bool {
        matches!(
            self,
            Command::Recv {
                wait_trace: true,
                ..
            }
        )
    }

    /// Whether it is a `skald recv --wait` ([`wait`]).
    fn waits(&self) -> bool {
        matches!(self, Command::Recv { wait: true, .. })
    }

    /// Whether the run takes the local state's lock before the command, for the whole run, as one
    /// that may change the local state does: `None` if not; otherwise whether it creates a missing
    /// state directory. While a helper runs, such a run goes through it ([`helper::route`]).
    fn locks(&self) -> Option<bool> {
        match self {
            Command::Status { .. }
            | Command::Rules
            | Command::Folder { .. }
            | Command::Update { .. }
            | Command::Helper { .. } => None,
            // It takes the lock once its server's URL passes: a refused one leaves no file.
            Command::SignIn { .. } => None,
            Command::AddRule { .. } | Command::ApproveBinding { .. } => Some(true),
            _ => Some(false),
        }
    }
}

/// A member of a space in a channel of it, as a command names them.
#[derive(clap::Args, Serialize, Deserialize)]
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
#[derive(Clone, Copy, clap::ValueEnum, Serialize, Deserialize)]
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
    let one_shot = cli.one_shot;
    if let Some(seconds) = cli.websocket_retry {
        wait::set_retry(Duration::from_secs(seconds.get()));
    }
    let operation = Operation {
        speaks: cli.protocol_versions.unwrap_or_default(),
        timeout: cli
            .timeout
            .map_or(TIMEOUT, |seconds| Duration::from_secs(seconds.get())),
        clock_offset: cli.clock_offset.unwrap_or(0),
        state_dir: cli.state_dir,
        key_storage: cli.key_storage,
        agent: cli.agent,
        command: cli.command,
    };
    if let Err(err) = operation.agent() {
        return exit(Err(err));
    }
    if let Command::Mcp {
        relayed,
        call: None,
    } = operation.command
    {
        return exit(if relayed {
            mcp::relayed(one_shot)
        } else {
            mcp::serve(operation, one_shot)
        });
    }
    if let Command::Helper { idle } = operation.command {
        let idle = idle.map_or(helper::IDLE, |seconds| Duration::from_secs(seconds.get()));
        return exit(
            state::dir(operation.state_dir)
                .and_then(|dir| helper::serve(&dir, idle, operation.timeout)),
        );
    }
    let Some(create) = operation.command.locks() else {
        return exit(here(operation));
    };
    let dir = match state::dir(operation.state_dir.clone()) {
        Ok(dir) => dir,
        Err(err) => return exit(Err(err)),
    };
    let starts = !one_shot && operation.command.starts_helper();
    match helper::route(&dir, &operation, create, starts) {
        Ok(helper::Route::Here(lock)) if operation.command.waits() => {
            let waited = wait::here(&dir, &operation, lock);
            mailbox::close_kept();
            match waited {
                Ok(Some(code)) => code,
                Ok(None) => ExitCode::SUCCESS,
                Err(err) => exit(Err(err)),
            }
        }
        // Held until the run exits.
        Ok(helper::Route::Here(_lock)) => exit(here(operation)),
        Ok(helper::Route::Helped(code)) => code,
        Err(err) => exit(Err(err)),
    }
}

/// Runs `operation` in this process, then closes the connection it kept.
fn here(operation: Operation) -> Result<(), String> {
    let result = perform(operation);
    mailbox::close_kept();
    result
}

/// A run's operation, its settings resolved: what a command hands to the helper.
#[derive(Serialize, Deserialize)]
struct Operation {
    #[serde(with = "helper::versions")]
    speaks: VersionRange,
    timeout: Duration,
    clock_offset: u64,
    state_dir: Option<PathBuf>,
    key_storage: secrets::Storage,
    agent: Option<String>,
    command: Command,
}

impl Operation {
    /// Whether it shows as JSON: a `skald recv --json`.
    fn json(&self) -> bool {
        matches!(self.command, Command::Recv { json: true, .. })
    }

    /// The agent the run acts as, if any; fails for a name that is not an agent's, or a command of
    /// the person's.
    fn agent(&self) -> Result<Option<skald_proto::Name>, String> {
        let agent = self.agent.as_deref().map(agents::name).transpose()?;
        if agent.is_some()
            && let Some(command) = self.command.person_only()
        {
            return Err(agents::person_only(command));
        }
        Ok(agent)
    }
}

/// Runs `operation`; a run whose command changes the local state holds its lock already, or a
/// helper does.
fn perform(operation: Operation) -> Result<(), String> {
    let agent = operation.agent()?;
    clock::set_offset(operation.clock_offset);
    let Operation {
        speaks,
        timeout,
        state_dir,
        key_storage,
        command,
        ..
    } = operation;
    // An MCP server says it once, to its agent CLI's log, not in a tool's result.
    if !matches!(
        command,
        Command::SignIn { .. } | Command::Helper { .. } | Command::Mcp { .. }
    ) && let Ok(dir) = state::dir(state_dir.clone())
    {
        recovery::say_if_waiting(&dir);
    }
    match command {
        Command::Status { server } => status(&server, speaks, timeout),
        Command::SignIn {
            server,
            first_device,
            name,
        } => {
            let options = sign_in::Options {
                storage: key_storage,
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
                // A recovery key is shown only to the person, who reads it at a terminal.
                shows: agent.is_none() && io::stdin().is_terminal() && io::stdout().is_terminal(),
            };
            state::dir(state_dir).and_then(|dir| {
                let mut connection =
                    Connection::new(&server, speaks, timeout).map_err(|err| err.to_string())?;
                helper::stop(&dir, timeout)?;
                let _lock = lock::take(&dir, true, timeout)?;
                sign_in::run(&mut connection, &dir, &options).map_err(|err| err.to_string())?;
                // An agent's run takes nothing of another space than its folder's.
                if agent.is_none() {
                    spaces::take_personal(&dir, options.storage, speaks, timeout);
                }
                Ok(())
            })
        }
        Command::Devices => {
            state::dir(state_dir).and_then(|dir| devices::run(&dir, key_storage, speaks, timeout))
        }
        Command::RevokeDevice { device, keep } => state::dir(state_dir)
            .and_then(|dir| revocations::run(&dir, key_storage, speaks, timeout, &device, &keep)),
        Command::SignOut => {
            state::dir(state_dir).and_then(|dir| sign_out::run(&dir, key_storage, speaks, timeout))
        }
        Command::SetUsername { username } => state::dir(state_dir)
            .and_then(|dir| set_username::run(&dir, key_storage, speaks, timeout, &username)),
        Command::CreateSpace { name } => state::dir(state_dir)
            .and_then(|dir| spaces::create(&dir, key_storage, speaks, timeout, &name)),
        Command::Spaces => state::dir(state_dir).and_then(|dir| match agent {
            Some(_) => agents::spaces(&dir, key_storage, speaks, timeout),
            None => spaces::list(&dir, key_storage, speaks, timeout, Scope::All),
        }),
        Command::Members { space } => synced(
            state_dir,
            key_storage,
            speaks,
            timeout,
            &space,
            agent.is_some(),
        )
        .and_then(spaces::members),
        Command::CreateChannel { space } => synced(
            state_dir,
            key_storage,
            speaks,
            timeout,
            &space,
            agent.is_some(),
        )
        .and_then(channels::create),
        Command::RemoveMember { space, handle } => edit(
            (state_dir, key_storage, speaks, timeout),
            &space,
            admin::Edit::RemoveMember(handle),
        ),
        Command::ExcludeDevice { space, device } => edit(
            (state_dir, key_storage, speaks, timeout),
            &space,
            admin::Edit::Exclude(device),
        ),
        Command::AddToChannel { member, history } => {
            let (space, change) = member.edit(|channel, handle| admin::Edit::AddToChannel {
                channel,
                handle,
                history: matches!(history, Answer::Yes),
            });
            edit((state_dir, key_storage, speaks, timeout), &space, change)
        }
        Command::RemoveFromChannel { member } => {
            let (space, change) =
                member.edit(|channel, handle| admin::Edit::RemoveFromChannel { channel, handle });
            edit((state_dir, key_storage, speaks, timeout), &space, change)
        }
        Command::SetHistoryPeriod { space, period } => edit(
            (state_dir, key_storage, speaks, timeout),
            &space,
            admin::Edit::HistoryPeriod(period),
        ),
        Command::Channels { space } => synced(
            state_dir,
            key_storage,
            speaks,
            timeout,
            &space,
            agent.is_some(),
        )
        .and_then(channels::list),
        Command::Send {
            to,
            channel,
            text,
            json,
            ..
        } => state::dir(state_dir).and_then(|dir| {
            send::run(
                &dir,
                key_storage,
                (speaks, timeout),
                (to.as_deref(), channel.as_deref(), &text),
                agent.as_ref(),
                json,
            )
        }),
        Command::Recv { json, .. } => state::dir(state_dir).and_then(|dir| {
            recv::run(
                &dir,
                key_storage,
                (speaks, timeout),
                agent.as_ref(),
                (false, json),
            )
            .map(drop)
        }),
        Command::History { channel, text } => state::dir(state_dir).and_then(|dir| {
            history_reading::run(
                &dir,
                key_storage,
                (speaks, timeout),
                &channel,
                text.as_deref(),
                agent.is_some(),
            )
        }),
        Command::AddRule { folder, space } => {
            state::dir(state_dir).and_then(|dir| folders::add_rule(&dir, &folder, &space))
        }
        Command::Rules => state::dir(state_dir).and_then(|dir| folders::rules(&dir)),
        Command::RemoveRule { folder } => {
            state::dir(state_dir).and_then(|dir| folders::remove_rule(&dir, &folder))
        }
        Command::ApproveBinding { folder } => {
            state::dir(state_dir).and_then(|dir| folders::approve(&dir, &folder))
        }
        Command::Folder { folder } => state::dir(state_dir)
            .and_then(|dir| folders::show(&dir, folder.as_deref(), agent.is_some())),
        Command::Address => agent
            .ok_or_else(|| "name the agent: skald --agent <name> address".to_owned())
            .and_then(|agent| {
                state::dir(state_dir)
                    .and_then(|dir| agents::address(&dir, key_storage, &agent, speaks, timeout))
            }),
        Command::Update { github_api_url } => update::run(&github_api_url, timeout),
        Command::Mcp { call, .. } => state::dir(state_dir).and_then(|dir| {
            mcp::perform(call, &dir, key_storage, (speaks, timeout), agent.as_ref())
        }),
        Command::Helper { .. } => Err("the helper runs only as itself".to_owned()),
    }
}

/// One run of `skald recv` for `operation`, a `skald recv --wait`, which leaves its connection to
/// watch ([`wait`]): whether it showed what the wait waits for.
fn received(operation: &Operation) -> Result<bool, String> {
    let agent = operation.agent()?;
    clock::set_offset(operation.clock_offset);
    let dir = state::dir(operation.state_dir.clone())?;
    let settings = (operation.speaks, operation.timeout);
    recv::run(
        &dir,
        operation.key_storage,
        settings,
        agent.as_ref(),
        (true, operation.json()),
    )
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
) -> Result<(), String> {
    // Agents run none of the admin commands.
    synced(state_dir, storage, speaks, timeout, space, false)
        .and_then(|synced| admin::run(synced, &change))
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
fn status(server: &str, speaks: VersionRange, timeout: Duration) -> Result<(), String> {
    let mut connection = Connection::new(server, speaks, timeout).map_err(|err| err.to_string())?;
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
            Ok(())
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
            Err(err.to_string())
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
