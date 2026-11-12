//! GitHub sign-in in the browser: `skald-server` against the GitHub stand-in, with the person's part
//! in headless Chrome, so cookies act as in a browser. Responses are checked against the OpenAPI
//! spec in `api/openapi.json`.

mod common;

use std::process::Command;
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use common::browser::{Browser, Cookie, Page};
use common::sign_in::{Account, CALLBACK, CLIENT_ID, CLIENT_SECRET, SESSION, START, SignIn};
use common::spec::{spec, spec_violations};
use common::{Response, SERVER, Server, TestDb, get_url};
use skald_github_stand_in::{Endpoint, Fault, User};

/// The host of the server's public address in the browser tests.
const HOST: &str = "skald.test";
/// A client secret other than the one the stand-in's app has.
const WRONG_SECRET: &str = "standin-secret-0000000000000000000000000000wrong";

#[test]
fn first_sign_in_creates_account_and_next_reaches_it() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(4_710_293_856, "ada-standin");

    let first = browser.page();
    stand.assert_signed_out(&first);
    stand.sign_in(&first, &ada);
    let account = stand.account(&first);
    assert_eq!(account.github_login, ada.login);
    assert!(
        !stand.rows_of(&ada).is_empty(),
        "the database holds the account: {:?}",
        stand.db.rows()
    );

    // Another browser of the same person reaches the same account.
    let next = browser.page();
    stand.sign_in(&next, &ada);
    assert_eq!(stand.account(&next), account);
    assert_eq!(stand.account(&first), account);
}

#[test]
fn sign_in_asks_github_for_no_scope_and_returns_to_public_address() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(4_710_293_857, "ada-scope-standin");
    for _ in 0..2 {
        let page = browser.page();
        stand.sign_in(&page, &ada);
        stand.account(&page);
    }

    let authorize = stand.github.requests_to(Endpoint::Authorize);
    assert_eq!(authorize.len(), 2, "{authorize:?}");
    let callback = format!("{}{CALLBACK}", stand.public);
    for request in &authorize {
        let param = |name: &str| request.params.get(name).map(String::as_str);
        assert_eq!(param("client_id"), Some(CLIENT_ID), "{request:?}");
        assert_eq!(
            param("redirect_uri"),
            Some(callback.as_str()),
            "{request:?}"
        );
        assert!(
            param("scope").is_none_or(str::is_empty),
            "no scope: {request:?}"
        );
        assert!(param("state").is_some_and(|state| !state.is_empty()));
    }
    assert_ne!(
        authorize[0].params["state"], authorize[1].params["state"],
        "each sign-in has a state of its own"
    );
    // The stand-in gives a token only for the app's client ID and secret.
    assert_eq!(stand.github.tokens().len(), 2);
}

#[test]
fn login_or_email_change_keeps_the_account() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let grace = User {
        email: Some("grace@example.test".to_owned()),
        ..User::new(5_820_174_663, "grace-standin")
    };
    stand.github.put_user(grace.clone());
    let first = browser.page();
    stand.sign_in(&first, &grace);
    let account = stand.account(&first);

    let renamed = User {
        login: "grace-renamed-standin".to_owned(),
        email: Some("grace.new@example.test".to_owned()),
        ..grace.clone()
    };
    stand.github.put_user(renamed.clone());
    let next = browser.page();
    stand.sign_in(&next, &renamed);
    let updated = Account {
        id: account.id.clone(),
        github_login: renamed.login.clone(),
    };
    assert_eq!(stand.account(&next), updated);
    assert_eq!(
        stand.account(&first),
        updated,
        "the account shows the new login"
    );

    // Another GitHub user who takes the old login and email gets an account of their own.
    let newcomer = User {
        id: 5_820_174_664,
        ..grace.clone()
    };
    stand.github.put_user(newcomer.clone());
    let theirs = browser.page();
    stand.sign_in(&theirs, &newcomer);
    let other = stand.account(&theirs);
    assert_ne!(other.id, account.id);
    assert_eq!(other.github_login, grace.login);
    assert_eq!(stand.account(&next), updated);
}

#[test]
fn two_github_users_get_two_accounts() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(6_301_552_817, "ada-two-standin");
    let bob = stand.user(6_301_552_818, "bob-two-standin");

    let ada_page = browser.page();
    stand.sign_in(&ada_page, &ada);
    let bob_page = browser.page();
    stand.sign_in(&bob_page, &bob);

    let ada_account = stand.account(&ada_page);
    let bob_account = stand.account(&bob_page);
    assert_ne!(ada_account.id, bob_account.id);
    assert_eq!(ada_account.github_login, ada.login);
    assert_eq!(bob_account.github_login, bob.login);
}

