//! What the tests share: a new device of a person's account signing in at a terminal and showing
//! its link code, `skald link` at a terminal on a linked device, the codes a run shows, and what the
//! relay saw of them.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::DeviceId;

use crate::browser::APPROVE;
use crate::cloud::{Cloud, WAIT, ids};
use crate::keys;
use crate::machine::{FIRST_DEVICE, Machine, NO, SERVER, SIGN_IN};
use crate::terminal::{Shown, Terminal};
use crate::world::{Person, World};

/// The command that links a new device on a linked one.
pub const LINK: &str = "link";
/// The client's setting of how long a code lives, in seconds.
pub const LIFETIME_SETTING: &str = "SKALD_LINK_CODE_LIFETIME";
/// A run's hello, as the relay sees its request line start.
pub const HELLO: &str = "POST /account/links ";
/// Every request of the link code relay has a path that starts so.
pub const RELAYED: &str = "/account/links";
/// The new device's request that stores its certificate, as its line starts.
pub const STORE_CERT: &str = "PUT /devices/";
/// The answers to the requests for a run's later messages, as their lines start.
pub const MESSAGES: &str = "GET /account/links/messages/";
/// What the issuing device says before it takes a code, what it asks after 3 failed runs, and what
/// the new device says of a failed run and of an expired code.
pub const OWN_DEVICE: &str = "own device";
pub const NEW_CODE: &str = "new code";
pub const FAIL: &str = "fail";
pub const EXPIRED: &str = "expired";
/// What the new device says after its code failed 3 times.
pub const THREE_TIMES: [&str; 2] = ["3 times", "someone else"];
/// The confirmations `skald link` takes.
pub const YES: &str = "yes";
pub const NO_CONFIRM: &str = "no";

/// The base32 alphabet of RFC 4648, §6.
const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Each string in `text` in the shown form of a code, two groups of three characters of the
/// alphabet joined by `-`, in order.
pub fn codes_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let digit = |c: &char| ALPHABET.contains(*c);
    let edge = |c: Option<&char>| c.is_none_or(|c| !(digit(c) || *c == '-'));
    (0..chars.len().saturating_sub(6))
        .filter(|&at| {
            let window = &chars[at..at + 7];
            window[3] == '-'
                && window[..3].iter().all(digit)
                && window[4..].iter().all(digit)
                && edge(at.checked_sub(1).and_then(|before| chars.get(before)))
                && edge(chars.get(at + 7))
        })
        .map(|at| chars[at..at + 7].iter().collect())
        .collect()
}

/// The lines of `text` that hold `part`, case aside.
pub fn lines_holding(text: &str, part: &str) -> Vec<String> {
    let part = part.to_lowercase();
    text.lines()
        .filter(|line| line.to_lowercase().contains(&part))
        .map(str::to_owned)
        .collect()
}

/// How many runs `text`, what a new device showed, says failed: its lines that hold [`FAIL`],
/// apart from those that say the code failed 3 times.
pub fn failures(text: &str) -> usize {
    lines_holding(text, FAIL)
        .iter()
        .filter(|line| !says(line, &THREE_TIMES[..1]))
        .count()
}

/// Whether a line of `text` holds every one of `parts`, case aside.
pub fn says(text: &str, parts: &[&str]) -> bool {
    text.lines().any(|line| {
        let line = line.to_lowercase();
        parts.iter().all(|part| line.contains(&part.to_lowercase()))
    })
}

/// Another code of the alphabet than `code`, in its shown form: its first character changed.
pub fn other_code(code: &str) -> String {
    let first = code.chars().next().expect("a code");
    let at = ALPHABET.find(first).expect("a code of the alphabet");
    let changed = ALPHABET
        .chars()
        .nth((at + 1) % ALPHABET.len())
        .expect("a character");
    format!("{changed}{}", &code[1..])
}

/// `code` as a person may type it: in lowercase, without its `-`, spaced.
pub fn typed(code: &str) -> String {
    let plain: String = code.chars().filter(|c| *c != '-').collect();
    format!(
        "{} {}",
        plain[..3].to_lowercase(),
        plain[3..].to_lowercase()
    )
}

