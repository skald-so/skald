//! The local page of `skald sign-in --browser` and `skald link --browser` (`docs/dev/architecture.md`,
//! "Device linking"): the person takes there the steps of a link code, and reads a recovery key,
//! that the command would take at a terminal, so none of them passes through an agent that runs
//! the command.
//!
//! - The page listens on 127.0.0.1 only, on a port the system picks, while its command runs. It
//!   answers only a request whose query holds its one-time secret, whose Host is the page's own
//!   and whose Origin, if any, is the page's own; every answer carries `Cache-Control: no-store`.
//! - `skald` never prints the secret: it writes a file to the state directory, readable only by its
//!   user, that takes the browser to the page, and opens that file in the browser; the file is gone
//!   once the command ends.
//! - The page shows the command's output, what only its person may see, such as a link code, and
//!   its person's next question, whose answer it sends back. An agent's run shows its warnings
//!   there only, and keeps them for the person's run, as other runs of an agent do
//!   ([`crate::recv::holding_warnings`]).

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::output::{self, Stream};

/// The setting that names the program that opens the browser.
const BROWSER: &str = "BROWSER";
/// The page's path, and those of its view and of its person's answers.
const PAGE_PATH: &str = "/skald";
const VIEW_PATH: &str = "/skald/view";
const ANSWER_PATH: &str = "/skald/answer";
/// The query parameter that holds the secret.
const SECRET: &str = "s";
/// The most a request's head or body may hold.
const REQUEST_MAX: usize = 16 * 1024;
/// How long a request for the view waits for it to change.
const VIEW_WAIT: Duration = Duration::from_secs(10);
/// How long the end of a command waits for the page to take its last view.
const LAST_VIEW_WAIT: Duration = Duration::from_secs(2);
/// How long `skald` waits for the program that opens the browser to say it failed.
const OPENER_WAIT: Duration = Duration::from_secs(2);
/// How often the listener looks for a connection, and for its end.
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// What a command says without a browser to open.
pub const NO_BROWSER: &str =
    "no browser for the local page: run this at a terminal, without --browser";
/// What a command says once the program that opens the browser failed.
pub const NOT_OPENED: &str =
    "the browser did not open the local page: run this at a terminal, without --browser";

/// The program that opens a file in the browser.
pub struct Opener {
    program: PathBuf,
    /// The arguments before the file's path.
    before: Vec<OsString>,
    /// Whether the program's exit code says whether it opened the file.
    exits_failing: bool,
}

impl Opener {
    /// The program `BROWSER` names, or the system's own; fails, before anything listens or is
    /// sent, without one.
    pub fn find() -> Result<Self, String> {
        let named = env::var_os(BROWSER).filter(|named| !named.is_empty());
        let found = match named {
            Some(named) => program(Path::new(&named)).map(|program| Opener {
                program,
                before: Vec::new(),
                exits_failing: true,
            }),
            None => system(),
        };
        found.ok_or_else(|| NO_BROWSER.to_owned())
    }

    /// Opens `file` in the browser; fails if the program does not start, or exits failing soon.
    fn open(&self, file: &Path) -> Result<(), String> {
        let mut child = Command::new(&self.program)
            .args(&self.before)
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| NOT_OPENED.to_owned())?;
        let ends = Instant::now() + OPENER_WAIT;
        while Instant::now() < ends {
            match child.try_wait() {
                Ok(Some(status)) if self.exits_failing && !status.success() => {
                    return Err(NOT_OPENED.to_owned());
                }
                Ok(Some(_)) => return Ok(()),
                Ok(None) => thread::sleep(ACCEPT_POLL),
                Err(_) => break,
            }
        }
        // A browser that keeps running is reaped once it exits.
        thread::spawn(move || child.wait());
        Ok(())
    }
}

/// The program `named` names: a path to it, or a name to look up on `PATH`.
pub fn program(named: &Path) -> Option<PathBuf> {
    if named.components().count() > 1 {
        return named.is_file().then(|| named.to_owned());
    }
    let extensions: Vec<String> = if cfg!(windows) && named.extension().is_none() {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_owned())
            .split(';')
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|dir| {
        extensions.iter().find_map(|extension| {
            let mut candidate = dir.join(named).into_os_string();
            candidate.push(extension);
            let candidate = PathBuf::from(candidate);
            candidate.is_file().then_some(candidate)
        })
    })
}