#[test]
fn callback_without_state_signs_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(7_104_839_221, "ada-nostate-standin");
    let page = browser.page();
    let callback = stand.held_callback(&page, &ada);
    let jar = page.cookies();

    for without in [
        with_param(&callback, "state", None),
        with_param(&callback, "state", Some("")),
    ] {
        page.add_cookies(&jar);
        page.goto(&without);
        stand.assert_signed_out(&page);
    }
    assert_eq!(stand.rows_of(&ada), Vec::<String>::new());
}

#[test]
fn callback_with_wrong_state_signs_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(7_104_839_222, "ada-wrongstate-standin");
    let page = browser.page();
    let callback = stand.held_callback(&page, &ada);
    let state = param(&callback, "state").expect("the callback has a state");
    let jar = page.cookies();

    let mut flipped = state.clone().into_bytes();
    let last = flipped.last_mut().expect("a state is not empty");
    *last = if *last == b'A' { b'B' } else { b'A' };
    let flipped = String::from_utf8(flipped).expect("ASCII");
    for wrong in [
        format!("{state}A"),
        flipped,
        state[..state.len() / 2].to_owned(),
        "0123456789abcdef0123456789abcdef".to_owned(),
    ] {
        page.add_cookies(&jar);
        page.goto(&with_param(&callback, "state", Some(&wrong)));
        stand.assert_signed_out(&page);
    }
    assert_eq!(stand.rows_of(&ada), Vec::<String>::new());
}

#[test]
fn state_works_only_in_its_own_browser() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(7_104_839_223, "ada-otherbrowser-standin");
    let callback = stand.held_callback(&browser.page(), &ada);

    // A browser that never started a sign-in.
    let fresh = browser.page();
    fresh.goto(&callback);
    stand.assert_signed_out(&fresh);

    // A browser that started a sign-in of its own, as a person lured to someone else's callback.
    let lured = browser.page();
    stand.held_callback(&lured, &ada);
    lured.goto(&callback);
    stand.assert_signed_out(&lured);

    assert_eq!(stand.rows_of(&ada), Vec::<String>::new());
}

#[test]
fn expired_state_signs_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start_with(&["--sign-in-state-lifetime", "5"], CLIENT_SECRET);
    let ada = stand.user(7_104_839_224, "ada-expired-standin");
    let bob = stand.user(7_104_839_225, "bob-expired-standin");

    // Within its lifetime a state works.
    let quick = browser.page();
    stand.sign_in(&quick, &ada);
    stand.account(&quick);

    let slow = browser.page();
    let callback = stand.held_callback(&slow, &bob);
    thread::sleep(Duration::from_secs(7));
    slow.goto(&callback);
    stand.assert_signed_out(&slow);
    assert_eq!(stand.rows_of(&bob), Vec::<String>::new());
}

#[test]
fn reused_state_signs_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(7_104_839_226, "ada-reused-standin");
    let bob = stand.user(7_104_839_227, "bob-reused-standin");

    let first = browser.page();
    let callback = stand.held_callback(&first, &ada);
    let state = param(&callback, "state");
    // The browser as it was before the callback.
    let jar = first.cookies();
    first.goto(&callback);
    stand.account(&first);

    // The same browser state and `state` again, with a new code GitHub gives for that `state`.
    let again = browser.page();
    again.add_cookies(&jar);
    again.goto(&stand.github.asking_sign_in_url(bob.id));
    let authorize = &stand.github.requests_to(Endpoint::Authorize)[0];
    let query = serde_urlencoded::to_string(&authorize.params).expect("a query");
    let visit = again.goto(&format!(
        "{}/login/oauth/authorize?{query}",
        stand.github.url()
    ));
    let replay = stand.authorize_link(&again, &visit);
    assert_eq!(param(&replay, "state"), state);
    assert_ne!(param(&replay, "code"), param(&callback, "code"));

    again.goto(&replay);
    stand.assert_signed_out(&again);
    assert_eq!(stand.rows_of(&bob), Vec::<String>::new());
}

