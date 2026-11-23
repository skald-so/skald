//! What the tests share: the stand-in for the program that opens the browser and what it was
//! given, a command's page as the person drives it in headless Chrome, a new device and a linked
//! one taking the link code's steps on their pages, and requests to a page made by hand.

use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_github_stand_in::User;
use skald_proto::DeviceId;

use crate::at_terminal::{as_agent, keys_in};
use crate::browser::{APPROVE, Browser, CODE_FIELD, Page};
use crate::cloud::{Cloud, WAIT, ids};
use crate::linking::{LINK, OWN_DEVICE, codes_in, forms, says, typed, until_run_open};
use crate::machine::{Finished, Machine, RUN_LIMIT, Run, SERVER, SIGN_IN};
use crate::world::{Person, World};

/// The option that opens the page, and the setting that names the program that opens the browser.
pub const PAGE: &str = "--browser";
pub const BROWSER: &str = "BROWSER";

/// What the confirmation of `skald link` asks at a terminal.
pub const CONFIRMATION: &str = "link it to this account";

/// The buttons of the pages, as patterns of their names, case aside: yes, no, a fresh code, and the
/// one under a recovery key, which is neither yes nor no.
pub const YES_BUTTON: &str = r"^\s*yes\b";
pub const NO_BUTTON: &str = r"^\s*no\b";
pub const FRESH_CODE_BUTTON: &str = r"\b(fresh|new|another)\b.*\bcode\b";
pub const WRITTEN_DOWN_BUTTON: &str = r"^(?!\s*(yes|no)\b)\s*\S";

/// What a command says on one line without a browser to open.
pub const NO_BROWSER: [&str; 2] = ["browser", "terminal"];
/// What a command says once the browser did not open, on one line.
pub const NOT_OPENED: [&str; 3] = ["browser", "not open", "terminal"];
/// What it does not say then: the program exists, and the command may have sent requests.
pub const NOT_SAID_ONCE_RUN: [&str; 2] = ["names no program", "nothing is sent"];

/// A stand-in for the program that opens the browser: it writes down the arguments of each of its
/// runs and opens nothing.
pub struct Opener {
    program: PathBuf,
    log: PathBuf,
}

impl Opener {
    /// The stand-in on `machine`, in a directory of its own under the machine's.
    pub fn on(machine: &Machine) -> Self {
        let dir = machine.root().join("opener");
        fs::create_dir_all(&dir).expect("make the stand-in's directory");
        let log = dir.join("opened");
        #[cfg(unix)]
        let program = {
            use std::os::unix::fs::PermissionsExt as _;
            let program = dir.join("open-page");
            let script = format!(
                "#!/bin/sh\n{{ echo run; for arg in \"$@\"; do printf 'arg %s\\n' \"$arg\"; done; \
                 echo end; }} >> '{}'\n",
                log.display()
            );
            fs::write(&program, script).expect("write the stand-in");
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755))
                .expect("make the stand-in runnable");
            program
        };
        #[cfg(windows)]
        let program = {
            let program = dir.join("open-page.cmd");
            let log = log.display();
            let script = format!(
                "@echo off\r\n>>\"{log}\" echo run\r\n>>\"{log}\" echo arg %*\r\n\
                 >>\"{log}\" echo end\r\n"
            );
            fs::write(&program, script).expect("write the stand-in");
            program
        };
        Opener { program, log }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The arguments of each of its finished runs, in order.
    pub fn openings(&self) -> Vec<Vec<String>> {
        let log = fs::read_to_string(&self.log).unwrap_or_default();
        let mut runs = Vec::new();
        let mut current: Option<Vec<String>> = None;
        for line in log.lines() {
            match line.trim_end() {
                "run" => current = Some(Vec::new()),
                "end" => runs.extend(current.take()),
                other => {
                    if let (Some(args), Some(arg)) = (current.as_mut(), other.strip_prefix("arg "))
                    {
                        args.push(if cfg!(windows) {
                            arg.trim().trim_matches('"').to_owned()
                        } else {
                            arg.to_owned()
                        });
                    }
                }
            }
        }
        runs
    }
}

