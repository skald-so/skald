//! The checks of a server's sign-in, `script/check-github-sign-in` and `script/check-google-sign-in`,
//! against a local `skald-server` whose web address, the setting `--web-url`, is apart from its
//! public address, with the GitHub and OIDC stand-ins. Two fronts forward to the server: one is its
//! public address, the other its web address; the latter can change the server's answers as a
//! misconfigured server would.
//!
//! Each check takes the web address as an optional argument:
//! `script/check-<provider>-sign-in <server address> <client ID> [<web address>]`. It asks for
//! device authorization, and GitHub's check polls, at the server address, the public one; it
//! expects the verification page and the provider's callback on the web address. Without the web
//! address, all of them are on the server address, as the checks' other tests show. A verification
//! page or callback off the web address fails the step `verification page`; a verification page off
//! it is named in the reason.

mod common;

use skald_oidc_stand_in::{Client, Endpoint};

use common::check::{
    Front, Line, Run, Tamper, assert_no_codes, run_script, set_param, to_provider, unchanged,
};
use common::device_flow::{DEVICE_AUTHORIZATION, INTERVAL, TOKEN};
use common::google::{self, Google, Setup};
use common::sign_in::{self, SignIn, param};

/// The web address's setting.
const WEB: &str = "--web-url";
/// The verification page.
const PAGE: &str = "/device";
/// The step that checks the verification page.
const PAGE_STEP: &str = "verification page";
/// The parameters of a sign-in start's redirect a check must not print.
const HELD_BACK: [&str; 3] = ["state", "nonce", "code_challenge"];

/// The provider a check is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    GitHub,
    Google,
}

impl Provider {
    const ALL: [Provider; 2] = [Provider::GitHub, Provider::Google];

    /// The check's name: its file in `script/` and the prefix its lines may start with.
    fn check(self) -> &'static str {
        match self {
            Provider::GitHub => "check-github-sign-in",
            Provider::Google => "check-google-sign-in",
        }
    }

    fn start(self) -> &'static str {
        match self {
            Provider::GitHub => sign_in::START,
            Provider::Google => google::START,
        }
    }

    fn callback(self) -> &'static str {
        match self {
            Provider::GitHub => sign_in::CALLBACK,
            Provider::Google => google::CALLBACK,
        }
    }

    fn client_id(self) -> &'static str {
        match self {
            Provider::GitHub => sign_in::CLIENT_ID,
            Provider::Google => google::CLIENT_ID,
        }
    }
}

/// The server and its provider's stand-in.
enum Served {
    GitHub(SignIn),
    Google(Google),
}

/// A server with one provider's sign-in behind two fronts: `api`, its public address, and `app`,
/// its web address if it has the setting.
struct Stage {
    provider: Provider,
    served: Served,
    api: Front,
    app: Front,
    /// The variable that gives the check the provider's address, and its value.
    provider_env: (&'static str, String),
}

impl Stage {
    /// A server with `provider`'s sign-in whose web address is the `app` front's if `web`, and none
    /// otherwise. A Google client's redirect URI is the callback on the server's web address.
    fn start(provider: Provider, web: bool) -> Self {
        let mut api = Front::bind();
        let mut app = Front::bind();
        let setting = [WEB, app.url.as_str()];
        let setting: &[&str] = if web { &setting } else { &[] };
        let (served, provider_env) = match provider {
            Provider::GitHub => {
                let mut args = vec![INTERVAL, "1"];
                args.extend_from_slice(setting);
                let stand = SignIn::behind(&api.url, &args);
                let env = ("GITHUB_URL", stand.github.url().to_owned());
                (Served::GitHub(stand), env)
            }
            Provider::Google => {
                let server = Google::launch(Setup {
                    public: Some(&api.url),
                    args: setting,
                    ..Setup::default()
                });
                let web_address = if web { &app.url } else { &api.url };
                server.provider.register(Client {
                    client_id: google::CLIENT_ID.to_owned(),
                    client_secret: google::CLIENT_SECRET.to_owned(),
                    redirect_uri: format!("{web_address}{}", google::CALLBACK),
                });
                let env = ("GOOGLE_ISSUER", server.provider.issuer().to_owned());
                (Served::Google(server), env)
            }
        };
        let base = match &served {
            Served::GitHub(stand) => stand.server.base().to_owned(),
            Served::Google(server) => server.server.base().to_owned(),
        };
        api.serve(&base);
        app.serve(&base);
        Stage {
            provider,
            served,
            api,
            app,
            provider_env,
        }
    }

    /// Runs the check with the server's public address, and `web` as the web address if given, the
    /// answers at the web front changed by `tamper`; checks that it printed no code it got.
    fn run(&self, case: &str, web: Option<&str>, tamper: Tamper) -> Run {
        self.app.set_tamper(tamper);
        let mut args = vec![self.api.url.as_str(), self.provider.client_id()];
        args.extend(web);
        let script = format!(
            "{}/../../script/{}",
            env!("CARGO_MANIFEST_DIR"),
            self.provider.check()
        );
        let (name, value) = &self.provider_env;
        let case = format!("{}: {case}", self.provider.check());
        let run = run_script(&case, &script, &args, &[(*name, value.as_str())]);
        self.api.assert_tamper_held();
        self.app.assert_tamper_held();
        assert_no_codes(&run, &self.codes());
        run
    }

    /// The codes the check must not print: the device and user codes the server gave, and the
    /// secrets of each sign-in it started.
    fn codes(&self) -> Vec<String> {
        let mut codes = self.api.codes();
        codes.extend(self.app.codes());
        for exchange in self.api.exchanges().into_iter().chain(self.app.exchanges()) {
            if exchange.path != self.provider.start() {
                continue;
            }
            let location = exchange.answer.header("location").unwrap_or_default();
            for name in HELD_BACK {
                codes.extend(param(location, name).filter(|value| value.len() >= 8));
            }
        }
        codes
    }

