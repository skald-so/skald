//! A server's web address, apart from its public address: the address browsers sign in at, while
//! the CLI and devices keep the public address.
//!
//! The setting `--web-url <URL>`, or `SKALD_WEB_URL`, an http or https origin, by default the public
//! address `--public-url`. With it set:
//! - the providers' callbacks, `<web URL>/auth/github/callback` and `<web URL>/auth/google/callback`,
//!   and the pages a sign-in returns to are on the web address;
//! - sign-in's cookies are `Secure` under an https web address, whatever the public address's scheme;
//! - the browser forms, the device flow's decision on `POST /device` and a link's start on
//!   `POST /auth/github/link`, are taken only with the web address's `Origin`;
//! - device authorization's `verification_uri` and `verification_uri_complete` are on the web
//!   address: `<web URL>/device` and `<web URL>/device?user_code=<user code>`;
//! - a device proves its key over the public address's origin (`docs/protocol/delivery.md`), and the
//!   server's name defaults to the public address's host.
//!
//! Without it, all of them are on the public address, as the other sign-in tests show. A value that
//! is not an http or https URL stops the server at start, with an error naming the setting. Every
//! response to a machine is checked against the OpenAPI spec in `api/openapi.json`.

mod common;

use std::io::Read;
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use common::browser::{Browser, SIGN_IN_LINK};
use common::device_flow::{DeviceFlow, INTERVAL, json, text};
use common::devices::{
    CHALLENGE, Device, Key, assert_taken, auth_frame, auth_signature, machine, unb64,
};
use common::google::{self, Google, Setup};
use common::sign_in::{
    self, CLIENT_SECRET, Jar, SECRET_ENV, SignIn, cookie_set, github_args, github_stand_in, param,
    set_cookie, start_url, to_callback,
};
use common::{
    Closed, Response, SERVER, Server, TestDb, WebSocket, get_url, on_free_port, post_form, send,
    wait,
};
use skald_oidc_stand_in::Client;
use skald_proto::{PROTOCOL_VERSION, VERSION_HEADER};
use tungstenite::Message;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;

/// The setting, as an argument and as an environment variable.
const WEB: &str = "--web-url";
const WEB_ENV: &str = "SKALD_WEB_URL";
/// The public address's setting.
const PUBLIC: &str = "--public-url";
/// The device flow's verification page; `POST` takes the person's decision.
const PAGE: &str = "/device";
/// Starts a link of a GitHub sign-in method to the session's account: `POST`.
const GITHUB_LINK: &str = "/auth/github/link";
/// The server's name: `GET`.
const SERVER_NAME: &str = "/server";
/// How a refused proof closes the connection.
const SIGNATURE_REFUSED: &str = "signature-refused";

/// How the server is given its web address.
#[derive(Clone, Copy, Debug)]
enum Given {
    Argument,
    Environment,
}

/// A server with GitHub sign-in through the stand-in, whose app's callback is on the web address,
/// and the device flow, its codes polled every second.
struct Stage {
    /// The device flow; its `stand.public` is the web address, where browsers sign in.
    flow: DeviceFlow,
    /// Where machines and devices reach the server.
    public: String,
    /// Where browsers sign in.
    web: String,
}

impl Stage {
    /// Its public address `http://api.skald.test:<port>` and its web address
    /// `http://app.skald.test:<port>`, `<port>` the one it listens on, so a browser reaches both.
    fn start() -> Self {
        Self::launch(Given::Argument, |port| {
            (
                format!("http://api.skald.test:{port}"),
                format!("http://app.skald.test:{port}"),
            )
        })
    }

