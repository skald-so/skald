//! Signing a device key with a GitHub SSH key (`docs/protocol/membership.md`, "Device certificate"
//! and "Linking a device"; `docs/threat-model.md`, "An agent on the person's own device").
//!
//! - A `client` device that issues its own certificate, as the account's first device or with the
//!   recovery key, first makes its `ssh`: the GitHub user is the login of the account's GitHub
//!   sign-in method, or one an option names; the device takes the user's ID and SSH keys from
//!   GitHub's API and signs with a key on that list that ssh-agent holds or that is in the person's
//!   SSH directory, through OpenSSH's `ssh-keygen`. Options name the key or turn signing off.
//! - It says which key of which GitHub user signed, and that members of its spaces see that user.
//! - Without `ssh-keygen`, a GitHub user, a listed key or GitHub, the certificate goes without
//!   `ssh`, the command says why, and the sign-in goes on. The signature is written to the local
//!   state with the certificate before it is sent, so a resend keeps one body.
//! - As an agent, or with no terminal, it signs only with a key ssh-agent holds, so nothing asks
//!   for a passphrase.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, or Google sign-in
//! through the OIDC stand-in, on PostgreSQL, behind the recording relay; a test plays the person
//! on the verification page in headless Chrome, and at a terminal in a pseudo-terminal, or a
//! pseudo-console on Windows (`recovery_key/terminal.rs`).
//!
//! GitHub's API is a GitHub stand-in of its own ([`Api`]), apart from the one the server signs in
//! with, reached through the hidden setting [`GITHUB_API`] (`github_checks/main.rs`). It gives
//! Ada's GitHub user, by the login she signs in with, and the SSH keys the tests list for it.
//!
//! The machines' `PATH` holds nothing but what a test puts there: [`with_ssh_keygen`] puts
//! OpenSSH's `ssh-keygen` of the machine running the tests there. The tests make SSH keys with that
//! `ssh-keygen` in the machine's home, `<home>/.ssh` being the person's SSH directory, and, on
//! Unix, run an ssh-agent of their own (`agent.rs`) whose socket only the machine's runs get through
//! `SSH_AUTH_SOCK`. Nothing touches the real home, SSH directory or ssh-agent of the person running
//! the tests.
//!
//! The CLI, as these tests use it, beside `skald sign-in` (`device_keys/main.rs`,
//! `recovery_linking/main.rs`) and `skald members <space>` (`github_checks/main.rs`):
//! - `skald sign-in --ssh-key <file>` signs with the key of that private key file; `--no-ssh`
//!   signs with none; `--github <login>` takes that GitHub user in place of the account's GitHub
//!   sign-in method's;
//! - a run that signs says so on a line holding the key's SHA256 fingerprint, as `ssh-keygen -l`
//!   shows it, and the GitHub user's login as a word, and, on a line holding that login, that
//!   `members` see it;
//! - a run whose certificate goes without `ssh` because of a missing `ssh-keygen`, GitHub user,
//!   listed key or GitHub says so on a line holding `SSH key`, in any case, and the reason:
//!   `ssh-keygen` when it is missing; `GitHub` without a GitHub user and when GitHub cannot be
//!   reached or answers with an error; the GitHub user's login as a word without a listed key it
//!   can sign with;
//! - `ssh-keygen` asks for a key file's passphrase with a line holding `passphrase`, in any case.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[cfg(unix)]
mod agent;
mod passphrase;
mod recovery;
mod resend;
mod signing;
mod without;

// The modules the tests of the channel history share (`history_common/mod.rs`), and the terminal,
// signing in there and linking with the recovery key of the tests of the recovery key and of
// linking with it.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../recovery_linking/recovering.rs"]
mod linking;
#[path = "../recovery_key/terminal.rs"]
mod terminal;

use std::ffi::OsString;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use skald_crypto::{SshCheck, check_ssh_signature};
use skald_github_stand_in::{App, GitHub, SshKeys, User};
use skald_proto::{Cert, DeviceId};

use at_terminal::{as_agent, first_at_terminal, shown_key};
use machine::{Finished, Machine};
use world::{ACME, Person, World, create_with_member, words};

/// The hidden setting of the GitHub API's address.
pub const GITHUB_API: &str = "SKALD_GITHUB_API_URL";
/// The setting through which OpenSSH reaches ssh-agent.
pub const SSH_AUTH_SOCK: &str = "SSH_AUTH_SOCK";

/// The options of `skald sign-in` that name the key, turn signing off and name the GitHub user.
pub const SSH_KEY: &str = "--ssh-key";
pub const NO_SSH: &str = "--no-ssh";
pub const GITHUB: &str = "--github";