/// A program no machine has, for a `BROWSER` that names nothing to run.
pub fn missing_program(machine: &Machine) -> PathBuf {
    machine.root().join("no-such-browser")
}

/// A program that opens no browser and exits failing, as one does without a desktop session.
pub fn failing_program(machine: &Machine) -> PathBuf {
    if cfg!(windows) {
        let program = machine.root().join("failing-open-page.cmd");
        fs::write(&program, "@exit /b 3\r\n").expect("write the stand-in");
        program
    } else {
        (["/usr/bin/false", "/bin/false"].into_iter())
            .map(PathBuf::from)
            .find(|program| program.is_file())
            .expect("a program false")
    }
}

/// `skald` with `args` on `machine`, its input empty and its outputs pipes, as an agent runs it,
/// with `BROWSER` naming `program`.
pub fn start_with(machine: &Machine, program: &Path, args: &[&str]) -> Run {
    let home = machine.root().join("home");
    let mut command = machine.command_in(&home, args);
    command.env(BROWSER, program).stdin(Stdio::null());
    Run::spawn(&mut command)
}

/// What the stand-in was given to open, and the page that file takes the browser to.
#[derive(Clone, Debug)]
pub struct Opening {
    /// The stand-in's arguments.
    pub args: Vec<String>,
    pub file: PathBuf,
    /// The page's URL, its secret included.
    pub url: String,
    /// The URL's host and port, as its Host header names them.
    pub authority: String,
    /// The URL's path and query.
    pub path: String,
    pub query: String,
    pub address: SocketAddr,
}

impl Opening {
    /// The URL's origin, as the page's own requests name it.
    pub fn origin(&self) -> String {
        format!("http://{}", self.authority)
    }

    /// The URL's path and query, as a request line names them.
    pub fn target(&self) -> String {
        format!("{}?{}", self.path, self.query)
    }
}

/// The path of the file an argument of the stand-in names, as a path or a `file:` URL.
pub fn named_file(arg: &str) -> PathBuf {
    let Some(rest) = arg.strip_prefix("file://") else {
        return PathBuf::from(arg);
    };
    let path = rest.replace("%20", " ");
    // `file:///C:/...` on Windows.
    if cfg!(windows) {
        PathBuf::from(path.trim_start_matches('/'))
    } else {
        PathBuf::from(path)
    }
}

