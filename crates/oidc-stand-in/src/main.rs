//! The `skald-oidc-stand-in` binary: the stand-in for tests in other processes, such as the self-host
//! stack's in `script/self-host-test`. It serves until killed, and prints
//! `skald-oidc-stand-in: listening on http://<address>` to stderr once it listens.
//!
//! Usage: `skald-oidc-stand-in --listen <address> --issuer <URL> --client-id <ID> --client-secret
//! <secret> --redirect-uri <URI> [--person <subject>=<email>]...`: the issuer as its metadata and ID
//! tokens name it, such as the address of a TLS proxy in front of `--listen`; the client registered
//! with it; the people with an account at it.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::thread;

use skald_oidc_stand_in::{Client, Key, Person, Provider};

const USAGE: &str = "usage: skald-oidc-stand-in --listen <address> --issuer <URL> --client-id <ID> \
                     --client-secret <secret> --redirect-uri <URI> [--person <subject>=<email>]...";

#[derive(Default)]
struct Args {
    listen: Option<SocketAddr>,
    issuer: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    redirect_uri: Option<String>,
    people: Vec<Person>,
}

fn parse(mut words: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    while let Some(flag) = words.next() {
        let value = words
            .next()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--listen" => {
                let listen = value
                    .parse()
                    .map_err(|_| format!("not an address: {value}"))?;
                args.listen = Some(listen);
            }
            "--issuer" => args.issuer = Some(value),
            "--client-id" => args.client_id = Some(value),
            "--client-secret" => args.client_secret = Some(value),
            "--redirect-uri" => args.redirect_uri = Some(value),
            "--person" => {
                let (subject, email) = value
                    .split_once('=')
                    .ok_or_else(|| format!("not <subject>=<email>: {value}"))?;
                args.people.push(Person::new(subject, email));
            }
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("skald-oidc-stand-in: {err}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (Some(listen), Some(issuer), Some(client_id), Some(client_secret), Some(redirect_uri)) = (
        args.listen,
        args.issuer,
        args.client_id,
        args.client_secret,
        args.redirect_uri,
    ) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let provider = match Provider::serve(listen, &issuer, &[Key::rsa()]) {
        Ok(provider) => provider,
        Err(err) => {
            eprintln!("skald-oidc-stand-in: cannot listen on {listen}: {err}");
            return ExitCode::FAILURE;
        }
    };
    provider.register(Client {
        client_id,
        client_secret,
        redirect_uri,
    });
    for person in args.people {
        provider.put_person(person);
    }
    eprintln!("skald-oidc-stand-in: listening on http://{listen}");
    loop {
        thread::park();
    }
}
