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
/// first, and a link or button that signs a browser without a session in: any, or the one of each
/// provider, whose text names it.
pub const CODE_FIELD: &str = "role=textbox";
pub const APPROVE: &str = "role=button[name=/approve/i]";
pub const DENY: &str = "role=button[name=/deny/i]";
pub const SUBMIT: &str = "role=button >> nth=0";
pub const SIGN_IN_LINK: &str = "a:has-text('sign in'), button:has-text('sign in')";
pub const SIGN_IN_GITHUB: &str = "a:has-text('GitHub'), button:has-text('GitHub')";
pub const SIGN_IN_GOOGLE: &str = "a:has-text('Google'), button:has-text('Google')";

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
    /// The job the driver and its browser run in, closed as this test binary ends.
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
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

/// Makes `command`'s program inherit no file descriptor but its standard streams. The driver lives
/// as long as its test binary. On macOS, which has no `pipe2`, a pipe that another test's thread is
/// making, such as for `Command::output`, is not close-on-exec yet for a moment; a program started
/// in that moment keeps the pipe open, and the other test waits for its end as long as that
/// program lives.
#[cfg(unix)]
fn inherit_only_stdio(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes only `limit`.
    let open = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0 {
        limit.rlim_cur.min(1 << 20)
    } else {
        1 << 16
    };
    let open = libc::c_int::try_from(open).expect("a number of descriptors");
    // SAFETY: between `fork` and `exec` the closure calls only `fcntl`, which is async-signal-safe.
    unsafe {
        command.pre_exec(move || {
            for fd in 3..open {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }
}

/// On Windows a program inherits only the handles Rust marks inheritable while it starts it.
#[cfg(not(unix))]
fn inherit_only_stdio(_command: &mut Command) {}

/// On Windows, where a process outlives its parent and the browser has no process group, the driver
/// runs in a job of this test binary's that kills it and its browser as the binary ends, however it
/// ends: a browser that playwright-core would close only after the driver has seen its stdin close
/// runs on for seconds on a busy machine, many of them at once while the tests run. Killed so,
/// playwright-core removes no profile: the driver's temporary directory, `skald-driver-<ID>` of
/// the binary's process ID, holds them and the web app's build, and a later driver removes those of
/// binaries no longer running.
#[cfg(windows)]
pub mod contained {
    use std::env;
    use std::fs;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{self, Child, Command};

    use windows_sys::Win32::Foundation::{INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, OpenProcess, OpenThread, PROCESS_SYNCHRONIZE, ResumeThread,
        SuspendThread, THREAD_SUSPEND_RESUME, WaitForSingleObject,
    };

    /// The start of the name of a driver's temporary directory.
    const PREFIX: &str = "skald-driver-";

    /// The temporary directory of the driver of the test binary whose process ID is `pid`.
    pub fn dir_of(pid: u32) -> PathBuf {
        env::temp_dir().join(format!("{PREFIX}{pid}"))
    }

    /// Makes `command` start suspended, to run once [`contain`] has put it in its job, with this
    /// binary's driver directory for its temporary directory; removes those of binaries no longer
    /// running first.
    pub fn prepare(command: &mut Command) -> io::Result<()> {
        remove_abandoned();
        let dir = dir_of(process::id());
        fs::create_dir_all(&dir)?;
        command
            .env("TEMP", &dir)
            .env("TMP", &dir)
            .creation_flags(CREATE_SUSPENDED);
        Ok(())
    }

    /// Puts `child`, which [`prepare`] made start suspended, in a job that kills it and every
    /// process it starts once the job's only handle, given here, closes; then lets it run.
    pub fn contain(child: &Child) -> io::Result<OwnedHandle> {
        // SAFETY: no attributes and no name; the handle is owned from here on.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new handle that nothing else owns.
        let job = unsafe { OwnedHandle::from_raw_handle(job) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of_val(&limits)).map_err(io::Error::other)?;
        // SAFETY: `limits` is the information of that class, of that size; the handles are open.
        let set = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        };
        // SAFETY: both handles are open.
        if set == 0
            || unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        each_thread(child.id(), |thread| {
            // SAFETY: an open handle of a thread with the right to resume it.
            unsafe { ResumeThread(thread) }
        })?;
        Ok(job)
    }

    /// The processes in `job`: the driver, the browser and every process they started.
    pub fn members(job: &OwnedHandle) -> io::Result<Vec<u32>> {
        /// Room for more processes than a driver starts.
        const ROOM: usize = 1024;
        // The list's two counts take the first entry, as they take one `usize` of the struct.
        let mut list = vec![0usize; 1 + ROOM];
        let size =
            u32::try_from(std::mem::size_of_val(list.as_slice())).map_err(io::Error::other)?;
        let header = list.as_mut_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
        // SAFETY: `list` is `size` bytes, aligned for the struct, which it starts as; the handle is
        // open.
        let queried = unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicProcessIdList,
                header.cast(),
                size,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call filled the counts in.
        let listed = unsafe { (*header).NumberOfProcessIdsInList } as usize;
        list.drain(..1);
        list.truncate(listed);
        list.into_iter()
            .map(|pid| u32::try_from(pid).map_err(io::Error::other))
            .collect()
    }

    /// Suspends every thread of the process `pid`, so that it cannot exit by itself.
    pub fn suspend(pid: u32) -> io::Result<()> {
        each_thread(pid, |thread| {
            // SAFETY: an open handle of a thread with the right to suspend it.
            unsafe { SuspendThread(thread) }
        })
    }

    /// Calls `change`, `ResumeThread` or `SuspendThread`, with each thread of the process `pid`;
    /// fails if it has none or a call fails.
    fn each_thread(pid: u32, change: impl Fn(RawHandle) -> u32) -> io::Result<()> {
        // SAFETY: it takes no pointer; the snapshot is owned from here on.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new handle that nothing else owns.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        let mut entry = THREADENTRY32 {
            dwSize: u32::try_from(std::mem::size_of::<THREADENTRY32>())
                .map_err(io::Error::other)?,
            ..THREADENTRY32::default()
        };
        let mut found = false;
        // SAFETY: `entry` is an entry with its size set, for the snapshot taken.
        let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: it takes no pointer; the handle is owned from here on.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: a new handle that nothing else owns.
                let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
                if change(thread.as_raw_handle()) == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                found = true;
            }
            // SAFETY: as for the first entry.
            more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        if found {
            Ok(())
        } else {
            Err(io::Error::other(format!("process {pid} has no thread")))
        }
    }

    /// Whether the process `pid` runs, or still ends: Windows gives a process its exit code before
    /// it closes its files, and signals it once it has.
    pub fn runs(pid: u32) -> bool {
        // SAFETY: it takes no pointer; the handle is owned from here on.
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if process.is_null() {
            return false;
        }
        // SAFETY: a new handle that nothing else owns.
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        // SAFETY: an open handle with the right to wait for it.
        let waited = unsafe { WaitForSingleObject(process.as_raw_handle(), 0) };
        waited == WAIT_TIMEOUT
    }

    /// Removes the drivers' directories of the test binaries no longer running.
    fn remove_abandoned() {
        let Ok(entries) = fs::read_dir(env::temp_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let pid = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix(PREFIX))
                .and_then(|pid| pid.parse::<u32>().ok());
            if pid.is_some_and(|pid| pid != process::id() && !runs(pid))
                && let Err(err) = fs::remove_dir_all(entry.path())
            {
                eprintln!("remove {}: {err}", entry.path().display());
            }
        }
    }
}

