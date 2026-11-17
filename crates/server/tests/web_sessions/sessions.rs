//! The account's sessions through the API, the sessions the tests make, and the checks they share.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use skald_github_stand_in::User;
use skald_proto::{PROTOCOL_VERSION, VERSION_HEADER};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

use crate::common::browser::Page;
use crate::common::cbor::{array_of, uint};
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::{
    CHALLENGE, Device, Key, assert_taken, auth_frame, auth_signature, b64, machine, unb64,
};
use crate::common::google::{Google, HOST};
use crate::common::sign_in::{START, cookie_set, provider_back};
use crate::common::spec::{operation_violations, spec};
use crate::common::{Closed, Response, Server, TestDb, WebSocket, delete_url, get_url, hex, send};

/// The account's sessions: `GET`.
pub const SESSIONS: &str = "/account/sessions";
/// One session of the account: `DELETE`.
pub const ONE: &str = "/account/sessions/{session}";
/// Every session of the account but the request's own: `POST`.
pub const END_OTHERS: &str = "/account/sessions/end-others";
/// The request's own session: `GET`, and `DELETE` to sign it out.
pub const SESSION: &str = "/session";

/// How long a web session lasts after its sign-in, in seconds: 30 days.
pub const LIFETIME: i64 = 30 * 86_400;
/// The session cookie's name.
pub const COOKIE: &str = "skald_session";

/// The `User-Agent` of Chrome on macOS, and of Firefox on Windows.
pub const CHROME_ON_MACOS: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
pub const FIREFOX_ON_WINDOWS: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0";

/// A session's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Web,
    Device,
}

/// A session as the list gives it, its date-times in seconds since the Unix epoch.
#[derive(Clone, Debug)]
pub struct Listed {
    pub id: String,
    pub kind: Kind,
    pub browser: Option<String>,
    pub os: Option<String>,
    pub device_name: Option<String>,
    pub started_at: i64,
    pub last_used_at: i64,
    pub expires_at: Option<i64>,
    pub current: bool,
    /// The session as listed.
    pub item: Value,
}

/// Seconds since the Unix epoch.
pub fn now() -> i64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.expect("a clock after 1970").as_secs() as i64
}

/// The `Cookie` header of the web session of `page`'s browser at the host `HOST`.
pub fn cookie_of(page: &Page) -> String {
    let cookies = page.cookies_of(HOST);
    let found = cookies.iter().find(|cookie| cookie.name == COOKIE);
    found
        .unwrap_or_else(|| panic!("a session cookie: {cookies:?}"))
        .pair()
}

/// The token in `cookie`, a `Cookie` header of the session cookie alone.
pub fn token_of(cookie: &str) -> &str {
    cookie
        .strip_prefix(&format!("{COOKIE}="))
        .unwrap_or_else(|| panic!("a session cookie: {cookie}"))
}

/// A server's API as a test reaches it: straight, at `base`; `public`, its public address, is the
/// `Origin` of its own pages.
pub struct Api<'a> {
    pub base: &'a str,
    pub public: &'a str,
    pub db: &'a TestDb,
}

impl<'a> Api<'a> {
    /// The API of `flow`'s server.
    pub fn of(flow: &'a DeviceFlow) -> Self {
        Api {
            base: flow.stand.server.base(),
            public: &flow.stand.public,
            db: &flow.stand.db,
        }
    }

    /// The API of `google`'s server.
    pub fn of_google(google: &'a Google) -> Self {
        Api {
            base: google.server.base(),
            public: &google.public,
            db: &google.db,
        }
    }

    /// The same API through `server`, another instance on its database.
    pub fn on(&self, server: &'a Server) -> Self {
        Api {
            base: server.base(),
            public: self.public,
            db: self.db,
        }
    }

    /// `<method> <path>` with `headers`, stating the protocol version, without a body; checked
    /// against the spec's operation at `template`.
    pub fn call(
        &self,
        method: &str,
        path: &str,
        template: &str,
        headers: &[(&str, &str)],
    ) -> Response {
        let version = PROTOCOL_VERSION.to_string();
        let mut all = headers.to_vec();
        all.push((VERSION_HEADER, &version));
        let url = format!("{}{path}", self.base);
        let response = match method {
            "get" => get_url(&url, &all),
            "delete" => delete_url(&url, &all),
            _ => send(method, &url, &all, &[]),
        };
        let violations = operation_violations(&spec(), method, template, &response);
        assert_eq!(
            violations,
            Vec::<String>::new(),
            "{method} {path}: {}",
            text(&response)
        );
        response
    }