/// The system's program that opens a file in the default browser, if it has one.
fn system() -> Option<Opener> {
    let opener = |program, before: &[&str], exits_failing| Opener {
        program,
        before: before.iter().map(OsString::from).collect(),
        exits_failing,
    };
    if cfg!(target_os = "macos") {
        program(Path::new("/usr/bin/open")).map(|found| opener(found, &[], true))
    } else if cfg!(windows) {
        // Explorer opens a file with its default program, and exits failing either way.
        program(Path::new("explorer.exe")).map(|found| opener(found, &[], false))
    } else {
        let graphical = ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|name| env::var_os(name).is_some_and(|value| !value.is_empty()));
        graphical
            .then(|| program(Path::new("xdg-open")))
            .flatten()
            .map(|found| opener(found, &[], true))
    }
}

/// A person's answer to a question of the page.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// The button at this place, in the question's order.
    Button(usize),
    /// The text entered in the question's field.
    Text(String),
}

/// A question the page asks.
struct Asked {
    id: u64,
    text: String,
    buttons: Vec<String>,
    /// The label of its text field, if it has one.
    field: Option<String>,
    /// Whether it takes one answer, or one per press.
    once: bool,
    answers: Sender<Answer>,
}

/// What the page shows.
#[derive(Default)]
struct View {
    /// Raised with every change; the browser's first request asks past 0.
    version: u64,
    /// The command's output, in lines, and the start of its next one.
    lines: Vec<String>,
    partial: String,
    /// What only the person sees.
    shown: Option<String>,
    question: Option<Asked>,
    asked: u64,
    /// Whether the command ended.
    ended: bool,
    /// The latest version the browser took.
    taken: u64,
    /// Each version, as the browser would take it.
    #[cfg(test)]
    versions: Vec<serde_json::Value>,
}

impl View {
    fn changed(&mut self) {
        self.version += 1;
        #[cfg(test)]
        {
            let version = serde_json::from_str(&self.json()).expect("the view's JSON");
            self.versions.push(version);
        }
    }

    /// The view as the browser takes it.
    fn json(&self) -> String {
        let question = self.question.as_ref().map(|asked| {
            json!({
                "id": asked.id,
                "text": asked.text,
                "buttons": asked.buttons,
                "field": asked.field,
                "once": asked.once,
            })
        });
        json!({
            "version": self.version,
            "lines": self.lines,
            "shown": self.shown,
            "question": question,
            "ended": self.ended,
        })
        .to_string()
    }
}

/// The view, shared with the page's server.
#[derive(Default)]
struct Shared {
    view: Mutex<View>,
    changed: Condvar,
    /// The warnings an agent's run held back from its output.
    held: Mutex<Vec<String>>,
}

impl Shared {
    fn view(&self) -> MutexGuard<'_, View> {
        self.view.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds `bytes` of the command's output to the view.
    fn log(&self, bytes: &[u8]) {
        let mut view = self.view();
        view.partial.push_str(&String::from_utf8_lossy(bytes));
        while let Some(end) = view.partial.find('\n') {
            let line: String = view.partial.drain(..=end).collect();
            let line = line.trim_end_matches(['\r', '\n']).to_owned();
            view.lines.push(line);
        }
        view.changed();
        self.changed.notify_all();
    }
}

/// The page of a command whose state directory is `dir`, opened at its first question or at the
/// first thing it shows, so that a command with nothing to ask opens none.
pub struct Page {
    dir: PathBuf,
    opener: Opener,
    shared: Arc<Shared>,
    server: Mutex<Option<Server>>,
}

/// The page's server while it listens, and the file that takes the browser to it.
struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    file: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.file);
    }
}

impl Page {
    /// The page of the command whose state directory is `dir`, opened in the browser by `opener`.
    pub fn new(dir: &Path, opener: Opener) -> Self {
        Page {
            dir: dir.to_owned(),
            opener,
            shared: Arc::default(),
            server: Mutex::new(None),
        }
    }