/// What the line holds that says a certificate goes without `ssh`.
pub const SAYS_NO_SSH: &str = "ssh key";
/// What the line holds that says members see the GitHub user.
pub const SAYS_MEMBERS: &str = "members";
/// What `ssh-keygen` asks for a key file's passphrase with.
pub const ASKS_PASSPHRASE: &str = "passphrase";

/// The GitHub users Ada and Bob sign in as, and their logins; another GitHub user of Ada's, which
/// an option names.
pub const ADA_GITHUB: u64 = 7_350_101;
pub const ADA_LOGIN: &str = "ada-ssh-k4t9";
pub const BOB_GITHUB: u64 = 7_350_102;
pub const BOB_LOGIN: &str = "bob-ssh-m2x8";
pub const WORK_GITHUB: u64 = 7_350_103;
pub const WORK_LOGIN: &str = "ada-work-r5v1";

/// The host names of Ada's first machine, of her machine linked with the recovery key and of Bob's
/// machine.
pub const ADA_HOST: &str = "ada-workstation";
pub const NEW_HOST: &str = "ada-recovered";
pub const BOB_HOST: &str = "bob-workstation";

/// The passphrase of the tests' encrypted key files.
pub const PASSPHRASE: &str = "correct horse 7q2w";

/// A GitHub stand-in that plays GitHub's API.
pub struct Api {
    pub github: GitHub,
}

impl Api {
    /// A stand-in with no users.
    pub fn start() -> Self {
        Api {
            github: GitHub::start(App {
                client_id: "Ov23li0standin0ssh0api".to_owned(),
                client_secret: "standin-secret-ssh-api-9a8b7c6d5e4f".to_owned(),
                callback: None,
            }),
        }
    }

    /// Its address, for [`GITHUB_API`].
    pub fn url(&self) -> String {
        self.github.url().to_owned()
    }

    /// Makes it give the GitHub user `id` with `login`, with the SSH keys `authentication` and
    /// `signing` and nothing else, as GitHub lists them: without their comments.
    pub fn list(&self, (id, login): (u64, &str), authentication: &[&SshKey], signing: &[&SshKey]) {
        self.github.put_user(User::new(id, login));
        let lines = |keys: &[&SshKey]| keys.iter().map(|key| key.line.clone()).collect();
        self.github.put_ssh_keys(SshKeys {
            user: id,
            authentication: lines(authentication),
            signing: lines(signing),
        });
    }
}

/// An address on this machine where nothing answers: GitHub's API, unreachable.
pub fn nowhere() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let port = listener.local_addr().expect("the port's address").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

/// A machine with the host name `host` whose runs reach GitHub's API at `api`.
pub fn machine(host: &str, api: &str) -> Machine {
    Machine::new()
        .with_host_name(host)
        .with_env(GITHUB_API, api)
}

/// `machine` with OpenSSH's `ssh-keygen` on its `PATH`.
pub fn with_ssh_keygen(machine: Machine) -> Machine {
    let tool = openssh_tool("ssh-keygen");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&tool, machine.root().join("bin").join("ssh-keygen"))
            .expect("put ssh-keygen on the machine's PATH");
        machine
    }
    #[cfg(windows)]
    {
        // Windows' OpenSSH keeps its programs and their libraries in a directory of their own.
        let dir = tool.parent().expect("ssh-keygen's directory");
        let path =
            std::env::join_paths([machine.root().join("bin"), dir.to_path_buf()]).expect("a PATH");
        machine.with_env("PATH", path)
    }
}

/// The OpenSSH program `name` of the machine running the tests: on its `PATH`, or where its system
/// keeps OpenSSH. Fails without it: the tests need OpenSSH.
pub fn openssh_tool(name: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        dirs.push(PathBuf::from(root).join("System32").join("OpenSSH"));
    }
    #[cfg(unix)]
    dirs.extend(["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"].map(PathBuf::from));
    dirs.into_iter()
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{name} is missing; install OpenSSH: https://www.openssh.com"))
}

/// The OpenSSH program `name` with an empty environment but what Windows needs to run it.
pub fn openssh(name: &str) -> Command {
    let mut command = Command::new(openssh_tool(name));
    command.env_clear();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    command
}

/// Runs `command`; fails unless it succeeds. Gives its standard output.
#[track_caller]
pub fn succeed(command: &mut Command) -> String {
    let output = command.output().expect("run an OpenSSH program");
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("OpenSSH prints UTF-8")
}

/// The person's SSH directory on `machine`, made if missing.
pub fn ssh_dir(machine: &Machine) -> PathBuf {
    let dir = machine.root().join("home").join(".ssh");
    fs::create_dir_all(&dir).expect("make the SSH directory");
    dir
}