    /// The headers of a request of the web session `cookie` from a page of this server.
    fn page_headers<'c>(&'c self, cookie: &'c str) -> [(&'c str, &'c str); 2] {
        [("cookie", cookie), ("origin", self.public)]
    }

    /// `GET /account/sessions` with `headers`.
    pub fn list_response(&self, headers: &[(&str, &str)]) -> Response {
        self.call("get", SESSIONS, SESSIONS, headers)
    }

    /// The sessions the web session `cookie` lists, as a page of this server fetches them; fails
    /// unless it lists them.
    #[track_caller]
    pub fn list(&self, cookie: &str) -> Vec<Listed> {
        let response = self.list_response(&[("cookie", cookie)]);
        assert_eq!(response.status, 200, "{}", text(&response));
        self.listed(&json(&response))
    }

    /// The sessions in `body`, a `200` answer to `GET /account/sessions`.
    pub fn listed(&self, body: &Value) -> Vec<Listed> {
        let items = body["sessions"]
            .as_array()
            .unwrap_or_else(|| panic!("a list of sessions: {body}"));
        items.iter().map(|item| self.session_in(item)).collect()
    }

    fn session_in(&self, item: &Value) -> Listed {
        let string = |name: &str| match item.get(name) {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(other) => panic!("{name} {other} in {item}"),
        };
        let time = |name: &str| {
            let value = string(name).unwrap_or_else(|| panic!("no {name} in {item}"));
            epoch(self.db, &value)
        };
        let kind = match item["kind"].as_str() {
            Some("web") => Kind::Web,
            Some("device") => Kind::Device,
            _ => panic!("a kind, web or device: {item}"),
        };
        let current = match item.get("current") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            Some(other) => panic!("current {other} in {item}"),
        };
        Listed {
            id: string("session").unwrap_or_else(|| panic!("no session ID in {item}")),
            kind,
            browser: string("browser"),
            os: string("os"),
            device_name: string("device_name"),
            started_at: time("started_at"),
            last_used_at: time("last_used_at"),
            expires_at: string("expires_at").map(|value| epoch(self.db, &value)),
            current,
            item: item.clone(),
        }
    }

    /// `DELETE /account/sessions/<id>` with `headers`.
    pub fn end_with(&self, id: &str, headers: &[(&str, &str)]) -> Response {
        self.call("delete", &format!("{SESSIONS}/{id}"), ONE, headers)
    }

    /// The web session `cookie` ends the session `id`, from a page of this server.
    pub fn end(&self, cookie: &str, id: &str) -> Response {
        self.end_with(id, &self.page_headers(cookie))
    }

    /// `POST /account/sessions/end-others` with `headers`.
    pub fn end_others_with(&self, headers: &[(&str, &str)]) -> Response {
        self.call("post", END_OTHERS, END_OTHERS, headers)
    }

    /// The web session `cookie` ends every other session of its account, from a page of this
    /// server.
    pub fn end_others(&self, cookie: &str) -> Response {
        self.end_others_with(&self.page_headers(cookie))
    }

    /// `DELETE /session` with `headers`.
    pub fn sign_out_with(&self, headers: &[(&str, &str)]) -> Response {
        self.call("delete", SESSION, SESSION, headers)
    }

    /// The web session `cookie` signs itself out, from a page of this server.
    pub fn sign_out(&self, cookie: &str) -> Response {
        self.sign_out_with(&self.page_headers(cookie))
    }

    /// Whether the server takes the session that `headers` carry: `GET /session` answers `200`,
    /// or `401` when it does not.
    #[track_caller]
    fn takes(&self, headers: &[(&str, &str)]) -> bool {
        let response = self.call("get", SESSION, SESSION, headers);
        match response.status {
            200 => true,
            401 => false,
            _ => panic!("GET /session: {}", text(&response)),
        }
    }

    /// Whether the server takes the web session `cookie`.
    #[track_caller]
    pub fn web_works(&self, cookie: &str) -> bool {
        self.takes(&[("cookie", cookie)])
    }

    /// Whether the server takes the device session `token`, on `GET /session` and on each other
    /// request a machine reads with it: all of them or none.
    #[track_caller]
    pub fn device_works(&self, token: &str) -> bool {
        let bearer = format!("Bearer {token}");
        let headers = [("authorization", bearer.as_str())];
        let session = self.takes(&headers);
        for path in ["/devices", "/account/master"] {
            let status = self.call("get", path, path, &headers).status;
            assert_eq!(status == 200, session, "GET {path}: {status}");
        }
        session
    }

    /// Moves the sign-in of the session of `token` back by `interval`, a PostgreSQL interval such
    /// as `30 days`.
    pub fn move_sign_in_back(&self, token: &str, interval: &str) {
        self.db.execute(&format!(
            "UPDATE sessions SET created_at = created_at - interval '{interval}'
             WHERE token_hash = sha256(convert_to('{token}', 'UTF8'))"
        ));
    }
}

