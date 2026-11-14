//! The person's browser: Chrome Headless Shell driven by playwright-core through `browser.mjs`, so
//! cookies act as in a browser. One driver and one browser serve all tests of a test binary, and
//! each page is a browser context of its own. Needs Node.js, `npm ci` in `web/` and Chrome Headless
//! Shell of its playwright-core, or `SKALD_CHROME`: CONTRIBUTING.md.
//! The client's tests include this file too, so its paths start from the `crates/` directory.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The device flow's verification page as the person finds their way on it: its field for the
/// code, the buttons that approve and deny the code, the button of a page that asks for the code
/// first, and a link or button that signs a browser without a session in.
pub const CODE_FIELD: &str = "role=textbox";
pub const APPROVE: &str = "role=button[name=/approve/i]";
pub const DENY: &str = "role=button[name=/deny/i]";
pub const SUBMIT: &str = "role=button >> nth=0";
pub const SIGN_IN_LINK: &str = "a:has-text('sign in'), button:has-text('sign in')";

/// A test's share of the browser: the pages and servers it opens, closed on drop.
pub struct Browser {
    driver: &'static Driver,
    session: u64,
}

/// How long the driver may take to answer: past every timeout of playwright-core, the 180 s that
/// `browser.mjs` gives the browser to start included.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(200);

/// The driver of this test binary, started by its first browser test. It never stops before the
/// binary does: the driver exits once the binary has, and its browser with it.
static DRIVER: OnceLock<Result<Driver, String>> = OnceLock::new();

struct Driver {
    stdin: Mutex<ChildStdin>,
    waiting: Arc<Mutex<Waiting>>,
    next_call: AtomicU64,
    next_session: AtomicU64,
}

/// Where to send the answer to each call, by its ID; the thread that reads the driver's stdout takes
/// them out. `None` once the driver's stdout has closed.
type Waiting = Option<HashMap<u64, Sender<String>>>;

/// The ID of the driver's first line, which has none.
const READY: u64 = 0;

/// The lock, also when a test that panicked held it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Driver {
    fn start() -> Result<Self, String> {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut child = Command::new("node")
            .arg(crates.join("server/tests/common/browser.mjs"))
            .arg(crates.join("../web"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| format!("start node: the browser tests need Node.js: {err}"))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let (ready, started) = mpsc::channel();
        let waiting = Arc::new(Mutex::new(Some(HashMap::from([(READY, ready)]))));
        let answers = Arc::clone(&waiting);
        thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                let id = serde_json::from_str::<Value>(&line)
                    .ok()
                    .and_then(|answer| answer["id"].as_u64())
                    .unwrap_or(READY);
                let found = lock(&answers).as_mut().and_then(|calls| calls.remove(&id));
                if let Some(send) = found {
                    let _ = send.send(line);
                }
            }
            // Every call that waits fails now, and so does every later one.
            lock(&answers).take();
        });
        match started.recv_timeout(ANSWER_TIMEOUT) {
            Ok(line) if line == r#"{"ready":true}"# => Ok(Driver {
                stdin: Mutex::new(stdin),
                waiting,
                next_call: AtomicU64::new(READY + 1),
                next_session: AtomicU64::new(1),
            }),
            Ok(line) => Err(format!("the browser driver started with {line:?}")),
            Err(err) => Err(format!(
                "the browser driver did not start ({err}); its stderr says why"
            )),
        }
    }

    /// Sends the call `args` and waits for its answer.
    fn call(&self, mut args: Value) -> Result<Value, String> {
        let id = self.next_call.fetch_add(1, Ordering::Relaxed);
        args["id"] = json!(id);
        let (send, answer) = mpsc::channel();
        lock(&self.waiting)
            .as_mut()
            .ok_or("the browser driver exited; its stderr says why")?
            .insert(id, send);
        writeln!(lock(&self.stdin), "{args}")
            .map_err(|err| format!("write to the browser driver: {err}"))?;
        let line = answer.recv_timeout(ANSWER_TIMEOUT).map_err(|err| {
            lock(&self.waiting).as_mut().map(|calls| calls.remove(&id));
            format!("the browser driver did not answer: {err}")
        })?;
        let mut answer: Value = serde_json::from_str(&line)
            .map_err(|err| format!("browser driver answer {line:?}: {err}"))?;
        match answer.get("error") {
            Some(error) => Err(error.to_string()),
            None => Ok(answer["ok"].take()),
        }
    }
}