/// A directory on `machine` outside the person's SSH directory, made if missing.
pub fn keys_dir(machine: &Machine) -> PathBuf {
    let dir = machine.root().join("keys");
    fs::create_dir_all(&dir).expect("make a directory of keys");
    dir
}

/// The type of an SSH key the tests make.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Ed25519,
    /// ECDSA P-256, whose signatures differ each time, unlike Ed25519's.
    Ecdsa,
}

/// An SSH key the tests made.
#[derive(Debug, Clone)]
pub struct SshKey {
    /// Its private key file; the public key is beside it, with `.pub` appended.
    pub path: PathBuf,
    /// Its public key as an OpenSSH public key line without a comment, as GitHub's API lists it.
    pub line: String,
    /// Its SHA256 fingerprint, as `ssh-keygen -l` shows it: `SHA256:<base64>`.
    pub fingerprint: String,
}

impl SshKey {
    /// Its public key file.
    pub fn public(&self) -> PathBuf {
        let mut name = OsString::from(self.path.as_os_str());
        name.push(".pub");
        PathBuf::from(name)
    }

    /// Removes its files.
    pub fn remove_files(&self) {
        fs::remove_file(&self.path).expect("remove a private key file");
        fs::remove_file(self.public()).expect("remove a public key file");
    }

    /// Its private key file's path as text.
    pub fn file(&self) -> &str {
        self.path.to_str().expect("a UTF-8 path")
    }
}

/// Makes an SSH key of `kind` with `ssh-keygen` in the file `path`, encrypted with `passphrase`,
/// or unencrypted if it is empty.
pub fn new_key(path: &Path, kind: Kind, passphrase: &str) -> SshKey {
    let kind = match kind {
        Kind::Ed25519 => "ed25519",
        Kind::Ecdsa => "ecdsa",
    };
    succeed(
        openssh("ssh-keygen")
            .args([
                "-q",
                "-t",
                kind,
                "-C",
                "skald-test-key",
                "-N",
                passphrase,
                "-f",
            ])
            .arg(path),
    );
    let mut key = SshKey {
        path: path.to_path_buf(),
        line: String::new(),
        fingerprint: String::new(),
    };
    let public = fs::read_to_string(key.public()).expect("read a public key file");
    let fields: Vec<&str> = public.split_whitespace().take(2).collect();
    key.line = fields.join(" ");
    let listed = succeed(openssh("ssh-keygen").arg("-l").arg("-f").arg(key.public()));
    key.fingerprint = (listed.split_whitespace().nth(1))
        .expect("a fingerprint")
        .to_owned();
    assert!(key.fingerprint.starts_with("SHA256:"), "{listed}");
    key
}

/// An SSH key that is on no machine, for GitHub's API to list beside the others.
pub fn elsewhere(machine: &Machine, name: &str) -> SshKey {
    new_key(&keys_dir(machine).join(name), Kind::Ed25519, "")
}

