//! The `skald-server` binary.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use skald_proto::ServerName;
use skald_server::{
    AttachmentStorage, BucketEndpoint, BucketSettings, ClientSecret, GitHubApp, GitHubUrl, Listed,
    OidcSettings, PrivacyPolicyUrl, PublicUrl, SentryDsn, Settings, SignInSettings, SignUp,
};
use tokio::net::TcpListener;

/// The environment variable that holds the GitHub OAuth App's client secret.
const GITHUB_SECRET_ENV: &str = "SKALD_GITHUB_CLIENT_SECRET";
/// The environment variable that holds the Google OAuth client's secret.
const GOOGLE_SECRET_ENV: &str = "SKALD_GOOGLE_CLIENT_SECRET";
/// The environment variable that holds the attachment bucket's secret key.
const BUCKET_SECRET_ENV: &str = "SKALD_ATTACHMENTS_BUCKET_SECRET_KEY";
/// The least time between two reads of Google's keys.
const GOOGLE_KEY_REFRESH: Duration = Duration::from_secs(60);
/// The longest `--long-poll-max-wait`, in seconds: a day.
const LONG_POLL_LONGEST: u64 = 24 * 3_600;
/// The longest `--sse-keep-alive`, in seconds: an hour.
const SSE_KEEP_ALIVE_LONGEST: u64 = 3_600;

