//! `skald update` against a stand-in of GitHub Releases over HTTPS (`release.rs`): it replaces a
//! copy of `skald` in a temporary directory, never the `skald` these tests build or one on `PATH`.
//!
//! The settings these tests give `skald`:
//! - `SKALD_GITHUB_API_URL`: the base URL of GitHub's REST API, `https://api.github.com` by
//!   default; `skald update` looks up the latest release of `skald-so/skald` there and downloads
//!   its files from their `browser_download_url`;
//! - `SSL_CERT_FILE`: the root certificates `skald` trusts, instead of the platform's;
//! - `HTTPS_PROXY`: the proxy of its HTTPS requests.
//!
//! A `skald` installed from npm has a file `installed-by-npm` in its directory; the `skald-so`
//! package's launcher writes it beside the `skald` it unpacks.

mod release;
#[path = "../common/tls.rs"]
mod tls;

use std::fmt;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use release::{Certificates, Proxy, Release, StandIn, TARGET, archive, checksum, checksum_of};

/// The running version: the version of the `skald` these tests build.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The file beside a `skald` installed from npm.
const NPM_MARKER: &str = "installed-by-npm";

/// The host of the stand-in when only a proxy reaches it: no resolver knows it.
const PROXIED_HOST: &str = "releases.skald.test";

/// The most bytes `skald update` reads of an archive, of a checksum and of the API's answer.
const ARCHIVE_LIMIT: usize = 64 << 20;
const CHECKSUM_LIMIT: usize = 4 << 10;
const API_LIMIT: usize = 4 << 20;

/// How long one run of `skald` may take.
const LIMIT: Duration = Duration::from_secs(120);

/// The major, minor and patch numbers of the running version.
fn numbers() -> [u64; 3] {
    let core = VERSION.split('-').next().expect("a version");
    let numbers: Vec<u64> = core
        .split('.')
        .map(|n| n.parse().expect("a version number"))
        .collect();
    numbers.try_into().expect("major.minor.patch")
}

/// A stable version newer than the running one, `steps` minor versions up.
fn newer(steps: u64) -> String {
    let [major, minor, _] = numbers();
    format!("{major}.{}.0", minor + steps)
}

/// A stable version older than the running one.
fn older() -> String {
    match numbers() {
        [major, minor, patch] if patch > 0 => format!("{major}.{minor}.{}", patch - 1),
        [major, minor, _] if minor > 0 => format!("{major}.{}.0", minor - 1),
        [major, _, _] if major > 0 => format!("{}.0.0", major - 1),
        _ => panic!("no version is older than {VERSION}"),
    }
}

/// A machine with a copy of `skald` in a temporary directory, removed on drop.
struct Machine {
    dir: PathBuf,
    skald: PathBuf,
    original: Vec<u8>,
    certificates: Certificates,
}

impl Machine {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("skald-update-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).expect("create the machine's directory");
        let skald = dir
            .join("bin")
            .join(format!("skald{}", std::env::consts::EXE_SUFFIX));
        fs::copy(env!("CARGO_BIN_EXE_skald"), &skald).expect("copy skald");
        let original = fs::read(&skald).expect("read the copy of skald");
        let certificates = release::certificates(&dir.join("tls"));
        Machine {
            dir,
            skald,
            original,
            certificates,
        }
    }

    /// The release of `tag`, as the release workflow publishes it.
    fn release(&self, tag: &str) -> Release {
        Release::new(&self.dir.join("releases"), tag)
    }

    /// Serves `releases`, newest first, on 127.0.0.1.
    fn serve(&self, releases: Vec<Release>) -> StandIn {
        StandIn::start(&self.certificates, "127.0.0.1", releases)
    }

    /// Runs `skald update` against `releases`, trusting the stand-in's CA.
    fn update(&self, releases: &StandIn) -> Run {
        self.run_update(releases, true, &[])
    }

    /// Runs `skald update` against `releases` with `env` set, trusting the stand-in's CA if
    /// `trusted`; no other proxy or root certificates than `env` gives.
    fn run_update(&self, releases: &StandIn, trusted: bool, env: &[(&str, &str)]) -> Run {
        let mut command = Command::new(&self.skald);
        command.arg("update");
        for name in [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "NO_PROXY",
            "no_proxy",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "SKALD_TIMEOUT",
        ] {
            command.env_remove(name);
        }
        command
            .env("SKALD_GITHUB_API_URL", &releases.url)
            .env("SKALD_STATE_DIR", self.dir.join("state"))
            .env("SKALD_KEY_STORAGE", "file");
        if trusted {
            command.env("SSL_CERT_FILE", &self.certificates.ca);
        }
        command.envs(env.iter().copied());
        run(command, &self.dir)
    }