    /// Its public and web addresses as `addresses` gives them for the port it listens on, the web
    /// address given as `given` says.
    fn launch(given: Given, addresses: impl Fn(u16) -> (String, String)) -> Self {
        let db = TestDb::create();
        let (github, server, (public, web)) = on_free_port(Duration::from_secs(30), |port| {
            let (public, web) = addresses(port);
            let github = github_stand_in(&web);
            let mut args = vec![PUBLIC, public.as_str(), INTERVAL, "1"];
            args.extend(github_args(&github));
            let mut envs = vec![(SECRET_ENV, CLIENT_SECRET)];
            match given {
                Given::Argument => args.extend([WEB, web.as_str()]),
                Given::Environment => envs.push((WEB_ENV, web.as_str())),
            }
            let listen = format!("127.0.0.1:{port}");
            let server = Server::try_launch(&db, &listen, &args, &envs)?;
            Ok((github, server, (public, web)))
        });
        let stand = SignIn {
            db,
            github,
            server,
            public: web.clone(),
        };
        Stage {
            flow: DeviceFlow::of(stand),
            public,
            web,
        }
    }

    /// Where the test reaches the server.
    fn base(&self) -> &str {
        self.flow.stand.server.base()
    }

    /// `POST <path>` of the form `fields` straight to the server, with the web session's `cookie`
    /// and the `Origin` `origin`, as a browser submits a form.
    fn submit(&self, path: &str, cookie: &str, origin: &str, fields: &[(&str, &str)]) -> Response {
        let headers = [("cookie", cookie), ("origin", origin)];
        post_form(&format!("{}{path}", self.base()), &headers, fields)
    }

    /// A new challenge of the device session `token`; then a WebSocket at `/ws` whose first frame
    /// proves `device`'s key on it, signed over `origin`.
    fn prove(&self, token: &str, device: &Device, origin: &str) -> WebSocket {
        let version = PROTOCOL_VERSION.to_string();
        let bearer = format!("Bearer {token}");
        let headers = [
            ("authorization", bearer.as_str()),
            (VERSION_HEADER, version.as_str()),
        ];
        let answer = send(
            "post",
            &format!("{}{CHALLENGE}", self.base()),
            &headers,
            &[],
        );
        self.flow.conforms("post", CHALLENGE, &answer);
        assert_eq!(answer.status, 200, "{}", text(&answer));
        let challenge = json(&answer)["challenge"]
            .as_str()
            .map(unb64)
            .unwrap_or_else(|| panic!("a challenge: {}", text(&answer)));
        let offer = skald_proto::subprotocol(PROTOCOL_VERSION);
        let (mut socket, _) = self
            .flow
            .stand
            .server
            .open_websocket("/ws", Some(&offer))
            .unwrap_or_else(|refused| panic!("/ws: {}", text(&refused)));
        let signature = auth_signature(&device.signing.key, &challenge, origin);
        let frame = auth_frame(&challenge, device.id, &signature);
        socket
            .send(Message::Binary(frame.into()))
            .expect("send the proof");
        socket
    }
}

/// What came of a proof sent on `socket`: `Ok` once the server answers a ping sent after it, so it
/// took the connection; otherwise how the server closed it.
fn taken(socket: &mut WebSocket) -> Result<(), Closed> {
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        TcpStream::set_read_timeout(stream, Some(Duration::from_secs(20))).expect("a timeout");
    }
    // A closed connection fails the ping; the read then tells how it closed.
    let _ = socket.send(Message::Ping(b"taken?".to_vec().into()));
    loop {
        match socket.read() {
            Ok(Message::Pong(payload)) if payload.as_ref() == b"taken?".as_slice() => {
                return Ok(());
            }
            Ok(Message::Close(frame)) => return Err(Closed::of(frame)),
            Ok(_) => {}
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                panic!("the server neither took the connection nor closed it");
            }
            Err(_) => return Err(Closed::Dropped),
        }
    }
}

/// Whether the `Set-Cookie` value `cookie` has the attribute `Secure`.
fn is_secure(cookie: &str) -> bool {
    cookie
        .split(';')
        .skip(1)
        .any(|attribute| attribute.trim().eq_ignore_ascii_case("secure"))
}

/// The address the page `response` gives refreshes to, as a sign-in's callback ends.
fn refreshes_to(response: &Response) -> String {
    let page = String::from_utf8_lossy(&response.body);
    let (_, after) = page
        .split_once("content=\"0;url=")
        .unwrap_or_else(|| panic!("a page that refreshes: {page}"));
    let to = after.split('"').next().unwrap_or_default();
    to.replace("&amp;", "&")
}