/// The forms of `code` that must never leave a machine: shown, without its `-`, typed, each in
/// upper and lower case.
pub fn forms(code: &str) -> Vec<String> {
    let plain: String = code.chars().filter(|c| *c != '-').collect();
    let spaced = typed(code);
    [code.to_owned(), plain, spaced]
        .into_iter()
        .flat_map(|form| [form.to_uppercase(), form.to_lowercase()])
        .collect()
}

/// Whether `bytes` hold `form`.
pub fn holds(bytes: &[u8], form: &str) -> bool {
    bytes
        .windows(form.len())
        .any(|window| window == form.as_bytes())
}

/// A new device of a person's account, signed in as no first device at a terminal.
pub struct NewDevice {
    /// Dropped first, so its run ends before the machine's directories go: Windows removes no file
    /// a running process holds.
    run: Option<Terminal>,
    pub machine: Machine,
    pub host: &'static str,
    /// Its session token and its device.
    pub token: String,
    pub device: DeviceId,
    /// When it started, from which the relay's requests are counted.
    pub started: Instant,
    /// The hellos it sent that a run took.
    pub runs: usize,
}

impl NewDevice {
    /// A new machine with the host name `host` signs in to `first`'s account at a terminal, as no
    /// first device; its person approves its code. Gives it once it shows a link code.
    pub fn start(world: &World, first: &Person, host: &'static str) -> Self {
        Self::start_on(world, first, Machine::new().with_host_name(host), host)
    }

    /// As [`NewDevice::start`], on `machine`, whose host name is `host`.
    pub fn start_on(world: &World, first: &Person, machine: Machine, host: &'static str) -> Self {
        let cloud = &world.cloud;
        let known = ids(&cloud.devices(&first.token));
        let grants = cloud.grants().len();
        let started = cloud.relay.mark();
        let args = [SIGN_IN, SERVER, &cloud.url, FIRST_DEVICE, NO];
        let mut run = Terminal::start(&machine, &args);
        approve(world, first, &mut run, grants);
        run.wait_for("a link code", |shown| !codes_in(shown).is_empty());
        let token = cloud.last_token();
        let new: Vec<DeviceId> = ids(&cloud.devices(&token))
            .into_iter()
            .filter(|device| !known.contains(device))
            .collect();
        assert_eq!(new.len(), 1, "the new device: {new:?}");
        NewDevice {
            machine,
            host,
            token,
            device: new[0],
            started,
            runs: 0,
            run: Some(run),
        }
    }

    /// Its run at the terminal.
    pub fn run(&mut self) -> &mut Terminal {
        self.run.as_mut().expect("the new device's run")
    }

    /// What it showed so far.
    pub fn shown(&mut self) -> String {
        self.run().shown()
    }

    /// The code it shows last.
    pub fn code(&mut self) -> String {
        codes_in(&self.shown()).pop().expect("a code shown")
    }

    /// Waits until a run of its code is open on the server: it sent a hello after the runs it
    /// opened so far, and the account's sessions see it waiting. `first` is a linked device.
    #[track_caller]
    pub fn open_run(&mut self, cloud: &Cloud, first: &Person) {
        let deadline = Instant::now() + WAIT;
        let next = self.runs + 1;
        while cloud.count_sent(self.started, HELLO) < next || !waiting(cloud, first, self.device) {
            assert!(
                Instant::now() < deadline,
                "no run {next} of the new device: {}",
                self.shown()
            );
            thread::sleep(Duration::from_millis(50));
        }
        self.runs = next;
    }

    /// Waits until it has said of `count` runs that they failed.
    #[track_caller]
    pub fn wait_for_failures(&mut self, count: usize) -> String {
        self.run()
            .wait_for(&format!("{count} failed runs"), |shown| {
                failures(shown) >= count
            })
    }

