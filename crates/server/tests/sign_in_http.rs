//! GitHub sign-in as plain HTTP shows it: the statuses of the sign-in's answers, its cookies'
//! attributes, the server's own expiry of `state`, its requests to GitHub and its settings; and, in
//! headless Chrome, the request for the return page, which carries the session cookie.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use common::browser::Browser;
use common::sign_in::{CALLBACK, CLIENT_ID, CLIENT_SECRET, START, SignIn, github_settings};
use common::{Response, SERVER, Server, TestDb, free_port, get_url};
use skald_github_stand_in::{App, Endpoint, Fault, GitHub, User};

/// A client secret other than the one the stand-in's app has.
const WRONG_SECRET: &str = "standin-secret-0000000000000000000000000000wrong";

/// Runs a sign-in of `user` on `stand` by plain HTTP, straight to the server whatever its public
/// address, up to GitHub's redirect back: the start's answer, the `Cookie` header of the browser that
/// started it, with another cookie of the site before its own, and the callback URL on the server.
fn to_callback(stand: &SignIn, user: &User) -> (Response, String, String) {
    let start = stand.server.get(START);
    assert_eq!(start.status, 302, "GET {START}");
    let state = start.header("set-cookie").expect("a state cookie");
    let jar = format!(
        "theme=dark; {}",
        state.split(';').next().unwrap_or_default()
    );
    let signed_in = get_url(&stand.github.sign_in_url(user.id), &[]);
    let github = signed_in.header("set-cookie").expect("a stand-in cookie");
    let github = github.split(';').next().unwrap_or_default();
    let authorize = start.header("location").expect("a redirect to GitHub");
    let back = get_url(authorize, &[("cookie", github)]);
    let callback = back.header("location").expect("GitHub goes back");
    let callback = callback.replacen(&stand.public, stand.server.base(), 1);
    (start, jar, callback)
}

/// The rows of `stand`'s `accounts` table.
fn accounts(stand: &SignIn) -> Vec<String> {
    let mut rows = stand.db.rows();
    rows.retain(|row| row.starts_with("accounts: "));
    rows
}

/// The attributes of the `Set-Cookie` value `cookie`, its name and value first.
fn attributes(cookie: &str) -> Vec<&str> {
    cookie.split(';').map(str::trim).collect()
}

/// The `Set-Cookie` value of `response` for the cookie `name`.
fn set_cookie<'a>(response: &'a Response, name: &str) -> &'a str {
    response
        .header_values("set-cookie")
        .into_iter()
        .find(|cookie| cookie.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no cookie {name}: {:?}", response.headers))
}

#[test]
fn sign_in_cookies_have_their_attributes() {
    let browser = Browser::launch();
    for (stand, secure) in [
        (SignIn::start(), false),
        (SignIn::start_https(&browser), true),
    ] {
        let scheme = &stand.public;
        let ada = stand.user(1_402_000_001, "ada-attributes-standin");
        let (start, jar, callback) = to_callback(&stand, &ada);

        let state = attributes(set_cookie(&start, "skald_sign_in"));
        for attribute in [
            "HttpOnly",
            "SameSite=Lax",
            "Path=/auth/github/callback",
            "Max-Age=600",
        ] {
            assert!(state.contains(&attribute), "{scheme}: {state:?}");
        }
        assert_eq!(state.contains(&"Secure"), secure, "{scheme}: {state:?}");
        assert_eq!(start.header("cache-control"), Some("no-store"));

        let done = get_url(&callback, &[("cookie", &jar)]);
        assert_eq!(done.status, 200, "{scheme}");
        let session = attributes(set_cookie(&done, "skald_session"));
        for attribute in ["HttpOnly", "SameSite=Strict", "Path=/"] {
            assert!(session.contains(&attribute), "{scheme}: {session:?}");
        }
        assert_eq!(session.contains(&"Secure"), secure, "{scheme}: {session:?}");
        let cleared = attributes(set_cookie(&done, "skald_sign_in"));
        assert_eq!(cleared[0], "skald_sign_in=", "{scheme}: {cleared:?}");
        assert!(cleared.contains(&"Max-Age=0"), "{scheme}: {cleared:?}");
        let page = String::from_utf8_lossy(&done.body);
        assert!(
            page.contains(&format!("content=\"0;url={}/\"", stand.public)),
            "{scheme}: {page}"
        );
    }
}

