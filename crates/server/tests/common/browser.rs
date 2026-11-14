//! The person's browser: headless Chrome driven by playwright-core through `browser.mjs`, so cookies
//! act as in a browser. Needs Node.js, `npm ci` in `web/` and Google Chrome, or `SKALD_CHROME`.
//! The client's tests include this file too, so its paths start from the `crates/` directory.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The device flow's verification page as the person finds their way on it: its field for the
/// code, the buttons that approve and deny the code, the button of a page that asks for the code
/// first, and a link or button that signs a browser without a session in: any, or the one of each
/// provider, whose text names it.
pub const CODE_FIELD: &str = "role=textbox";
pub const APPROVE: &str = "role=button[name=/approve/i]";
pub const DENY: &str = "role=button[name=/deny/i]";
pub const SUBMIT: &str = "role=button >> nth=0";
pub const SIGN_IN_LINK: &str = "a:has-text('sign in'), button:has-text('sign in')";
pub const SIGN_IN_GITHUB: &str = "a:has-text('GitHub'), button:has-text('GitHub')";
pub const SIGN_IN_GOOGLE: &str = "a:has-text('Google'), button:has-text('Google')";

/// The driver process; closed on drop, which closes Chrome.
pub struct Browser {
    driver: Mutex<Driver>,
}

/// How long the driver may take to answer: past every timeout of playwright-core, the 180 s that
/// `browser.mjs` gives Chrome to start included.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(200);

struct Driver {
    child: Child,
    stdin: Option<ChildStdin>,
    /// The driver's stdout, line by line, read by a thread of its own.
    answers: Receiver<String>,
    next: u64,
}

impl Driver {
    fn answer(&self) -> String {
        self.answers
            .recv_timeout(ANSWER_TIMEOUT)
            .unwrap_or_else(|err| panic!("the browser driver did not answer: {err}"))
    }
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
    /// Starts the driver and Chrome.
    pub fn launch() -> Self {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut child = Command::new("node")
            .arg(crates.join("server/tests/common/browser.mjs"))
            .arg(crates.join("../web"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start node: the browser tests need Node.js");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let (send, answers) = mpsc::channel();
        thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let driver = Driver {
            child,
            stdin,
            answers,
            next: 0,
        };
        assert_eq!(
            driver.answer(),
            r#"{"ready":true}"#,
            "the browser driver did not start; its stderr says why"
        );
        Browser {
            driver: Mutex::new(driver),
        }
    }

    fn call(&self, op: &str, mut args: Value) -> Value {
        let mut driver = self.driver.lock().expect("browser driver");
        driver.next += 1;
        let id = driver.next;
        args["id"] = json!(id);
        args["op"] = json!(op);
        let stdin = driver.stdin.as_mut().expect("driver stdin is open");
        writeln!(stdin, "{args}").expect("write to the browser driver");
        let line = driver.answer();
        let mut answer: Value = serde_json::from_str(&line)
            .unwrap_or_else(|err| panic!("browser driver answer {line:?}: {err}"));
        assert_eq!(answer["id"], json!(id), "browser driver answer {line:?}");
        if let Some(error) = answer.get("error") {
            panic!("browser {op}: {error}");
        }
        answer["ok"].take()
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
        // A test that failed in a call leaves the lock poisoned; the driver still has to go.
        let driver = self
            .driver
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Closing stdin closes Chrome and the driver.
        drop(driver.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = driver.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = driver.child.kill();
        let _ = driver.child.wait();
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

    /// Clicks the element `selector` finds as [`Page::click`] does, then follows each page that
    /// refreshes itself as [`Page::goto_settled`] does. The visit's status stays that of the click's
    /// navigation.
    pub fn click_settled(&self, selector: &str) -> Visit {
        self.visit("click", json!({ "selector": selector, "settle": true }))
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