    /// The person asks for a fresh code; gives it once shown, and fails unless it is another one.
    #[track_caller]
    pub fn fresh_code(&mut self) -> String {
        let before = codes_in(&self.shown()).len();
        let old = self.code();
        self.run().enter();
        self.run()
            .wait_for("a fresh code", |shown| codes_in(shown).len() > before);
        let fresh = self.code();
        assert_ne!(fresh, old, "the fresh code is the old one");
        fresh
    }

    /// Waits for its run to exit; gives what it showed.
    pub fn finish(&mut self) -> Shown {
        self.run.take().expect("the new device's run").finish()
    }

    /// Kills its run at once; gives what it showed.
    pub fn kill(&mut self) -> Shown {
        self.run.take().expect("the new device's run").kill()
    }

    /// The person on this machine, once its certificate is on the server, as a test of `first`'s
    /// account uses it.
    pub fn into_person(self, cloud: &Cloud, first: &Person) -> Person {
        let cert = cloud
            .devices(&self.token)
            .into_iter()
            .find(|device| device.id == self.device)
            .and_then(|device| device.cert)
            .expect("the new device's certificate on the server");
        let holders = self.machine.files_holding(&self.token);
        Person {
            user: first.user.clone(),
            machine: self.machine,
            token: self.token,
            account: first.account,
            master: first.master,
            handle: first.handle.clone(),
            device: self.device,
            host: self.host,
            key: keys::signing_key(&holders, &cert.body().signing),
        }
    }
}

/// Approves, as the person of `first`'s account, the device authorization `run` shows after the
/// `before` ones the server granted.
pub fn approve(world: &World, first: &Person, run: &mut Terminal, before: usize) {
    let cloud = &world.cloud;
    let grant = crate::terminal_grants::shown_grant(cloud, run, before);
    cloud.decide(&world.browser.page(), &first.user, &grant, APPROVE);
}

/// Whether `first`'s account has `device` waiting with an open run.
pub fn waiting(cloud: &Cloud, first: &Person, device: DeviceId) -> bool {
    let (status, body) = cloud.get_as(RELAYED_WAITING, &first.token);
    assert_eq!(status, 200, "GET {RELAYED_WAITING}: {body}");
    let id = device.to_string();
    body["devices"]
        .as_array()
        .is_some_and(|devices| devices.iter().any(|entry| entry["device"] == id.as_str()))
}

/// The account's devices with an open run.
const RELAYED_WAITING: &str = "/account/links/waiting";

/// `skald link` on `first`'s machine at a terminal, started; gives it once it said to enter only a
/// code of the person's own device.
pub fn start_link(first: &Person, extra: &[&str]) -> Terminal {
    let args = [extra, &[LINK]].concat();
    let mut run = Terminal::start(&first.machine, &args);
    run.wait_to_show(OWN_DEVICE);
    run
}

/// `skald link` on `first`'s machine at a terminal: the person types `entry`, and once it shows
/// the new device named `name` as a `client` device, answers `confirm`. Gives what it showed.
pub fn link_with(first: &Person, entry: &str, name: &str, confirm: &str) -> Shown {
    let mut run = start_link(first, &[]);
    run.type_line(entry);
    run.wait_for("the new device's name and kind", |shown| {
        says(shown, &[name]) && says(shown, &["client"])
    });
    run.type_line(confirm);
    run.finish()
}

/// `skald link` on `first`'s machine at a terminal, the person typing `entry`, which no run of it
/// opens: gives what it showed once it exits.
pub fn link_failing(first: &Person, entry: &str) -> Shown {
    let run = start_link(first, &[]);
    run.type_line(entry);
    let shown = run.finish();
    assert!(!shown.success, "skald link succeeded: {}", shown.text);
    shown
}

/// Links `new` on `first`'s machine with its code: waits for its run, links with the code typed
/// as a person types it and confirms; fails unless both succeed.
#[track_caller]
pub fn link(cloud: &Cloud, first: &Person, new: &mut NewDevice) -> Shown {
    new.open_run(cloud, first);
    let code = new.code();
    let linked = link_with(first, &typed(&code), new.host, YES);
    assert!(linked.success, "skald link failed: {}", linked.text);
    let shown = new.finish();
    assert!(shown.success, "the new device failed: {}", shown.text);
    linked
}