/// The device flow's verification URIs are on the web address, given as an argument or in the
/// environment.
#[test]
fn verification_uris_are_on_the_web_address() {
    for given in [Given::Argument, Given::Environment] {
        let stage = Stage::launch(given, |port| {
            (
                format!("http://api.skald.test:{port}"),
                format!("https://app.skald.test:{port}"),
            )
        });

        let grant = stage.flow.authorize();

        let page = format!("{}{PAGE}", stage.web);
        assert_eq!(grant.verification_uri, page, "{given:?}");
        let query =
            serde_urlencoded::to_string([("user_code", &grant.user_code)]).expect("a query");
        assert_eq!(
            grant.verification_uri_complete,
            format!("{page}?{query}"),
            "{given:?}"
        );
    }
}

/// GitHub's and Google's sign-in starts send the browser to the provider with the callback on the
/// web address.
#[test]
fn sign_in_callbacks_are_on_the_web_address() {
    let web = "https://app.skald.test";
    let args = [WEB, web];
    let server = Google::launch(Setup {
        public: Some("http://api.skald.test"),
        github: true,
        args: &args,
        ..Setup::default()
    });

    for (start, callback) in [
        (sign_in::START, sign_in::CALLBACK),
        (google::START, google::CALLBACK),
    ] {
        let started = server.server.get(start);
        assert_eq!(started.status, 302, "GET {start}: {}", text(&started));
        let to = started.header("location").expect("a redirect");
        assert_eq!(
            param(to, "redirect_uri"),
            Some(format!("{web}{callback}")),
            "GET {start}: {to}"
        );
    }
}

/// A GitHub sign-in returns the browser to a page of the web address: the one it asked for there,
/// or the web address's root for none or one of the public address.
#[test]
fn github_sign_in_returns_to_pages_of_the_web_address() {
    let stage = Stage::start();
    let stand = &stage.flow.stand;
    let ada = stand.user(2_510_100_001, "ada-web-return-standin");
    let root = format!("{}/", stage.web);
    let landing = format!("{}/landing?tab=1", stage.web);

    for (asked, returned) in [
        (None, &root),
        (Some("/landing?tab=1".to_owned()), &landing),
        (Some(landing.clone()), &landing),
        (Some(format!("{}/landing?tab=1", stage.public)), &root),
    ] {
        let start = start_url("", sign_in::START, asked.as_deref());
        let sign_in = stand.github.sign_in_url(ada.id);
        let (started, callback) = to_callback(&stand.server, &stage.web, &start, &sign_in);
        let done = get_url(
            &callback,
            &[("cookie", &cookie_set(&started, "skald_sign_in"))],
        );
        assert_eq!(done.status, 200, "return_to {asked:?}: {}", text(&done));

        assert_eq!(&refreshes_to(&done), returned, "return_to {asked:?}");
    }
}

/// A Google sign-in, through the callback on the web address, signs the person in and returns the
/// browser to the web address.
#[test]
fn google_sign_in_returns_to_the_web_address() {
    let web = "http://app.skald.test";
    let args = [WEB, web];
    let mut server = Google::launch(Setup {
        public: Some("http://api.skald.test"),
        args: &args,
        ..Setup::default()
    });
    server.provider.register(Client {
        client_id: google::CLIENT_ID.to_owned(),
        client_secret: google::CLIENT_SECRET.to_owned(),
        redirect_uri: format!("{web}{}", google::CALLBACK),
    });
    server.public = web.to_owned();
    let ada = server.person("ada-web-google-subject", "ada.web@example.test");

    let (jar, callback) = server.to_callback(&ada);
    let done = get_url(&callback, &[("cookie", &jar.header())]);

    assert_eq!(done.status, 200, "{}", text(&done));
    assert_eq!(refreshes_to(&done), format!("{web}/"));
    let mut jar = jar;
    jar.take(&done);
    let session = server.session_of(&jar);
    assert_eq!(session.status, 200, "{}", text(&session));
}

