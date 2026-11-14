//! The `skald-oidc-stand-in` binary: the stand-in for tests in other processes, such as the self-host
//! stack's in `script/self-host-test`. It serves until killed, and prints
//! `skald-oidc-stand-in: listening on http://<address>` to stderr once it listens.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::thread;

use clap::Parser;
use skald_oidc_stand_in::{Client, Key, Person, Provider};

/// A local stand-in for an OpenID Connect provider, for tests.
#[derive(Parser)]
#[command(name = "skald-oidc-stand-in")]
struct Cli {
    /// Address to listen on.
    #[arg(long)]
    listen: SocketAddr,

    /// The issuer its metadata and ID tokens name, such as the address of a TLS proxy in front of
    /// --listen.
    #[arg(long, value_name = "URL")]
    issuer: String,

    /// The registered client's ID.
    #[arg(long)]
    client_id: String,

    /// The registered client's secret.
    #[arg(long)]
    client_secret: String,

    /// The registered client's redirect URI.
    #[arg(long, value_name = "URI")]
    redirect_uri: String,

    /// A person with an account at it; repeatable.
    #[arg(long = "person", value_name = "SUBJECT=EMAIL", value_parser = person)]
    people: Vec<Person>,
}

/// A person given as `<subject>=<email>`.
fn person(value: &str) -> Result<Person, String> {
    let (subject, email) = value
        .split_once('=')
        .ok_or_else(|| format!("not <subject>=<email>: {value}"))?;
    Ok(Person::new(subject, email))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let provider = match Provider::serve(cli.listen, &cli.issuer, &[Key::rsa()]) {
        Ok(provider) => provider,
        Err(err) => {
            eprintln!(
                "skald-oidc-stand-in: cannot listen on {}: {err}",
                cli.listen
            );
            return ExitCode::FAILURE;
        }
    };
    provider.register(Client {
        client_id: cli.client_id,
        client_secret: cli.client_secret,
        redirect_uri: cli.redirect_uri,
    });
    for person in cli.people {
        provider.put_person(person);
    }
    eprintln!("skald-oidc-stand-in: listening on http://{}", cli.listen);
    loop {
        thread::park();
    }
}