/// `--sign-up`.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SignUpMode {
    Open,
    Closed,
}

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

    /// The server's public address, the origin the CLI and devices reach, e.g.
    /// https://skald.example. Devices prove their keys over it; the server's name defaults to its
    /// host.
    #[arg(long, env = "SKALD_PUBLIC_URL", value_name = "URL")]
    public_url: Option<PublicUrl>,

    /// The origin browsers sign in at, e.g. https://app.skald.example [default: the public
    /// address]. GitHub's callback is <URL>/auth/github/callback, Google's
    /// <URL>/auth/google/callback; the verification page is <URL>/device; browser forms are taken
    /// only from it; under https, cookies go over HTTPS only.
    #[arg(long, env = "SKALD_WEB_URL", value_name = "URL")]
    web_url: Option<PublicUrl>,

    /// The GitHub OAuth App's client ID. Its client secret comes only from the environment
    /// variable SKALD_GITHUB_CLIENT_SECRET. Without both, GitHub sign-in is off.
    #[arg(long, env = "SKALD_GITHUB_CLIENT_ID", value_name = "ID")]
    github_client_id: Option<String>,

    /// GitHub's address: its OAuth authorize and token endpoints.
    #[arg(
        long,
        env = "SKALD_GITHUB_URL",
        value_name = "URL",
        default_value = "https://github.com"
    )]
    github_url: GitHubUrl,

    /// GitHub's API address: its user endpoint.
    #[arg(
        long,
        env = "SKALD_GITHUB_API_URL",
        value_name = "URL",
        default_value = "https://api.github.com"
    )]
    github_api_url: GitHubUrl,

    /// How long a sign-in in the browser may take from its start to the provider's callback.
    #[arg(
        long,
        env = "SKALD_SIGN_IN_STATE_LIFETIME",
        value_name = "SECONDS",
        default_value_t = 600,
        value_parser = clap::value_parser!(u64).range(1..=86_400)
    )]
    sign_in_state_lifetime: u64,

    /// How recent a web session's sign-in has to be to link another sign-in method to its account
    /// or remove one; with an older one, the person signs in again first.
    #[arg(
        long,
        env = "SKALD_LINK_SIGN_IN_AGE",
        value_name = "SECONDS",
        default_value_t = 300,
        value_parser = clap::value_parser!(u64).range(1..=86_400)
    )]
    link_sign_in_age: u64,

    /// The Google OAuth client's ID. Its client secret comes only from the environment variable
    /// SKALD_GOOGLE_CLIENT_SECRET. Without both, Google sign-in is off.
    #[arg(long, env = "SKALD_GOOGLE_CLIENT_ID", value_name = "ID")]
    google_client_id: Option<String>,

    /// Google's issuer: its OpenID Connect metadata is at <URL>/.well-known/openid-configuration.
    #[arg(
        long,
        env = "SKALD_GOOGLE_ISSUER",
        value_name = "URL",
        default_value = "https://accounts.google.com"
    )]
    google_issuer: String,

    /// How long a device flow's codes work, from device authorization to the machine's last poll.
    #[arg(
        long,
        env = "SKALD_DEVICE_CODE_LIFETIME",
        value_name = "SECONDS",
        default_value_t = 600,
        value_parser = clap::value_parser!(u64).range(1..=3_600)
    )]
    device_code_lifetime: u64,

    /// The least time between a device flow's polls; a poll more than 1 s sooner answers slow_down.
    #[arg(
        long,
        env = "SKALD_DEVICE_POLL_INTERVAL",
        value_name = "SECONDS",
        default_value_t = 5,
        value_parser = clap::value_parser!(u64).range(1..=60)
    )]
    device_poll_interval: u64,

    /// Who may sign up: open, anyone the sign-in providers sign in; closed, only the identities on
    /// the --sign-up-* lists. Existing accounts sign in and link methods either way.
    #[arg(
        long,
        env = "SKALD_SIGN_UP",
        value_name = "MODE",
        value_enum,
        default_value_t = SignUpMode::Open
    )]
    sign_up: SignUpMode,

    /// While sign-up is closed, the GitHub logins that may sign up, comma-separated, without case.
    #[arg(
        long,
        env = "SKALD_SIGN_UP_GITHUB_LOGINS",
        value_name = "LOGINS",
        value_delimiter = ','
    )]
    sign_up_github_logins: Vec<String>,

    /// While sign-up is closed, the Google emails that may sign up, comma-separated, without case;
    /// an email counts only once Google has verified it.
    #[arg(
        long,
        env = "SKALD_SIGN_UP_GOOGLE_EMAILS",
        value_name = "EMAILS",
        value_delimiter = ','
    )]
    sign_up_google_emails: Vec<String>,

    /// While sign-up is closed, the Google Workspace or Cloud organizations whose accounts may sign
    /// up, by the domain of their ID tokens' hd claim, comma-separated, without case; an email's
    /// domain does not count.
    #[arg(
        long,
        env = "SKALD_SIGN_UP_GOOGLE_DOMAINS",
        value_name = "DOMAINS",
        value_delimiter = ','
    )]
    sign_up_google_domains: Vec<String>,

    /// The address of the server's privacy policy, an https URL, e.g.
    /// https://skald.example/privacy: the web app's sign-in page, the verification page and the page
    /// of the account's sign-in methods link it [default: no link].
    #[arg(long, env = "SKALD_PRIVACY_POLICY_URL", value_name = "URL")]
    privacy_policy_url: Option<PrivacyPolicyUrl>,

    /// The number of spaces an account may own: the ones it created, its personal space included,
    /// which counts also before it is created [default: no limit].
    #[arg(
        long,
        env = "SKALD_OWNED_SPACES_LIMIT",
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    owned_spaces_limit: Option<u32>,

    /// How long an undelivered mailbox entry waits at most, at most 100 years; the server keeps
    /// each idempotency key as long [default: entries wait for their ack, keys are kept for good].
    #[arg(
        long,
        env = "SKALD_MAILBOX_RETENTION",
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..=skald_server::MAILBOX_LONGEST)
    )]
    mailbox_retention: Option<u64>,

    /// How long the server keeps a channel's history entries, at most 100 years; it deletes the
    /// links and wraps of epochs before those of the entries it deletes [default: kept for good].
    #[arg(
        long,
        env = "SKALD_HISTORY_RETENTION",
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..=skald_server::HISTORY_LONGEST)
    )]
    history_retention: Option<u64>,

    /// The longest payload of an envelope a device sends, in bytes; a longer one is refused with
    /// too-large [default: no limit].
    #[arg(
        long,
        env = "SKALD_MESSAGE_SIZE_LIMIT",
        value_name = "BYTES",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    message_size_limit: Option<u64>,

    /// The number of messages a space stores per calendar month in UTC, each counted once whatever
    /// its envelopes; the next one is refused with quota-exceeded [default: no limit].
    #[arg(
        long,
        env = "SKALD_MONTHLY_MESSAGES_LIMIT",
        value_name = "N",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    monthly_messages_limit: Option<u64>,

    /// The number of devices with a certificate an account may have, service and revoked devices
    /// aside; a certificate that would give an account more is not stored [default: no limit].
    #[arg(
        long,
        env = "SKALD_DEVICES_LIMIT",
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    devices_limit: Option<u32>,

    /// The number of service agent entries a space's latest revision may list; a revision that
    /// lists more than the setting and than the revision before it is not stored [default: no
    /// limit].
    #[arg(
        long,
        env = "SKALD_SERVICE_AGENTS_LIMIT",
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    service_agents_limit: Option<u32>,

    /// The longest a long-poll waits for its device's mailbox, in seconds, at most a day; one that
    /// asks for longer ends at it [default: 30].
    #[arg(
        long,
        env = "SKALD_LONG_POLL_MAX_WAIT",
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..=LONG_POLL_LONGEST)
    )]
    long_poll_max_wait: Option<u64>,

    /// How often an idle mailbox stream over server-sent events sends a keep-alive, in seconds, at
    /// most an hour [default: 15].
    #[arg(
        long,
        env = "SKALD_SSE_KEEP_ALIVE",
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..=SSE_KEEP_ALIVE_LONGEST)
    )]
    sse_keep_alive: Option<u64>,

    /// How long after its hello a link code's run ends, in seconds, at most the code's 5 minutes
    /// [default: 300].
    #[arg(
        long,
        env = "SKALD_LINK_CODE_LIFETIME",
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..=skald_server::LINK_CODE_LIFETIME.as_secs())
    )]
    link_code_lifetime: Option<u64>,

    /// The server's name in agent addresses, a DNS name such as skald.example; it should never
    /// change [default: the host of --public-url, unless that is an IP address].
    #[arg(long, env = "SKALD_SERVER_NAME", value_name = "NAME")]
    server_name: Option<ServerName>,

    /// The directory that keeps attachments; a bucket replaces it [default: no attachment storage].
    #[arg(long, env = "SKALD_ATTACHMENTS_DIR", value_name = "DIR")]
    attachments_dir: Option<PathBuf>,

    /// The S3-compatible bucket that keeps attachments, in place of the directory. Its secret key
    /// comes only from the environment variable SKALD_ATTACHMENTS_BUCKET_SECRET_KEY.
    #[arg(long, env = "SKALD_ATTACHMENTS_BUCKET", value_name = "NAME")]
    attachments_bucket: Option<String>,

    /// The bucket's S3 endpoint, https or http on a loopback address; the bucket is addressed in
    /// its path, e.g. https://s3.example/<NAME>/<object>.
    #[arg(long, env = "SKALD_ATTACHMENTS_BUCKET_ENDPOINT", value_name = "URL")]
    attachments_bucket_endpoint: Option<BucketEndpoint>,

    /// The bucket's region.
    #[arg(long, env = "SKALD_ATTACHMENTS_BUCKET_REGION", value_name = "REGION")]
    attachments_bucket_region: Option<String>,

    /// The bucket's access key ID.
    #[arg(
        long,
        env = "SKALD_ATTACHMENTS_BUCKET_ACCESS_KEY_ID",
        value_name = "ID"
    )]
    attachments_bucket_access_key_id: Option<String>,

    /// The Sentry DSN the server reports its errors to, https or http on a loopback address
    /// [default: no error reports].
    #[arg(
        long,
        env = "SKALD_SENTRY_DSN",
        value_name = "DSN",
        hide_env_values = true
    )]
    sentry_dsn: Option<SentryDsn>,
}