impl Driver {
    fn start() -> Result<Self, String> {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut command = Command::new("node");
        inherit_only_stdio(&mut command);
        #[cfg(windows)]
        contained::prepare(&mut command)
            .map_err(|err| format!("make the browser driver's directory: {err}"))?;
        let mut child = command
            .arg(crates.join("server/tests/common/browser.mjs"))
            .arg(crates.join("../web"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| format!("start node: the browser tests need Node.js: {err}"))?;
        #[cfg(windows)]
        let job = contained::contain(&child).map_err(|err| {
            let _ = child.kill();
            format!("put the browser driver in a job: {err}")
        })?;
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
                #[cfg(windows)]
                job,
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

    /// The processes in the job of this binary's driver: the driver, the browser and every process
    /// they started.
    #[cfg(windows)]
    pub fn contained(&self) -> Vec<u32> {
        contained::members(&self.driver.job).expect("the processes in the driver's job")
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

    /// As [`Browser::page`], its window `width` CSS pixels wide, as a phone's at 360.
    pub fn page_of_width(&self, width: u32) -> Page<'_> {
        let id = self.call("newPage", json!({ "width": width }));
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

    /// Starts a front of the web app on a local port, built once by the first front of the test
    /// binary: the app's production build and the API on one origin. A request whose path starts
    /// with the first segment of a path of the spec goes to the API, as the dev server sends it,
    /// once [`Browser::spa_target`] names where; any other gets the build's file at its path or its
    /// index page. Gives its port.
    pub fn spa_front(&self) -> u16 {
        serde_json::from_value(self.call("spaFront", json!({}))).expect("a port")
    }

    /// Sends the API requests of the front on `port` to `target`, e.g. `http://127.0.0.1:41234`.
    pub fn spa_target(&self, port: u16, target: &str) {
        self.call("spaTarget", json!({ "port": port, "target": target }));
    }
}

/// An answer to a `fetch` or XHR request of a page, as [`Page::record_answers`] recorded it.
#[derive(Clone, Debug, Deserialize)]
pub struct Answer {
    /// The request's method, such as `GET`.
    pub method: String,
    pub url: String,
    pub status: u16,
    /// Lowercase names and their values.
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// An HTTP request recorded by [`Page::record_requests`].
#[derive(Debug, Deserialize)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    /// The bytes of the request body, as Latin-1 so ASCII survives in binary bodies too.
    pub body: Option<String>,
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

    /// From now on records the answers to the `fetch` and XHR requests of this browser.
    pub fn record_answers(&self) {
        self.browser
            .call("recordAnswers", json!({ "page": self.id }));
    }

    /// The answers recorded since [`Page::record_answers`], in the order they came.
    pub fn answers(&self) -> Vec<Answer> {
        serde_json::from_value(self.browser.call("answers", json!({ "page": self.id })))
            .expect("answers")
    }

    /// From now on records the HTTP requests of this browser, including navigations, assets and
    /// requests that finish without a response.
    pub fn record_requests(&self) {
        self.browser
            .call("recordRequests", json!({ "page": self.id }));
    }

    /// Waits for network idle and gives the requests since [`Page::record_requests`], in order.
    pub fn requests(&self) -> Vec<Request> {
        serde_json::from_value(self.browser.call("requests", json!({ "page": self.id })))
            .expect("requests")
    }
}