    /// What takes the command's output: the page, and its own output, but for a warning of an
    /// `agent`'s run, which it holds back ([`Page::held`]).
    pub fn tee(&self, agent: bool) -> impl FnMut(Stream, &[u8]) -> io::Result<()> + 'static {
        let shared = Arc::clone(&self.shared);
        move |stream, bytes| {
            shared.log(bytes);
            if agent && stream == Stream::Err && bytes.starts_with(output::WARNING.as_bytes()) {
                let warning = String::from_utf8_lossy(bytes);
                let mut held = shared.held.lock().unwrap_or_else(PoisonError::into_inner);
                held.push(warning.trim_end_matches(['\r', '\n']).to_owned());
                return Ok(());
            }
            output::direct(stream, bytes)
        }
    }

    /// The warnings [`Page::tee`] held back so far, each without its line's end.
    pub fn held(&self) -> Vec<String> {
        let mut held = self
            .shared
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *held)
    }

    /// Opens the page in the browser, if it is not open.
    pub fn open(&self) -> Result<(), String> {
        let mut server = self.server.lock().unwrap_or_else(PoisonError::into_inner);
        if server.is_none() {
            *server = Some(self.start()?);
        }
        Ok(())
    }

    /// Listens, writes the file that takes the browser to the page and opens it.
    fn start(&self) -> Result<Server, String> {
        let cannot = |err: io::Error| format!("cannot open the local page: {err}");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(cannot)?;
        let port = listener.local_addr().map_err(cannot)?.port();
        listener.set_nonblocking(true).map_err(cannot)?;
        let mut secret = [0; 32];
        getrandom::fill(&mut secret).map_err(|err| format!("cannot open the local page: {err}"))?;
        let guard = Guard {
            authority: format!("127.0.0.1:{port}"),
            secret: URL_SAFE_NO_PAD.encode(secret),
        };
        let url = format!(
            "http://{}{PAGE_PATH}?{SECRET}={}",
            guard.authority, guard.secret
        );
        let file = write_file(&self.dir, &url).map_err(cannot)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (stop, shared) = (Arc::clone(&stop), Arc::clone(&self.shared));
            thread::spawn(move || listen(&listener, &Arc::new(guard), &shared, &stop))
        };
        let server = Server {
            stop,
            thread: Some(thread),
            file,
        };
        self.opener.open(&server.file)?;
        // Not on the page: it is the agent's to read.
        let _ = output::direct(
            Stream::Out,
            b"skald opened a local page in the browser: its person answers there.\n",
        );
        Ok(server)
    }

    /// Shows `text` to the person in place of what the page showed, opening the page; `None`
    /// shows nothing.
    pub fn show(&self, text: Option<&str>) -> Result<(), String> {
        if text.is_some() {
            self.open()?;
        }
        let mut view = self.shared.view();
        view.shown = text.map(str::to_owned);
        view.changed();
        self.shared.changed.notify_all();
        Ok(())
    }

    /// Asks `text` with `buttons` and, if `field` names one, a text field, in place of the question
    /// before, opening the page; gives its answer, or each, unless `once`.
    pub fn ask(
        &self,
        text: &str,
        buttons: &[&str],
        field: Option<&str>,
        once: bool,
    ) -> Result<Receiver<Answer>, String> {
        self.asking(None, text, buttons, field, once)
    }

    /// Shows `shown` and asks `text` with `buttons`, each in place of what the page showed and
    /// asked before, in one change, opening the page; gives its one answer. The browser never
    /// takes `shown` under the question before.
    pub fn show_asking(
        &self,
        shown: &str,
        text: &str,
        buttons: &[&str],
    ) -> Result<Receiver<Answer>, String> {
        self.asking(Some(shown), text, buttons, None, true)
    }

    /// [`Page::ask`], showing `shown`, if any, in the same change.
    fn asking(
        &self,
        shown: Option<&str>,
        text: &str,
        buttons: &[&str],
        field: Option<&str>,
        once: bool,
    ) -> Result<Receiver<Answer>, String> {
        self.open()?;
        let (answers, answered) = mpsc::channel();
        let mut view = self.shared.view();
        if let Some(shown) = shown {
            view.shown = Some(shown.to_owned());
        }
        view.asked += 1;
        view.question = Some(Asked {
            id: view.asked,
            text: text.to_owned(),
            buttons: buttons.iter().map(|button| (*button).to_owned()).collect(),
            field: field.map(str::to_owned),
            once,
            answers,
        });
        view.changed();
        self.shared.changed.notify_all();
        Ok(answered)
    }

    /// Ends the page once the command ends: shows `error`, the command's, if any, and that it
    /// ended, waits a little for the browser to take that, then stops listening and deletes the
    /// file.
    pub fn finish(&self, error: Option<&str>) {
        let mut server = self.server.lock().unwrap_or_else(PoisonError::into_inner);
        if server.is_none() {
            return;
        }
        let mut view = self.shared.view();
        if let Some(err) = error {
            view.lines.push(format!("skald: {err}"));
        }
        view.shown = None;
        view.question = None;
        view.ended = true;
        view.changed();
        self.shared.changed.notify_all();
        let ends = Instant::now() + LAST_VIEW_WAIT;
        while view.taken < view.version {
            let now = Instant::now();
            if now >= ends {
                break;
            }
            view = (self.shared.changed.wait_timeout(view, ends - now))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        drop(view);
        server.take();
    }
}