#[test]
fn refused_code_signs_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(8_250_663_101, "ada-refused-standin");

    refused(&stand, &browser, &ada, Fault::BadVerificationCode);
    signs_in_again(&stand, &browser, &ada);
}

#[test]
fn github_errors_sign_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(8_250_663_102, "ada-errors-standin");

    for fault in [
        Fault::AccessDenied,
        Fault::TokenStatus { status: 500 },
        Fault::TokenStatus { status: 404 },
        Fault::UserStatus { status: 500 },
        Fault::UserStatus { status: 401 },
    ] {
        refused(&stand, &browser, &ada, fault);
    }
    signs_in_again(&stand, &browser, &ada);
}

#[test]
fn refused_client_secret_signs_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start_with(&[], WRONG_SECRET);
    let ada = stand.user(8_250_663_103, "ada-secret-standin");

    let page = browser.page();
    stand.sign_in(&page, &ada);
    stand.assert_signed_out(&page);
    assert_eq!(stand.rows_of(&ada), Vec::<String>::new());
    assert_eq!(stand.github.tokens(), Vec::<String>::new());
    assert!(
        !stand.github.requests_to(Endpoint::Token).is_empty(),
        "the server sent the code to GitHub"
    );
}

/// Checks that a sign-in of `user` while the stand-in plays `fault` signs no one in and creates
/// no account.
fn refused(stand: &SignIn, browser: &Browser, user: &User, fault: Fault) {
    stand.github.set_faults(&[fault]);
    let page = browser.page();
    stand.sign_in(&page, user);
    stand.assert_signed_out(&page);
    assert_eq!(stand.rows_of(user), Vec::<String>::new(), "{fault:?}");
    if fault != Fault::AccessDenied {
        assert!(
            !stand.github.requests_to(Endpoint::Token).is_empty(),
            "the server sent the code to GitHub"
        );
    }
}

/// Checks that `user` signs in once GitHub plays no fault: a refusal left nothing behind.
fn signs_in_again(stand: &SignIn, browser: &Browser, user: &User) {
    stand.github.set_faults(&[]);
    let page = browser.page();
    stand.sign_in(&page, user);
    assert_eq!(stand.account(&page).github_login, user.login);
}

#[test]
fn return_address_stays_on_this_server() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(9_015_337_480, "ada-return-standin");
    let elsewhere = browser.echo(false);
    let server = format!("{}/", stand.public);
    let github = format!("{}/", stand.github.url());

    let off_server = [
        "https://evil.test/".to_owned(),
        "http://evil.test/".to_owned(),
        "//evil.test/".to_owned(),
        "/\\evil.test/".to_owned(),
        "\\\\evil.test/".to_owned(),
        "/\t/evil.test/".to_owned(),
        "https:evil.test/".to_owned(),
        format!("http://{HOST}.evil.test/"),
        format!("http://{HOST}:{elsewhere}/"),
        "javascript:alert(document.cookie)".to_owned(),
    ];
    for to in &off_server {
        let page = browser.page();
        stand.github_session(&page, &ada);
        let visit = page.goto_settled(&stand.start_url(Some(to)));
        assert!(
            visit
                .navigations
                .iter()
                .all(|url| url.starts_with(&server) || url.starts_with(&github)),
            "return_to {to:?}: the browser left the server: {visit:?}"
        );
        assert!(
            visit.last().starts_with(&server),
            "return_to {to:?}: {visit:?}"
        );
    }
    assert!(
        browser.hits(elsewhere).is_empty(),
        "the browser went to another port"
    );

    // A page of this server is where the browser returns.
    let page = browser.page();
    stand.github_session(&page, &ada);
    let visit = page.goto_settled(&stand.start_url(Some("/settings?tab=devices")));
    assert_eq!(
        visit.last(),
        format!("{}/settings?tab=devices", stand.public),
        "{visit:?}"
    );
    assert_eq!(stand.account(&page).github_login, ada.login);

    // Without a return address, it returns to this server too.
    let page = browser.page();
    stand.github_session(&page, &ada);
    let visit = page.goto_settled(&stand.start_url(None));
    assert!(visit.last().starts_with(&server), "{visit:?}");
}