/// `file`, as a browser opens it.
pub fn file_url(file: &Path) -> String {
    let path = file.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// Whether `host`, of a URL, names a loopback address.
fn loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The first URL of a loopback address in `html`, the opened file.
fn page_url(html: &str) -> Option<String> {
    html.match_indices("http://").find_map(|(at, _)| {
        let rest = &html[at..];
        let end = rest
            .find(|c: char| "\"'<> \t\r\n".contains(c))
            .unwrap_or(rest.len());
        let url = rest[..end].replace("&amp;", "&");
        let authority = url["http://".len()..].split(['/', '?', '#']).next()?;
        let (host, _) = authority.rsplit_once(':')?;
        loopback(host).then_some(url)
    })
}

/// Waits until `found` gives something of the stand-in's runs so far, the arguments of each, for up
/// to [`RUN_LIMIT`], while `run` goes on; gives it.
#[track_caller]
pub fn until_opened<T>(
    opener: &Opener,
    run: &mut Run,
    found: impl Fn(&[Vec<String>]) -> Option<T>,
) -> T {
    let deadline = Instant::now() + RUN_LIMIT;
    loop {
        if let Some(found) = found(&opener.openings()) {
            return found;
        }
        assert!(
            !run.exited(),
            "skald exited opening no page: {}",
            run.shown()
        );
        assert!(
            Instant::now() < deadline,
            "skald opened no page: {}",
            run.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Waits until the stand-in has run after the `before` runs it had, for up to [`RUN_LIMIT`], while
/// `run` goes on; gives what it was given. Fails unless it was given one argument, a file that
/// holds a loopback URL with a query.
#[track_caller]
pub fn wait_for_opening(opener: &Opener, before: usize, run: &mut Run) -> Opening {
    let args = until_opened(opener, run, |openings| openings.get(before).cloned());
    assert_eq!(args.len(), 1, "the stand-in's arguments: {args:?}");
    let file = named_file(&args[0]);
    let html = fs::read_to_string(&file)
        .unwrap_or_else(|err| panic!("read the opened file {file:?}: {err}"));
    let url = page_url(&html).unwrap_or_else(|| panic!("no page's URL in {file:?}: {html}"));
    let rest = &url["http://".len()..];
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = rest[..split].to_owned();
    let (path, query) = rest[split..]
        .split('#')
        .next()
        .and_then(|target| target.split_once('?'))
        .unwrap_or_else(|| panic!("the page's URL holds its secret in no query: {url}"));
    assert!(!query.is_empty(), "an empty query: {url}");
    let path = if path.is_empty() { "/" } else { path }.to_owned();
    let connect = authority.replace("localhost", "127.0.0.1");
    let address = connect
        .parse()
        .unwrap_or_else(|err| panic!("the page's address {authority}: {err}"));
    Opening {
        args,
        file,
        url: url.clone(),
        authority,
        path,
        query: query.to_owned(),
        address,
    }
}

/// A command's page in a browser of its own, the person's.
pub struct Paged<'b> {
    pub opening: Opening,
    pub page: Page<'b>,
}

impl<'b> Paged<'b> {
    /// Waits until the stand-in has run after the `before` runs it had while `run` goes on, then
    /// opens the file it was given in a new browser of `browser`, which records its answers.
    #[track_caller]
    pub fn opened(browser: &'b Browser, opener: &Opener, before: usize, run: &mut Run) -> Self {
        let opening = wait_for_opening(opener, before, run);
        let page = browser.page();
        page.record_answers();
        let visit = page.goto_settled(&file_url(&opening.file));
        assert!(visit.error.is_none(), "the opened file: {visit:?}");
        Paged { opening, page }
    }

    /// Waits until the page's text passes `seen`, for up to [`RUN_LIMIT`], while `run` goes on;
    /// gives the text.
    #[track_caller]
    pub fn wait_for(&self, run: &mut Run, what: &str, seen: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + RUN_LIMIT;
        loop {
            let text = self.page.text();
            if seen(&text) {
                return text;
            }
            assert!(
                !run.exited(),
                "skald exited before its page showed {what}: page {text:?}, output {}",
                run.shown()
            );
            assert!(
                Instant::now() < deadline,
                "the page did not show {what}: {text:?}"
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// The person clicks the button whose name matches `pattern`, case aside, once there is one.
    #[track_caller]
    pub fn press(&self, pattern: &str) {
        let script = format!(
            r#"(() => {{
              const pattern = new RegExp({}, "i");
              const name = (e) => (e.getAttribute("aria-label")
                || (e.tagName === "INPUT" ? e.value : e.innerText) || "").trim();
              const shown = [...document.querySelectorAll(
                "button, input[type=submit], input[type=button], [role=button], a[href]")]
                .filter((e) => !e.disabled && e.getClientRects().length > 0);
              const found = shown.find((e) => pattern.test(name(e)));
              if (!found) return shown.map(name);
              setTimeout(() => found.click(), 0);
              return true;
            }})()"#,
            Value::from(pattern)
        );
        let deadline = Instant::now() + WAIT;
        loop {
            let found = self.page.evaluate(&script);
            if found == Value::Bool(true) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no button /{pattern}/i on the page, only {found}: {}",
                self.page.text()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// The person types `entry` into the page's text field and sends it: with the field's form,
    /// or with the page's first button.
    pub fn enter(&self, entry: &str) {
        self.page.fill(CODE_FIELD, entry);
        let sent = self.page.evaluate(
            r#"(() => {
              const field = document.querySelector("input:not([type=hidden]), textarea");
              if (field && field.form) {
                setTimeout(() => field.form.requestSubmit(), 0);
                return true;
              }
              const button = [...document.querySelectorAll("button, input[type=submit]")]
                .find((e) => !e.disabled && e.getClientRects().length > 0);
              if (!button) return false;
              setTimeout(() => button.click(), 0);
              return true;
            })()"#,
        );
        assert_eq!(
            sent,
            Value::Bool(true),
            "nothing sends the code: {}",
            self.page.text()
        );
    }

    /// The link code the page shows last.
    pub fn code(&self) -> String {
        codes_in(&self.page.text())
            .pop()
            .expect("a code on the page")
    }

    /// Fails unless every answer the page's own requests got carries `Cache-Control: no-store`.
    #[track_caller]
    pub fn assert_answers_not_stored(&self) {
        for answer in self.page.answers() {
            let control = answer.headers.get("cache-control").cloned();
            assert!(
                control.is_some_and(|control| control.to_lowercase().contains("no-store")),
                "{} {} without Cache-Control: no-store: {:?}",
                answer.method,
                answer.url,
                answer.headers
            );
        }
    }
}

/// A new device of a person's account, signed in as an agent with `--browser`; its person answers
/// on its page.
pub struct NewOnPage<'b> {
    /// Dropped first, so its run ends before the machine's directories go.
    run: Option<Run>,
    pub paged: Paged<'b>,
    pub opener: Opener,
    pub machine: Machine,
    pub host: &'static str,
    pub token: String,
    pub device: DeviceId,
    /// When it started, from which the relay's requests are counted.
    pub started: Instant,
    /// The hellos it sent that a run took.
    pub runs: usize,
}

impl<'b> NewOnPage<'b> {
    /// A new machine with the host name `host` and the settings `settings` signs in to `first`'s
    /// account through a pipe, its person approving its code, so it waits for linking with a
    /// session; then its `skald sign-in` as an agent with `--browser`. Gives it once its page shows
    /// a link code of its session and where it goes.
    pub fn start(
        world: &'b World,
        first: &Person,
        host: &'static str,
        settings: &[(&str, &str)],
    ) -> Self {
        let machine = (settings.iter())
            .fold(Machine::new().with_host_name(host), |machine, set| {
                machine.with_env(set.0, set.1)
            });
        Self::start_on(world, first, machine, host)
    }

    /// As [`NewOnPage::start`], on `machine`, whose host name is `host`.
    pub fn start_on(
        world: &'b World,
        first: &Person,
        machine: Machine,
        host: &'static str,
    ) -> Self {
        let cloud = &world.cloud;
        let opener = Opener::on(&machine);
        let known = ids(&cloud.devices(&first.token));
        let grants = cloud.grants().len();
        let started = cloud.relay.mark();
        let mut piped = machine.start(&[SIGN_IN, SERVER, &cloud.url]);
        approve_next_as(world, &first.user, grants, &mut piped);
        let signed_in = piped.finish();
        assert!(signed_in.success, "{signed_in:?}");
        let args = as_agent(&[SIGN_IN, SERVER, &cloud.url, PAGE]);
        let mut run = start_with(&machine, opener.program(), &args);
        let paged = Paged::opened(&world.browser, &opener, 0, &mut run);
        paged.wait_for(&mut run, "a link code and where it goes", |text| {
            !codes_in(text).is_empty() && says(text, &["skald link"])
        });
        let token = cloud.last_token();
        let device = cloud.new_device(&token, &known);
        NewOnPage {
            run: Some(run),
            paged,
            opener,
            machine,
            host,
            token,
            device,
            started,
            runs: 0,
        }
    }

    pub fn run(&mut self) -> &mut Run {
        self.run.as_mut().expect("the new device's run")
    }

    /// Waits until its page passes `seen`, as [`Paged::wait_for`] does.
    #[track_caller]
    pub fn wait_for(&mut self, what: &str, seen: impl Fn(&str) -> bool) -> String {
        let run = self.run.as_mut().expect("the new device's run");
        self.paged.wait_for(run, what, seen)
    }

    /// The code its page shows last.
    pub fn code(&self) -> String {
        self.paged.code()
    }

    /// Waits until the account's sessions see a run of its code open, its hellos past the runs
    /// counted so far; `first` is a linked device of the account.
    #[track_caller]
    pub fn open_run(&mut self, cloud: &Cloud, first: &Person) {
        self.runs += 1;
        let (device, started, run) = (self.device, self.started, self.runs);
        until_run_open(cloud, first, device, (started, run), || self.run().shown());
    }

    /// The person asks on its page for a fresh code; gives it once shown, and fails unless it is
    /// another one.
    #[track_caller]
    pub fn fresh_code(&mut self) -> String {
        let old = self.code();
        self.paged.press(FRESH_CODE_BUTTON);
        let text = self.wait_for("a fresh code", |text| {
            codes_in(text).last().is_some_and(|code| *code != old)
        });
        codes_in(&text).pop().expect("a code")
    }

    /// Has an agent's `skald link --browser` on `first`'s machine take its code: the person enters
    /// it on that page and confirms the new device there. Gives that page and what the command
    /// printed once it exits with success.
    #[track_caller]
    pub fn link_from(&mut self, world: &'b World, first: &Person) -> (Paged<'b>, Finished) {
        let typed_code = typed(&self.code());
        self.open_run(&world.cloud, first);
        let at_first = Opener::on(&first.machine);
        let (run, paged) = link_on_page(world, first, &at_first, OWN_DEVICE);
        let finished = confirm_on_page(run, &paged, &typed_code, self.host);
        assert!(finished.success, "skald link: {finished:?}");
        (paged, finished)
    }

    /// Waits for its run to exit; gives what it printed.
    pub fn finish(&mut self) -> Finished {
        self.run.take().expect("the new device's run").finish()
    }
}

/// Approves, as `user`, the device authorization that `run` gets after the `before` grants the
/// server gave, whether or not its output shows it.
#[track_caller]
pub fn approve_next_as(world: &World, user: &User, before: usize, run: &mut Run) {
    let cloud = &world.cloud;
    let grant = cloud.next_grant(before, || {
        assert!(!run.exited(), "skald exited: {}", run.shown());
        run.shown()
    });
    cloud.decide(&world.browser.page(), user, &grant, APPROVE);
}

/// `skald --agent <agent> link --browser` on `first`'s machine, its page opened; gives its run and
/// page once the page shows `shows`.
#[track_caller]
pub fn link_on_page<'b>(
    world: &'b World,
    first: &Person,
    opener: &Opener,
    shows: &str,
) -> (Run, Paged<'b>) {
    let before = opener.openings().len();
    let mut run = start_with(&first.machine, opener.program(), &as_agent(&[LINK, PAGE]));
    let paged = Paged::opened(&world.browser, opener, before, &mut run);
    paged.wait_for(&mut run, shows, |text| says(text, &[shows]));
    (run, paged)
}

/// On the page of `skald link` that shows the reminder, the person enters `entry`, and once the
/// page shows the new device named `name` as a `client` device, answers `yes`; gives the run once
/// it exits.
#[track_caller]
pub fn confirm_on_page(mut run: Run, paged: &Paged, entry: &str, name: &str) -> Finished {
    paged.enter(entry);
    paged.wait_for(&mut run, "the new device's name and kind", |text| {
        says(text, &[name]) && says(text, &["client"])
    });
    paged.press(YES_BUTTON);
    run.finish()
}

/// `skald link --browser` on `first`'s machine as an agent: the person enters `entry` on its page,
/// which starts a run that fails; gives the run once it exits failing.
#[track_caller]
pub fn link_failing_on_page(
    world: &World,
    first: &Person,
    opener: &Opener,
    entry: &str,
) -> Finished {
    let (run, paged) = link_on_page(world, first, opener, OWN_DEVICE);
    paged.enter(entry);
    let finished = run.finish();
    assert!(!finished.success, "skald link succeeded: {finished:?}");
    finished
}

/// Fails unless `text`, what a command printed, holds no form of any of `codes`, no other code, no
/// confirmation, no recovery key and none of `secrets`.
#[track_caller]
pub fn assert_holds_none(what: &str, text: &str, codes: &[String], secrets: &[&str]) {
    for code in codes {
        for form in forms(code) {
            assert!(
                !text.contains(&form),
                "{what} holds the code as {form:?}: {text}"
            );
        }
    }
    assert_eq!(
        codes_in(text),
        Vec::<String>::new(),
        "{what} holds a code: {text}"
    );
    assert!(
        !says(text, &[CONFIRMATION]),
        "{what} holds the confirmation: {text}"
    );
    assert_eq!(
        keys_in(text),
        Vec::<String>::new(),
        "{what} holds a recovery key: {text}"
    );
    for secret in secrets {
        assert!(
            !text.contains(secret),
            "{what} holds the page's secret: {text}"
        );
    }
}

/// What a page answered to a request made by hand: its status and head.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub head: String,
}