#[test]
fn refused_and_failed_sign_ins_answer_their_status() {
    let stand = SignIn::start_local();
    let ada = stand.user(1_402_000_002, "ada-status-standin");

    let (_, jar, callback) = to_callback(&stand, &ada);
    let refused = [
        (format!("{}{CALLBACK}", stand.server.base()), jar.clone()),
        (callback.clone(), "theme=dark".to_owned()),
        (
            callback.clone(),
            "skald_sign_in=0123456789abcdef".to_owned(),
        ),
    ];
    for (url, cookie) in &refused {
        let answer = get_url(url, &[("cookie", cookie)]);
        assert_eq!(answer.status, 400, "{url} with {cookie}");
        assert_eq!(answer.header("cache-control"), Some("no-store"));
    }

    for (fault, status) in [
        (Fault::AccessDenied, 400),
        (Fault::BadVerificationCode, 400),
        (Fault::TokenStatus { status: 500 }, 502),
        (Fault::UserStatus { status: 401 }, 502),
    ] {
        stand.github.set_faults(&[fault]);
        let (_, jar, callback) = to_callback(&stand, &ada);
        let answer = get_url(&callback, &[("cookie", &jar)]);
        assert_eq!(answer.status, status, "{fault:?}");
        assert_eq!(answer.header_values("set-cookie"), Vec::<&str>::new());
    }
    assert_eq!(accounts(&stand), Vec::<String>::new());

    let refused = SignIn::start_with(&[], WRONG_SECRET);
    let bob = refused.user(1_402_000_003, "bob-status-standin");
    let (_, jar, callback) = to_callback(&refused, &bob);
    assert_eq!(get_url(&callback, &[("cookie", &jar)]).status, 502);
}

#[test]
fn the_server_expires_state_though_the_browser_keeps_its_cookie() {
    let stand = SignIn::start_with(&["--sign-in-state-lifetime", "1"], CLIENT_SECRET);
    let ada = stand.user(1_402_000_004, "ada-expiry-standin");
    let (start, jar, callback) = to_callback(&stand, &ada);
    assert!(attributes(set_cookie(&start, "skald_sign_in")).contains(&"Max-Age=1"));

    thread::sleep(Duration::from_millis(2500));
    assert_eq!(get_url(&callback, &[("cookie", &jar)]).status, 400);
    assert_eq!(accounts(&stand), Vec::<String>::new());
}

#[test]
fn requests_to_github_name_the_server_and_the_callback() {
    let stand = SignIn::start_local();
    let ada = stand.user(1_402_000_005, "ada-requests-standin");
    let (_, jar, callback) = to_callback(&stand, &ada);
    assert_eq!(get_url(&callback, &[("cookie", &jar)]).status, 200);

    let token = &stand.github.requests_to(Endpoint::Token)[0];
    let redirect_uri = format!("{}{CALLBACK}", stand.public);
    assert_eq!(token.params.get("redirect_uri"), Some(&redirect_uri));
    for request in [token, &stand.github.requests_to(Endpoint::User)[0]] {
        let agent = request.headers.get("user-agent").map(String::as_str);
        assert!(
            agent.is_some_and(|agent| agent.starts_with("skald-server/")),
            "{request:?}"
        );
    }
}

#[test]
fn github_settings_that_do_not_fit_together_stop_the_start() {
    // The settings are checked before the database is reached.
    let database = "postgres://skald@127.0.0.1:1/skald";
    let public = "http://127.0.0.1:8080";
    for (args, secret, names) in [
        (
            vec!["--public-url", public, "--github-client-id", CLIENT_ID],
            None,
            "SKALD_GITHUB_CLIENT_SECRET",
        ),
        (
            vec!["--public-url", public],
            Some(CLIENT_SECRET),
            "--github-client-id",
        ),
        (
            vec!["--github-client-id", CLIENT_ID],
            Some(CLIENT_SECRET),
            "--public-url",
        ),
        (
            vec!["--public-url", "https://skald.example/skald"],
            None,
            "--public-url",
        ),
        (vec!["--github-url", "github.com"], None, "--github-url"),
        (
            vec!["--github-api-url", "github.com"],
            None,
            "--github-api-url",
        ),
        (
            vec!["--sign-in-state-lifetime", "0"],
            None,
            "--sign-in-state-lifetime",
        ),
    ] {
        let mut command = Command::new(SERVER);
        command
            .args(["--listen", "127.0.0.1:0", "--database-url", database])
            .args(&args)
            .env_remove("SKALD_GITHUB_CLIENT_SECRET");
        if let Some(secret) = secret {
            command.env("SKALD_GITHUB_CLIENT_SECRET", secret);
        }
        let output = command.output().expect("run skald-server");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{args:?}");
        assert!(stderr.contains(names), "{args:?}: {stderr}");
        assert!(!stderr.contains(CLIENT_SECRET), "{args:?}: {stderr}");
    }
}