#[cfg(test)]
impl Page {
    /// A page in `dir` that is open already, with nothing listening or run.
    pub fn open_for_tests(dir: &Path) -> Self {
        let opener = Opener {
            program: PathBuf::new(),
            before: Vec::new(),
            exits_failing: true,
        };
        let page = Page::new(dir, opener);
        *page.server.lock().unwrap() = Some(Server {
            stop: Arc::default(),
            thread: None,
            file: dir.join("page.html"),
        });
        page
    }

    /// A page in `dir`, open already, as the link code step leaves it: showing a link code under
    /// its "Fresh code" question, whose presses it gives.
    pub fn showing_a_link_code_for_tests(dir: &Path) -> (Self, Receiver<Answer>) {
        let page = Page::open_for_tests(dir);
        let presses = page.ask("", &["Fresh code"], None, false).unwrap();
        page.show(Some("  ABCD-EFGH-JKMN")).unwrap();
        (page, presses)
    }

    /// Each version of the view so far, as the browser would take it.
    pub fn versions(&self) -> Vec<serde_json::Value> {
        self.shared.view().versions.clone()
    }

    /// The person presses the button `button` of the question the page asks now, if it has one;
    /// gives whether the question took it.
    pub fn press(&self, button: &str) -> bool {
        let pressed = {
            let view = self.shared.view();
            let Some(asked) = view.question.as_ref() else {
                return false;
            };
            let Some(at) = asked.buttons.iter().position(|named| named == button) else {
                return false;
            };
            Given {
                question: asked.id,
                button: at,
                text: None,
            }
        };
        take(&self.shared, &pressed)
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        let mut view = self.shared.view();
        view.ended = true;
        view.question = None;
        self.shared.changed.notify_all();
    }
}

/// A file [`open_url`] opened in the browser; deleted on drop.
pub struct Opened(PathBuf);

impl Drop for Opened {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Opens `url`, an `http` or `https` URL, in the browser through a file in `dir` that only its
/// user reads, so no process argument holds the URL. Gives the file, which stays until dropped so
/// the browser can read it; none without a browser, or if it did not open.
pub fn open_url(dir: &Path, url: &str) -> Option<Opened> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    let opener = Opener::find().ok()?;
    let file = Opened(write_file(dir, url).ok()?);
    opener.open(&file.0).ok()?;
    Some(file)
}

/// `text` with the characters that HTML gives a meaning in an attribute's value escaped.
fn escaped(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// A random name for a file of the command's own: 16 hexadecimal digits.
pub fn random_name() -> io::Result<String> {
    let mut name = [0; 8];
    getrandom::fill(&mut name).map_err(io::Error::other)?;
    Ok(name.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Writes the file that takes the browser to `url` into `dir`, readable only by its user; gives
/// its path.
fn write_file(dir: &Path, url: &str) -> io::Result<PathBuf> {
    let url = escaped(url);
    let file = std::path::absolute(dir.join(format!("page-{}.html", random_name()?)))?;
    let html = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <meta http-equiv=\"refresh\" content=\"0;url={url}\">\
         <title>skald</title></head>\
         <body><p>Opening the page of skald: <a href=\"{url}\">go there</a>.</p></body></html>\n"
    );
    skald_adapters::file::write_private(&file, html.as_bytes())?;
    Ok(file)
}

/// What a request must carry: the page's Host, its Origin or none, and its secret.
struct Guard {
    authority: String,
    secret: String,
}

impl Guard {
    /// Whether `request` carries the secret, the page's Host and its Origin or none.
    fn admits(&self, request: &Request) -> bool {
        let origin = format!("http://{}", self.authority);
        let host = request.header("host");
        let origins = request.headers("origin");
        let secrets: Vec<&str> = request.params(SECRET).collect();
        host.is_some_and(|host| host == self.authority)
            && origins.iter().all(|given| **given == origin)
            && origins.len() <= 1
            && secrets.len() == 1
            && is_secret(secrets[0], &self.secret)
    }
}

/// Whether `given` is `secret`: their SHA-256 digests are compared, so the time it takes tells
/// nothing of the secret.
fn is_secret(given: &str, secret: &str) -> bool {
    Sha256::digest(given.as_bytes()) == Sha256::digest(secret.as_bytes())
}

/// Takes connections until `stop`, each on a thread of its own.
fn listen(
    listener: &TcpListener,
    guard: &Arc<Guard>,
    shared: &Arc<Shared>,
    stop: &Arc<AtomicBool>,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let (guard, shared, stop) =
                    (Arc::clone(guard), Arc::clone(shared), Arc::clone(stop));
                thread::spawn(move || serve(stream, &guard, &shared, &stop));
            }
            Err(_) => thread::sleep(ACCEPT_POLL),
        }
    }
}