/// The processes of the browser of this test binary.
#[derive(Debug, Deserialize, Serialize)]
pub struct Processes {
    /// The driver's.
    pub driver: u32,
    /// The browser's own first, then its helpers'.
    pub browser: Vec<u32>,
    /// The program the browser runs.
    pub program: String,
}

/// What a navigation gave.
#[derive(Debug, Deserialize)]
pub struct Visit {
    /// The page's URL afterwards; `about:blank` when its page failed to load.
    pub url: String,
    /// The status of the last response of the navigation itself, after its redirects and before any
    /// followed refresh, if any.
    pub status: Option<u16>,
    /// Every URL the page requested on the way, redirects and followed refreshes included, in
    /// order.
    pub navigations: Vec<String>,
    /// Why the navigation failed, if it did.
    pub error: Option<String>,
}

impl Visit {
    /// The last URL the page requested: where the navigation ended, also when its page failed to
    /// load, such as an empty `404`.
    pub fn last(&self) -> &str {
        self.navigations.last().map_or(&self.url, String::as_str)
    }
}

/// What a button submits: its form's action and method, its own if it overrides them, and the
/// form's fields with the button's own.
#[derive(Clone, Debug, Deserialize)]
pub struct Submission {
    pub action: String,
    /// `get` or `post`.
    pub method: String,
    pub fields: Vec<(String, String)>,
}

/// A cookie in a browser's jar, as playwright-core gives it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: f64,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: String,
}

impl Cookie {
    /// The cookie as a `Cookie` request header carries it.
    pub fn pair(&self) -> String {
        format!("{}={}", self.name, self.value)
    }
}

/// A request an echo server got.
#[derive(Clone, Debug, Deserialize)]
pub struct Hit {
    pub method: String,
    pub host: String,
    pub url: String,
    pub cookie: Option<String>,
}

impl Hit {
    /// Whether the request carried `cookie`.
    pub fn carries(&self, cookie: &Cookie) -> bool {
        self.cookie
            .as_deref()
            .is_some_and(|header| header.split("; ").any(|pair| pair == cookie.pair()))
    }
}

impl Browser {
    /// A share of the browser of this test binary; the first starts the driver and the browser.
    pub fn launch() -> Self {
        let driver = DRIVER
            .get_or_init(Driver::start)
            .as_ref()
            .unwrap_or_else(|err| panic!("{err}"));
        Browser {
            driver,
            session: driver.next_session.fetch_add(1, Ordering::Relaxed),
        }
    }

    fn call(&self, op: &str, args: Value) -> Value {
        self.try_call(op, args)
            .unwrap_or_else(|err| panic!("browser {op}: {err}"))
    }

    fn try_call(&self, op: &str, mut args: Value) -> Result<Value, String> {
        args["op"] = json!(op);
        args["session"] = json!(self.session);
        self.driver.call(args)
    }

    /// The processes of the browser.
    pub fn processes(&self) -> Processes {
        serde_json::from_value(self.call("processes", json!({}))).expect("processes")
    }

    /// A page in a browser with no cookies: another person's browser, or the same person's new one.
    pub fn page(&self) -> Page<'_> {
        let id = self.call("newPage", json!({}));
        Page { browser: self, id }
    }

    /// Starts an echo server, over HTTPS with `tls`; gives its port. It answers any host, any path.
    pub fn echo(&self, tls: bool) -> u16 {
        serde_json::from_value(self.call("echo", json!({ "tls": tls }))).expect("a port")
    }

    /// The requests the echo server on `port` got so far.
    pub fn hits(&self, port: u16) -> Vec<Hit> {
        serde_json::from_value(self.call("hits", json!({ "port": port }))).expect("hits")
    }

    /// Waits until the echo server on `port` has `count` requests whose URL contains `part`, for
    /// up to 10 s; gives them.
    pub fn wait_for_hits(&self, port: u16, part: &str, count: usize) -> Vec<Hit> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut hits = self.hits(port);
            hits.retain(|hit| hit.url.contains(part));
            if hits.len() >= count {
                return hits;
            }
            assert!(
                Instant::now() < deadline,
                "{count} requests to {part} expected, got {hits:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Clicks each `(page, selector)` of `clicks` at once, as [`Page::click`] does; gives their
    /// visits, in order.
    pub fn click_all(&self, clicks: &[(&Page, &str)]) -> Vec<Visit> {
        let clicks: Vec<Value> = clicks
            .iter()
            .map(|(page, selector)| json!({ "page": page.id, "selector": selector }))
            .collect();
        serde_json::from_value(self.call("clickAll", json!({ "clicks": clicks }))).expect("visits")
    }

    /// Starts an HTTPS server that forwards every request to `target`, e.g.
    /// `http://127.0.0.1:41234`; gives its port.
    pub fn tls_proxy(&self, target: &str) -> u16 {
        serde_json::from_value(self.call("tlsProxy", json!({ "target": target }))).expect("a port")
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Also after a test failed: nothing to do when the driver is gone.
        let _ = self.try_call("close", json!({}));
    }
}