    /// The requests `front` forwarded, each as `<method> <path>`.
    fn requests(front: &Front) -> Vec<String> {
        let exchanges = front.exchanges().into_iter();
        exchanges
            .map(|exchange| format!("{} {}", exchange.method, exchange.path))
            .collect()
    }
}

/// The lines of `run`'s output, each read as a [`Line`]; fails on any other line.
fn lines(run: &Run, provider: Provider) -> Vec<Line> {
    run.read_lines(&format!("{}: ", provider.check()))
}

/// Checks that `run` passed: every step it ran, `verification page` among them, then the result.
fn assert_passes(run: &Run, provider: Provider) {
    let lines = lines(run, provider);
    let page = Line::Step(PAGE_STEP.to_owned(), true);
    let passed = lines
        .iter()
        .filter(|line| matches!(line, Line::Step(_, true)));
    let passes = run.status.success()
        && lines.last() == Some(&Line::Result(true))
        && passed.count() + 1 == lines.len()
        && lines.contains(&page);
    assert!(passes, "{}: expected to pass\n{}", run.case, run.output);
}

/// Checks that `run` failed at `verification page`, after every step before it passed; gives the
/// reason it printed.
fn assert_fails_at_the_page(run: &Run, provider: Provider) -> String {
    let lines = lines(run, provider);
    let end = [Line::Step(PAGE_STEP.to_owned(), false), Line::Result(false)];
    let before = &lines[..lines.len().saturating_sub(end.len())];
    let fails = !run.status.success()
        && lines.ends_with(&end)
        && before
            .iter()
            .all(|line| matches!(line, Line::Step(_, true)));
    assert!(
        fails,
        "{}: expected to fail at {PAGE_STEP:?}\n{}",
        run.case, run.output
    );
    let failure = format!("{PAGE_STEP}: fail");
    let prefix = format!("{}: ", provider.check());
    let reason = run.output.lines().map(str::trim).find_map(|line| {
        let line = line.strip_prefix(&prefix).unwrap_or(line);
        line.strip_prefix(&failure)
    });
    reason.unwrap_or_default().to_owned()
}

/// Whether `text` names the address `url`, an `http://<host>:<port>`, not a longer port.
fn names(text: &str, url: &str) -> bool {
    text.match_indices(url).any(|(at, _)| {
        let after = text[at + url.len()..].chars().next();
        !after.is_some_and(|c| c.is_ascii_digit())
    })
}

/// With both addresses, each check passes against a server with the web address: it asks for
/// device authorization at the public address, and finds the verification page and the callback on
/// the web address.
#[test]
fn checks_pass_with_both_addresses_of_a_server_with_a_web_address() {
    for provider in Provider::ALL {
        let stage = Stage::start(provider, true);

        let run = stage.run("both addresses", Some(&stage.app.url), unchanged());

        assert_passes(&run, provider);
        let (api, app) = (Stage::requests(&stage.api), Stage::requests(&stage.app));
        let authorization = format!("POST {DEVICE_AUTHORIZATION}");
        let page = format!("GET {PAGE}");
        let start = format!("GET {}", provider.start());
        assert!(api.contains(&authorization), "{provider:?}: {api:?}");
        for request in [&page, &start] {
            assert!(app.contains(request), "{provider:?}: {app:?}");
            assert!(!api.contains(request), "{provider:?}: {api:?}");
        }
        assert!(!app.contains(&authorization), "{provider:?}: {app:?}");
        let callback = format!("{}{}", stage.app.url, provider.callback());
        match &stage.served {
            Served::GitHub(_) => {
                let poll = format!("POST {TOKEN}");
                assert!(api.contains(&poll), "{api:?}");
                assert!(!app.contains(&poll), "{app:?}");
            }
            Served::Google(server) => {
                let asked = server.provider.requests_to(Endpoint::Authorize);
                let last = asked
                    .last()
                    .expect("the check followed the authorization URL");
                assert_eq!(last.params["redirect_uri"], callback);
            }
        }
    }
}

/// With only the public address, each check fails at the verification page of a server with the
/// web address, and names the page's address.
#[test]
fn checks_fail_at_the_verification_page_with_only_the_public_address() {
    for provider in Provider::ALL {
        let stage = Stage::start(provider, true);

        let run = stage.run("only the public address", None, unchanged());

        let reason = assert_fails_at_the_page(&run, provider);
        assert!(
            names(&reason, &stage.app.url),
            "{}: the reason names the page's address {}\n{}",
            run.case,
            stage.app.url,
            run.output
        );
    }
}

/// With both addresses, each check fails at the verification page when the sign-in start sends a
/// callback on the public address.
#[test]
fn checks_fail_at_the_verification_page_with_a_callback_on_the_public_address() {
    for provider in Provider::ALL {
        let stage = Stage::start(provider, true);
        let callback = format!("{}{}", stage.api.url, provider.callback());
        let tamper = to_provider(provider.start(), move |url| {
            set_param(url, "redirect_uri", Some(&callback));
        });

        let run = stage.run(
            "a callback on the public address",
            Some(&stage.app.url),
            tamper,
        );

        assert_fails_at_the_page(&run, provider);
    }
}

/// With both addresses, each check fails at the verification page of a server without the web
/// address, whose page is on its public address.
#[test]
fn checks_fail_at_the_verification_page_of_a_server_without_a_web_address() {
    for provider in Provider::ALL {
        let stage = Stage::start(provider, false);

        let run = stage.run(
            "a server without the setting",
            Some(&stage.app.url),
            unchanged(),
        );

        assert_fails_at_the_page(&run, provider);
    }
}