/// A request to the page.
struct Request {
    method: String,
    path: String,
    query: String,
    /// Each header's name in lowercase, and its value.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// The values of the headers named `name`, in lowercase.
    fn headers(&self, name: &str) -> Vec<&String> {
        (self.headers.iter())
            .filter(|(header, _)| header == name)
            .map(|(_, value)| value)
            .collect()
    }

    /// The value of the one header named `name`; `None` if there are none or several.
    fn header(&self, name: &str) -> Option<&str> {
        match self.headers(name)[..] {
            [value] => Some(value.as_str()),
            _ => None,
        }
    }

    /// The values of the query's parameter `name`.
    fn params<'r>(&'r self, name: &'r str) -> impl Iterator<Item = &'r str> + 'r {
        (self.query.split('&'))
            .filter_map(|pair| pair.split_once('='))
            .filter(move |(key, _)| *key == name)
            .map(|(_, value)| value)
    }
}

/// Reads a request from `stream`: its head, then its body as `Content-Length` says.
fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut read = Vec::new();
    let mut buffer = [0; 4096];
    let end = loop {
        if let Some(at) = read.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
        if read.len() > REQUEST_MAX {
            return None;
        }
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return None,
            Ok(count) => read.extend_from_slice(&buffer[..count]),
        }
    };
    let head = std::str::from_utf8(&read[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut start = lines.next()?.split(' ');
    let (method, target, version) = (start.next()?, start.next()?, start.next()?);
    if start.next().is_some() || !version.starts_with("HTTP/1.") {
        return None;
    }
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_lowercase(), value.trim().to_owned()));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut request = Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        headers,
        body: read[end + 4..].to_vec(),
    };
    let length = match request.headers("content-length")[..] {
        [] => 0,
        [length] => length.parse::<usize>().ok()?,
        _ => return None,
    };
    if length > REQUEST_MAX || request.body.len() > length {
        return None;
    }
    while request.body.len() < length {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return None,
            Ok(count) => request.body.extend_from_slice(&buffer[..count]),
        }
    }
    request.body.truncate(length);
    Some(request)
}

/// Answers one request on `stream`.
fn serve(mut stream: TcpStream, guard: &Guard, shared: &Shared, stop: &AtomicBool) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(VIEW_WAIT));
    let _ = stream.set_write_timeout(Some(VIEW_WAIT));
    let reply = match read_request(&mut stream) {
        None => Reply::status(400),
        Some(_) if stop.load(Ordering::SeqCst) => Reply::status(410),
        Some(request) if !guard.admits(&request) => Reply::status(403),
        Some(request) => answer(&request, shared),
    };
    let _ = stream.write_all(&reply.bytes());
    let _ = stream.flush();
}

/// The answer to `request`, which carries the secret.
fn answer(request: &Request, shared: &Shared) -> Reply {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", PAGE_PATH) => Reply::html(PAGE_HTML),
        ("GET", VIEW_PATH) => {
            let after = request
                .params("after")
                .next()
                .and_then(|after| after.parse::<u64>().ok())
                .unwrap_or(0);
            Reply::json(view_after(shared, after))
        }
        ("POST", ANSWER_PATH) => {
            let json = request
                .header("content-type")
                .is_some_and(|kind| kind.starts_with("application/json"));
            if !json {
                return Reply::status(415);
            }
            match serde_json::from_slice::<Given>(&request.body) {
                Ok(given) if take(shared, &given) => Reply::json("{}".to_owned()),
                Ok(_) => Reply::status(409),
                Err(_) => Reply::status(400),
            }
        }
        _ => Reply::status(404),
    }
}