#[test]
fn https_public_address_sends_both_cookies_only_over_https() {
    let browser = Browser::launch();
    let stand = SignIn::start_https(&browser);
    let ada = stand.user(9_220_481_006, "ada-https-standin");
    let plain = browser.echo(false);
    let secure = browser.echo(true);

    // The cookie that binds `state`, set when the browser leaves for GitHub: the person is not
    // signed in there, so the browser stays at GitHub.
    let page = browser.page();
    let visit = page.goto(&stand.start_url(None));
    let authorize = format!("{}/login/oauth/authorize?", stand.github.url());
    assert!(visit.url.starts_with(&authorize), "at GitHub: {visit:?}");
    let state_cookies = page.cookies_of(HOST);
    assert!(!state_cookies.is_empty(), "the sign-in start sets a cookie");
    for (port, scheme) in [(plain, "http"), (secure, "https")] {
        page.goto(&format!("{scheme}://{HOST}:{port}{CALLBACK}?probe=state"));
    }
    let over_http = &browser.wait_for_hits(plain, "probe=state", 1)[0];
    let over_https = &browser.wait_for_hits(secure, "probe=state", 1)[0];
    assert_eq!(over_http.cookie, None, "plain HTTP carries no cookie");
    for cookie in &state_cookies {
        assert!(over_https.carries(cookie), "{cookie:?}: {over_https:?}");
    }

    // The session's cookie.
    let signed_in = browser.page();
    let session = session_cookies(&stand, &signed_in, &ada);
    for path in ["/", SESSION, CALLBACK] {
        for (port, scheme) in [(plain, "http"), (secure, "https")] {
            signed_in.goto(&format!("{scheme}://{HOST}:{port}{path}?probe=session"));
        }
    }
    for hit in browser.wait_for_hits(plain, "probe=session", 3) {
        assert_eq!(hit.cookie, None, "plain HTTP carries no cookie: {hit:?}");
    }
    let over_https = browser.wait_for_hits(secure, "probe=session", 3);
    assert!(
        session
            .iter()
            .all(|cookie| over_https.iter().any(|hit| hit.carries(cookie))),
        "HTTPS carries the session's cookie: {session:?}, {over_https:?}"
    );
    assert_eq!(stand.account(&signed_in).github_login, ada.login);
}

#[test]
fn page_scripts_cannot_read_the_session_cookie() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(9_220_481_007, "ada-script-standin");
    let page = browser.page();
    let session = session_cookies(&stand, &page, &ada);

    page.goto(&format!("{}/health", stand.public));
    let readable = page.evaluate("document.cookie");
    let readable = readable.as_str().expect("document.cookie is a string");
    for cookie in &session {
        assert!(
            !readable.split("; ").any(|pair| pair == cookie.pair()),
            "a page script reads {cookie:?}"
        );
    }
    assert_eq!(stand.account(&page).github_login, ada.login);
}

#[test]
fn other_sites_requests_do_not_carry_the_session_cookie() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(9_220_481_008, "ada-crosssite-standin");
    let echo = browser.echo(false);
    let page = browser.page();
    let session = session_cookies(&stand, &page, &ada);
    let target = format!("http://{HOST}:{echo}/hit");

    // The same requests from a page of the server's site, then from another site's page.
    for (site, label) in [(HOST, "same"), ("other.test", "cross")] {
        page.goto(&format!("http://{site}:{echo}/blank"));
        page.evaluate(&format!(
            r#"(() => {{
                const via = (kind) => "{target}?via={label}-" + kind;
                new Image().src = via("image");
                fetch(via("fetch"), {{ mode: "no-cors", credentials: "include" }});
                const frame = document.createElement("iframe");
                frame.src = via("frame");
                document.body.append(frame);
                const sink = document.createElement("iframe");
                sink.name = "sink";
                document.body.append(sink);
                const form = document.createElement("form");
                form.method = "POST";
                form.action = via("form");
                form.target = "sink";
                document.body.append(form);
                form.submit();
            }})()"#
        ));
        browser.wait_for_hits(echo, &format!("via={label}-"), 4);
        page.evaluate(&format!(
            r#"setTimeout(() => {{ location.href = "{target}?via={label}-navigation"; }}, 0)"#
        ));
        let hits = browser.wait_for_hits(echo, &format!("via={label}-"), 5);
        for hit in &hits {
            let carried = session.iter().any(|cookie| hit.carries(cookie));
            match label {
                "same" => assert!(carried, "the server's site: {hit:?}"),
                _ => assert!(!carried, "another site's request carries it: {hit:?}"),
            }
        }
    }
}