    /// What the copy of `skald` prints for `--version`, or why it does not run.
    fn version(&self) -> String {
        match Command::new(&self.skald).arg("--version").output() {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).into_owned()
            }
            Ok(output) => format!(
                "a failure, {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(err) => format!("nothing: it does not run: {err}"),
        }
    }

    /// The copy of `skald` is the one the machine started with, and it runs.
    fn assert_unchanged(&self) {
        assert_eq!(self.version(), format!("skald {VERSION}\n"));
        assert!(
            fs::read(&self.skald).ok().as_deref() == Some(&self.original[..]),
            "the copy of skald changed"
        );
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// What a run of `skald` printed, and how it exited.
struct Run {
    success: bool,
    status: String,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Whether the run's output has `text`, in any case.
    fn says(&self, text: &str) -> bool {
        format!("{}\n{}", self.stdout, self.stderr)
            .to_lowercase()
            .contains(&text.to_lowercase())
    }
}

impl fmt::Display for Run {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "skald update: {}\nstdout: {}\nstderr: {}",
            self.status, self.stdout, self.stderr
        )
    }
}

/// Runs `command`, its output in files of `dir`; fails if it runs for longer than `LIMIT`.
fn run(mut command: Command, dir: &Path) -> Run {
    let (out, err) = (dir.join("update.stdout"), dir.join("update.stderr"));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(fs::File::create(&out).expect("create a file for stdout"))
        .stderr(fs::File::create(&err).expect("create a file for stderr"))
        .spawn()
        .expect("run skald");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for skald") {
            break status;
        }
        if started.elapsed() > LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("skald update ran for more than {LIMIT:?}");
        }
        thread::sleep(Duration::from_millis(50));
    };
    Run {
        success: status.success(),
        status: status.to_string(),
        stdout: fs::read_to_string(&out).unwrap_or_default(),
        stderr: fs::read_to_string(&err).unwrap_or_default(),
    }
}

/// `skald update` looked the release up, said the running version is current and changed nothing.
fn assert_current(machine: &Machine, releases: &StandIn, run: &Run) {
    assert!(run.success, "{run}");
    assert!(
        run.says("current") && run.says(VERSION),
        "skald update does not say that {VERSION} is current: {run}"
    );
    assert!(
        releases.looked_up(),
        "skald update did not look up the latest release"
    );
    assert_eq!(
        releases.archives(),
        Vec::<String>::new(),
        "archives downloaded"
    );
    machine.assert_unchanged();
}

/// `skald update` exited with an error and left the old `skald`, which still runs.
fn assert_refused(machine: &Machine, run: &Run) {
    assert!(!run.success, "{run}");
    assert!(!run.stderr.trim().is_empty(), "no error: {run}");
    machine.assert_unchanged();
}

/// `skald update` exited with an error that names the checksum, and left the old `skald`.
fn assert_refused_on_checksum(machine: &Machine, run: &Run) {
    assert_refused(machine, run);
    assert!(
        run.says("checksum"),
        "the error does not name the checksum: {run}"
    );
}

/// `skald update` installed the release of `tag`, from this machine's archive, the only one it
/// downloaded.
fn assert_installed(machine: &Machine, releases: &StandIn, run: &Run, tag: &str) {
    assert!(run.success, "{run}");
    let version = tag.strip_prefix('v').expect("a release tag");
    assert_eq!(machine.version(), format!("skald {version}\n"));
    assert_eq!(releases.archives(), vec![archive_of(tag)]);
}

/// This machine's archive in the release of `tag`, as `<tag>/<name>`.
fn archive_of(tag: &str) -> String {
    format!("{tag}/{}", archive(TARGET))
}

#[test]
fn newer_release_replaces_the_binary() {
    let machine = Machine::new("newer");
    let tag = format!("v{}", newer(1));
    let releases = machine.serve(vec![
        machine.release(&tag),
        machine.release(&format!("v{VERSION}")),
    ]);

    let run = machine.update(&releases);

    assert_installed(&machine, &releases, &run, &tag);
}

#[test]
fn newer_stable_release_is_installed_not_a_newer_beta() {
    let machine = Machine::new("stable-over-beta");
    let stable = format!("v{}", newer(1));
    let releases = machine.serve(vec![
        machine.release(&format!("v{}-beta.1", newer(2))),
        machine.release(&stable),
        machine.release(&format!("v{VERSION}")),
    ]);

    let run = machine.update(&releases);

    assert_installed(&machine, &releases, &run, &stable);
}