/// The person of GitHub user `id` with `login` signs `machine`, with the host name `host`, in as
/// the first device of a new account, with the options `extra`, as [`Cloud::signed_in`] does,
/// without a terminal; fails unless the run succeeds. Gives the person and what the run printed.
///
/// [`Cloud::signed_in`]: cloud::Cloud::signed_in
#[track_caller]
pub fn sign_up(
    world: &World,
    (id, login): (u64, &str),
    (machine, host): (Machine, &'static str),
    extra: &[&str],
) -> (Person, Finished) {
    let user = world.cloud.user(id, login);
    let signed_in = world
        .cloud
        .signed_in(&world.browser.page(), &machine, &user, extra);
    assert!(signed_in.success, "the sign-in failed: {signed_in:?}");
    (world.person(user, machine, host), signed_in)
}

/// As [`sign_up`], at a terminal, as the person or, with `agent`, as an agent: the person approves
/// the code, then, as the person, presses Enter once the run shows the recovery key; as an agent it
/// shows none. Gives the person, the recovery key the run showed, if any, and what it showed.
#[track_caller]
pub fn sign_up_at_terminal(
    world: &World,
    (id, login): (u64, &str),
    (machine, host): (Machine, &'static str),
    agent: bool,
) -> (Person, Option<String>, String) {
    let user = world.cloud.user(id, login);
    let page = world.browser.page();
    let before = if agent { as_agent(&[]) } else { Vec::new() };
    let mut run = first_at_terminal(&world.cloud, &page, &machine, &user, &before);
    let key = (!agent).then(|| shown_key(&mut run));
    if key.is_some() {
        run.enter();
    }
    let shown = run.finish();
    assert!(shown.success, "the sign-in failed: {shown:?}");
    (world.person(user, machine, host), key, shown.text)
}

/// A server and GitHub's API, and Ada's first machine, not yet signed in, with an SSH key in her
/// SSH directory.
pub struct Setup {
    pub world: World,
    pub api: Api,
    pub machine: Machine,
    /// The key in her SSH directory.
    pub key: SshKey,
}

impl Setup {
    /// `ssh-keygen` on the machine's `PATH`, and an Ed25519 key in `id_ed25519` in Ada's SSH
    /// directory, encrypted with `passphrase` unless it is empty.
    pub fn new(passphrase: &str) -> Self {
        Self::with_key(true, ("id_ed25519", Kind::Ed25519), passphrase)
    }

    /// As [`Setup::new`], unencrypted, but no `ssh-keygen` on the machine's `PATH`.
    pub fn without_ssh_keygen() -> Self {
        Self::with_key(false, ("id_ed25519", Kind::Ed25519), "")
    }

    /// `ssh-keygen` on the machine's `PATH` if `ssh_keygen`, and a key of `kind` in the file `name`
    /// in Ada's SSH directory, encrypted with `passphrase` unless it is empty.
    pub fn with_key(ssh_keygen: bool, (name, kind): (&str, Kind), passphrase: &str) -> Self {
        let world = World::start();
        let api = Api::start();
        let mut machine = machine(ADA_HOST, &api.url());
        if ssh_keygen {
            machine = with_ssh_keygen(machine);
        }
        let key = new_key(&ssh_dir(&machine).join(name), kind, passphrase);
        Setup {
            world,
            api,
            machine,
            key,
        }
    }

    /// The machine's runs reach GitHub's API at `api` instead.
    pub fn reaching(mut self, api: &str) -> Self {
        self.machine = self.machine.with_env(GITHUB_API, api);
        self
    }

    /// Makes GitHub's API give Ada's GitHub user with the SSH keys `authentication` and `signing`.
    pub fn list(&self, authentication: &[&SshKey], signing: &[&SshKey]) {
        self.api
            .list((ADA_GITHUB, ADA_LOGIN), authentication, signing);
    }

    /// Makes GitHub's API give Ada's GitHub user with the key in her SSH directory, as an
    /// authentication key, alone.
    pub fn list_key(&self) {
        self.list(&[&self.key], &[]);
    }

    /// Ada signs up on the machine, with the options `extra`, as [`sign_up`] does.
    #[track_caller]
    pub fn sign_up(self, extra: &[&str]) -> Signed {
        let identity = (ADA_GITHUB, ADA_LOGIN);
        let (ada, signed_in) = sign_up(&self.world, identity, (self.machine, ADA_HOST), extra);
        Signed {
            world: self.world,
            api: self.api,
            ada,
            key: self.key,
            shown: signed_in.shown(),
        }
    }

    /// Ada signs up on the machine at a terminal, as [`sign_up_at_terminal`] does.
    #[track_caller]
    pub fn sign_up_at_terminal(self, agent: bool) -> Signed {
        let identity = (ADA_GITHUB, ADA_LOGIN);
        let machine = (self.machine, ADA_HOST);
        let (ada, _, shown) = sign_up_at_terminal(&self.world, identity, machine, agent);
        Signed {
            world: self.world,
            api: self.api,
            ada,
            key: self.key,
            shown,
        }
    }
}

/// A [`Setup`] once Ada signed up.
pub struct Signed {
    pub world: World,
    pub api: Api,
    pub ada: Person,
    pub key: SshKey,
    /// What her sign-in showed.
    pub shown: String,
}

impl Signed {
    /// Her first device's certificate, as the server holds it.
    pub fn cert(&self) -> Cert {
        own_cert(&self.world, &self.ada)
    }

    /// Fails unless her certificate names the GitHub user `(id, login)` and carries a signature by
    /// `key`, not by any of `others`, and her sign-in said so.
    #[track_caller]
    pub fn assert_signed(&self, (id, login): (u64, &str), key: &SshKey, others: &[&SshKey]) {
        assert_signed_by(&self.cert(), id, key, others);
        assert_says_signed(&self.shown, key, login);
    }

    /// Fails unless her certificate goes without `ssh` and her sign-in said so, for `reason`.
    #[track_caller]
    pub fn assert_unsigned(&self, reason: &str) {
        assert_unsigned(&self.cert());
        assert_says_unsigned(&self.shown, reason);
    }
}

/// `person`'s first device's certificate, as the server holds it.
#[track_caller]
pub fn own_cert(world: &World, person: &Person) -> Cert {
    (world.cloud.cert(&person.token, person.device)).expect("the device's certificate")
}

/// Fails unless `cert` names the GitHub user `github` and carries an SSH signature by `key`, which
/// passes the check against a list of `key` alone and fails against `others` alone.
#[track_caller]
pub fn assert_signed_by(cert: &Cert, github: u64, key: &SshKey, others: &[&SshKey]) {
    let body = cert.body();
    assert!(body.ssh.is_some(), "no SSH signature: {body:?}");
    assert_eq!(body.github, Some(github), "the GitHub user: {body:?}");
    assert_eq!(
        check_ssh_signature(body, &[&key.line]),
        SshCheck::Passed { key: 0 },
        "the signature is not by {}: {body:?}",
        key.fingerprint
    );
    for other in others {
        assert_ne!(
            check_ssh_signature(body, &[&other.line]),
            SshCheck::Passed { key: 0 },
            "the signature is by {}: {body:?}",
            other.fingerprint
        );
    }
}

/// Fails unless `cert` carries no SSH signature and names no GitHub user.
#[track_caller]
pub fn assert_unsigned(cert: &Cert) {
    let body = cert.body();
    assert_eq!(body.ssh, None, "an SSH signature: {body:?}");
    assert_eq!(body.github, None, "a GitHub user: {body:?}");
}

/// Fails unless `shown` says that `key` of the GitHub user `login` signed, and that members see
/// that user.
#[track_caller]
pub fn assert_says_signed(shown: &str, key: &SshKey, login: &str) {
    let naming: Vec<&str> = (shown.lines())
        .filter(|line| words(line).contains(&login))
        .collect();
    assert!(
        naming.iter().any(|line| line.contains(&key.fingerprint)),
        "no line names {} and {login}: {shown}",
        key.fingerprint
    );
    assert!(
        (naming.iter()).any(|line| line.to_lowercase().contains(SAYS_MEMBERS)),
        "no line says members see {login}: {shown}"
    );
}

/// Fails unless `shown` says on a line that the certificate goes without an SSH signature, and
/// why: that line holds `reason`, in any case.
#[track_caller]
pub fn assert_says_unsigned(shown: &str, reason: &str) {
    let reason = reason.to_lowercase();
    assert!(
        (shown.lines().map(str::to_lowercase))
            .any(|line| line.contains(SAYS_NO_SSH) && line.contains(&reason)),
        "no line says the certificate goes without an SSH signature for {reason}: {shown}"
    );
}

/// As [`assert_says_unsigned`], the reason the GitHub user's login `login` as a word.
#[track_caller]
pub fn assert_says_unsigned_naming(shown: &str, login: &str) {
    assert!(
        (shown.lines()).any(|line| {
            line.to_lowercase().contains(SAYS_NO_SSH) && words(line).contains(&login)
        }),
        "no line says the certificate goes without an SSH signature by a key of {login}: {shown}"
    );
}

/// Fails if `shown` asks for a passphrase.
#[track_caller]
pub fn assert_asks_nothing(shown: &str) {
    assert!(
        !shown.to_lowercase().contains(ASKS_PASSPHRASE),
        "a passphrase asked: {shown}"
    );
}

/// Bob, whose runs reach GitHub's API at `api`, signs up on a machine without `ssh-keygen`,
/// creates `acme` and stores its revision 3, signed by his device, which adds `ada` as a member.
#[track_caller]
pub fn bob_with(world: &World, api: &str, ada: &Person) -> Person {
    let (bob, _) = sign_up(
        world,
        (BOB_GITHUB, BOB_LOGIN),
        (machine(BOB_HOST, api), BOB_HOST),
        &[],
    );
    create_with_member(&world.cloud, &bob, ACME, ada);
    bob
}

/// Fails unless `bob`'s `skald members acme` shows `device` with the GitHub login `login` on a
/// line that is no warning, and no warning names `device`: its signature passed his check.
#[track_caller]
pub fn assert_member_sees_passed(bob: &Person, device: DeviceId, login: &str) {
    let shown = world::members(&bob.machine, ACME);
    let id = device.to_string();
    assert!(
        (world::plain_lines(&shown).iter())
            .any(|line| line.contains(&id) && words(line).contains(&login)),
        "{device} is not shown with {login}: {shown:?}"
    );
    let id = id.to_lowercase();
    let warned: Vec<String> = (world::warnings(&shown).into_iter())
        .filter(|line| line.contains(&id))
        .collect();
    assert_eq!(warned, Vec::<String>::new(), "a warning of {device}");
}