#[test]
fn database_holds_no_github_token_client_secret_or_session_token() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let mut secrets = vec![CLIENT_SECRET.to_owned()];
    for (id, login) in [
        (9_514_002_731, "ada-db-standin"),
        (9_514_002_732, "bob-db-standin"),
    ] {
        let user = stand.user(id, login);
        let page = browser.page();
        let session = session_cookies(&stand, &page, &user);
        secrets.extend(session.into_iter().map(|cookie| cookie.value));
    }
    let tokens = stand.github.tokens();
    assert_eq!(tokens.len(), 2, "each sign-in got a GitHub token");
    secrets.extend(tokens);

    let rows = stand.db.rows();
    assert!(!rows.is_empty(), "the database holds the accounts");
    for secret in &secrets {
        for form in stored_forms(secret) {
            let found: Vec<_> = rows.iter().filter(|row| row.contains(&form)).collect();
            assert!(found.is_empty(), "{secret:?} as {form:?} in {found:?}");
        }
    }
}

#[test]
fn server_output_never_shows_the_client_secret() {
    let help = Command::new(SERVER)
        .arg("--help")
        .env("SKALD_GITHUB_CLIENT_SECRET", CLIENT_SECRET)
        .output()
        .expect("run skald-server --help");
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    assert!(
        help.contains("SKALD_GITHUB_CLIENT_SECRET"),
        "--help names the setting: {help}"
    );
    assert!(!help.contains(CLIENT_SECRET), "{help}");

    // A start that fails on a bad setting.
    let db = TestDb::create();
    let url = db.url();
    let failed = Command::new(SERVER)
        .args(["--listen", "127.0.0.1:0", "--database-url", &url])
        .args(["--public-url", "not a URL", "--github-client-id", CLIENT_ID])
        .env("SKALD_GITHUB_CLIENT_SECRET", CLIENT_SECRET)
        .output()
        .expect("run skald-server");
    assert!(!failed.status.success());
    for output in [&failed.stdout, &failed.stderr] {
        assert!(!String::from_utf8_lossy(output).contains(CLIENT_SECRET));
    }

    // A sign-in, and sign-ins that GitHub fails.
    let browser = Browser::launch();
    let mut stand = SignIn::start();
    let ada = stand.user(9_633_870_114, "ada-output-standin");
    let page = browser.page();
    stand.sign_in(&page, &ada);
    stand.account(&page);
    for fault in [
        Fault::BadVerificationCode,
        Fault::TokenStatus { status: 500 },
        Fault::UserStatus { status: 500 },
    ] {
        stand.github.set_faults(&[fault]);
        stand.sign_in(&browser.page(), &ada);
    }
    let output = stand.server.stop();
    assert!(!output.contains(CLIENT_SECRET), "{output}");

    // A sign-in that GitHub refuses for the secret.
    let mut refused = SignIn::start_with(&[], WRONG_SECRET);
    let bob = refused.user(9_633_870_115, "bob-output-standin");
    refused.sign_in(&browser.page(), &bob);
    assert!(!refused.github.requests_to(Endpoint::Token).is_empty());
    let output = refused.server.stop();
    assert!(!output.contains(WRONG_SECRET), "{output}");
}