// On a beta build no stable release equals the running version: `v{VERSION}` is a beta, so this is
// the case of an older release, and the unit test `installs_only_a_newer_stable_release` covers
// the same version.
#[test]
fn same_version_changes_nothing() {
    let machine = Machine::new("same");
    let releases = machine.serve(vec![
        machine.release(&format!("v{VERSION}")),
        machine.release(&format!("v{}", older())),
    ]);

    let run = machine.update(&releases);

    assert_current(&machine, &releases, &run);
}

#[test]
fn older_release_changes_nothing() {
    let machine = Machine::new("older");
    let releases = machine.serve(vec![machine.release(&format!("v{}", older()))]);

    let run = machine.update(&releases);

    assert_current(&machine, &releases, &run);
}

#[test]
fn newer_beta_changes_nothing() {
    let machine = Machine::new("beta");
    let releases = machine.serve(vec![
        machine.release(&format!("v{}-beta.1", newer(1))),
        machine.release(&format!("v{VERSION}")),
    ]);

    let run = machine.update(&releases);

    assert_current(&machine, &releases, &run);
}

#[test]
fn no_stable_release_changes_nothing() {
    let machine = Machine::new("no-stable");
    // The API answers 404 for the latest release: every release is a beta.
    let releases = machine.serve(vec![machine.release(&format!("v{}-beta.1", newer(1)))]);

    let run = machine.update(&releases);

    assert_current(&machine, &releases, &run);
}

#[test]
fn mismatched_checksum_leaves_the_old_binary() {
    let machine = Machine::new("mismatched");
    let name = archive(TARGET);
    // A well-formed checksum, of other bytes.
    let release = machine
        .release(&format!("v{}", newer(1)))
        .with(&checksum_of(&name), checksum(&name, b"other bytes"));
    let releases = machine.serve(vec![release]);

    let run = machine.update(&releases);

    assert_refused_on_checksum(&machine, &run);
}

#[test]
fn missing_checksum_leaves_the_old_binary() {
    let machine = Machine::new("no-checksum");
    let release = machine
        .release(&format!("v{}", newer(1)))
        .without(&checksum_of(&archive(TARGET)));
    let releases = machine.serve(vec![release]);

    let run = machine.update(&releases);

    assert_refused_on_checksum(&machine, &run);
}

#[test]
fn missing_archive_leaves_the_old_binary() {
    let machine = Machine::new("no-archive");
    let release = machine
        .release(&format!("v{}", newer(1)))
        .without(&archive(TARGET));
    let releases = machine.serve(vec![release]);

    let run = machine.update(&releases);

    assert_refused(&machine, &run);
    assert!(
        releases.looked_up(),
        "skald update did not look up the latest release"
    );
}

#[test]
fn cut_connection_leaves_the_old_binary() {
    let machine = Machine::new("cut");
    let tag = format!("v{}", newer(1));
    let release = machine.release(&tag).cut(&archive(TARGET));
    let releases = machine.serve(vec![release]);

    let run = machine.update(&releases);

    assert_refused(&machine, &run);
    assert_eq!(
        releases.archives(),
        vec![archive_of(&tag)],
        "no download cut off"
    );
}

#[test]
fn untrusted_certificate_leaves_the_old_binary() {
    let machine = Machine::new("untrusted");
    let releases = machine.serve(vec![machine.release(&format!("v{}", newer(1)))]);

    // The platform's root certificates only: none of them issued the stand-in's.
    let run = machine.run_update(&releases, false, &[]);

    assert_refused(&machine, &run);
    assert!(
        releases.connections() > 0,
        "skald update did not connect to the release"
    );
    assert!(
        releases.requests().is_empty(),
        "requests over a connection skald should not trust: {:?}",
        releases.requests()
    );
}

#[test]
fn requests_go_through_the_https_proxy() {
    let machine = Machine::new("proxy");
    let tag = format!("v{}", newer(1));
    let releases = StandIn::start(
        &machine.certificates,
        PROXIED_HOST,
        vec![machine.release(&tag)],
    );
    let authority = format!("{PROXIED_HOST}:{}", releases.addr.port());
    let proxy = Proxy::start(authority.clone(), releases.addr);

    let run = machine.run_update(&releases, true, &[("HTTPS_PROXY", &proxy.url)]);

    assert_installed(&machine, &releases, &run, &tag);
    let tunnels = proxy.requests();
    assert!(!tunnels.is_empty(), "no request went through the proxy");
    assert!(
        tunnels
            .iter()
            .all(|line| *line == format!("CONNECT {authority} HTTP/1.1")),
        "the proxy got other requests than tunnels to {authority}: {tunnels:?}"
    );
    let hosts: Vec<String> = releases.requests().into_iter().map(|r| r.host).collect();
    assert!(
        hosts.iter().all(|host| *host == authority),
        "requests for other hosts than {authority}: {hosts:?}"
    );
}

