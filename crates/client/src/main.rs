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
mod claims;
#[cfg(unix)]
mod claude_relay;
mod claude_socket;
mod clock;
mod codex;
#[cfg(unix)]
mod codex_app_server;
mod connection;
mod copies;
mod delivery;
mod device;
mod devices;
mod domains;
mod events;
mod fingerprints;
mod folders;
mod follow;
mod forms;
mod gaps;
mod groups;
mod heartbeat;
mod held;
mod helper;
mod history;
mod history_reading;
mod hook;
mod inbound;
mod intake;
mod invites;
mod joined;
mod kept_certs;
mod key_chain;
mod key_packages;
mod link_code;
mod link_prompt;
mod lock;
mod mailbox;
mod mcp;
mod message_status;
mod output;
mod page;
mod permission_prompts;
mod person;
mod plain_path;
mod presence;
mod proxy;
mod queue_keys;
mod queues;
mod receipts;
mod records;
mod recovery;
mod recovery_link;
mod recovery_wraps;
mod recv;
mod reply;
mod resend;
mod revocations;
mod secrets;
mod send;
mod service;
mod service_agents;
mod set_username;
mod sign_in;
mod sign_out;
mod spaces;
mod ssh_checks;
mod ssh_signing;
mod state;
mod stateless;
mod tasks;
#[cfg(test)]
mod test_dir;
#[cfg(test)]
mod test_dir_tests;
mod tls;
mod update;
mod uploads;
mod wait;
mod wraps;

