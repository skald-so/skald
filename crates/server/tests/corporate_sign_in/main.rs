//! Corporate browser sign-in against local OIDC fixtures, through the server's settings and HTTP
//! API. No test uses a real identity provider.

#[path = "../common/mod.rs"]
mod common;

mod client_auth;
mod flow;
mod settings;
mod web;

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;
use skald_github_stand_in::GitHub;
use skald_oidc_stand_in::{Client, Person, Provider};

use common::browser::{Page, Visit};
use common::device_flow::{json, text};
use common::sign_in::{self, start_url};
use common::spec::{spec, spec_violations};
use common::{Response, Server, TestDb, on_free_port};

const START: &str = "/auth/corporate";
const CALLBACK: &str = "/auth/corporate/callback";
const ISSUER_FLAG: &str = "--corporate-issuer";
const CLIENT_FLAG: &str = "--corporate-client-id";
const SECRET_ENV: &str = "SKALD_CORPORATE_CLIENT_SECRET";
const CLIENT_ID: &str = "skald+stand-in:corporate";
const CLIENT_SECRET: &str = "stand-in-corporate-secret-9baf0637f00e+%&:";
const PROVIDERS: &str = "/auth/providers";
const SESSION: &str = "/session";
const SESSIONS: &str = "/account/sessions";
const METHODS: &str = "/account/sign-in-methods";

struct Site {
    server: Server,
    web: String,
    github: Option<GitHub>,
}

impl Site {
    fn start(db: &TestDb, corporate: &Provider, google: Option<&Provider>, github: bool) -> Self {
        Self::with_web(db, corporate, google, github, None)
    }

    fn with_web(
        db: &TestDb,
        corporate: &Provider,
        google: Option<&Provider>,
        github: bool,
        web: Option<&str>,
    ) -> Self {
        on_free_port(Duration::from_secs(30), |port| {
            let web = web.map_or_else(|| format!("http://skald.test:{port}"), str::to_owned);
            corporate.register(Client {
                client_id: CLIENT_ID.to_owned(),
                client_secret: CLIENT_SECRET.to_owned(),
                redirect_uri: format!("{web}{CALLBACK}"),
            });
            let github = github.then(|| sign_in::github_stand_in(&web));
            let (mut args, mut env) = common::google::settings(
                "http://127.0.0.1:1",
                google.map(|provider| (provider.issuer(), common::google::CLIENT_SECRET)),
                github.as_ref(),
            );
            if let Some(google) = google {
                google.register(Client {
                    client_id: common::google::CLIENT_ID.to_owned(),
                    client_secret: common::google::CLIENT_SECRET.to_owned(),
                    redirect_uri: format!("{web}{}", common::google::CALLBACK),
                });
            }
            args.extend([
                "--web-url",
                &web,
                "--sign-up",
                "open",
                ISSUER_FLAG,
                corporate.issuer(),
                CLIENT_FLAG,
                CLIENT_ID,
            ]);
            env.push((SECRET_ENV, CLIENT_SECRET));
            let server = Server::try_launch(db, &format!("127.0.0.1:{port}"), &args, &env)?;
            Ok(Self {
                server,
                web,
                github,
            })
        })
    }

    fn sign_in(&self, page: &Page, provider: &Provider, person: &Person, start: &str) -> Visit {
        provider.put_person(person.clone());
        let visit = page.goto(&provider.sign_in_url(&person.subject));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
        page.goto_settled(&start_url(&self.web, start, Some("/health")))
    }
}

#[track_caller]
fn assert_conforms(path: &str, response: &Response) {
    let violations = spec_violations(&spec(), path, response);
    assert!(violations.is_empty(), "{violations:?}");
}

#[track_caller]
fn api(server: &Server, path: &str, cookie: &str) -> Value {
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let response = server.get_with(
        path,
        &[(skald_proto::VERSION_HEADER, &version), ("cookie", cookie)],
    );
    assert_eq!(response.status, 200, "GET {path}: {}", text(&response));
    assert_conforms(path, &response);
    json(&response)
}

#[track_caller]
fn assert_providers(server: &Server, expected: &[&str]) {
    let body = api(server, PROVIDERS, "");
    let providers = body["providers"].as_array().expect("the provider list");
    let names: BTreeSet<_> = providers
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    assert_eq!(providers.len(), expected.len(), "{body}");
    assert_eq!(names, expected.iter().copied().collect(), "{body}");
    assert_eq!(body["sign_up"], "open", "{body}");
}