#[test]
fn npm_install_is_left_alone() {
    let machine = Machine::new("npm");
    fs::write(machine.skald.with_file_name(NPM_MARKER), "").expect("mark the install as npm's");
    let releases = machine.serve(vec![machine.release(&format!("v{}", newer(1)))]);

    let run = machine.update(&releases);

    assert!(
        run.says("npm"),
        "skald update does not say to update through npm: {run}"
    );
    assert_eq!(
        releases.archives(),
        Vec::<String>::new(),
        "archives downloaded"
    );
    machine.assert_unchanged();
}

#[test]
fn archive_over_its_limit_leaves_the_old_binary() {
    let machine = Machine::new("archive-limit");
    let tag = format!("v{}", newer(1));
    let name = archive(TARGET);
    // Its checksum is the release's archive's: only the limit stops it before the checksum does.
    let release = machine
        .release(&tag)
        .with(&name, vec![0; ARCHIVE_LIMIT + 1]);
    let releases = machine.serve(vec![release]);

    let run = machine.update(&releases);

    assert_refused(&machine, &run);
    assert!(
        !run.says("checksum") && run.says(&name),
        "the error is not that the archive is too large: {run}"
    );
    assert_eq!(releases.archives(), vec![archive_of(&tag)]);
}

#[test]
fn checksum_over_its_limit_leaves_the_old_binary() {
    let machine = Machine::new("checksum-limit");
    let release = machine.release(&format!("v{}", newer(1)));
    let name = checksum_of(&archive(TARGET));
    // The archive's checksum, padded with blank lines past the limit.
    let mut padded = release.bytes(&name);
    padded.resize(CHECKSUM_LIMIT + 1, b'\n');
    let releases = machine.serve(vec![release.with(&name, padded)]);

    let run = machine.update(&releases);

    assert_refused_on_checksum(&machine, &run);
    assert_eq!(releases.archives(), Vec::<String>::new());
}

#[test]
fn api_answer_over_its_limit_leaves_the_old_binary() {
    let machine = Machine::new("api-limit");
    // A file whose name alone makes the API's answer longer than the limit.
    let release = machine
        .release(&format!("v{}", newer(1)))
        .adding(&"x".repeat(API_LIMIT), Vec::new());
    let releases = machine.serve(vec![release]);

    let run = machine.update(&releases);

    assert_refused(&machine, &run);
    assert!(
        releases.looked_up(),
        "skald update did not look up the latest release"
    );
    assert_eq!(releases.archives(), Vec::<String>::new());
}

#[test]
fn redirect_to_plain_http_leaves_the_old_binary() {
    let machine = Machine::new("plain-http");
    // A plain HTTP server that counts the connections it gets and answers none.
    let plain = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let to = format!(
        "http://{}/{}",
        plain.local_addr().expect("the server's address"),
        archive(TARGET)
    );
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&connections);
    thread::spawn(move || {
        for stream in plain.incoming().flatten() {
            counted.fetch_add(1, Ordering::Relaxed);
            drop(stream);
        }
    });
    let release = machine
        .release(&format!("v{}", newer(1)))
        .redirect(&archive(TARGET), to);
    let releases = machine.serve(vec![release]);

    let run = machine.update(&releases);

    assert_refused(&machine, &run);
    assert_eq!(
        connections.load(Ordering::Relaxed),
        0,
        "skald update followed a redirect to plain HTTP"
    );
}

#[test]
fn files_set_aside_by_earlier_updates_are_removed() {
    let machine = Machine::new("set-aside");
    let name = machine
        .skald
        .file_name()
        .expect("a file name")
        .to_string_lossy()
        .into_owned();
    // What an update on Windows leaves beside the skald it replaced, and a file of another program.
    let aside = machine.skald.with_file_name(format!(".{name}.12345.old"));
    let other = machine.skald.with_file_name(".other.12345.old");
    fs::write(&aside, "the skald an earlier update replaced").expect("write a file set aside");
    fs::write(&other, "").expect("write another file");
    let releases = machine.serve(vec![machine.release(&format!("v{VERSION}"))]);

    let run = machine.update(&releases);

    assert_current(&machine, &releases, &run);
    assert!(!aside.exists(), "the file set aside is still there");
    assert!(other.exists(), "another program's file is removed");
}
