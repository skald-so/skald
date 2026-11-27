//! Starts the gateway handler on loopback with the integration fixture's secrets and task records.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write as _};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::ExitCode;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rustls::pki_types::{CertificateDer, pem::PemObject as _};
use serde::Deserialize;
use skald_crypto::SigningKey;
use skald_proto::{ChannelId, QueueId, SpaceId};
use skald_server::PublicUrl;
use skald_server::a2a::{Binding, Config, Correlation, Credential, Destination, ServiceSecret};
use ureq::tls::Certificate;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    public_origin: String,
    skald_ca_file: PathBuf,
    routes: HashMap<String, Route>,
    credentials: Vec<Client>,
    tasks: Vec<Task>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Route {
    skald_origin: String,
    space: String,
    channel: String,
    queue: String,
    service_token: Option<String>,
    signing_key: Option<String>,
}

impl Route {
    fn binding(self) -> Option<Binding> {
        let secret = match (self.service_token, self.signing_key) {
            (Some(token), Some(key)) => {
                let key = Zeroizing::new(key);
                let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(key.as_bytes()).ok()?);
                Some(ServiceSecret::new(
                    token,
                    SigningKey::from_bytes(bytes.as_slice().try_into().ok()?),
                ))
            }
            _ => None,
        };
        let skald_origin: PublicUrl = self.skald_origin.parse().ok()?;
        // The example can contact only the test's TLS proxy.
        let url = url::Url::parse(&self.skald_origin).ok()?;
        if url.host() != Some(url::Host::Ipv4(Ipv4Addr::LOCALHOST)) {
            return None;
        }
        Some(Binding {
            skald_origin,
            space: self.space.parse::<SpaceId>().ok()?,
            channel: self.channel.parse::<ChannelId>().ok()?,
            destination: Destination::Queue(self.queue.parse::<QueueId>().ok()?),
            secret,
        })
    }
}

#[derive(Deserialize)]
struct Client {
    client: String,
    bearer: String,
    routes: HashSet<String>,
    revoked: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    id: String,
    context_id: String,
    client: String,
    route: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    if serve().await.is_some() {
        ExitCode::SUCCESS
    } else {
        eprintln!("Invalid A2A fixture configuration or listener.");
        ExitCode::FAILURE
    }
}

async fn serve() -> Option<()> {
    let mut input = Zeroizing::new(Vec::new());
    io::stdin().read_to_end(&mut input).ok()?;
    let Fixture {
        public_origin,
        skald_ca_file,
        routes,
        credentials,
        tasks,
    } = serde_json::from_slice(&input).ok()?;
    drop(input);
    let roots = CertificateDer::pem_file_iter(skald_ca_file)
        .ok()?
        .map(|cert| {
            cert.ok()
                .map(|cert| Certificate::from_der(cert.as_ref()).to_owned())
        })
        .collect::<Option<Vec<_>>>()?;
    let routes = routes
        .into_iter()
        .map(|(alias, route)| Some((alias, route.binding()?)))
        .collect::<Option<_>>()?;
    let credentials = credentials
        .into_iter()
        .map(|client| {
            let bearer = Zeroizing::new(client.bearer);
            Credential::new(client.client, &bearer, client.routes, client.revoked)
        })
        .collect::<Option<_>>()?;
    let tasks = tasks
        .into_iter()
        .map(|task| {
            (
                task.id,
                Correlation {
                    context_id: task.context_id,
                    client: task.client,
                    route: task.route,
                },
            )
        })
        .collect();
    let router = Config {
        public_origin: public_origin.parse().ok()?,
        advertised_route: "primary".to_owned(),
        routes,
        credentials,
        tasks,
    }
    .router(Some(roots))?;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .ok()?;
    writeln!(io::stdout(), "http://{}", listener.local_addr().ok()?).ok()?;
    axum::serve(listener, router).await.ok()
}