impl Reply {
    pub fn taken(&self) -> bool {
        self.status < 400
    }

    /// Whether it carries `Cache-Control: no-store`.
    pub fn not_stored(&self) -> bool {
        self.head.lines().any(|line| {
            let line = line.to_lowercase();
            line.starts_with("cache-control:") && line.contains("no-store")
        })
    }
}

/// Sends `method target` to `address`, with `host` as its Host and `origin` as its Origin if any;
/// gives the answer, or none if nothing answers.
pub fn request(
    address: SocketAddr,
    method: &str,
    target: &str,
    host: &str,
    origin: Option<&str>,
) -> Option<Reply> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .ok()?;
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(origin) = origin {
        head.push_str(&format!("Origin: {origin}\r\n"));
    }
    let body = if method == "GET" { "" } else { "{}" };
    head.push_str(&format!(
        "Content-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ));
    stream.write_all(head.as_bytes()).ok()?;
    let mut answered = Vec::new();
    let mut buffer = [0; 4096];
    while !answered.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => answered.extend_from_slice(&buffer[..read]),
        }
    }
    let answered = String::from_utf8_lossy(&answered).into_owned();
    let status = answered.strip_prefix("HTTP/1.")?.get(2..5)?.parse().ok()?;
    Some(Reply {
        status,
        head: answered,
    })
}

/// Fails unless the page answers no request now: nothing answers, or it refuses.
#[track_caller]
pub fn assert_gone(what: &str, opening: &Opening) {
    let reply = request(
        opening.address,
        "GET",
        &opening.target(),
        &opening.authority,
        None,
    );
    assert!(
        reply.as_ref().is_none_or(|reply| !reply.taken()),
        "{what}'s page still answers: {reply:?}"
    );
    assert!(!opening.file.exists(), "{what} left {:?}", opening.file);
}

/// An address of this machine that is not loopback: the one it sends from off the machine.
pub fn outward_address() -> IpAddr {
    let socket = UdpSocket::bind("0.0.0.0:0").expect("a UDP socket");
    // A connected UDP socket sends nothing: it only picks the route.
    socket
        .connect("192.0.2.1:9")
        .expect("a route off this machine");
    let ip = socket.local_addr().expect("the socket's address").ip();
    assert!(
        !ip.is_loopback() && !ip.is_unspecified(),
        "no address but loopback: {ip}"
    );
    ip
}