/// Sign-in's cookies are `Secure` as the web address's scheme says, not the public address's.
#[test]
fn sign_in_cookies_are_secure_under_an_https_web_address() {
    for (public, web, secure) in [("http", "https", true), ("https", "http", false)] {
        let stage = Stage::launch(Given::Argument, |port| {
            (
                format!("{public}://api.skald.test:{port}"),
                format!("{web}://app.skald.test:{port}"),
            )
        });
        let stand = &stage.flow.stand;
        let ada = stand.user(2_510_100_002, "ada-web-cookies-standin");

        let (started, callback) = stand.to_callback(&ada);
        let done = get_url(
            &callback,
            &[("cookie", &cookie_set(&started, "skald_sign_in"))],
        );

        assert_eq!(done.status, 200, "{}", text(&done));
        for (response, name) in [(&started, "skald_sign_in"), (&done, "skald_session")] {
            let cookie = set_cookie(response, name);
            assert_eq!(
                is_secure(cookie),
                secure,
                "{public} public and {web} web address: {cookie}"
            );
        }
    }
}

/// The browser forms, the device flow's decision and a link's start, are taken from the web
/// address and refused from the public address.
#[test]
fn browser_forms_are_taken_only_from_the_web_address() {
    let stage = Stage::start();
    let flow = &stage.flow;
    let ada = flow.stand.user(2_510_100_003, "ada-web-forms-standin");
    let cookie = flow.web_cookie(&ada);
    let grant = flow.authorize();
    let approve = [
        ("user_code", grant.user_code.as_str()),
        ("decision", "approve"),
    ];

    let refused = stage.submit(PAGE, &cookie, &stage.public, &approve);
    assert_eq!(refused.status, 403, "{}", text(&refused));
    flow.assert_pending(&grant);
    let refused = stage.submit(GITHUB_LINK, &cookie, &stage.public, &[]);
    assert_eq!(refused.status, 403, "{}", text(&refused));

    let decided = stage.submit(PAGE, &cookie, &stage.web, &approve);
    assert_eq!(decided.status, 200, "{}", text(&decided));
    let token = flow.token(&grant);
    assert_eq!(flow.account_of(&token).github_login, ada.login);
    let linking = stage.submit(GITHUB_LINK, &cookie, &stage.web, &[]);
    assert_eq!(linking.status, 302, "{}", text(&linking));
    let to = linking.header("location").expect("a redirect to GitHub");
    assert_eq!(
        param(to, "redirect_uri"),
        Some(format!("{}{}", stage.web, sign_in::CALLBACK)),
        "{to}"
    );
}

/// A link whose callback comes from GitHub without the session's cookie, as a redirect from another
/// site does, opens the callback again on the web address.
#[test]
fn link_callback_from_another_site_reopens_on_the_web_address() {
    let stage = Stage::start();
    let stand = &stage.flow.stand;
    let ada = stand.user(2_510_100_006, "ada-web-hop-standin");
    let cookie = stage.flow.web_cookie(&ada);
    let linking = stage.submit(GITHUB_LINK, &cookie, &stage.web, &[]);
    assert_eq!(linking.status, 302, "{}", text(&linking));
    let authorize = linking.header("location").expect("a redirect to GitHub");
    let mut github = Jar::default();
    github.take(&get_url(&stand.github.sign_in_url(ada.id), &[]));
    let back = get_url(authorize, &[("cookie", &github.header())]);
    let callback = back.header("location").expect("GitHub goes back");
    assert!(
        callback.starts_with(&format!("{}{}?", stage.web, sign_in::CALLBACK)),
        "{callback}"
    );

    let state = cookie_set(&linking, "skald_sign_in");
    let hop = get_url(
        &callback.replacen(&stage.web, stage.base(), 1),
        &[("cookie", &state)],
    );

    assert_eq!(hop.status, 200, "{}", text(&hop));
    assert_eq!(refreshes_to(&hop), callback);
}