#[test]
fn the_return_page_request_carries_the_session_cookie() {
    let browser = Browser::launch();
    let port = free_port();
    let recorder = Recorder::start(port);
    let public = format!("http://skald.test:{}", recorder.port);
    let db = TestDb::create();
    // The stand-in's app takes any callback: this one is the proxy's.
    let github = GitHub::start(App {
        client_id: CLIENT_ID.to_owned(),
        client_secret: CLIENT_SECRET.to_owned(),
        callback: None,
    });
    let _server = Server::launch(
        &db,
        &format!("127.0.0.1:{port}"),
        &github_settings(&public, &github),
        &[("SKALD_GITHUB_CLIENT_SECRET", CLIENT_SECRET)],
    );
    let ada = User::new(1_402_000_006, "ada-landing-standin");
    github.put_user(ada.clone());

    // The person authorizes the app on GitHub's page, so the way back starts on another site.
    let page = browser.page();
    page.goto(&github.asking_sign_in_url(ada.id));
    let visit = page.goto(&format!("{public}{START}?return_to=%2Flanding%3Ftab%3D1"));
    let authorize = format!("{}/login/oauth/authorize?", github.url());
    assert!(visit.url.starts_with(&authorize), "at GitHub: {visit:?}");
    page.evaluate(r#"document.getElementById("authorize").click()"#);

    let landing = recorder.wait_for("/landing?tab=1");
    let carried = landing.1.as_deref().unwrap_or_default();
    assert!(
        carried
            .split("; ")
            .any(|pair| pair.starts_with("skald_session=")),
        "{:?}",
        recorder.hits()
    );
}

/// A request's path and `Cookie` header.
type Hit = (String, Option<String>);

/// A plain HTTP proxy in front of the server on a local port that records each request's path and
/// `Cookie` header. It closes each connection after one exchange, so it sees every request.
struct Recorder {
    port: u16,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Recorder {
    fn start(target: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("local address").port();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&hits);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let recorded = Arc::clone(&recorded);
                thread::spawn(move || forward(client, target, &recorded));
            }
        });
        Recorder { port, hits }
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().expect("hits").clone()
    }

    /// Waits up to 10 s for a request for `path`; gives it.
    fn wait_for(&self, path: &str) -> Hit {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(hit) = self.hits().into_iter().find(|(at, _)| at == path) {
                return hit;
            }
            assert!(
                Instant::now() < deadline,
                "no request for {path}: {:?}",
                self.hits()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Forwards one request without a body from `client` to the server on `target` with
/// `Connection: close`, records it in `hits`, and copies the answer back.
fn forward(client: TcpStream, target: u16, hits: &Mutex<Vec<Hit>>) {
    let mut reader = BufReader::new(client.try_clone().expect("clone the stream"));
    let mut head = Vec::new();
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|read| read > 0) && line != "\r\n" {
        head.push(line.trim_end().to_owned());
        line.clear();
    }
    let Some(request_line) = head.first() else {
        return;
    };
    let path = request_line
        .split(' ')
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let header = |name: &str| {
        head.iter().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    };
    hits.lock().expect("hits").push((path, header("cookie")));

    let mut upstream = TcpStream::connect(("127.0.0.1", target)).expect("reach the server");
    let mut forwarded: Vec<_> = head
        .into_iter()
        .filter(|line| {
            !line
                .split(':')
                .next()
                .unwrap_or_default()
                .eq_ignore_ascii_case("connection")
        })
        .collect();
    forwarded.push("Connection: close".to_owned());
    let request = format!("{}\r\n\r\n", forwarded.join("\r\n"));
    upstream
        .write_all(request.as_bytes())
        .expect("send the request");
    let mut answer = Vec::new();
    let _ = upstream.read_to_end(&mut answer);
    let mut client = client;
    let _ = client.write_all(&answer);
}