/// The view once its version is past `after`, it ends or a while passed.
fn view_after(shared: &Shared, after: u64) -> String {
    let ends = Instant::now() + VIEW_WAIT;
    let mut view = shared.view();
    while view.version <= after && !view.ended {
        let now = Instant::now();
        if now >= ends {
            break;
        }
        view = (shared.changed.wait_timeout(view, ends - now))
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
    view.taken = view.taken.max(view.version);
    shared.changed.notify_all();
    view.json()
}

/// An answer the browser sends: to the question `question`, by the button at `button`, with the
/// text of its field.
#[derive(Deserialize)]
struct Given {
    question: u64,
    button: usize,
    text: Option<String>,
}

/// Hands `given` to the question it answers, if it is the one the page asks; gives whether it did.
fn take(shared: &Shared, given: &Given) -> bool {
    let mut view = shared.view();
    let Some(asked) = view
        .question
        .as_ref()
        .filter(|asked| asked.id == given.question)
    else {
        return false;
    };
    let answer = match &asked.field {
        Some(_) => Answer::Text(given.text.clone().unwrap_or_default()),
        None if given.button < asked.buttons.len() => Answer::Button(given.button),
        None => return false,
    };
    let sent = asked.answers.send(answer).is_ok();
    if asked.once {
        view.question = None;
        view.changed();
        shared.changed.notify_all();
    }
    sent
}

/// An answer of the page's server.
struct Reply {
    status: u16,
    kind: &'static str,
    body: Vec<u8>,
}

impl Reply {
    fn status(status: u16) -> Self {
        Reply {
            status,
            kind: "text/plain; charset=utf-8",
            body: Vec::new(),
        }
    }

    fn html(body: &str) -> Self {
        Reply {
            status: 200,
            kind: "text/html; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    fn json(body: String) -> Self {
        Reply {
            status: 200,
            kind: "application/json",
            body: body.into_bytes(),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            409 => "Conflict",
            410 => "Gone",
            415 => "Unsupported Media Type",
            _ => "Error",
        };
        let mut bytes = format!(
            "HTTP/1.1 {} {reason}\r\n\
             Cache-Control: no-store\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n\
             Content-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; \
             style-src 'unsafe-inline'; connect-src 'self'; form-action 'none'; \
             frame-ancestors 'none'; base-uri 'none'\r\n\
             Referrer-Policy: no-referrer\r\n\
             X-Content-Type-Options: nosniff\r\n\
             X-Frame-Options: DENY\r\n\
             Connection: close\r\n\r\n",
            self.status,
            self.kind,
            self.body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

/// The page: it takes the view again each time it changes and shows it, and sends its person's
/// answers.
const PAGE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="referrer" content="no-referrer">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>skald</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; max-width: 44rem; margin: 2rem auto; padding: 0 1rem; }
  #shown { font: 1.6rem/1.4 ui-monospace, monospace; padding: 1rem; border: 2px solid; white-space: pre-wrap; word-break: break-all; }
  #question { margin: 1rem 0; }
  #question input { font: 1.2rem ui-monospace, monospace; margin-right: .5rem; }
  #question button { font-size: 1rem; margin-right: .5rem; }
  #log { color: #444; }
  #log p { margin: .25rem 0; white-space: pre-wrap; }
</style>
</head>
<body>
<h1>skald</h1>
<p id="state">This page belongs to a skald command on this computer. Only you answer here.</p>
<div id="shown" hidden></div>
<form id="question" hidden></form>
<div id="log"></div>
<script>
"use strict";
const query = location.search;
const base = location.pathname;
const byId = (id) => document.getElementById(id);
let version = 0;
let asked = null;
let ended = false;

function paragraph(parent, text) {
  const p = document.createElement("p");
  p.textContent = text;
  parent.append(p);
}

// A line of the command's output, each http or https link in it a link.
function line(parent, text) {
  const p = document.createElement("p");
  let at = 0;
  for (const found of text.matchAll(/https?:\/\/[^\s"<>]+/g)) {
    p.append(text.slice(at, found.index));
    const a = document.createElement("a");
    a.href = found[0];
    a.rel = "noreferrer noopener";
    a.target = "_blank";
    a.textContent = found[0];
    p.append(a);
    at = found.index + found[0].length;
  }
  p.append(text.slice(at));
  parent.append(p);
}

function disable() {
  for (const e of byId("question").querySelectorAll("button, input")) e.disabled = true;
}

function ask(question) {
  asked = question;
  const form = byId("question");
  form.replaceChildren();
  form.hidden = !question;
  if (!question) return;
  if (question.text) paragraph(form, question.text);
  if (question.field !== null) {
    const field = document.createElement("input");
    field.type = "text";
    field.autocomplete = "off";
    field.spellcheck = false;
    field.setAttribute("aria-label", question.field);
    field.placeholder = question.field;
    form.append(field);
    field.focus();
  }
  question.buttons.forEach((name, at) => {
    const button = document.createElement("button");
    button.type = "submit";
    button.value = String(at);
    button.textContent = name;
    form.append(button);
  });
}

function show(view) {
  version = view.version;
  const shown = byId("shown");
  shown.textContent = view.shown ?? "";
  shown.hidden = !view.shown;
  const log = byId("log");
  for (let at = log.childElementCount; at < view.lines.length; at++) line(log, view.lines[at]);
  const question = view.ended ? null : view.question;
  if ((question?.id ?? null) !== (asked?.id ?? null)) ask(question);
  if (view.ended) {
    ended = true;
    byId("state").textContent = "The skald command ended: you may close this page.";
  }
}

function gone() {
  if (ended) return;
  ended = true;
  disable();
  byId("state").textContent = "The skald command no longer answers: it ended.";
}

byId("question").addEventListener("submit", async (event) => {
  event.preventDefault();
  const question = asked;
  if (!question || ended) return;
  const button = event.submitter ? Number(event.submitter.value) : 0;
  const field = byId("question").querySelector("input");
  if (question.once) disable();
  const answer = { question: question.id, button, text: field ? field.value : null };
  try {
    await fetch(`${base}/answer${query}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(answer),
      cache: "no-store",
    });
  } catch {
    gone();
  }
});

async function follow() {
  let misses = 0;
  while (!ended) {
    try {
      const answer = await fetch(`${base}/view${query}&after=${version}`, { cache: "no-store" });
      if (!answer.ok) throw new Error(String(answer.status));
      show(await answer.json());
      misses = 0;
    } catch {
      if (++misses >= 3) return gone();
      await new Promise((done) => setTimeout(done, 500));
    }
  }
}
follow();
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn request(head: &str) -> Request {
        let (start, headers) = head.split_once("\r\n").unwrap_or((head, ""));
        let mut parts = start.split(' ');
        let (method, target) = (parts.next().unwrap(), parts.next().unwrap());
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        Request {
            method: method.to_owned(),
            path: path.to_owned(),
            query: query.to_owned(),
            headers: (headers.split("\r\n"))
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
                .collect(),
            body: Vec::new(),
        }
    }

    fn guard() -> Guard {
        Guard {
            authority: "127.0.0.1:4100".to_owned(),
            secret: "abc".to_owned(),
        }
    }

    /// Admitted: the secret, the page's Host, and its Origin or none. Refused: a request without
    /// either, with another, with two, or with a secret that is only alike.
    #[test]
    fn a_request_needs_the_secret_the_host_and_the_origin() {
        let guard = guard();
        for admitted in [
            "GET /skald?s=abc\r\nHost: 127.0.0.1:4100",
            "GET /skald/view?s=abc&after=3\r\nHost: 127.0.0.1:4100",
            "POST /skald/answer?s=abc\r\nHost: 127.0.0.1:4100\r\nOrigin: http://127.0.0.1:4100",
        ] {
            assert!(guard.admits(&request(admitted)), "{admitted}");
        }
        for refused in [
            "GET /skald\r\nHost: 127.0.0.1:4100",
            "GET /skald?s=abd\r\nHost: 127.0.0.1:4100",
            "GET /skald?s=ab\r\nHost: 127.0.0.1:4100",
            "GET /skald?s=abc&s=abc\r\nHost: 127.0.0.1:4100",
            "GET /skald?t=abc\r\nHost: 127.0.0.1:4100",
            "GET /skald?s=abc",
            "GET /skald?s=abc\r\nHost: localhost:4100",
            "GET /skald?s=abc\r\nHost: evil.example:4100",
            "GET /skald?s=abc\r\nHost: 127.0.0.1:4100\r\nHost: 127.0.0.1:4100",
            "GET /skald?s=abc\r\nHost: 127.0.0.1:4100\r\nOrigin: null",
            "GET /skald?s=abc\r\nHost: 127.0.0.1:4100\r\nOrigin: http://evil.example",
            "GET /skald?s=abc\r\nHost: 127.0.0.1:4100\r\nOrigin: http://127.0.0.1:4101",
            "GET /skald?s=abc\r\nHost: 127.0.0.1:4100\r\nOrigin: https://127.0.0.1:4100",
        ] {
            assert!(!guard.admits(&request(refused)), "{refused}");
        }
    }

    /// An answer reaches only the question the page asks, by one of its buttons, once if it takes
    /// one; a question with a field takes its text.
    #[test]
    fn an_answer_reaches_only_the_question_asked() {
        let shared = Shared::default();
        let given = |question, button, text: Option<&str>| Given {
            question,
            button,
            text: text.map(str::to_owned),
        };
        let ask = |field: Option<&str>, once| {
            let (answers, answered) = mpsc::channel();
            let mut view = shared.view();
            view.asked += 1;
            view.question = Some(Asked {
                id: view.asked,
                text: String::new(),
                buttons: vec!["Yes".to_owned(), "No".to_owned()],
                field: field.map(str::to_owned),
                once,
                answers,
            });
            answered
        };

        let answered = ask(None, true);
        assert!(!take(&shared, &given(0, 0, None)), "another question");
        assert!(!take(&shared, &given(1, 2, None)), "no such button");
        assert!(take(&shared, &given(1, 1, None)));
        assert!(!take(&shared, &given(1, 0, None)), "answered once");
        assert_eq!(answered.try_iter().collect::<Vec<_>>(), [Answer::Button(1)]);

        let answered = ask(Some("Link code"), true);
        assert!(take(&shared, &given(2, 0, Some("abc def"))));
        assert_eq!(answered.recv(), Ok(Answer::Text("abc def".to_owned())));

        let answered = ask(None, false);
        assert!(take(&shared, &given(3, 0, None)));
        assert!(take(&shared, &given(3, 0, None)), "one per press");
        assert_eq!(answered.try_iter().count(), 2);
    }

    /// What the page shows and its question replace the ones before in one change: no version of
    /// the view shows the new text under the question before, and the new question takes the
    /// answer.
    #[test]
    fn shown_text_and_its_question_change_in_one_view() {
        let dir = crate::test_dir::TestDir::new("page-show-asking");
        let (page, _presses) = Page::showing_a_link_code_for_tests(&dir);
        let before = page.versions().len();

        let answered = page.show_asking("KEY", "Write it down.", &["Written down"]);
        let versions = page.versions();
        assert_eq!(versions.len(), before + 1, "one change: {versions:?}");
        assert_eq!(versions[before]["shown"], "KEY");
        assert_eq!(
            versions[before]["question"]["buttons"],
            json!(["Written down"])
        );
        assert!(page.press("Written down"));
        assert_eq!(answered.unwrap().try_recv(), Ok(Answer::Button(0)));
    }

    /// The view takes the command's output in lines, its last until it ends.
    #[test]
    fn the_view_takes_the_output_in_lines() {
        let shared = Shared::default();
        shared.log(b"one\r\ntw");
        shared.log(b"o\nthr");
        assert_eq!(shared.view().lines, ["one", "two"]);
        assert_eq!(shared.view().partial, "thr");
    }

    /// Without the program `BROWSER` names, there is no browser to open, so nothing listens.
    #[test]
    fn a_program_that_is_not_there_opens_nothing() {
        let missing = std::env::temp_dir()
            .join("skald-no-such-browser")
            .join("browser");
        assert_eq!(program(&missing), None);
        assert_eq!(program(Path::new("skald-no-such-browser-on-path")), None);
    }

    /// The file takes the browser to its URL with the URL's quotes and markup escaped, and is
    /// readable only by its user.
    #[test]
    fn the_file_holds_its_url_escaped() {
        let dir = crate::test_dir::TestDir::new("page-file");
        let file = write_file(&dir, "http://a.test/d?c=X\"><b>&y='z'").expect("written");
        let html = fs::read_to_string(&file).expect("read");
        let url = "http://a.test/d?c=X&quot;&gt;&lt;b&gt;&amp;y=&#39;z&#39;";
        assert!(html.contains(&format!("content=\"0;url={url}\"")), "{html}");
        assert!(html.contains(&format!("href=\"{url}\"")), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&file).expect("the file").permissions().mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
    }

    /// Only an `http` or `https` URL is opened; nothing is written for another.
    #[test]
    fn only_a_web_url_is_opened() {
        let dir = crate::test_dir::TestDir::new("page-scheme");
        for url in ["file:///etc/passwd", "javascript:alert(1)", "ftp://a.test/"] {
            assert!(open_url(&dir, url).is_none(), "{url}");
        }
        assert_eq!(fs::read_dir(&*dir).expect("the directory").count(), 0);
    }

    /// Every answer of the page, a refusal too, carries `Cache-Control: no-store`.
    #[test]
    fn every_answer_is_not_to_be_stored() {
        for reply in [
            Reply::status(403),
            Reply::html(PAGE_HTML),
            Reply::json("{}".to_owned()),
        ] {
            let bytes = String::from_utf8(reply.bytes()).unwrap();
            assert!(bytes.contains("\r\nCache-Control: no-store\r\n"), "{bytes}");
        }
    }
}