/// A device proves its key over the public address's origin, not the web address's.
#[test]
fn devices_prove_their_key_over_the_public_address() {
    let stage = Stage::start();
    let flow = &stage.flow;
    let ada = flow.stand.user(2_510_100_004, "ada-web-proof-standin");
    let signed_in = machine(flow, &ada);
    let device = Device::new("Laptop");
    let master = Key::generate();
    signed_in.register_device(&device);
    assert_taken(&signed_in.give_master(&master, &device.cert(&signed_in.account, &master)));

    let mut over_web = stage.prove(&signed_in.token, &device, &stage.web);
    assert_eq!(
        taken(&mut over_web),
        Err(Closed::Frame(
            CloseCode::Policy,
            SIGNATURE_REFUSED.to_owned()
        )),
        "a proof over the web address"
    );

    let mut over_public = stage.prove(&signed_in.token, &device, &stage.public);
    assert_eq!(
        taken(&mut over_public),
        Ok(()),
        "a proof over the public address"
    );
}

/// The server's name defaults to the public address's host, not the web address's.
#[test]
fn server_name_defaults_to_the_public_address_host() {
    let stage = Stage::start();

    let answer = stage.flow.request("get", SERVER_NAME, &[], None, true);

    assert_eq!(answer.status, 200, "{}", text(&answer));
    assert_eq!(json(&answer)["name"], "api.skald.test", "{}", text(&answer));
}

/// The web address is an http or https URL: a server takes one and gives its verification page
/// there, and one that is not stops the server at start, naming the setting.
#[test]
fn web_address_is_an_http_or_https_url() {
    for web in ["https://app.skald.test", "http://app.skald.test:8080"] {
        let stage = Stage::launch(Given::Argument, |_| {
            ("https://api.skald.test".to_owned(), web.to_owned())
        });
        assert_eq!(
            stage.flow.authorize().verification_uri,
            format!("{web}{PAGE}")
        );
    }

    for web in ["ftp://app.skald.test", "app.skald.test"] {
        let db = TestDb::create();
        let github = github_stand_in("https://api.skald.test");
        let url = db.url();
        let mut args = vec!["--listen", "127.0.0.1:0", "--database-url", url.as_str()];
        args.extend([PUBLIC, "https://api.skald.test", WEB, web]);
        args.extend(github_args(&github));
        let mut child = Command::new(SERVER)
            .args(&args)
            .env(SECRET_ENV, CLIENT_SECRET)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start skald-server");

        let status = wait(&mut child, Duration::from_secs(30));

        let mut stderr = String::new();
        let mut pipe = child.stderr.take().expect("stderr is piped");
        pipe.read_to_string(&mut stderr).expect("read stderr");
        assert!(!status.success(), "{web}: {stderr}");
        assert!(
            stderr.contains(WEB) || stderr.contains(WEB_ENV),
            "{web}: {stderr}"
        );
    }
}

/// In a browser, a person signs in with GitHub on the verification page of the web address, never
/// leaving it but for GitHub, and approves a machine's code there; the machine gets a session of
/// their account.
#[test]
fn person_approves_a_machine_on_the_web_address() {
    let browser = Browser::launch();
    let stage = Stage::start();
    let flow = &stage.flow;
    let ada = flow.stand.user(2_510_100_005, "ada-web-browser-standin");
    let page = browser.page();
    flow.stand.github_session(&page, &ada);
    let grant = flow.authorize();

    let mut visit = flow.open(&page, &grant.verification_uri_complete);
    if page.count(SIGN_IN_LINK) > 0 {
        visit = page.click(&format!("{SIGN_IN_LINK} >> nth=0"));
    }

    let web = format!("{}/", stage.web);
    let github = format!("{}/", flow.stand.github.url());
    assert!(
        visit
            .navigations
            .iter()
            .all(|url| url.starts_with(&web) || url.starts_with(&github)),
        "only the web address and GitHub: {visit:?}"
    );
    let callback = format!("{}{}?", stage.web, sign_in::CALLBACK);
    assert!(
        visit
            .navigations
            .iter()
            .any(|url| url.starts_with(&callback)),
        "GitHub's callback on the web address: {visit:?}"
    );
    let account = flow.stand.account(&page);
    assert_eq!(account.github_login, ada.login);
    let token = flow.approve(&page, &grant);
    assert_eq!(flow.account_of(&token), account);
}