/// Seconds since the Unix epoch of `text`, an RFC 3339 date-time, as PostgreSQL reads it.
pub fn epoch(db: &TestDb, text: &str) -> i64 {
    let bytes = text.as_bytes();
    let zone = text.ends_with(['Z', 'z'])
        || (bytes.len() > 6 && matches!(bytes[bytes.len() - 6], b'+' | b'-'));
    assert!(
        bytes.len() >= 20 && matches!(bytes[10], b'T' | b't') && zone,
        "an RFC 3339 date-time: {text:?}"
    );
    let seconds: f64 = db
        .query(async |conn| {
            sqlx::query_scalar("SELECT extract(epoch FROM $1::timestamptz)::float8")
                .bind(text)
                .fetch_one(conn)
                .await
        })
        .unwrap_or_else(|err| panic!("{text:?}: {err}"));
    seconds.floor() as i64
}

/// Signs `user` in on `flow`'s server by plain HTTP, the browser's `User-Agent` `agent`, as a
/// browser follows the redirects; gives the `Cookie` header of the web session.
pub fn web_sign_in(flow: &DeviceFlow, user: &User, agent: &str) -> String {
    let stand = &flow.stand;
    let started = get_url(
        &format!("{}{START}", stand.server.base()),
        &[("user-agent", agent)],
    );
    assert_eq!(started.status, 302, "{}", text(&started));
    let authorize = started.header("location").expect("a redirect to GitHub");
    let back = provider_back(&stand.github.sign_in_url(user.id), authorize);
    let callback = back.replacen(&stand.public, stand.server.base(), 1);
    let state = cookie_set(&started, "skald_sign_in");
    let signed_in = get_url(&callback, &[("cookie", &state), ("user-agent", agent)]);
    assert_eq!(signed_in.status, 200, "{}", text(&signed_in));
    cookie_set(&signed_in, COOKIE)
}

/// A machine of `user`'s account on `flow`'s server: its device session token and the device it
/// registered, certified by the account's master key, named `name`.
pub struct Laptop {
    pub token: String,
    pub device: Device,
}

impl Laptop {
    /// Signs a machine in to `user`'s account by the device flow and registers its device.
    pub fn sign_in(flow: &DeviceFlow, user: &User, name: &str) -> Self {
        let signed_in = machine(flow, user);
        let device = Device::new(name);
        signed_in.register_device(&device);
        let master = Key::generate();
        let cert = device.cert(&signed_in.account, &master);
        assert_taken(&signed_in.give_master(&master, &cert));
        Laptop {
            token: signed_in.token,
            device,
        }
    }
}

/// How long a test waits for a connection's answer.
const ANSWER_WAIT: Duration = Duration::from_secs(20);

/// A device's WebSocket to an instance, proved and caught up.
pub struct Connection {
    socket: WebSocket,
}