#[test]
fn without_github_settings_sign_in_is_refused_and_health_is_ok() {
    let spec = spec();
    let db = TestDb::create();
    let server = Server::start(&db);

    let health = server.get("/health");
    assert_eq!(health.status, 200);
    assert_eq!(health.body, br#"{"status":"ok"}"#);

    let start = server.get(START);
    assert!(
        (400..600).contains(&start.status),
        "GET {START}: {}",
        start.status
    );
    assert_eq!(start.header("location"), None);
    assert_eq!(spec_violations(&spec, START, &start), Vec::<String>::new());

    let callback = server.get(&format!("{CALLBACK}?code=0123456789abcdef0123&state=abc"));
    assert!(
        (400..600).contains(&callback.status),
        "GET {CALLBACK}: {}",
        callback.status
    );
    assert_eq!(
        spec_violations(&spec, CALLBACK, &callback),
        Vec::<String>::new()
    );
}

#[test]
fn sign_in_responses_conform_to_spec() {
    let spec = spec();
    let stand = SignIn::start_local();
    let ada = stand.user(9_802_116_350, "ada-spec-standin");
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let versioned = [(skald_proto::VERSION_HEADER, version.as_str())];
    let check = |path: &str, response: &Response| {
        assert_eq!(spec_violations(&spec, path, response), Vec::<String>::new());
    };

    let none = get_url(&format!("{}{SESSION}", stand.public), &versioned);
    assert_eq!(none.status, 401);
    check(SESSION, &none);

    let mut jar = Jar::default();
    let start = get_url(&stand.start_url(Some("/settings")), &[]);
    check(START, &start);
    assert!((300..400).contains(&start.status), "{}", start.status);
    let authorize = start.header("location").expect("a redirect to GitHub");
    assert!(
        authorize.starts_with(&format!("{}/login/oauth/authorize?", stand.github.url())),
        "{authorize}"
    );
    jar.take(&start);

    let mut github = Jar::default();
    github.take(&get_url(&stand.github.sign_in_url(ada.id), &[]));
    let back = get_url(authorize, &[("cookie", &github.header())]);
    let callback = back.header("location").expect("GitHub goes back");
    assert!(
        callback.starts_with(&format!("{}{CALLBACK}?", stand.public)),
        "{callback}"
    );

    // Refused callbacks: no state, and a wrong one.
    for refused in [
        with_param(callback, "state", None),
        with_param(callback, "state", Some("0123456789abcdef")),
    ] {
        check(CALLBACK, &get_url(&refused, &[("cookie", &jar.header())]));
    }
    let bare = get_url(&format!("{}{CALLBACK}", stand.public), &[]);
    check(CALLBACK, &bare);

    let signed_in = get_url(callback, &[("cookie", &jar.header())]);
    check(CALLBACK, &signed_in);
    jar.take(&signed_in);
    let session = get_url(
        &format!("{}{SESSION}", stand.public),
        &[versioned[0], ("cookie", &jar.header())],
    );
    assert_eq!(session.status, 200);
    check(SESSION, &session);
    let body: serde_json::Value = serde_json::from_slice(&session.body).expect("JSON");
    assert_eq!(body["github_login"], ada.login.as_str());
}

/// Cookies as a client that follows no rules but name and value keeps them.
#[derive(Default)]
struct Jar(Vec<(String, String)>);

impl Jar {
    /// Keeps the cookies `response` sets, replacing those of the same name.
    fn take(&mut self, response: &Response) {
        for set in response.header_values("set-cookie") {
            let pair = set.split(';').next().unwrap_or_default();
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            self.0.retain(|(kept, _)| kept != name);
            self.0.push((name.to_owned(), value.to_owned()));
        }
    }

    fn header(&self) -> String {
        let pairs: Vec<_> = self
            .0
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        pairs.join("; ")
    }
}

/// Signs `user` in on `page` and gives the cookies of the server's host that the callback set or
/// changed: the session's.
fn session_cookies(stand: &SignIn, page: &Page, user: &User) -> Vec<Cookie> {
    let callback = stand.held_callback(page, user);
    let before = page.cookies_of(HOST);
    page.goto(&callback);
    let mut set = page.cookies_of(HOST);
    set.retain(|cookie| !before.contains(cookie));
    assert!(!set.is_empty(), "the callback sets the session's cookie");
    assert_eq!(stand.account(page).github_login, user.login);
    set
}

/// The forms in which a database could hold `secret` in the clear: as it is, in lowercase, and the
/// bytes it encodes, if it is Base64, in the hex of PostgreSQL's text form of `bytea`.
fn stored_forms(secret: &str) -> Vec<String> {
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut forms = vec![
        secret.to_owned(),
        secret.to_lowercase(),
        hex(secret.as_bytes()),
    ];
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(secret)
            && bytes.len() >= 8
        {
            forms.push(hex(&bytes));
        }
    }
    forms
}

/// The value of the query parameter `name` of `url`.
fn param(url: &str, name: &str) -> Option<String> {
    query(url)
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/// `url` with its query parameter `name` set to `value`, or removed for `None`.
fn with_param(url: &str, name: &str, value: Option<&str>) -> String {
    let base = url.split('?').next().unwrap_or(url);
    let mut pairs = query(url);
    pairs.retain(|(key, _)| key != name);
    if let Some(value) = value {
        pairs.push((name.to_owned(), value.to_owned()));
    }
    let encoded = serde_urlencoded::to_string(&pairs).expect("a query");
    format!("{base}?{encoded}")
}

fn query(url: &str) -> Vec<(String, String)> {
    let query = url
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default();
    serde_urlencoded::from_str(query).expect("a query")
}
