//! The `skald-server` binary.

use std::env;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use skald_server::{ClientSecret, GitHubApp, GitHubUrl, PublicUrl, Settings, SignInSettings};
use tokio::net::TcpListener;

/// The environment variable that holds the GitHub OAuth App's client secret.
const CLIENT_SECRET_ENV: &str = "SKALD_GITHUB_CLIENT_SECRET";

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

    /// The server's public address, the origin browsers open, e.g. https://skald.example. GitHub's
    /// callback is <URL>/auth/github/callback; under https, cookies go over HTTPS only.
    #[arg(long, env = "SKALD_PUBLIC_URL", value_name = "URL")]
    public_url: Option<PublicUrl>,

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

    /// How long a GitHub sign-in may take from its start to GitHub's callback.
    #[arg(
        long,
        env = "SKALD_SIGN_IN_STATE_LIFETIME",
        value_name = "SECONDS",
        default_value_t = 600,
        value_parser = clap::value_parser!(u64).range(1..=86_400)
    )]
    sign_in_state_lifetime: u64,

    /// How long a device flow's codes work, from device authorization to the machine's last poll.
    #[arg(
        long,
        env = "SKALD_DEVICE_CODE_LIFETIME",
        value_name = "SECONDS",
        default_value_t = 600,
        value_parser = clap::value_parser!(u64).range(1..=3_600)
    )]
    device_code_lifetime: u64,

    /// The least time between a device flow's polls; a poll sooner answers slow_down.
    #[arg(
        long,
        env = "SKALD_DEVICE_POLL_INTERVAL",
        value_name = "SECONDS",
        default_value_t = 5,
        value_parser = clap::value_parser!(u64).range(1..=60)
    )]
    device_poll_interval: u64,
}

impl Cli {
    /// The settings, or why they do not fit together.
    fn settings(self) -> Result<Settings, String> {
        let secret = env::var_os(CLIENT_SECRET_ENV)
            .map(|secret| {
                secret
                    .into_string()
                    .map_err(|_| format!("{CLIENT_SECRET_ENV} is not UTF-8"))
            })
            .transpose()?
            .and_then(ClientSecret::new);
        let sign_in = match (self.github_client_id, secret) {
            (None, None) => None,
            (Some(client_id), Some(client_secret)) => Some(SignInSettings {
                public_url: self.public_url.ok_or("GitHub sign-in needs --public-url")?,
                github: GitHubApp {
                    client_id,
                    client_secret,
                    url: self.github_url,
                    api_url: self.github_api_url,
                },
                state_lifetime: Duration::from_secs(self.sign_in_state_lifetime),
                device_code_lifetime: Duration::from_secs(self.device_code_lifetime),
                device_poll_interval: Duration::from_secs(self.device_poll_interval),
            }),
            (Some(_), None) => {
                return Err(format!(
                    "--github-client-id needs the client secret in {CLIENT_SECRET_ENV}"
                ));
            }
            (None, Some(_)) => {
                return Err(format!("{CLIENT_SECRET_ENV} needs --github-client-id"));
            }
        };
        Ok(Settings {
            versions: self.protocol_versions.unwrap_or_default(),
            sign_in,
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

/// Connects to PostgreSQL, applies pending migrations, then serves the API.
async fn run(cli: Cli) -> Result<(), String> {
    let listen = cli.listen;
    let database_url = cli.database_url.clone();
    let settings = cli.settings()?;
    let db = skald_server::connect(&database_url)
        .await
        .map_err(|err| format!("cannot connect to PostgreSQL: {err}"))?;
    skald_server::MIGRATOR
        .run(&db)
        .await
        .map_err(|err| format!("cannot apply migrations: {err}"))?;

    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| format!("cannot listen on {listen}: {err}"))?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    eprintln!("skald-server: listening on http://{addr}");
    axum::serve(listener, skald_server::app(db, settings))
        .await
        .map_err(|err| err.to_string())
}
