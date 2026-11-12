//! The `skald-github-stand-in` binary: the stand-in for tests in other processes, such as the web
//! tests. It prints `skald-github-stand-in: listening on http://<address>` to stderr once it listens.

use std::net::SocketAddr;
use std::process::ExitCode;

use clap::Parser;
use skald_github_stand_in::App;

/// A local stand-in for GitHub's OAuth and user endpoints, for tests.
#[derive(Parser)]
#[command(name = "skald-github-stand-in")]
struct Cli {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:0")]
    listen: SocketAddr,

    /// The OAuth App's client ID.
    #[arg(long)]
    client_id: String,

    /// The OAuth App's client secret.
    #[arg(long)]
    client_secret: String,

    /// The OAuth App's callback URL; without it, any redirect_uri is taken.
    #[arg(long)]
    callback: Option<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let listener = match tokio::net::TcpListener::bind(cli.listen).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!(
                "skald-github-stand-in: cannot listen on {}: {err}",
                cli.listen
            );
            return ExitCode::FAILURE;
        }
    };
    match listener.local_addr() {
        Ok(addr) => eprintln!("skald-github-stand-in: listening on http://{addr}"),
        Err(err) => {
            eprintln!("skald-github-stand-in: {err}");
            return ExitCode::FAILURE;
        }
    }
    let app = App {
        client_id: cli.client_id,
        client_secret: cli.client_secret,
        callback: cli.callback,
    };
    match skald_github_stand_in::serve(listener, app).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("skald-github-stand-in: {err}");
            ExitCode::FAILURE
        }
    }
}