/// A page in a browser of its own.
pub struct Page<'a> {
    browser: &'a Browser,
    id: Value,
}

impl Page<'_> {
    /// Navigates to `url` as a person typing it, and waits for the page to load.
    pub fn goto(&self, url: &str) -> Visit {
        self.goto_with(url, false)
    }

    /// Navigates to `url` as [`Page::goto`] does, then follows each page that refreshes itself
    /// with `<meta http-equiv="refresh">`, such as the sign-in callback's, until a page without one
    /// has loaded. The visit's status stays that of the navigation to `url`.
    pub fn goto_settled(&self, url: &str) -> Visit {
        self.goto_with(url, true)
    }

    fn goto_with(&self, url: &str, settle: bool) -> Visit {
        let visit = self.browser.call(
            "goto",
            json!({ "page": self.id, "url": url, "settle": settle }),
        );
        serde_json::from_value(visit).expect("a visit")
    }

    /// Clicks the element `selector` finds, a playwright-core selector such as
    /// `role=button[name=/approve/i]`, as a person does, and waits for the navigation it starts.
    pub fn click(&self, selector: &str) -> Visit {
        self.visit("click", json!({ "selector": selector }))
    }

    /// Evaluates the script `expression`, which starts a navigation of the page, and waits until
    /// the network has been idle for 0.5 s.
    pub fn navigate_by(&self, expression: &str) -> Visit {
        self.visit("navigateBy", json!({ "expression": expression }))
    }

    fn visit(&self, op: &str, mut args: Value) -> Visit {
        args["page"] = self.id.clone();
        serde_json::from_value(self.browser.call(op, args)).expect("a visit")
    }

    /// Types `value` into the element `selector` finds, replacing what it holds.
    pub fn fill(&self, selector: &str, value: &str) {
        self.browser.call(
            "fill",
            json!({ "page": self.id, "selector": selector, "value": value }),
        );
    }

    /// How many elements `selector` finds.
    pub fn count(&self, selector: &str) -> usize {
        let count = self
            .browser
            .call("count", json!({ "page": self.id, "selector": selector }));
        serde_json::from_value(count).expect("a count")
    }

    /// The value of each form field `selector` finds.
    pub fn values(&self, selector: &str) -> Vec<String> {
        let values = self
            .browser
            .call("values", json!({ "page": self.id, "selector": selector }));
        serde_json::from_value(values).expect("values")
    }

    /// The page's text as a person sees it.
    pub fn text(&self) -> String {
        let text = self.browser.call("text", json!({ "page": self.id }));
        serde_json::from_value(text).expect("a text")
    }

    /// What the button `selector` finds submits.
    pub fn submission(&self, selector: &str) -> Submission {
        let submission = self.browser.call(
            "submission",
            json!({ "page": self.id, "selector": selector }),
        );
        serde_json::from_value(submission).expect("a submission")
    }

    /// Evaluates the script `expression` in the page; a promise is awaited.
    pub fn evaluate(&self, expression: &str) -> Value {
        self.browser.call(
            "evaluate",
            json!({ "page": self.id, "expression": expression }),
        )
    }

    /// Every cookie in this browser's jar.
    pub fn cookies(&self) -> Vec<Cookie> {
        serde_json::from_value(self.browser.call("cookies", json!({ "page": self.id })))
            .expect("cookies")
    }

    /// The cookies in this browser's jar for `host`.
    pub fn cookies_of(&self, host: &str) -> Vec<Cookie> {
        let mut cookies = self.cookies();
        cookies.retain(|cookie| cookie.domain.trim_start_matches('.') == host);
        cookies
    }

    pub fn add_cookies(&self, cookies: &[Cookie]) {
        self.browser
            .call("addCookies", json!({ "page": self.id, "cookies": cookies }));
    }
}