impl Connection {
    /// Connects `device` to `server`, whose public address is `public`, on a challenge of the
    /// device session `token`, and states cursor 0; returns once the server has ended catch-up.
    pub fn open(server: &Server, public: &str, token: &str, device: &Device) -> Self {
        let bearer = format!("Bearer {token}");
        let version = PROTOCOL_VERSION.to_string();
        let asked = send(
            "post",
            &format!("{}{CHALLENGE}", server.base()),
            &[("authorization", &bearer), (VERSION_HEADER, &version)],
            &[],
        );
        assert_eq!(asked.status, 200, "{}", text(&asked));
        let challenge = unb64(json(&asked)["challenge"].as_str().expect("a challenge"));
        let protocol = skald_proto::subprotocol(PROTOCOL_VERSION);
        let opened = server.open_websocket("/ws", Some(&protocol));
        let mut connection = Connection {
            socket: opened
                .map(|(socket, _)| socket)
                .unwrap_or_else(|refused| panic!("the WebSocket is refused: {}", text(&refused))),
        };
        if let MaybeTlsStream::Plain(stream) = connection.socket.get_ref() {
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .expect("a read timeout");
        }
        let proof = auth_signature(&device.signing.key, &challenge, public);
        for frame in [
            auth_frame(&challenge, device.id, &proof),
            array_of(&[uint(4), uint(0)]),
        ] {
            connection
                .socket
                .send(Message::Binary(frame.into()))
                .expect("a frame sent");
        }
        let started = Instant::now();
        loop {
            match connection.read(started + ANSWER_WAIT) {
                Ok(Some(Message::Binary(_))) => return connection,
                Ok(Some(Message::Close(frame))) => panic!("closed: {:?}", Closed::of(frame)),
                Ok(Some(_)) => {}
                Ok(None) => panic!("no end of catch-up within {ANSWER_WAIT:?}"),
                Err(err) => panic!("the connection failed: {err}"),
            }
        }
    }

    /// The next message by `deadline`, `None` if none came.
    fn read(&mut self, deadline: Instant) -> Result<Option<Message>, tungstenite::Error> {
        use std::io::ErrorKind::{TimedOut, WouldBlock};
        while Instant::now() <= deadline {
            match self.socket.read() {
                Err(tungstenite::Error::Io(err))
                    if [WouldBlock, TimedOut].contains(&err.kind()) => {}
                read => return read.map(Some),
            }
        }
        Ok(None)
    }

    /// How the server closed the connection within `within`; `None` if it is still open.
    pub fn closed_within(&mut self, within: Duration) -> Option<Closed> {
        let deadline = Instant::now() + within;
        loop {
            match self.read(deadline) {
                Ok(Some(Message::Close(frame))) => return Some(Closed::of(frame)),
                Ok(Some(_)) => {}
                Ok(None) => return None,
                Err(_) => return Some(Closed::Dropped),
            }
        }
    }
}

/// Checks that `id`, a session's ID, reveals nothing of the session's `token`: neither holds the
/// other, nor does the ID hold the token's bytes or their SHA-256, in hex or base64url.
#[track_caller]
pub fn assert_reveals_nothing(id: &str, token: &str) {
    let digest = Sha256::digest(token.as_bytes());
    let id_lower = id.to_lowercase();
    let mut forms = vec![
        token.to_lowercase(),
        hex(&digest),
        b64(&digest).to_lowercase(),
    ];
    forms.extend(URL_SAFE_NO_PAD.decode(token).ok().map(|bytes| hex(&bytes)));
    for form in &forms {
        assert!(!id_lower.contains(form.as_str()), "{id} holds {form}");
    }
    assert!(!token.contains(id), "the token holds the ID {id}");
}

/// The JSON keys that would name an IP address or a location.
const PLACES: [&str; 8] = [
    "ip",
    "ip_address",
    "address",
    "location",
    "city",
    "country",
    "region",
    "geo",
];

/// Checks that `body`, a list of sessions, names no IP address or location.
#[track_caller]
pub fn assert_no_place(body: &Value) {
    let text = body.to_string();
    assert!(!text.contains("127.0.0.1"), "an IP address: {text}");
    for item in body["sessions"].as_array().into_iter().flatten() {
        let keys: Vec<&String> = item
            .as_object()
            .into_iter()
            .flat_map(|o| o.keys())
            .collect();
        for key in keys {
            assert!(!PLACES.contains(&key.as_str()), "{key} in {item}");
        }
    }
}