/// The secret in the environment variable `name`; `None` if it is unset or empty.
fn secret(name: &str) -> Result<Option<ClientSecret>, String> {
    Ok(env::var_os(name)
        .map(|secret| {
            secret
                .into_string()
                .map_err(|_| format!("{name} is not UTF-8"))
        })
        .transpose()?
        .and_then(ClientSecret::new))
}

/// A provider's client ID, given by `flag`, and its secret, from the environment variable
/// `secret_env`: both or neither.
fn client(
    client_id: Option<String>,
    flag: &str,
    secret_env: &str,
) -> Result<Option<(String, ClientSecret)>, String> {
    match (client_id, secret(secret_env)?) {
        (None, None) => Ok(None),
        (Some(client_id), Some(secret)) => Ok(Some((client_id, secret))),
        (Some(_), None) => Err(format!("{flag} needs the client secret in {secret_env}")),
        (None, Some(_)) => Err(format!("{secret_env} needs {flag}")),
    }
}

/// The entries of a comma-separated list, trimmed, without empty ones.
fn entries(list: Vec<String>) -> Vec<String> {
    list.into_iter()
        .map(|entry| entry.trim().to_owned())
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// The start's warning of `settings` while sign-up is open and a provider signs people in.
fn open_warning(settings: &Settings) -> Option<String> {
    let sign_in = settings.sign_in.as_ref()?;
    if settings.sign_up != SignUp::Open {
        return None;
    }
    let providers = match (sign_in.github.is_some(), sign_in.google.is_some()) {
        (true, true) => "GitHub or Google",
        (true, false) => "GitHub",
        (false, true) => "Google",
        (false, false) => return None,
    };
    Some(format!(
        "skald-server: warning: sign-up is open: any {providers} account can sign up; \
         --sign-up closed (SKALD_SIGN_UP=closed) admits only the listed ones"
    ))
}

impl Cli {
    /// The attachment storage: the bucket, given with all its settings and `secret_key`, or the
    /// directory, or none.
    fn attachment_storage(
        &mut self,
        secret_key: Option<ClientSecret>,
    ) -> Result<Option<AttachmentStorage>, String> {
        let Some(name) = self.attachments_bucket.take() else {
            let set = [
                (
                    self.attachments_bucket_endpoint.is_some(),
                    "--attachments-bucket-endpoint",
                ),
                (
                    self.attachments_bucket_region.is_some(),
                    "--attachments-bucket-region",
                ),
                (
                    self.attachments_bucket_access_key_id.is_some(),
                    "--attachments-bucket-access-key-id",
                ),
                (secret_key.is_some(), BUCKET_SECRET_ENV),
            ];
            if let Some((_, setting)) = set.iter().find(|(set, _)| *set) {
                return Err(format!("{setting} needs --attachments-bucket"));
            }
            return Ok(self
                .attachments_dir
                .take()
                .map(AttachmentStorage::Directory));
        };
        let needs = |setting: &str| format!("--attachments-bucket needs {setting}");
        Ok(Some(AttachmentStorage::Bucket(BucketSettings {
            name,
            endpoint: self
                .attachments_bucket_endpoint
                .take()
                .ok_or_else(|| needs("--attachments-bucket-endpoint"))?,
            region: self
                .attachments_bucket_region
                .take()
                .ok_or_else(|| needs("--attachments-bucket-region"))?,
            access_key_id: self
                .attachments_bucket_access_key_id
                .take()
                .ok_or_else(|| needs("--attachments-bucket-access-key-id"))?,
            secret_key: secret_key
                .ok_or_else(|| needs(&format!("the secret key in {BUCKET_SECRET_ENV}")))?,
        })))
    }

    /// Who may sign up; the lists only while sign-up is closed.
    fn sign_up(&mut self) -> Result<SignUp, String> {
        let listed = Listed {
            github_logins: entries(std::mem::take(&mut self.sign_up_github_logins)),
            google_emails: entries(std::mem::take(&mut self.sign_up_google_emails)),
            google_domains: entries(std::mem::take(&mut self.sign_up_google_domains)),
        };
        match self.sign_up {
            SignUpMode::Closed => Ok(SignUp::Closed(listed)),
            SignUpMode::Open if listed == Listed::default() => Ok(SignUp::Open),
            SignUpMode::Open => Err(
                "the lists of who may sign up need --sign-up closed (SKALD_SIGN_UP=closed)"
                    .to_owned(),
            ),
        }
    }

    /// The settings, or why they do not fit together.
    fn settings(mut self) -> Result<Settings, String> {
        let sign_up = self.sign_up()?;
        let server_name = self
            .server_name
            .or_else(|| self.public_url.as_ref().and_then(PublicUrl::server_name));
        let github = client(
            self.github_client_id,
            "--github-client-id",
            GITHUB_SECRET_ENV,
        )?
        .map(|(client_id, client_secret)| GitHubApp {
            client_id,
            client_secret,
            url: self.github_url,
            api_url: self.github_api_url,
        });
        let google = client(
            self.google_client_id,
            "--google-client-id",
            GOOGLE_SECRET_ENV,
        )?
        .map(|(client_id, client_secret)| {
            skald_server::issuer_url(&self.google_issuer)?;
            Ok::<_, String>(OidcSettings {
                issuer: self.google_issuer,
                client_id,
                client_secret,
                key_refresh: GOOGLE_KEY_REFRESH,
            })
        })
        .transpose()?;
        let sign_in = if github.is_none() && google.is_none() {
            None
        } else {
            Some(SignInSettings {
                public_url: self.public_url.ok_or("sign-in needs --public-url")?,
                web_url: self.web_url,
                github,
                google,
                state_lifetime: Duration::from_secs(self.sign_in_state_lifetime),
                link_age: Duration::from_secs(self.link_sign_in_age),
                device_code_lifetime: Duration::from_secs(self.device_code_lifetime),
                device_poll_interval: Duration::from_secs(self.device_poll_interval),
            })
        };
        Ok(Settings {
            versions: self.protocol_versions.unwrap_or_default(),
            sign_in,
            sign_up,
            privacy_policy_url: self.privacy_policy_url,
            owned_spaces_limit: self.owned_spaces_limit,
            server_name,
            mailbox_retention: self.mailbox_retention.map(Duration::from_secs),
            history_retention: self.history_retention.map(Duration::from_secs),
            message_size_limit: self.message_size_limit,
            monthly_messages_limit: self.monthly_messages_limit,
            devices_limit: self.devices_limit,
            service_agents_limit: self.service_agents_limit,
            long_poll_max_wait: self.long_poll_max_wait.map(Duration::from_secs),
            sse_keep_alive: self.sse_keep_alive.map(Duration::from_secs),
            link_code_lifetime: self.link_code_lifetime.map(Duration::from_secs),
        })
    }
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

/// Connects to PostgreSQL, applies pending migrations, checks the attachment storage, then serves
/// the API.
async fn run(mut cli: Cli) -> Result<(), String> {
    if let Some(dsn) = cli.sentry_dsn.take() {
        skald_server::report_errors_to(dsn);
    }
    let listen = cli.listen;
    let database_url = cli.database_url.clone();
    let attachments = cli.attachment_storage(secret(BUCKET_SECRET_ENV)?)?;
    let settings = cli.settings()?;
    let warning = open_warning(&settings);
    let db = skald_server::connect(&database_url)
        .await
        .map_err(|err| format!("cannot connect to PostgreSQL: {err}"))?;
    skald_server::MIGRATOR
        .run(&db)
        .await
        .map_err(|err| format!("cannot apply migrations: {err}"))?;
    if let Some(storage) = attachments.clone() {
        tokio::task::spawn_blocking(move || storage.check())
            .await
            .map_err(|err| format!("cannot check the attachment storage: {err}"))??;
    }

    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| format!("cannot listen on {listen}: {err}"))?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    eprintln!("skald-server: listening on http://{addr}");
    // After the line above, which the tests and the image's users read first.
    if let Some(storage) = &attachments {
        eprintln!("skald-server: attachments in {storage}");
    }
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    axum::serve(listener, skald_server::app(db, settings))
        .await
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command line of the arguments `args` and a database URL.
    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(
            [
                "skald-server",
                "--database-url",
                "postgres://localhost/skald",
            ]
            .iter()
            .chain(args),
        )
        .expect("valid arguments")
    }

    /// The attachment storage of the arguments `args`, with `secret_key` as the bucket's.
    fn storage(args: &[&str], secret_key: Option<&str>) -> Result<Option<String>, String> {
        let secret_key = secret_key.and_then(|key| ClientSecret::new(key.to_owned()));
        cli(args)
            .attachment_storage(secret_key)
            .map(|storage| storage.map(|storage| storage.to_string()))
    }

    const BUCKET: [&str; 8] = [
        "--attachments-bucket",
        "files",
        "--attachments-bucket-endpoint",
        "https://s3.example",
        "--attachments-bucket-region",
        "auto",
        "--attachments-bucket-access-key-id",
        "key",
    ];

    #[test]
    fn the_mailbox_retention_is_from_a_second_to_100_years() {
        let retention = |seconds: u64| {
            Cli::try_parse_from([
                "skald-server",
                "--database-url",
                "postgres://localhost/skald",
                "--mailbox-retention",
                &seconds.to_string(),
            ])
            .map(|cli| cli.mailbox_retention)
            .ok()
        };
        assert_eq!(retention(1), Some(Some(1)));
        let longest = skald_server::MAILBOX_LONGEST;
        assert_eq!(retention(longest), Some(Some(longest)));
        assert_eq!(retention(0), None);
        assert_eq!(retention(longest + 1), None);
    }

    #[test]
    fn the_history_retention_is_from_a_second_to_100_years() {
        let retention = |seconds: u64| {
            Cli::try_parse_from([
                "skald-server",
                "--database-url",
                "postgres://localhost/skald",
                "--history-retention",
                &seconds.to_string(),
            ])
            .map(|cli| cli.history_retention)
            .ok()
        };
        assert_eq!(retention(1), Some(Some(1)));
        let longest = skald_server::HISTORY_LONGEST;
        assert_eq!(retention(longest), Some(Some(longest)));
        assert_eq!(retention(0), None);
        assert_eq!(retention(longest + 1), None);
    }

    #[test]
    fn the_longest_wait_of_a_long_poll_is_from_a_second_to_a_day() {
        let wait = |seconds: u64| {
            Cli::try_parse_from([
                "skald-server",
                "--database-url",
                "postgres://localhost/skald",
                "--long-poll-max-wait",
                &seconds.to_string(),
            ])
            .map(|cli| cli.long_poll_max_wait)
            .ok()
        };
        assert_eq!(wait(1), Some(Some(1)));
        assert_eq!(wait(86_400), Some(Some(86_400)));
        assert_eq!(wait(0), None);
        assert_eq!(wait(86_401), None);
    }

    #[test]
    fn the_keep_alive_of_a_stream_is_from_a_second_to_an_hour() {
        let interval = |seconds: u64| {
            Cli::try_parse_from([
                "skald-server",
                "--database-url",
                "postgres://localhost/skald",
                "--sse-keep-alive",
                &seconds.to_string(),
            ])
            .map(|cli| cli.sse_keep_alive)
            .ok()
        };
        assert_eq!(interval(1), Some(Some(1)));
        assert_eq!(interval(3_600), Some(Some(3_600)));
        assert_eq!(interval(0), None);
        assert_eq!(interval(3_601), None);
    }

    #[test]
    fn a_link_codes_run_lasts_from_a_second_to_5_minutes() {
        let lifetime = |seconds: u64| {
            Cli::try_parse_from([
                "skald-server",
                "--database-url",
                "postgres://localhost/skald",
                "--link-code-lifetime",
                &seconds.to_string(),
            ])
            .map(|cli| cli.link_code_lifetime)
            .ok()
        };
        assert_eq!(lifetime(1), Some(Some(1)));
        assert_eq!(lifetime(300), Some(Some(300)));
        assert_eq!(lifetime(0), None);
        assert_eq!(lifetime(301), None);
    }

    #[test]
    fn the_cloud_limits_are_from_1() {
        let parsed = |setting: &str, value: &str| {
            Cli::try_parse_from([
                "skald-server",
                "--database-url",
                "postgres://localhost/skald",
                setting,
                value,
            ])
            .map(|cli| {
                [
                    cli.message_size_limit,
                    cli.monthly_messages_limit,
                    cli.devices_limit.map(u64::from),
                    cli.service_agents_limit.map(u64::from),
                ]
            })
            .ok()
        };
        let settings = [
            "--message-size-limit",
            "--monthly-messages-limit",
            "--devices-limit",
            "--service-agents-limit",
        ];
        for (at, setting) in settings.into_iter().enumerate() {
            let mut one = [None; 4];
            one[at] = Some(1);
            assert_eq!(parsed(setting, "1"), Some(one), "{setting}");
            assert_eq!(parsed(setting, "0"), None, "{setting}");
        }
        assert_eq!(parsed("--devices-limit", "4294967296"), None);
        assert_eq!(parsed("--service-agents-limit", "4294967296"), None);
    }

    #[test]
    fn a_bucket_needs_all_its_settings() {
        assert_eq!(
            storage(&BUCKET, Some("secret")),
            Ok(Some("the bucket files at https://s3.example/".to_owned()))
        );
        assert_eq!(
            storage(&BUCKET, None),
            Err(format!(
                "--attachments-bucket needs the secret key in {BUCKET_SECRET_ENV}"
            ))
        );
        for missing in [2, 4, 6] {
            let mut args = BUCKET.to_vec();
            let setting = args.remove(missing);
            args.remove(missing);
            assert_eq!(
                storage(&args, Some("secret")),
                Err(format!("--attachments-bucket needs {setting}"))
            );
        }
    }

    #[test]
    fn a_bucket_setting_needs_the_bucket() {
        for at in [2, 4, 6] {
            assert_eq!(
                storage(&BUCKET[at..at + 2], None),
                Err(format!("{} needs --attachments-bucket", BUCKET[at]))
            );
        }
        assert_eq!(
            storage(&[], Some("secret")),
            Err(format!("{BUCKET_SECRET_ENV} needs --attachments-bucket"))
        );
    }

    /// The sign-up of the arguments `args`.
    fn sign_up(args: &[&str]) -> Result<SignUp, String> {
        cli(args).sign_up()
    }

    #[test]
    fn sign_up_is_open_unless_closed_with_its_lists() {
        assert_eq!(sign_up(&[]), Ok(SignUp::Open));
        assert_eq!(sign_up(&["--sign-up", "open"]), Ok(SignUp::Open));
        assert_eq!(
            sign_up(&["--sign-up", "closed"]),
            Ok(SignUp::Closed(Listed::default()))
        );
        assert_eq!(
            sign_up(&[
                "--sign-up",
                "closed",
                "--sign-up-github-logins",
                " ada, ,bob,",
                "--sign-up-google-emails",
                "grace@example.com",
                "--sign-up-google-domains",
                "acme.example,",
            ]),
            Ok(SignUp::Closed(Listed {
                github_logins: vec!["ada".to_owned(), "bob".to_owned()],
                google_emails: vec!["grace@example.com".to_owned()],
                google_domains: vec!["acme.example".to_owned()],
            }))
        );
        assert_eq!(
            sign_up(&["--sign-up-github-logins", " , "]),
            Ok(SignUp::Open)
        );
        let needs = Err(
            "the lists of who may sign up need --sign-up closed (SKALD_SIGN_UP=closed)".to_owned(),
        );
        for list in [
            "--sign-up-github-logins",
            "--sign-up-google-emails",
            "--sign-up-google-domains",
        ] {
            assert_eq!(sign_up(&["--sign-up", "open", list, "x"]), needs, "{list}");
            assert_eq!(sign_up(&[list, "x"]), needs, "{list}");
        }
    }

    /// Settings with the sign-up `sign_up` and the sign-in of GitHub if `github` and Google if
    /// `google`, if either.
    fn with_sign_up(sign_up: SignUp, github: bool, google: bool) -> Settings {
        let secret = || ClientSecret::new("secret".to_owned()).expect("a secret");
        let url = |url: &str| url.parse::<GitHubUrl>().expect("a GitHub address");
        Settings {
            sign_in: (github || google).then(|| SignInSettings {
                public_url: "https://skald.example".parse().expect("a public address"),
                web_url: None,
                github: github.then(|| GitHubApp {
                    client_id: "Ov23li0fake".to_owned(),
                    client_secret: secret(),
                    url: url("https://github.com"),
                    api_url: url("https://api.github.com"),
                }),
                google: google.then(|| OidcSettings {
                    issuer: "https://accounts.google.com".to_owned(),
                    client_id: "fake.apps.googleusercontent.com".to_owned(),
                    client_secret: secret(),
                    key_refresh: GOOGLE_KEY_REFRESH,
                }),
                state_lifetime: Duration::from_secs(600),
                link_age: Duration::from_secs(300),
                device_code_lifetime: Duration::from_secs(600),
                device_poll_interval: Duration::from_secs(5),
            }),
            sign_up,
            ..Settings::default()
        }
    }

    #[test]
    fn open_sign_up_warns_of_the_providers_it_admits_from() {
        let warning = |github, google| open_warning(&with_sign_up(SignUp::Open, github, google));
        let any = |providers: &str| {
            Some(format!(
                "skald-server: warning: sign-up is open: any {providers} account can sign up; \
                 --sign-up closed (SKALD_SIGN_UP=closed) admits only the listed ones"
            ))
        };
        assert_eq!(warning(true, true), any("GitHub or Google"));
        assert_eq!(warning(true, false), any("GitHub"));
        assert_eq!(warning(false, true), any("Google"));
        assert_eq!(warning(false, false), None);
        let closed = SignUp::Closed(Listed::default());
        assert_eq!(open_warning(&with_sign_up(closed, true, true)), None);
    }

    #[test]
    fn the_directory_unless_a_bucket_replaces_it() {
        let dir = ["--attachments-dir", "/srv/attachments"];
        assert_eq!(
            storage(&dir, None),
            Ok(Some("the directory /srv/attachments".to_owned()))
        );
        assert_eq!(
            storage(&[&dir[..], &BUCKET[..]].concat(), Some("secret")),
            Ok(Some("the bucket files at https://s3.example/".to_owned()))
        );
        assert_eq!(storage(&[], None), Ok(None));
    }
}