use std::io::IsTerminal as _;
use std::io::Write as _;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::builder::FalseyValueParser;
use clap::{Parser, Subcommand};
use connection::{Connection, TIMEOUT};
use person::Person;
use serde::{Deserialize, Serialize};
use skald_proto::{DeviceId, Name, VersionRange};
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

    /// The base URL of GitHub's REST API, for tests: skald update takes releases from it, and
    /// commands check devices' SSH signatures against it.
    #[arg(
        long,
        env = "SKALD_GITHUB_API_URL",
        value_name = "URL",
        global = true,
        hide = true,
        default_value = update::GITHUB_API_URL
    )]
    github_api_url: String,

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
    /// Signs this machine in to the server, or links its device with no sign-in. Run again, checks
    /// the session with the server and says which account it is signed in to.
    ///
    /// Run by its person, a machine without a session and a linked device shows two ways at once.
    /// One is the server's sign-in page: it opens it with its code filled in, in the browser BROWSER
    /// names or the system's default, and prints its link for a browser on any device; its person
    /// approves the code there. The other is a link code to type into skald link on a linked device
    /// of the account: the code alone links the device, with no sign-in, and the device then gets
    /// a session by its own key. Whichever finishes first wins. A code works for 5 minutes and up
    /// to 3 runs; a failed run of a 12-character code changes its first 6 characters; Enter asks
    /// for a fresh one. Without its person, it shows only the sign-in page.
    ///
    /// At its first sign-in, the machine generates its device's keys, which never leave it, and
    /// registers the device. On an account the server reports no master key for, the device is the
    /// account's first: it creates the account's master key, issues its own certificate with it and
    /// signs the account's personal space as its admin; on any other, the device waits for
    /// linking: run by its person, it shows a link code and exits once a linked device linked it.
    /// A device that holds its certificate keeps its KeyPackages, with which others add it to
    /// channels, on the server, here and on each skald send and skald recv.
    ///
    /// A device that issues its own certificate, as the account's first or with the recovery key,
    /// first signs its key with an SSH key GitHub lists for the GitHub user, through OpenSSH's
    /// ssh-keygen 8.1 or later, so that other devices check it there and show the GitHub login.
    /// Unless run by its person at a terminal, only a key ssh-agent holds signs, so nothing asks for
    /// a passphrase. Without a key, ssh-keygen or GitHub, it says why and goes on without one.
    ///
    /// With --browser, as its person or as an agent, it shows its person a local page it opens in
    /// the browser instead: the sign-in link, the link codes or the recovery key. Its output holds
    /// neither code nor key.
    ///
    /// With --with-recovery-key, a device that waits for linking links with the account's recovery
    /// key instead, only as its person at a terminal, who types the key there without echo. It
    /// issues its own certificate with the account's master key and shows the account's devices.
    /// Then it revokes with the master key the devices named stolen, which voids the revocations
    /// they signed, starts from the spaces of the devices not named stolen, and revokes with its
    /// own key the devices named lost or wiped. It keeps neither key. Without --stolen, --lost,
    /// --keep or --keep-revoked it asks which devices are which. A run that stops before it is done
    /// leaves the rest to a later one with --with-recovery-key.
    SignIn {
        /// The server's URL, such as https://api.skald.so; plain http:// only to this machine, such as
        /// http://localhost:8080.
        #[arg(long, value_name = "URL")]
        server: String,
        /// The device's name, 1 to 64 bytes, set at its first sign-in [default: the host name].
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Links this device with the account's recovery key, typed at the terminal; on an account
        /// without a master key, creates none.
        #[arg(long)]
        with_recovery_key: bool,
        /// Asks its person on a local page it opens in the browser: the program BROWSER names, or
        /// the system's default browser.
        #[arg(long, conflicts_with = "with_recovery_key")]
        browser: bool,
        /// The ID of a device that is stolen, revoked with the master key, for scripts; once per
        /// device.
        #[arg(long = "stolen", value_name = "DEVICE", requires = "with_recovery_key")]
        stolen: Vec<String>,
        /// The ID of a device that is lost or wiped, revoked with this device's key, for scripts;
        /// once per device.
        #[arg(long = "lost", value_name = "DEVICE", requires = "with_recovery_key")]
        lost: Vec<String>,
        /// The ID of a device a stolen or lost one certified that stays valid, for scripts; once
        /// per device.
        #[arg(long = "keep", value_name = "DEVICE", requires = "with_recovery_key")]
        keep: Vec<String>,
        /// The ID of a device a stolen one revoked that stays revoked, for scripts; once per
        /// device.
        #[arg(
            long = "keep-revoked",
            value_name = "DEVICE",
            requires = "with_recovery_key"
        )]
        keep_revoked: Vec<String>,
        /// The private key file of the SSH key that signs the device's key, one GitHub lists for
        /// the GitHub user [default: a listed key ssh-agent holds or that is in ~/.ssh].
        #[arg(long, value_name = "FILE", conflicts_with = "no_ssh")]
        ssh_key: Option<PathBuf>,
        /// Signs the device's key with no SSH key.
        #[arg(long)]
        no_ssh: bool,
        /// The GitHub user whose SSH keys may sign the device's key [default: the login of the
        /// account's GitHub sign-in method].
        #[arg(long, value_name = "LOGIN", conflicts_with = "no_ssh")]
        github: Option<String>,
    },
    /// Lists the devices of the account this machine signed in to, from the server of its session.
    ///
    /// Each device is checked against the account's master key this machine pinned and the
    /// account's revocations: a device with a valid certificate shows by its certificate's name,
    /// kind and date issued, and a revoked one so too, marked revoked; any other, such as one only
    /// the server signed, is listed apart as not linked, by its ID only. Warns of each device
    /// revoked, valid again or new since this device last checked it.
    Devices,
    /// Links a new device of this account, which shows a link code, on a device with a valid
    /// certificate, as its person at a terminal: enter only a code shown on your own device. Once
    /// you confirm the new device's name and kind, this device issues its certificate. After 3
    /// failed runs in a row, it first asks whether the new device shows a new code.
    Link {
        /// Takes the code and the confirmation on a local page it opens in the browser, also as an
        /// agent: the program BROWSER names, or the system's default browser.
        #[arg(long)]
        browser: bool,
    },
    /// A step of skald link after its person's answers, which skald link hands to the helper.
    #[command(skip)]
    LinkStep { step: link_code::Step },
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
        /// The URL of the DNS-over-HTTPS resolver that looks up the records of the space's domain.
        #[arg(
            long,
            env = "SKALD_DNS_RESOLVER",
            value_name = "URL",
            hide = true,
            default_value = domains::RESOLVER
        )]
        dns_resolver: String,
    },
    /// Shows your account's fingerprint, or, given a space and a member's handle, that member's.
    /// Compare it with the one the other person's device shows, in person or on a call. Run by its
    /// person at a terminal for a member, it then asks whether they match, and a yes marks the
    /// account verified on this device, as skald members shows; a later change of its master key
    /// warns and ends that. As an agent or through a pipe, it marks nothing.
    Fingerprint {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true, requires = "handle")]
        space: Option<String>,
        /// The member's handle, as skald members shows it.
        #[arg(value_name = "HANDLE", allow_hyphen_values = true)]
        handle: Option<String>,
    },
    /// A step of skald fingerprint around its person's answer, which skald fingerprint hands to
    /// the helper.
    #[command(skip)]
    FingerprintStep { step: fingerprints::Step },
    /// Adds a channel to a space this device follows, on a device of an admin of it: the device
    /// signs the space's next roster revision with the channel, its account the channel's member.
    /// It first accepts the revisions the server has; when another revision comes first, it signs
    /// its change into the new one. Says the channel's ID, and creates the channel's group.
    CreateChannel {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Creates an invite to a space this device follows, on a device of an admin of it, as its
    /// person at a terminal: the device signs it, the server stores it, and the run shows its
    /// link, which admits one person. Send the link only to them, over a channel you trust. The
    /// invite names the space's default channel first, whose earlier history the newcomer reads;
    /// its secret is only in the link, which nothing keeps.
    Invite {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// Invites an admin, not a member.
        #[arg(long)]
        admin: bool,
        /// A channel's ID, as skald channels shows it, whose earlier history the newcomer reads;
        /// once per channel.
        #[arg(long = "channel", value_name = "CHANNEL")]
        channels: Vec<String>,
        /// A channel's ID whose earlier history the newcomer does not read; once per channel.
        #[arg(long = "channel-without-history", value_name = "CHANNEL")]
        without_history: Vec<String>,
        /// The days until the invite expires, 30 at most.
        #[arg(long, value_name = "DAYS", default_value_t = invites::DEFAULT_DAYS, value_parser = invite_days)]
        days: u64,
    },
    /// Creates a service agent of a space this device follows, on a device of an admin of it: a
    /// device of the admin's account that runs without a person, such as in CI, at the address
    /// OWNER/AGENT. Its keys are made here; it is registered and certified, and the space's next
    /// roster revision, which the device signs as skald create-channel does, lists it. Writes its
    /// key secret and first token to standard output as SKALD_SERVICE_KEY=... and
    /// SKALD_SERVICE_TOKEN=..., once: nothing keeps them.
    CreateServiceAgent {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The service agent's address, owner/agent, such as ci/deploy-bot. The owner is no
        /// member's handle.
        #[arg(value_name = "OWNER/AGENT")]
        address: String,
        #[command(flatten)]
        granted: service_agents::Granted,
        /// Makes the agent a member of its channels: it runs with a state directory and is in
        /// their MLS groups, so its token needs the receive permission.
        #[arg(long)]
        member: bool,
    },
    /// Creates a token of a service agent of a space this device follows, on a device of an admin
    /// of it, with a new grant; revokes the agent's earlier tokens. Writes the token to standard
    /// output as SKALD_SERVICE_TOKEN=..., once: nothing keeps it.
    CreateServiceToken {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The service agent's address, owner/agent, such as ci/deploy-bot.
        #[arg(value_name = "OWNER/AGENT")]
        address: String,
        #[command(flatten)]
        granted: service_agents::Granted,
        /// Keeps the agent's earlier tokens working.
        #[arg(long)]
        keep_earlier: bool,
    },
    /// Replaces the key of a service agent of a space this device follows, on a device of an admin
    /// of it whose account the agent's device is on: a new device, made as skald
    /// create-service-agent makes one, takes the old one's place in the space's next roster
    /// revision, which ends the old device's tokens, and this device revokes the old one. Writes
    /// the new key secret and a token to standard output as SKALD_SERVICE_KEY=... and
    /// SKALD_SERVICE_TOKEN=..., once: nothing keeps them.
    ReplaceServiceKey {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The service agent's address, owner/agent, such as ci/deploy-bot.
        #[arg(value_name = "OWNER/AGENT")]
        address: String,
        #[command(flatten)]
        granted: service_agents::Granted,
    },
    /// Removes a service agent from a space this device follows, on a device of an admin of it: the
    /// space's next roster revision, which the device signs as skald create-channel does, lists it
    /// no more, which ends its tokens. Revokes its device if it is of this device's account;
    /// otherwise that account revokes it with skald revoke-device.
    RemoveServiceAgent {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The service agent's address, owner/agent, such as ci/deploy-bot.
        #[arg(value_name = "OWNER/AGENT")]
        address: String,
    },
    /// Lists the live tokens of the service agents of a space this device follows, on a device of
    /// an admin of it: each one's ID, address, permissions, channel IDs, creation and expiry times
    /// and last use.
    ServiceTokens {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Revokes a token of a service agent of a space this device follows, on a device of an admin
    /// of it.
    RevokeServiceToken {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The token's ID, as skald service-tokens shows it.
        #[arg(value_name = "TOKEN")]
        id: String,
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
    /// Names an organization's domain in the policy of a space this device follows, on a device of
    /// an admin of it, by the space's next roster revision, which the device signs as skald
    /// create-channel does. Member devices then check the members against the fingerprints the
    /// domain's DNS lists in TXT records at _skald.<domain>, each "<account ID> <fingerprint>", as
    /// skald members shows.
    SetDomain {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The domain in lowercase, such as example.com.
        #[arg(value_name = "DOMAIN", value_parser = domains::name)]
        domain: String,
    },
    /// Clears the domain of a space this device follows, on a device of an admin of it, by the
    /// space's next roster revision, which the device signs as skald create-channel does.
    ClearDomain {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
    },
    /// Makes a member of a space this device follows an admin, on a device of an admin of it, by the
    /// space's next roster revision, which the device signs as skald create-channel does.
    MakeAdmin {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The member's handle, as skald members shows it.
        #[arg(value_name = "HANDLE", allow_hyphen_values = true)]
        handle: String,
    },
    /// Hands this account's admin rights in a space this device follows to a member of it, on a
    /// device of an admin of it: the space's next roster revision, which the device signs as skald
    /// create-channel does, makes the member an admin, if it is not one, and this account a member.
    HandOverAdmin {
        /// The space's name, such as acme.
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: String,
        /// The member's handle, as skald members shows it.
        #[arg(value_name = "HANDLE", allow_hyphen_values = true)]
        handle: String,
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
    /// to the group; once stored, this device's other agents it is for get it, and its person,
    /// unless the person sent it. Sends nothing, and says why, while the group holds a device the
    /// channel does not admit, or no other device and nothing on this device gets the message, or
    /// while this device's Commit there waits for the server.
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
    /// Lists the queues of a channel of a space this device follows, by default the default
    /// channel of the working directory's space, each with its mode, lease in seconds and
    /// max_attempts; as an agent, only in the space of the agent's folder, with no ID.
    Queues {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// Says each queue as a JSON object on a line of its own, with every field: id, name,
        /// channel, mode, lease and max_attempts.
        #[arg(long)]
        json: bool,
    },
    /// Creates a queue in a channel, by default the default channel of the working directory's
    /// space; its creator and the space's admins change and delete it.
    CreateQueue {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// Whether a task whose claim lapses is offered again [default: at-least-once].
        #[arg(long, value_enum)]
        mode: Option<queues::Mode>,
        /// How long a claim lasts without a renewal, in seconds, 30 to 3600 [default: 300].
        #[arg(long, value_name = "SECONDS")]
        lease: Option<u64>,
        /// How many claims a task gets before it goes to the dead-letter queue, 1 to 100
        /// [default: 5].
        #[arg(long, value_name = "COUNT")]
        max_attempts: Option<u64>,
        /// The queue's name: a-z, 0-9 and '-' inside, 1 to 63 characters.
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        name: String,
    },
    /// Changes the lease or max_attempts of a queue of a channel, by default the default channel
    /// of the working directory's space; its mode stays.
    ChangeQueue {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// How long a claim lasts without a renewal, in seconds, 30 to 3600.
        #[arg(long, value_name = "SECONDS")]
        lease: Option<u64>,
        /// How many claims a task gets before it goes to the dead-letter queue, 1 to 100.
        #[arg(long, value_name = "COUNT")]
        max_attempts: Option<u64>,
        /// The queue's name.
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        name: String,
    },
    /// Deletes a queue of a channel, by default the default channel of the working directory's
    /// space, with its tasks; the default queue stays.
    DeleteQueue {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// The queue's name.
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        name: String,
    },
    /// Enqueues a task in a queue of a channel, by default the default queue of the default
    /// channel of the working directory's space; as an agent, only in the space of the agent's
    /// folder. It first accepts the space's latest roster revisions and goes through the mailbox
    /// as skald send does, then encrypts the task to the channel's current epoch, signed by this
    /// device, and has the server store it. Enqueues nothing, and says why, while this device
    /// judges the current epoch unclean, for a queue the channel lacks, past the limit on agents
    /// answering each other, or with --to an agent no device or several devices list in the
    /// channel's presence. Without --deadline, a task a device of another account may claim gets
    /// one 7 days after enqueue. Says the task's ID and #<ref>; as an agent, only enqueued
    /// #<ref>. A task the server has not answered waits in the local state, and the next skald
    /// enqueue or skald recv uploads it first, with the same bytes. A stateless service agent's
    /// run encrypts it to the queue key the channel's member devices publish, and fails unless the
    /// server stores it within the run.
    Enqueue {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// The queue's name.
        #[arg(long, value_name = "NAME", default_value = queues::DEFAULT)]
        queue: String,
        /// The agent the task is for, as skald send --to names it: only the device whose presence
        /// lists it claims the task.
        #[arg(long, value_name = "ADDRESS")]
        to: Option<String>,
        /// The task is enqueued once per key in the queue: the server refuses a later task with
        /// it, naming the first. 1 to 32 bytes of UTF-8.
        #[arg(long, value_name = "KEY")]
        idempotency_key: Option<String>,
        /// When the task expires unclaimed, as an RFC 3339 date-time such as
        /// 2027-01-31T00:00:00Z.
        #[arg(long, value_name = "TIME")]
        deadline: Option<String>,
        /// The text of the task's request.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
        /// Says the task as a JSON object, with every field: task, ref, channel, queue,
        /// queue_name, deadline and device, the addressee's.
        #[arg(long)]
        json: bool,
    },
    /// Lists the tasks of a channel, by default the default channel of the working directory's
    /// space, or of one of its queues: each with its #<ref>, ID, queue, state, its holder's
    /// progress while claimed or its reason while dead, and its attempts. As an agent, only the
    /// tasks it enqueued, in the space of its folder, with no ID.
    Tasks {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// Only the tasks of the queue of this name.
        #[arg(long, value_name = "NAME")]
        queue: Option<String>,
        /// Says each task as a JSON object on a line of its own, with every field: task, ref,
        /// channel, queue, queue_name, state, progress, reason, attempts, claim_device, the
        /// device of its last claim, deadline and device, the addressee's.
        #[arg(long)]
        json: bool,
    },
    /// Restarts a dead task of a channel, by default the default channel of the working
    /// directory's space: it is queued again with attempts 0. Only for a task of your account or
    /// as an admin of the space.
    RestartTask {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// The task: its #<ref>, as skald tasks shows it, or its ID.
        #[arg(value_name = "TASK")]
        task: String,
    },
    /// Cancels a queued, claimed or dead task of a channel, by default the default channel of the
    /// working directory's space; the device holding it loses its claim. Only for a task of your
    /// account or as an admin of the space.
    CancelTask {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// The task: its #<ref>, as skald tasks shows it, or its ID.
        #[arg(value_name = "TASK")]
        task: String,
    },
    /// Claims one task from a queue of a channel, by default the default queue of the default
    /// channel of the working directory's space; as an agent, only in the space of the agent's
    /// folder. It first accepts the space's latest roster revisions and goes through the mailbox
    /// as skald send does, then claims a task of the trust level own without a template. Runs a
    /// task only once its checks pass: one for another device or agent, or whose sender's trust
    /// level or template the claim did not state, it hands back; one that fails another check it
    /// fails without retry, with a warning. Extends the claim at once with started, then shows the
    /// task in one line with its #<ref>, as skald recv shows a message; with no task, says so in
    /// one line.
    Claim {
        /// The channel's ID, as skald channels shows it [default: the default channel of the
        /// working directory's space].
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// The queue's name.
        #[arg(long, value_name = "NAME", default_value = queues::DEFAULT)]
        queue: String,
        /// Shows the task as a JSON object, with every field: task, ref, space, channel, queue,
        /// queue_name, from, to, device, device_name, level, text, hops, claim_id, generation,
        /// attempt and lease.
        #[arg(long)]
        json: bool,
    },
    /// Extends the claim of a task this device holds, as an agent only its own in the space of its
    /// folder, for one more lease, stating its progress. Fails, saying the claim is lost, once
    /// another device may hold the task: the server answered claim-lost, or the lease passed on
    /// this device's clock without an answered extend.
    Extend {
        /// The task's progress.
        #[arg(long, value_enum, default_value = "started")]
        progress: claims::Progress,
        /// The task's #<ref>, as skald claim shows it, or its ID.
        #[arg(value_name = "TASK", allow_hyphen_values = true)]
        task: String,
    },
    /// Completes a task this device holds a claim of, as an agent only its own in the space of its
    /// folder, then sends its result, with the text, to the task's sender. Fails, saying the claim
    /// is lost, as skald extend does; no result is sent then. The local state keeps the complete
    /// before its request: a run that ends before the server answers it or before the result is
    /// sent leaves them to the next skald recv, which sends the result once.
    Complete {
        /// The task's #<ref>, as skald claim shows it, or its ID.
        #[arg(value_name = "TASK", allow_hyphen_values = true)]
        task: String,
        /// The text of the result.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
    },
    /// Fails a task this device holds a claim of, as skald complete completes it; with --retry,
    /// the queue hands the task out again while its attempts last.
    Fail {
        /// The queue may hand the task out again.
        #[arg(long)]
        retry: bool,
        /// The task's #<ref>, as skald claim shows it, or its ID.
        #[arg(value_name = "TASK", allow_hyphen_values = true)]
        task: String,
        /// The text of the result: why it failed.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
    },
    /// Replies to a message, in its channel, to its sender, a person included: as skald send
    /// sends, with the message's ID as its reply_to; as with --to, the reply is handed only to its
    /// sender, but it is not private: every member of the channel gets it. The person replies to a
    /// message its skald recv showed; an agent only to one handed over to it in the space of its
    /// folder. A message can be replied to until its wait ends, as an agent's skald recv says.
    /// Sends nothing, and says why, for a message it cannot reply to, or a reference two messages
    /// it keeps share. Says what skald send says.
    Reply {
        /// The message: its reference, #<ref> or <ref>, as skald recv shows it, or its ID.
        #[arg(value_name = "MESSAGE", allow_hyphen_values = true)]
        message: String,
        /// The text.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
        /// Says the reply as a JSON object, as skald send --json does.
        #[arg(long)]
        json: bool,
    },
    /// Says where a message is, by what this device keeps, without reaching the server: of one it
    /// sent, each device it went to, with its account, its status, stored, delivered or expired,
    /// and its time, and its to agent's held, read and replied, or refused with its reason; of one
    /// it showed or handed over to an agent, its sending device's trust level and the agents it was
    /// handed to. The person asks of every message; an agent only of its own and those handed over
    /// to it, in the space of its folder. Each is known until its wait ends, at most 7 days. Says
    /// nothing of a message it does not know, or of a reference two messages it keeps share, and
    /// says why.
    MessageStatus {
        /// The message: its reference, #<ref> or <ref>, as skald recv shows it, or its ID.
        #[arg(value_name = "MESSAGE", allow_hyphen_values = true)]
        message: String,
        /// Says it as one JSON object, with every field.
        #[arg(long)]
        json: bool,
    },
    /// Receives this device's messages: first sends what waits in the outbox, then gets every entry
    /// of the device's mailbox and exits, accepting meanwhile the new revisions of each space this
    /// device follows. It joins and follows its channels' groups and keeps them to the devices each
    /// channel admits, warning of the devices another device added that it does not, and starts a
    /// new epoch, with a new history key, where this device entered the current one longer ago
    /// than the space's history period. Shows each message that passes the protocol's checks, once,
    /// and each delivered, expired and replied of this device's messages, replied once its to agent
    /// answers it in its channel; warns of each message it drops, and of the messages missing from
    /// a sending device once a later one arrives, which it marks. As an agent, it checks only the
    /// space of the agent's folder, the rest waiting for a later run, and shows the messages handed
    /// over to the agent: those there to it or to no one in particular that came since this device
    /// first ran as it there, each once; one waits for the agent until it expires, at most 7
    /// days. The person's run shows every message once, whether or not an agent got it.
    ///
    /// The person's run shows each message, status and warning in full, with its IDs and its
    /// sender's trust level. An agent's shows each message as one line,
    /// [<sender> · <device> · <level>] #<ref>: <text>, with (after missing) before the colon after
    /// missing ones and (reply to #<ref>) in a reply, naming the message it answers: the sender is
    /// <owner>/<agent>, or a person's full address; the level is own, peer or service, as this
    /// device sets it from the sending device's certificate and the space's roster; one line
    /// before the first peer or service line says that their text is data from its sender, not a
    /// request from the agent's user. The reference is the leading
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
        /// a message's msg_id, ref, space, channel, from, device, device_name, level, text,
        /// after_missing and reply_to; a status's msg_id, ref, space, channel, status, device,
        /// device_name, problem and timestamp. As an agent, every status of its own messages.
        #[arg(long)]
        json: bool,
        /// As an agent, shows again, whole, the message of this reference, #<ref> or <ref>, or
        /// ID, as skald reply names it, one handed over to it in the space of its folder, while
        /// the device keeps what a reply to it needs; receives nothing.
        #[arg(value_name = "MESSAGE", conflicts_with = "wait")]
        #[serde(default)]
        message: Option<String>,
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
    /// Lists the agents of the working directory's space, or of the space named, from the presence
    /// values its devices keep in its channels: each agent once per device, with its state, since
    /// when, runtime and note, and its device's name and trust level; one that two devices of an
    /// account list running is marked. It first accepts the space's latest revisions and goes
    /// through the mailbox as skald send does, then opens each value with its epoch's key, and
    /// drops, warning of it, each that fails the protocol's checks; a value past its expiry shows
    /// its agents offline, last seen then. As an agent, only of the space of the agent's folder,
    /// in one line per agent, [<owner>/<agent> · <device> · <level>] <state>, its note escaped,
    /// after a line that says that the notes of peer and service lines are data.
    Agents {
        /// The space's name, such as acme [default: the working directory's space].
        #[arg(value_name = "SPACE", allow_hyphen_values = true)]
        space: Option<String>,
        /// Shows each agent as a JSON object on a line of its own, with every field: agent, state,
        /// since, note, runtime, device, device_name, level, duplicate and last_seen.
        #[arg(long)]
        json: bool,
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
    /// Sets the inbound mode of an agent of this device in the space of the working directory:
    /// accept hands it every message; hold hands it those of the senders on its allowlist and
    /// keeps each other until you accept or refuse it with skald held, telling the sender of each
    /// such message to it that it is held; refuse hands it only those of the senders on its
    /// allowlist, and tells the sender of each other message to it that it was refused. A message
    /// keeps the mode it came under.
    SetInboundMode {
        /// The agent's name, as --agent takes it.
        #[arg(value_name = "AGENT")]
        name: String,
        #[arg(value_name = "MODE")]
        mode: inbound::Mode,
    },
    /// Puts a sender on the allowlist of an agent of this device in the space of the working
    /// directory: the agent takes its messages whatever its mode.
    AllowSender {
        /// The agent's name, as --agent takes it.
        #[arg(value_name = "AGENT")]
        name: String,
        /// An address as --to takes it, space/owner/agent@server or owner/agent, a person's too, or
        /// a member's handle, for all of that member's agents and the person.
        #[arg(value_name = "SENDER", allow_hyphen_values = true)]
        sender: String,
    },
    /// Lists each agent of this device in the space of the working directory with its inbound mode
    /// and the senders on its allowlist.
    InboundModes,
    /// Sets how messages from senders of a trust level in a channel of the working directory's
    /// space reach this device's agent sessions by the paths that push them in: now, each as it
    /// comes; batch, those of each window together once it ends, the window opened by the first of
    /// them; pull, none, so the agent takes them with inbox or skald recv.
    SetDeliveryMode {
        /// The channel's ID, as skald channels shows it.
        #[arg(value_name = "CHANNEL")]
        channel: String,
        /// The senders' trust level.
        #[arg(value_name = "LEVEL")]
        level: delivery::Level,
        #[arg(value_name = "MODE")]
        mode: delivery::Mode,
        /// The window of batch: whole seconds, minutes or hours, such as 30s, 5m or 2h.
        #[arg(long, value_name = "WINDOW", value_parser = delivery::window)]
        window: Option<u64>,
    },
    /// Sets how the text of messages from peer or service senders in a channel of the working
    /// directory's space reaches this device's agent sessions by the paths that push them in:
    /// signal pushes only a line that tells the agent to call inbox; inline pushes the text,
    /// labelled as data from its sender, which lets another person's text into the agent's context
    /// unasked.
    SetDeliveryText {
        /// The channel's ID, as skald channels shows it.
        #[arg(value_name = "CHANNEL")]
        channel: String,
        /// The senders' trust level: peer or service.
        #[arg(value_name = "LEVEL")]
        level: delivery::Level,
        #[arg(value_name = "TEXT")]
        text: delivery::Text,
    },
    /// Lists, for each channel of the working directory's space this device is in, the delivery
    /// mode of each sender trust level, the text setting of peer and service, and the wakes this
    /// device caused there: each handover a path pushed into an agent's session counts one in each
    /// channel whose messages it carried.
    DeliveryModes {
        /// Shows each channel as a JSON object on a line of its own: channel, wakes, and own, peer
        /// and service, each with mode, window in seconds or null, and, but for own, text.
        #[arg(long)]
        json: bool,
    },
    /// Shows each message an agent in hold keeps for your decision, one at a time, and takes your
    /// decision at the terminal: accept hands it to the agent; refuse, then a reason or an empty
    /// line, tells its sender it was refused. Only as the person at a terminal.
    Held,
    /// A step of skald held after its person's answers, which skald held hands to the helper.
    #[command(skip)]
    HeldStep { step: held::Step },
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
    /// Sets the state of the agent --agent names in the space of the working directory, busy or
    /// available, and its note, which this device's presence shows the space's members while a
    /// session of the agent runs: its skald mcp or skald recv --wait through the helper. A new MCP
    /// session of the agent starts it available, with no note.
    SetStatus {
        /// The agent's state.
        #[arg(value_name = "STATE")]
        state: uploads::Settable,
        /// What it works on, up to 128 bytes in UTF-8; without it, no note.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
    },
    /// Updates skald to the latest stable release on GitHub Releases: downloads the release's
    /// archive for this machine, checks it against the release's checksum and replaces this skald
    /// with it. A beta or an older release is never installed; a skald installed from npm is left
    /// to npm. A token in GITHUB_TOKEN, or else GH_TOKEN, goes to GitHub's API only, for its higher
    /// rate limit.
    Update,
    /// Serves MCP on standard input and output for an agent session, as an agent CLI starts it from
    /// its config; the session's folder is the working directory. Once the client initializes or
    /// discovers the server, the session registers as its agent in the folder's space: the messages
    /// this device takes in from then on are kept for it. Without --agent, the agent's name joins
    /// with '-' the name the client gives itself, the device's name and the device's reference, cut
    /// to 63 characters, so each device gives one agent CLI its own address; two sessions of one
    /// agent CLI in one space on this device share it unless --agent names them. Its inbox tool
    /// returns the agent's lines of skald recv, each once; its send tool sends as the agent's skald
    /// send does, to an agent or, with to *, to every agent in the channel; its set_status tool
    /// sets the agent's status as skald set-status does. Out of one-shot mode it holds a session
    /// of the helper while it runs, so the helper shows its agent running. Each tool call goes
    /// through the helper or runs one-shot, as a command does, and through a newer helper too; once
    /// the skald installed where it was is newer, as when skald was updated while it runs, that
    /// skald runs each call.
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
    /// The hook handler of an agent CLI, which runs it from its hooks' config: Claude Code's
    /// SessionStart, SessionEnd, PostToolUse, Stop, PermissionRequest and Notification; Codex's
    /// SessionStart, PostToolUse, UserPromptSubmit, Stop and PermissionRequest. It reads the hook input on standard
    /// input and runs in its folder, as the agent --agent names or, without it, the agent the session's skald mcp
    /// registers by default. SessionStart registers the session with the helper, starting one if
    /// none runs. On Claude Code, the helper writes the agent's messages into the session's
    /// cross-session socket as they come, and SessionEnd ends that; PostToolUse and Stop give the
    /// agent's waiting messages to the session as additional context, each once, within the CLI's
    /// hook output limit, 10,000 characters: a message from the person's own devices as its line,
    /// as skald recv shows it; for those from others, only a line that tells the agent to call
    /// inbox, or their lines under inline. Both follow the delivery settings, skald
    /// delivery-modes. A message longer than the limit is cut, and skald recv #<ref> shows it
    /// whole. Stop continues the turn with them, up to the CLI's cap of continuations in a row. A
    /// message the socket, a channel event of skald mcp, the --wait hook or the mod took, which
    /// the session does not confirm within 30 seconds, or SKALD_RESEND_AFTER seconds, goes by the
    /// next of them, then by these hooks and inbox. On Codex, every message goes in only as that
    /// line, once for the new ones, to the session whose hook ran last, and the hooks never wait
    /// on the network. On Claude Code, PermissionRequest and a Notification of a permission prompt
    /// record that the session waits on its permission prompt, once per prompt; on Codex,
    /// PermissionRequest does, as does the session's background app-server, which the helper
    /// watches, until its next hook or the app-server says the prompt ended. The person's skald
    /// recv shows each prompt once, without its content. It prints nothing while nothing waits, on
    /// PermissionRequest and Notification, and on a failure, which exits with 1.
    Hook {
        /// The agent CLI.
        #[arg(value_name = "CLI")]
        cli: hook::Cli,
        /// As Claude Code's asyncRewake hook on SessionStart and Stop: waits up to SECONDS for the
        /// agent's messages, prints them as the other hooks give them on standard error and exits
        /// with 2, which starts a turn; with none, it exits with success, printing nothing.
        #[arg(long, value_name = "SECONDS")]
        #[serde(default)]
        wait: Option<u64>,
        /// As the poll of the mod, a Claude Code plugin: runs without hook input and prints, as
        /// JSON, how many messages wait for the agent, and, with --idle, which says no turn runs,
        /// a prompt that asks the agent to call inbox, when the mod is to submit it.
        #[arg(long = "mod", conflicts_with = "wait")]
        #[serde(default)]
        mods: bool,
        /// With --mod: no turn runs in the session.
        #[arg(long, requires = "mods")]
        #[serde(default)]
        idle: bool,
        /// What the hook input gives.
        #[arg(skip)]
        #[serde(default)]
        input: Option<hook::Input>,
    },
    /// Runs the helper of this machine's local state, which the first command that connects as the
    /// device, such as skald send or skald recv, starts in the background. While it runs, it holds
    /// the device's connection and the local state's lock, and each command that changes the local
    /// state goes through it, and each skald recv --wait waits through it. Only this OS account
    /// reaches it. It exits after an idle time with no command, wait, registered Claude Code
    /// session or live Codex session, on sign-out, and for a command of a newer skald.
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
            Command::Invite { .. } => Some("invite"),
            Command::CreateServiceAgent { .. } => Some("create-service-agent"),
            Command::CreateServiceToken { .. } => Some("create-service-token"),
            Command::ReplaceServiceKey { .. } => Some("replace-service-key"),
            Command::RemoveServiceAgent { .. } => Some("remove-service-agent"),
            Command::ServiceTokens { .. } => Some("service-tokens"),
            Command::RevokeServiceToken { .. } => Some("revoke-service-token"),
            Command::RemoveMember { .. } => Some("remove-member"),
            Command::CreateQueue { .. } => Some("create-queue"),
            Command::ChangeQueue { .. } => Some("change-queue"),
            Command::DeleteQueue { .. } => Some("delete-queue"),
            Command::RestartTask { .. } => Some("restart-task"),
            Command::CancelTask { .. } => Some("cancel-task"),
            Command::ExcludeDevice { .. } => Some("exclude-device"),
            Command::AddToChannel { .. } => Some("add-to-channel"),
            Command::RemoveFromChannel { .. } => Some("remove-from-channel"),
            Command::SetHistoryPeriod { .. } => Some("set-history-period"),
            Command::SetDomain { .. } => Some("set-domain"),
            Command::ClearDomain { .. } => Some("clear-domain"),
            Command::MakeAdmin { .. } => Some("make-admin"),
            Command::HandOverAdmin { .. } => Some("hand-over-admin"),
            Command::AddRule { .. } => Some("add-rule"),
            Command::Rules => Some("rules"),
            Command::RemoveRule { .. } => Some("remove-rule"),
            Command::ApproveBinding { .. } => Some("approve-binding"),
            Command::SetInboundMode { .. } => Some("set-inbound-mode"),
            Command::AllowSender { .. } => Some("allow-sender"),
            Command::InboundModes => Some("inbound-modes"),
            Command::SetDeliveryMode { .. } => Some("set-delivery-mode"),
            Command::SetDeliveryText { .. } => Some("set-delivery-text"),
            Command::DeliveryModes { .. } => Some("delivery-modes"),
            Command::Held | Command::HeldStep { .. } => Some("held"),
            Command::FingerprintStep { .. } => Some("fingerprint"),
            Command::Link { browser: false } | Command::LinkStep { .. } => Some("link"),
            _ => None,
        }
    }

    /// Whether, out of one-shot mode, it starts the helper when none runs: it connects as the
    /// device, or, as `message-status`, reads what the device's runs keep.
    fn starts_helper(&self) -> bool {
        matches!(
            self,
            Command::Send { .. }
                | Command::Queues { .. }
                | Command::CreateQueue { .. }
                | Command::ChangeQueue { .. }
                | Command::DeleteQueue { .. }
                | Command::Enqueue { .. }
                | Command::Tasks { .. }
                | Command::RestartTask { .. }
                | Command::CancelTask { .. }
                | Command::Claim { .. }
                | Command::Extend { .. }
                | Command::Complete { .. }
                | Command::Fail { .. }
                | Command::Reply { .. }
                | Command::MessageStatus { .. }
                | Command::Recv { .. }
                | Command::Hook { .. }
                | Command::Agents { .. }
                | Command::Spaces
                | Command::Members { .. }
                | Command::Fingerprint { space: Some(_), .. }
                | Command::Channels { .. }
                | Command::CreateSpace { .. }
                | Command::CreateChannel { .. }
                | Command::Invite { .. }
                | Command::RemoveMember { .. }
                | Command::ExcludeDevice { .. }
                | Command::AddToChannel { .. }
                | Command::RemoveFromChannel { .. }
                | Command::SetHistoryPeriod { .. }
                | Command::SetDomain { .. }
                | Command::ClearDomain { .. }
                | Command::MakeAdmin { .. }
                | Command::HandOverAdmin { .. }
                | Command::Address
        )
    }

    /// Whether it connects to the server of the machine's session as its device.
    fn connects(&self) -> bool {
        (self.starts_helper() && !matches!(self, Command::MessageStatus { .. }))
            || matches!(
                self,
                Command::Devices
                    | Command::RevokeDevice { .. }
                    | Command::SetUsername { .. }
                    | Command::History { .. }
                    | Command::FingerprintStep {
                        step: fingerprints::Step::Sync { .. }
                    }
            )
    }

    /// Whether a run of it says which devices wait for linking ([`link_prompt`]): one that
    /// reaches the server from a linked device, or a tool call of the MCP server.
    fn prompts(&self) -> bool {
        self.connects()
            || matches!(
                self,
                Command::CreateServiceAgent { .. }
                    | Command::CreateServiceToken { .. }
                    | Command::ReplaceServiceKey { .. }
                    | Command::RemoveServiceAgent { .. }
                    | Command::ServiceTokens { .. }
                    | Command::RevokeServiceToken { .. }
                    | Command::SignOut
            )
            || matches!(
                self,
                Command::Mcp { call: Some(call), .. }
                    if !matches!(call, mcp::Call::Register | mcp::Call::Beat)
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
            | Command::InboundModes
            | Command::DeliveryModes { .. }
            | Command::Folder { .. }
            | Command::Update
            | Command::Helper { .. } => None,
            // It takes the lock once its server's URL passes: a refused one leaves no file.
            Command::SignIn { .. } => None,
            // It takes its person's answers here and its steps lock ([`link`], [`held`]).
            Command::Link { .. } | Command::Held => None,
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

/// The devices `skald sign-in --with-recovery-key` names, by the IDs given to `--stolen`,
/// `--lost`, `--keep` and `--keep-revoked`; refused, before it connects, unless its person runs it
/// at a terminal, `person_at_terminal`.
fn recovery_named(
    person_at_terminal: bool,
    [stolen, lost, keep, keep_revoked]: [&[String]; 4],
) -> Result<recovery_link::Named, String> {
    if !person_at_terminal {
        return Err(
            "skald reads the recovery key only from its person typing it at a terminal, never \
             from a pipe, an argument or the environment, and not as an agent; nothing is signed"
                .to_owned(),
        );
    }
    let ids = |given: &[String]| -> Result<Vec<DeviceId>, String> {
        let mut ids: Vec<DeviceId> = Vec::new();
        for text in given {
            let id = text
                .parse()
                .map_err(|_| format!("\"{}\" is not a device ID", connection::printable(text)))?;
            if ids.contains(&id) {
                return Err(format!("the device {id} is named twice"));
            }
            ids.push(id);
        }
        Ok(ids)
    };
    Ok(recovery_link::Named {
        stolen: ids(stolen)?,
        lost: ids(lost)?,
        keep: ids(keep)?,
        keep_revoked: ids(keep_revoked)?,
    })
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

/// The days until an invite expires: at least 1, at most [`invites::MOST_DAYS`].
fn invite_days(given: &str) -> Result<u64, String> {
    given
        .parse::<u64>()
        .ok()
        .filter(|days| (1..=invites::MOST_DAYS).contains(days))
        .ok_or_else(|| format!("give whole days from 1 to {}", invites::MOST_DAYS))
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
        github_api: cli.github_api_url,
        command: cli.command,
        said: link_prompt::Said::default(),
    };
    if let Err(err) = operation.agent() {
        return exit(Err(err));
    }
    match service::from_env() {
        Ok(Some(_)) => return as_service(operation),
        Ok(None) => {}
        Err(err) => return exit(Err(err)),
    }
    // The link holds the invite's secret: it goes only to a terminal, checked before anything is
    // sent.
    if matches!(operation.command, Command::Invite { .. }) && !std::io::stdout().is_terminal() {
        return exit(Err(invites::NOT_AT_TERMINAL.to_owned()));
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
        let settings = (operation.key_storage, operation.speaks, operation.timeout);
        return exit(
            state::dir(operation.state_dir).and_then(|dir| helper::serve(&dir, idle, settings)),
        );
    }
    if let Command::Link { browser } = operation.command {
        return link(operation, !one_shot, browser);
    }
    if let Command::Hook { cli, mods, .. } = operation.command {
        return match (mods, cli) {
            (true, _) => mod_poll(operation, one_shot),
            (false, hook::Cli::Claude) => run_hook(operation, cli, one_shot, None),
            (false, hook::Cli::Codex) => {
                patiently(move |gate| run_hook(operation, cli, one_shot, Some(gate)))
            }
        };
    }
    if let Command::Held = operation.command {
        return held(operation, !one_shot);
    }
    if let Command::Fingerprint {
        space: Some(space),
        handle: Some(handle),
    } = &operation.command
        && operation.agent.is_none()
        && link_code::at_terminal()
    {
        let (space, handle) = (space.clone(), handle.clone());
        return fingerprint(operation, !one_shot, &space, &handle);
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
            waits_here(&dir, &operation, lock)
        }
        route => routed(route, operation),
    }
}

/// Runs `operation` as `route` says: here, holding the lock until it ends, or by the helper, which
/// gave its exit code.
fn routed(route: Result<helper::Route, String>, operation: Operation) -> ExitCode {
    match route {
        Ok(helper::Route::Here(_lock)) => exit(here(operation)),
        Ok(helper::Route::Helped(code)) => code,
        Err(err) => exit(Err(err)),
    }
}

/// Runs `operation` as a service agent's run ([`service`]): only `skald send`, `skald recv`, the
/// commands of tasks a member runs ([`claims`]) and `skald enqueue` without a state directory, as no
/// agent, in this process, without a helper, holding the lock of the state directory, which it
/// creates if it is missing. Without a state directory the run is stateless: it takes no lock and
/// does not wait.
fn as_service(operation: Operation) -> ExitCode {
    let stateless = operation.state_dir.is_none();
    if !matches!(
        operation.command,
        Command::Send { .. }
            | Command::Recv { .. }
            | Command::Claim { .. }
            | Command::Extend { .. }
            | Command::Complete { .. }
            | Command::Fail { .. }
    ) && !(stateless && matches!(operation.command, Command::Enqueue { .. }))
        || operation.agent.is_some()
    {
        return exit(Err(format!(
            "with {} and {} set, skald runs as the service agent of the key secret, which only \
             sends and receives, and as a member runs tasks: skald send, skald recv, skald claim, \
             skald extend, skald complete and skald fail, without --agent; without \
             SKALD_STATE_DIR it also enqueues: skald enqueue",
            service::KEY_VARIABLE,
            service::TOKEN_VARIABLE
        )));
    }
    if stateless {
        if operation.command.waits() {
            return exit(Err(
                "a service agent's skald recv --wait needs a state directory: \
                 set SKALD_STATE_DIR"
                    .to_owned(),
            ));
        }
        service::set_stateless();
        return exit(here(operation));
    }
    let lock = state::dir(operation.state_dir.clone())
        .and_then(|dir| lock::take(&dir, true, operation.timeout).map(|lock| (dir, lock)));
    match lock {
        Ok((dir, lock)) if operation.command.waits() => waits_here(&dir, &operation, lock),
        // Held until the run exits.
        Ok((_, _lock)) => exit(here(operation)),
        Err(err) => exit(Err(err)),
    }
}

/// Runs `operation`, a command that waits, in this process with `lock`, the lock of the state
/// directory `dir` ([`wait::here`]), then closes the connection it kept.
fn waits_here(dir: &Path, operation: &Operation, lock: Option<lock::Lock>) -> ExitCode {
    let waited = wait::here(dir, operation, lock);
    mailbox::close_kept();
    match waited {
        Ok(Some(code)) => code,
        Ok(None) => ExitCode::SUCCESS,
        Err(err) => exit(Err(err)),
    }
}

/// `skald link` as `operation` says ([`link_code::run`]), its person on its local page if
/// `browser`: each step goes through the helper that runs, or one it starts if `starts`; otherwise
/// it runs here, holding the lock until the run ends.
fn link(operation: Operation, starts: bool, browser: bool) -> ExitCode {
    let opener = match browser.then(page::Opener::find).transpose() {
        Ok(opener) => opener,
        Err(err) => return exit(Err(err)),
    };
    let dir = match state::dir(operation.state_dir.clone()) {
        Ok(dir) => dir,
        Err(err) => return exit(Err(err)),
    };
    let Some(opener) = opener else {
        return link_steps(&operation, &dir, starts, Person::Terminal);
    };
    let page = page::Page::new(&dir, opener);
    let code = output::into(page.tee(operation.agent.is_some()), || {
        link_steps(&operation, &dir, starts, Person::Page(&page))
    });
    page.finish(None);
    recv::keep(&dir, &page.held());
    code
}

/// The steps of `skald link` as `operation` says, on the machine whose state is in `dir`, its
/// person's answers taken from `person` ([`link`]).
fn link_steps(operation: &Operation, dir: &Path, starts: bool, person: Person) -> ExitCode {
    recovery::say_if_waiting(dir);
    let mut held: Option<Option<lock::Lock>> = None;
    let mut starts = starts;
    link_code::run(dir, person, |step| {
        let operation = operation.step(Command::LinkStep { step });
        if held.is_none() {
            // A later step needs the helper that holds its run. What it shows goes to the run's
            // output, and to its page.
            let starts = (false, std::mem::take(&mut starts), false);
            let mut shown = |stream, bytes: &[u8]| output::write(stream, bytes);
            match helper::route_into(dir, &operation, starts, &mut shown) {
                Ok(helper::Route::Here(lock)) => held = Some(lock),
                Ok(helper::Route::Helped(code)) => return code,
                Err(err) => return exit(Err(err)),
            }
        }
        exit(here(operation))
    })
}

/// `skald hook <cli>` as `operation` says ([`hook`]): reads the hook input, then runs in its
/// folder, through the helper that runs, or one it starts unless `one_shot`, or here; a Codex hook
/// runs here ([`codex_here`]). Prints only what the run gives the session, unless `gate` says the
/// hook no longer waits for it, and exits with 1 on a failure; 2 only as the `asyncRewake` hook
/// ([`rewake`]).
fn run_hook(
    mut operation: Operation,
    cli: hook::Cli,
    one_shot: bool,
    gate: Option<Gate>,
) -> ExitCode {
    let wait = match operation.command {
        Command::Hook { wait, .. } => wait,
        _ => None,
    };
    let cap = std::env::var(hook::CAP_SETTING).ok();
    let Ok(mut input) = hook::input(std::io::stdin().lock(), cap.as_deref()) else {
        return ExitCode::FAILURE;
    };
    let Some(event) = input.event.filter(|event| cli.handles(*event)) else {
        return ExitCode::SUCCESS;
    };
    let waits_on = matches!(event, hook::Event::SessionStart | hook::Event::Stop { .. });
    if wait.is_some() && !waits_on {
        return ExitCode::SUCCESS;
    }
    if cli == hook::Cli::Codex {
        input.session.home = std::env::var_os(hook::CODEX_HOME)
            .filter(|home| !home.is_empty())
            .map(PathBuf::from);
        input.session.process = codex::parent();
        input.session.codex = input.session.process.and_then(codex::exe_of);
    } else if event == hook::Event::SessionStart {
        let setting = |name| std::env::var_os(name).filter(|value| !value.is_empty());
        input.session.socket = setting(hook::SOCKET_SETTING).map(PathBuf::from);
        input.session.token =
            setting(hook::TOKEN_SETTING).and_then(|token| token.into_string().ok());
    }
    if std::env::set_current_dir(&input.cwd).is_err() {
        return ExitCode::FAILURE;
    }
    let Ok(dir) = state::dir(operation.state_dir.clone()) else {
        return ExitCode::FAILURE;
    };
    // A machine with no state has no message for an agent.
    if !dir.join(state::FILE).is_file() {
        return ExitCode::SUCCESS;
    }
    // A Codex session starts the helper only in a folder in a space; its other hooks start none.
    let starts = !one_shot
        && match cli {
            hook::Cli::Claude => true,
            hook::Cli::Codex => {
                event == hook::Event::SessionStart
                    && output::into(|_, _| Ok(()), || folders::working_space(&dir).is_ok())
            }
        };
    operation.command = Command::Hook {
        cli,
        input: Some(input),
        wait,
        mods: false,
        idle: false,
    };
    if let Some(seconds) = wait {
        return rewake(operation, &dir, Duration::from_secs(seconds), one_shot);
    }
    if cli == hook::Cli::Codex && state::current(&dir) {
        return codex_here(operation, &dir, starts, gate);
    }
    hooked(operation, &dir, starts, gate)
}

/// Runs `operation`, a hook's, by the state in `dir` through the helper that runs, or one it
/// starts if `starts`, or here, printing what [`given`] lets through; exits with 1 on a failure.
fn hooked(operation: Operation, dir: &Path, starts: bool, gate: Option<Gate>) -> ExitCode {
    let failed = |code| match code == ExitCode::SUCCESS {
        true => code,
        false => ExitCode::FAILURE,
    };
    let mut sink = given(gate.clone());
    match helper::route_into(dir, &operation, (false, starts, false), &mut sink) {
        Ok(helper::Route::Helped(code)) => failed(code),
        // Held until the run exits.
        Ok(helper::Route::Here(_lock)) => match output::into(given(gate), || here(operation)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        },
        Err(_) => ExitCode::FAILURE,
    }
}

/// How long the `asyncRewake` hook waits before it tries again once its helper went away.
const REWAKE_AGAIN: Duration = Duration::from_secs(1);

/// `skald hook claude --wait` as `operation` says, by the state in `dir`: waits through the helper
/// that runs, or one it starts unless `one_shot`, as the agent `--agent` names or, without it, the
/// default one, for up to `wait`, until the agent's paths give it messages ([`resend`]); prints them
/// on standard error and exits with 2, which starts a turn. With none, without a helper, or in a
/// folder in no space, at once, it exits with success, printing nothing.
fn rewake(mut operation: Operation, dir: &Path, wait: Duration, one_shot: bool) -> ExitCode {
    let deadline = Instant::now() + wait;
    if folders::working_space(dir).is_err() {
        return ExitCode::SUCCESS;
    }
    if operation.agent.is_none() {
        let Some(agent) = mcp::default_agent(dir, hook::CLAUDE_CODE) else {
            return ExitCode::SUCCESS;
        };
        operation.agent = Some(agent.to_string());
    }
    let (give, given) = std::sync::mpsc::channel();
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        loop {
            let mut said = Vec::new();
            let mut sink = |stream, bytes: &[u8]| {
                if stream == output::Stream::Err {
                    said.extend_from_slice(bytes);
                }
                Ok(())
            };
            match helper::route_into(&dir, &operation, (false, !one_shot, false), &mut sink) {
                Ok(helper::Route::Helped(code))
                    if code == ExitCode::SUCCESS && !said.is_empty() =>
                {
                    let _ = give.send(said);
                    return;
                }
                Ok(helper::Route::Here(_)) => return,
                // The helper went away, or ended the wait giving nothing.
                _ => std::thread::sleep(REWAKE_AGAIN),
            }
        }
    });
    match given.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(said) => match std::io::stderr().lock().write_all(&said) {
            Ok(()) => ExitCode::from(REWAKES),
            Err(_) => ExitCode::FAILURE,
        },
        Err(_) => ExitCode::SUCCESS,
    }
}

/// The exit code with which the `asyncRewake` hook starts a turn.
const REWAKES: u8 = 2;

/// `skald hook claude --mod` as `operation` says: runs in the session's folder, the working
/// directory, as the agent `--agent` names or, without it, the default one, through the helper
/// that runs, or one it starts unless `one_shot`, or here, which gives no prompt; prints only
/// what the run gives ([`resend::mod_poll`]).
fn mod_poll(mut operation: Operation, one_shot: bool) -> ExitCode {
    let Ok(dir) = state::dir(operation.state_dir.clone()) else {
        return ExitCode::FAILURE;
    };
    let agent = match &operation.agent {
        Some(_) => operation.agent.clone(),
        None if dir.join(state::FILE).is_file() => {
            mcp::default_agent(&dir, hook::CLAUDE_CODE).map(|agent| agent.to_string())
        }
        None => None,
    };
    let Some(agent) = agent else {
        println!("{}", serde_json::json!({ "waiting": 0 }));
        return ExitCode::SUCCESS;
    };
    operation.agent = Some(agent);
    hooked(operation, &dir, !one_shot, None)
}

/// Runs `operation`, a Codex hook's, here by the state in `dir`, which has this client's schema,
/// without the lock, so it never waits behind a run that holds it on the network: it only reads
/// and writes the local state, whose WAL checkpoint waits for it. Then starts the helper, without
/// waiting for it, if `starts`. Prints what [`given`] lets through; exits with 1 on a failure.
fn codex_here(operation: Operation, dir: &Path, starts: bool, gate: Option<Gate>) -> ExitCode {
    let timeout = operation.timeout;
    let ran = output::into(given(gate), || here(operation));
    if starts {
        helper::start(dir, timeout);
    }
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// What a hook's run gives the agent CLI: its output, unless `gate` says the hook no longer waits
/// for it; not what else it says, such as its error.
fn given(gate: Option<Gate>) -> impl FnMut(output::Stream, &[u8]) -> std::io::Result<()> + Clone {
    move |stream, bytes: &[u8]| match (stream, &gate) {
        (output::Stream::Err, _) => Ok(()),
        (output::Stream::Out, None) => output::direct(stream, bytes),
        (output::Stream::Out, Some(gate)) => {
            let waits = gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match *waits {
                true => output::direct(stream, bytes),
                false => Err(std::io::Error::other("the hook no longer waits")),
            }
        }
    }
}

/// Whether a hook still waits for its run's output.
type Gate = std::sync::Arc<std::sync::Mutex<bool>>;

/// Runs `run`, the whole of a Codex hook, its input's read included, on a thread of its own and
/// waits for it at most [`codex::PATIENCE`], or as [`codex::PATIENCE_SETTING`] sets; then the run
/// writes nothing more, and the hook exits
/// with success, having printed what it wrote.
fn patiently(run: impl FnOnce(Gate) -> ExitCode + Send + 'static) -> ExitCode {
    let gate: Gate = std::sync::Arc::new(std::sync::Mutex::new(true));
    let (done, finished) = std::sync::mpsc::channel();
    let given = std::sync::Arc::clone(&gate);
    std::thread::spawn(move || {
        let _ = done.send(run(given));
    });
    let patience = (std::env::var(codex::PATIENCE_SETTING).ok())
        .and_then(|seconds| seconds.trim().parse().ok())
        .map_or(codex::PATIENCE, Duration::from_secs);
    match finished.recv_timeout(patience) {
        Ok(code) => code,
        Err(_) => {
            // A write under way ends first.
            *gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
            ExitCode::SUCCESS
        }
    }
}

/// `skald held` as `operation` says ([`held::run`]): each step goes through the helper that runs,
/// or one it starts if `starts`; otherwise it runs here, holding the lock for the step.
fn held(operation: Operation, starts: bool) -> ExitCode {
    if let Err(err) = operation.agent() {
        return exit(Err(err));
    }
    let dir = match state::dir(operation.state_dir.clone()) {
        Ok(dir) => dir,
        Err(err) => return exit(Err(err)),
    };
    let mut starts = starts;
    held::run(&dir, |step| {
        let operation = operation.step(Command::HeldStep { step });
        let route = helper::route(&dir, &operation, false, std::mem::take(&mut starts));
        routed(route, operation)
    })
}

/// `skald fingerprint <space> <handle>` as `operation` says, by its person at a terminal
/// ([`fingerprints::run`]): each step goes through the helper that runs, or one it starts if
/// `starts`; otherwise it runs here, holding the lock for the step.
fn fingerprint(operation: Operation, starts: bool, space: &str, handle: &str) -> ExitCode {
    let dir = match state::dir(operation.state_dir.clone()) {
        Ok(dir) => dir,
        Err(err) => return exit(Err(err)),
    };
    let mut starts = starts;
    fingerprints::run(&dir, space, handle, |step| {
        let operation = operation.step(Command::FingerprintStep { step });
        let route = helper::route(&dir, &operation, false, std::mem::take(&mut starts));
        routed(route, operation)
    })
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
    /// The base URL of GitHub's REST API.
    #[serde(default = "github_api")]
    github_api: String,
    command: Command,
    /// The devices waiting for linking its runs said ([`link_prompt`]): a `skald recv --wait`
    /// runs `skald recv` again and again, and says each once.
    #[serde(skip)]
    said: link_prompt::Said,
}

/// The base URL of GitHub's REST API by default.
fn github_api() -> String {
    update::GITHUB_API_URL.to_owned()
}

impl Operation {
    /// The operation of `command`, a step of its own interactive command, as the person.
    fn step(&self, command: Command) -> Self {
        Operation {
            speaks: self.speaks,
            timeout: self.timeout,
            clock_offset: self.clock_offset,
            state_dir: self.state_dir.clone(),
            key_storage: self.key_storage,
            agent: None,
            github_api: self.github_api.clone(),
            command,
            said: link_prompt::Said::default(),
        }
    }

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
/// helper does. Out of the helper, a run as an agent then checks the device's presence
/// ([`uploads::keep`]), but an MCP server's own and its heartbeat: the helper keeps its session's.
fn perform(operation: Operation) -> Result<(), String> {
    // A Codex hook never waits on the network.
    let checks = !uploads::in_helper()
        && operation.agent.is_some()
        && !matches!(
            operation.command,
            Command::Mcp {
                call: None | Some(mcp::Call::Beat),
                ..
            } | Command::Hook {
                cli: hook::Cli::Codex,
                ..
            }
        );
    let settings = uploads::Settings::of(&operation);
    let state_dir = operation.state_dir.clone();
    let ran = performed(operation);
    if checks && let Ok(dir) = state::dir(state_dir) {
        uploads::keep(&dir, settings, &uploads::Live::default());
    }
    ran
}

/// [`perform`] without its check of the presence.
fn performed(operation: Operation) -> Result<(), String> {
    let agent = operation.agent()?;
    clock::set_offset(operation.clock_offset);
    ssh_checks::set(&operation.github_api, operation.timeout);
    let Operation {
        speaks,
        timeout,
        state_dir,
        key_storage,
        github_api,
        command,
        said,
        ..
    } = operation;
    // A hook says nothing but what it gives the session; a stateless service agent's run opens no
    // state directory.
    let hooked = matches!(command, Command::Hook { .. });
    if !service::stateless()
        && !hooked
        && let Ok(dir) = state::dir(state_dir.clone())
    {
        // A grant the device took waits for the server to store its certificate.
        if command.connects() {
            link_code::finish(&dir, key_storage, speaks, timeout, agent.is_none());
        }
        if command.prompts() {
            let json = matches!(
                command,
                Command::Recv { json: true, .. }
                    | Command::Send { json: true, .. }
                    | Command::Agents { json: true, .. }
                    | Command::Queues { json: true, .. }
                    | Command::Enqueue { json: true, .. }
                    | Command::Tasks { json: true, .. }
                    | Command::Claim { json: true, .. }
            );
            link_prompt::show(&dir, key_storage, (speaks, timeout), json, &said);
        }
        // An MCP server says it once, to its agent CLI's log, not in a tool's result.
        if !matches!(
            command,
            Command::SignIn { .. }
                | Command::Helper { .. }
                | Command::LinkStep { .. }
                | Command::HeldStep { .. }
                | Command::FingerprintStep {
                    step: fingerprints::Step::Verify { .. }
                }
                | Command::Mcp { .. }
        ) {
            recovery::say_if_waiting(&dir);
        }
    }
    match command {
        Command::Status { server } => status(&server, speaks, timeout),
        Command::SignIn {
            server,
            name,
            with_recovery_key,
            stolen,
            lost,
            keep,
            keep_revoked,
            browser,
            ssh_key,
            no_ssh,
            github,
        } => {
            // Without a browser, nothing is sent and nothing listens.
            let opener = browser.then(page::Opener::find).transpose()?;
            let recovery = with_recovery_key
                .then(|| {
                    recovery_named(
                        agent.is_none() && link_code::at_terminal(),
                        [&stolen, &lost, &keep, &keep_revoked],
                    )
                })
                .transpose()?;
            let host_name = std::env::var_os(HOST_NAME)
                .unwrap_or_else(gethostname::gethostname)
                .to_string_lossy()
                .into_owned();
            state::dir(state_dir).and_then(|dir| {
                let page = opener.map(|opener| page::Page::new(&dir, opener));
                // A recovery key or a link code is shown only to the person: at a terminal, or on
                // the page.
                let person = match &page {
                    Some(page) => Some(Person::Page(page)),
                    None if agent.is_none() && link_code::at_terminal() => Some(Person::Terminal),
                    None => None,
                };
                let options = sign_in::Options {
                    storage: key_storage,
                    name,
                    host_name,
                    person,
                    recovery,
                    ssh: ssh_signing::Options {
                        key: ssh_key,
                        off: no_ssh,
                        github,
                        may_ask: agent.is_none() && link_code::at_terminal(),
                    },
                };
                let signed_in = || {
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
                };
                let Some(page) = &page else {
                    return signed_in();
                };
                let result = output::into(page.tee(agent.is_some()), signed_in);
                page.finish(result.as_ref().err().map(String::as_str));
                recv::keep(&dir, &page.held());
                result
            })
        }
        Command::Devices => state::dir(state_dir)
            .and_then(|dir| devices::run(&dir, key_storage, speaks, timeout, agent.is_none())),
        // It runs through `link`.
        Command::Link { .. } => Err("skald link runs only through its own process".to_owned()),
        Command::LinkStep { step } => state::dir(state_dir)
            .and_then(|dir| link_code::step(&dir, key_storage, speaks, timeout, step)),
        // It runs through `held`.
        Command::Held => Err("skald held runs only through its own process".to_owned()),
        Command::HeldStep { step } => state::dir(state_dir)
            .and_then(|dir| held::step(&dir, key_storage, (speaks, timeout), step)),
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
        Command::Members {
            space,
            dns_resolver,
        } => synced(
            state_dir,
            key_storage,
            speaks,
            timeout,
            &space,
            agent.is_some(),
        )
        .and_then(|synced| spaces::members(synced, (&dns_resolver, timeout))),
        Command::Fingerprint { space: None, .. } => {
            state::dir(state_dir).and_then(|dir| fingerprints::own(&dir))
        }
        Command::Fingerprint {
            space: Some(space),
            handle,
        } => synced(
            state_dir,
            key_storage,
            speaks,
            timeout,
            &space,
            agent.is_some(),
        )
        .and_then(|synced| fingerprints::show(&synced, handle.as_deref().unwrap_or_default())),
        Command::FingerprintStep { step } => state::dir(state_dir)
            .and_then(|dir| fingerprints::step(&dir, key_storage, (speaks, timeout), step)),
        Command::CreateChannel { space } => synced(
            state_dir,
            key_storage,
            speaks,
            timeout,
            &space,
            agent.is_some(),
        )
        .and_then(channels::create),
        Command::Invite {
            space,
            admin,
            channels,
            without_history,
            days,
        } => synced(state_dir, key_storage, speaks, timeout, &space, false).and_then(|synced| {
            let asked = invites::Asked {
                admin,
                with_history: &channels,
                without_history: &without_history,
                days,
            };
            invites::create(synced, &asked)
        }),
        Command::CreateServiceAgent {
            space,
            address,
            granted,
            member,
        } => synced(state_dir, key_storage, speaks, timeout, &space, false)
            .and_then(|synced| service_agents::create(synced, &address, &granted, member)),
        Command::CreateServiceToken {
            space,
            address,
            granted,
            keep_earlier,
        } => synced(state_dir, key_storage, speaks, timeout, &space, false)
            .and_then(|synced| service_agents::renew(synced, &address, &granted, keep_earlier)),
        Command::ReplaceServiceKey {
            space,
            address,
            granted,
        } => synced(state_dir, key_storage, speaks, timeout, &space, false)
            .and_then(|synced| service_agents::replace(synced, &address, &granted)),
        Command::RemoveServiceAgent { space, address } => {
            synced(state_dir, key_storage, speaks, timeout, &space, false)
                .and_then(|synced| service_agents::remove(synced, &address))
        }
        Command::ServiceTokens { space } => {
            synced(state_dir, key_storage, speaks, timeout, &space, false)
                .and_then(service_agents::list)
        }
        Command::RevokeServiceToken { space, id } => {
            synced(state_dir, key_storage, speaks, timeout, &space, false)
                .and_then(|synced| service_agents::revoke(synced, &id))
        }
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
        Command::SetDomain { space, domain } => edit(
            (state_dir, key_storage, speaks, timeout),
            &space,
            admin::Edit::Domain(Some(domain)),
        ),
        Command::ClearDomain { space } => edit(
            (state_dir, key_storage, speaks, timeout),
            &space,
            admin::Edit::Domain(None),
        ),
        Command::MakeAdmin { space, handle } => edit(
            (state_dir, key_storage, speaks, timeout),
            &space,
            admin::Edit::MakeAdmin(handle),
        ),
        Command::HandOverAdmin { space, handle } => edit(
            (state_dir, key_storage, speaks, timeout),
            &space,
            admin::Edit::HandOver(handle),
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
        Command::Queues { channel, json } => queue_command(
            state_dir,
            (key_storage, (speaks, timeout)),
            (channel.as_deref(), agent.is_some()),
            |run| queues::list(run, json),
        ),
        Command::CreateQueue {
            channel,
            mode,
            lease,
            max_attempts,
            name,
        } => {
            let settings = queues::Settings {
                mode,
                lease,
                max_attempts,
            };
            queue_command(
                state_dir,
                (key_storage, (speaks, timeout)),
                (channel.as_deref(), false),
                |run| queues::create(run, &name, &settings),
            )
        }
        Command::ChangeQueue {
            channel,
            lease,
            max_attempts,
            name,
        } => {
            let settings = queues::Settings {
                mode: None,
                lease,
                max_attempts,
            };
            queue_command(
                state_dir,
                (key_storage, (speaks, timeout)),
                (channel.as_deref(), false),
                |run| queues::change(run, &name, &settings),
            )
        }
        Command::DeleteQueue { channel, name } => queue_command(
            state_dir,
            (key_storage, (speaks, timeout)),
            (channel.as_deref(), false),
            |run| queues::delete(run, &name),
        ),
        Command::Enqueue {
            channel,
            queue,
            to,
            idempotency_key,
            deadline,
            text,
            json,
        } => {
            let given = queues::Enqueue {
                queue: &queue,
                to: to.as_deref(),
                idempotency_key: idempotency_key.as_deref(),
                deadline: deadline.as_deref(),
                text: &text,
                json,
            };
            queue_command(
                state_dir,
                (key_storage, (speaks, timeout)),
                (channel.as_deref(), agent.is_some()),
                |run| queues::enqueue(run, &given, agent.as_ref()),
            )
        }
        Command::Tasks {
            channel,
            queue,
            json,
        } => queue_command(
            state_dir,
            (key_storage, (speaks, timeout)),
            (channel.as_deref(), agent.is_some()),
            |run| tasks::list(run, queue.as_deref(), json, agent.as_ref()),
        ),
        Command::RestartTask { channel, task } => (tasks::Change::Restart).run(
            state_dir,
            (key_storage, (speaks, timeout)),
            (channel, task),
        ),
        Command::CancelTask { channel, task } => (tasks::Change::Cancel).run(
            state_dir,
            (key_storage, (speaks, timeout)),
            (channel, task),
        ),
        Command::Claim {
            channel,
            queue,
            json,
        } => queue_command(
            state_dir,
            (key_storage, (speaks, timeout)),
            (channel.as_deref(), agent.is_some()),
            |run| claims::claim(run, &queue, agent.as_ref(), json),
        ),
        Command::Extend { progress, task } => queue_command(
            state_dir,
            (key_storage, (speaks, timeout)),
            (None, agent.is_some()),
            |run| claims::extend(run, &task, progress, agent.as_ref()),
        ),
        Command::Complete { task, text } => end_command(
            state_dir,
            (key_storage, (speaks, timeout)),
            (&task, &text),
            claims::Ending::Complete,
            agent.as_ref(),
        ),
        Command::Fail { retry, task, text } => end_command(
            state_dir,
            (key_storage, (speaks, timeout)),
            (&task, &text),
            claims::Ending::Fail(retry),
            agent.as_ref(),
        ),
        Command::Reply {
            message,
            text,
            json,
        } => state::dir(state_dir).and_then(|dir| {
            reply::run(
                &dir,
                key_storage,
                (speaks, timeout),
                (&message, &text),
                agent.as_ref(),
                json,
            )
        }),
        Command::MessageStatus { message, json } => state::dir(state_dir)
            .and_then(|dir| message_status::run(&dir, key_storage, &message, agent.as_ref(), json)),
        Command::Recv {
            json,
            message: Some(message),
            ..
        } => {
            let agent =
                agent.ok_or("skald recv <message> runs as an agent: give --agent <name>")?;
            let dir = state::dir(state_dir)?;
            recv::reread(
                &dir,
                key_storage,
                &agent,
                &message,
                forms::Form::of(true, json),
            )
        }
        Command::Hook { mods: true, .. } => {
            let dir = state::dir(state_dir)?;
            let Some(agent) = agent else {
                return Err("skald hook --mod runs only as itself".to_owned());
            };
            // Without the helper, no path gives the mod its prompt.
            resend::mod_poll(&dir, &agent, false)
        }
        Command::Hook { cli, input, .. } => {
            let (Some(input), Ok(dir)) = (input, state::dir(state_dir)) else {
                return Err("skald hook runs only as itself".to_owned());
            };
            let Some(event) = input.event else {
                return Ok(());
            };
            let client = match cli {
                hook::Cli::Claude => hook::CLAUDE_CODE,
                hook::Cli::Codex => hook::CODEX,
            };
            // Without --agent, the agent the session's skald mcp registers by default.
            let Some(agent) = agent.or_else(|| mcp::default_agent(&dir, client)) else {
                return Ok(());
            };
            if cli == hook::Cli::Codex {
                return codex::hook(&dir, &agent, (event, &input.session));
            }
            // A later hook of the session tells that the prompt it waited on ended.
            if matches!(
                event,
                hook::Event::SessionEnd | hook::Event::PostToolUse | hook::Event::Stop { .. }
            ) {
                permission_prompts::ended(&dir, input.session.id.as_deref())?;
            }
            match event {
                hook::Event::SessionStart => claude_socket::register(&dir, &agent, &input.session),
                hook::Event::SessionEnd => claude_socket::end(&dir, &agent, &input.session),
                hook::Event::PermissionRequest | hook::Event::PermissionPrompt => {
                    permission_prompts::told(&dir, &agent, &input)
                }
                _ => recv::hook(
                    &dir,
                    key_storage,
                    (speaks, timeout),
                    &agent,
                    (event, input.cap, &input.session),
                ),
            }
        }
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
        Command::Agents { space, json } => state::dir(state_dir).and_then(|dir| {
            presence::run(
                &dir,
                key_storage,
                (speaks, timeout),
                space.as_deref(),
                agent.is_some(),
                json,
            )
        }),
        Command::AddRule { folder, space } => {
            state::dir(state_dir).and_then(|dir| folders::add_rule(&dir, &folder, &space))
        }
        Command::Rules => state::dir(state_dir).and_then(|dir| folders::rules(&dir)),
        Command::RemoveRule { folder } => {
            state::dir(state_dir).and_then(|dir| folders::remove_rule(&dir, &folder))
        }
        Command::SetInboundMode { name, mode } => {
            state::dir(state_dir).and_then(|dir| inbound::set_mode(&dir, &name, mode))
        }
        Command::AllowSender { name, sender } => {
            state::dir(state_dir).and_then(|dir| inbound::allow(&dir, &name, &sender))
        }
        Command::InboundModes => state::dir(state_dir).and_then(|dir| inbound::list(&dir)),
        Command::SetDeliveryMode {
            channel,
            level,
            mode,
            window,
        } => state::dir(state_dir)
            .and_then(|dir| delivery::set_mode(&dir, &channel, level, (mode, window))),
        Command::SetDeliveryText {
            channel,
            level,
            text,
        } => state::dir(state_dir).and_then(|dir| delivery::set_text(&dir, &channel, level, text)),
        Command::DeliveryModes { json } => {
            state::dir(state_dir).and_then(|dir| delivery::list(&dir, json))
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
        Command::SetStatus { state, note } => agent
            .ok_or_else(|| "name the agent: skald --agent <name> set-status".to_owned())
            .and_then(|agent| {
                state::dir(state_dir).and_then(|dir| {
                    recv::holding_warnings(&dir, true, || {
                        uploads::set_status(&dir, &agent, state, note.as_deref())
                    })
                })
            }),
        Command::Update => update::run(&github_api, timeout),
        Command::Mcp { call, .. } => state::dir(state_dir).and_then(|dir| {
            mcp::perform(call, &dir, key_storage, (speaks, timeout), agent.as_ref())
        }),
        Command::Helper { .. } => Err("the helper runs only as itself".to_owned()),
    }
}

/// One run of `skald recv` for `operation`, a `skald recv --wait`, which leaves its connection to
/// watch ([`wait`]), and the devices waiting for linking it did not say yet ([`link_prompt`]):
/// whether it showed what the wait waits for.
fn received(operation: &Operation) -> Result<bool, String> {
    let agent = operation.agent()?;
    clock::set_offset(operation.clock_offset);
    ssh_checks::set(&operation.github_api, operation.timeout);
    let dir = state::dir(operation.state_dir.clone())?;
    let settings = (operation.speaks, operation.timeout);
    let shown = recv::run(
        &dir,
        operation.key_storage,
        settings,
        agent.as_ref(),
        (true, operation.json()),
    )?;
    // After the run, so that a `link-notice` it read is in what the server gives.
    let prompted = link_prompt::show(
        &dir,
        operation.key_storage,
        settings,
        operation.json(),
        &operation.said,
    );
    // The person's wait shows it as a warning; an agent's waits for a message.
    Ok(shown || (prompted && agent.is_none()))
}

/// Runs `command` on the state in `state_dir`, its secrets in `storage`, with `settings`, its
/// protocol versions and timeout, in `channel`, if given, as an agent if `agent`.
fn queue_command(
    state_dir: Option<PathBuf>,
    (storage, settings): (secrets::Storage, (VersionRange, Duration)),
    (channel, agent): (Option<&str>, bool),
    command: impl FnOnce(&queues::Run) -> Result<(), String>,
) -> Result<(), String> {
    let dir = state::dir(state_dir)?;
    command(&queues::Run {
        dir: &dir,
        storage,
        settings,
        channel,
        agent,
    })
}

impl tasks::Change {
    /// Makes this change to the task `task` names in `channel`, if given, by the state in
    /// `state_dir`, its secrets in `storage`, with `settings`, its protocol versions and timeout; a
    /// change is the person's.
    fn run(
        self,
        state_dir: Option<PathBuf>,
        (storage, settings): (secrets::Storage, (VersionRange, Duration)),
        (channel, task): (Option<String>, String),
    ) -> Result<(), String> {
        queue_command(
            state_dir,
            (storage, settings),
            (channel.as_deref(), false),
            |run| tasks::change(run, &task, self),
        )
    }
}

/// Ends the claim of `task` with `text` as `ending` says ([`claims::end`]), on the state in
/// `state_dir`, its secrets in `storage`, with `settings`, as `agent` if given.
fn end_command(
    state_dir: Option<PathBuf>,
    (storage, settings): (secrets::Storage, (VersionRange, Duration)),
    (task, text): (&str, &str),
    ending: claims::Ending,
    agent: Option<&Name>,
) -> Result<(), String> {
    queue_command(
        state_dir,
        (storage, settings),
        (None, agent.is_some()),
        |run| claims::end(run, (task, text), ending, agent),
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

    /// An invite expires in whole days from 1 to 30; anything else is refused.
    #[test]
    fn invite_days_are_whole_days_from_1_to_30() {
        for days in [1, 7, 30] {
            assert_eq!(invite_days(&days.to_string()), Ok(days), "{days}");
        }
        for given in ["0", "31", "", "7d", "1.5", "-1", "99999999999999999999"] {
            assert!(invite_days(given).is_err(), "{given:?}");
        }
    }

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
