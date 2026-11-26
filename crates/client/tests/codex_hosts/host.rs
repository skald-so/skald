//! What the checks with both hosts share: Ada's machine, alone in `clienta`, with a Codex session's
//! folder `~/work/clientA/session` and its config home `~/.codex`; the model stub; Skald's setup and
//! the config the checks add for the stub; the session's settings; and what the stub saw of a
//! session.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::app_server::{AppServer, noticed_models};
use crate::claude;
use crate::codex::{CODEX, CODEX_HOME, Codex, Thread, codex_client};
use crate::hooks::api_sends;
use crate::lines::reference;
use crate::machine::THIS_DEVICE;
use crate::stub::{PLACEHOLDER, Step, Stub, output_of, played};
use crate::talk::ADA_HOST;

/// The setting naming a directory where a check saves what the hosts and Codex gave it, from which
/// the recorded outputs of the unit tests come.
const RECORD: &str = "SKALD_HOSTS_RECORD";

/// The setting naming the key of the stub's model provider.
const STUB_KEY: &str = "SKALD_STUB_API_KEY";

/// The session's model: none of Codex's catalog, so Codex sends its tools as the stub reads them,
/// in `tools`, and offers no newer model.
pub const STUB_MODEL: &str = "skald-stub";

/// How many times the terminal UI counts each availability notice shown, so it shows none.
const NOTICES_SEEN: u32 = 100;

/// What `inbox` answers a session with no messages.
const NO_MESSAGES: &str = "No new messages.";

/// The command the session runs as Skald's CLI.
const CLI: &str = "skald devices";

/// How long a check waits for a session's turn, or for a host.
pub const WAIT: Duration = Duration::from_secs(120);

/// How long a session takes after its turn ends to run its `Stop` hooks.
const SETTLE: Duration = Duration::from_secs(5);

/// The length of a device's reference in a default agent name.
const DEVICE_REFERENCE: usize = 6;

/// The setup command and the agent CLI it sets Skald up in.
const SETUP: [&str; 2] = ["setup", CODEX];

/// What Codex prints of a shell command that succeeded.
const EXITED_0: &str = "exited with code 0";

/// The text of the signal, as it starts a woken turn.
const SIGNAL: &str = "messages wait for you";

/// The program `name` on `PATH`, if any; without it, the check says it is skipped, or fails as the
/// e2e tests do.
pub fn on_path(name: &str) -> Option<PathBuf> {
    claude::on_path(&[name], name)
}

/// The version `program --version` prints: its last word.
pub fn version(program: &Path) -> String {
    let printed = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .expect("run --version");
    let text = String::from_utf8_lossy(&printed.stdout);
    (text.split_whitespace().last())
        .unwrap_or_default()
        .to_owned()
}

/// Saves `text` as `name` in the directory [`RECORD`] names, if set.
pub fn record(name: &str, text: &str) {
    if let Some(dir) = env::var_os(RECORD) {
        fs::create_dir_all(&dir).expect("make the record directory");
        fs::write(Path::new(&dir).join(name), text).expect("save a record");
    }
}

/// `written`, a file a host wrote into the config home `home`, as the fixtures hold it: in Ada's
/// config home `/home/ada/.codex`.
pub fn as_fixture(written: &str, home: &Path) -> String {
    written.replace(&*home.to_string_lossy(), "/home/ada/.codex")
}

/// How a session went, as the check prints it.
#[derive(Debug, Default)]
pub struct Results {
    /// The `inbox` tool of Skald's MCP server answered.
    pub mcp: bool,
    /// `skald devices` succeeded in the session.
    pub cli: bool,
    /// After its turn ended, a message woke it: how long after sending the woken turn reached the
    /// stub.
    pub woke: Option<Duration>,
    /// No request to the stub held the message's text.
    pub text_stayed_out: bool,
}

impl Results {
    /// The result line of a session in `host`.
    pub fn line(&self, host: &str) -> String {
        let yes = |done: bool| if done { "yes" } else { "no" };
        let woke = match self.woke {
            Some(after) => format!("yes, {:.1} s", after.as_secs_f64()),
            None => "no".to_owned(),
        };
        format!(
            "{host}: MCP server {}; CLI {}; idle wake by the hooks' registration and codex queue {woke}; message text kept out {}",
            yes(self.mcp),
            yes(self.cli),
            yes(self.text_stayed_out),
        )
    }
}

/// Ada's machine with a Codex session's folder and config home, and the stub.
pub struct Live {
    pub codex: Codex,
    pub session: Thread,
    pub stub: Stub,
    /// The real `codex`.
    pub program: PathBuf,
}

impl Live {
    /// Ada, GitHub user `id` with `login`, alone in `clienta`, and `codex` at `program`.
    pub fn new(id: u64, login: &str, program: PathBuf) -> Self {
        let codex = Codex::new(id, login);
        let dir = codex.web.solo.dir("session");
        let session = Thread::new(codex.machine(), &dir, None);
        let stub = Stub::start(vec![Step::Skald("inbox", json!({})), Step::Bash(CLI)], None);
        Live {
            codex,
            session,
            stub,
            program,
        }
    }

    /// The session's folder.
    pub fn dir(&self) -> &Path {
        &self.session.dir
    }

    /// The session's config home.
    pub fn home(&self) -> &Path {
        &self.session.home
    }

    /// The settings of the session's processes, and of the hosts that start them: those of the
    /// machine's runs of `skald`; `PATH` with its `skald`, `codex` and the dirs of `programs`;
    /// the config home; the stub's key.
    pub fn settings(&self, programs: &[&Path]) -> Vec<(String, String)> {
        let template = self.codex.helped.command_in(self.dir(), &[], &[]);
        let lossy = |text: &std::ffi::OsStr| text.to_string_lossy().into_owned();
        let mut settings: Vec<(String, String)> = (template.get_envs())
            .filter_map(|(name, value)| Some((lossy(name), lossy(value?))))
            .filter(|(name, _)| name != "PATH")
            .collect();
        let bin = self.codex.helped.exe().parent().expect("skald's folder");
        let mut path = vec![bin.to_owned()];
        for program in [self.program.as_path()].iter().chain(programs) {
            path.extend(program.parent().map(Path::to_owned));
        }
        path.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
        let path = env::join_paths(path).expect("a PATH");
        settings.extend([
            ("PATH".to_owned(), lossy(&path)),
            (CODEX_HOME.to_owned(), lossy(self.home().as_os_str())),
            (STUB_KEY.to_owned(), PLACEHOLDER.to_owned()),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
        ]);
        settings
    }

    /// `program` with `settings` alone, in the session's folder.
    pub fn command(&self, program: &Path, settings: &[(String, String)]) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs(settings.iter().map(|(name, value)| (name, value)))
            .current_dir(self.dir());
        command
    }

    /// Runs `skald setup codex` on the session's config home; fails unless it succeeds.
    pub fn set_up(&self) {
        let home = self.home().to_string_lossy().into_owned();
        let set_up = (self.codex.helped).run_with(&[(CODEX_HOME, home.as_str())], &SETUP);
        assert!(set_up.success, "the setup failed: {set_up:?}");
    }

    /// Adds to `config.toml` what the session needs on the stub, as a person's config would hold
    /// it: the stub as the model provider, with [`STUB_MODEL`], and the catalog's availability
    /// notices seen, so the terminal UI asks nothing; the folder trusted; full access with no
    /// approval prompts, so the session runs `skald`; no update check. Then trusts every hook, as
    /// Review hooks does, and runs the setup again: it changes nothing, so no hook needs new trust.
    /// Gives the commands of the hooks it trusted.
    pub fn configure(&self, settings: &[(String, String)]) -> Vec<String> {
        let listed =
            AppServer::start(self.command(&self.program, settings)).call("model/list", json!({}));
        record(
            "model-list.json",
            &serde_json::to_string_pretty(&listed).expect("JSON"),
        );
        let noticed = noticed_models(&listed);
        let seen: Vec<String> = (noticed.iter())
            .map(|model| format!("{model:?} = {NOTICES_SEEN}"))
            .collect();
        let seen = seen.join(", ");
        let config = self.home().join("config.toml");
        let written = fs::read_to_string(&config).expect("the setup wrote config.toml");
        // Codex keys a project by its canonical path, such as `/private/var/…` for `/var/…`.
        let canonical = fs::canonicalize(self.dir()).expect("the session's folder");
        let dir = canonical.to_string_lossy().replace('\\', "\\\\");
        let stub = self.stub.url();
        let added = format!(
            "# The live check's: the model stub, the session's folder and access.\n\
             model_provider = \"stub\"\n\
             model = \"{STUB_MODEL}\"\n\
             tui.model_availability_nux = {{ {seen} }}\n\
             model_reasoning_effort = \"low\"\n\
             approval_policy = \"never\"\n\
             sandbox_mode = \"danger-full-access\"\n\
             check_for_update_on_startup = false\n\
             projects.\"{dir}\".trust_level = \"trusted\"\n\
             model_providers.stub = {{ name = \"stub\", base_url = \"{stub}/v1\", env_key = \"{STUB_KEY}\", wire_api = \"responses\" }}\n\n\
             {written}"
        );
        fs::write(&config, added).expect("write config.toml");
        let mut app_server = AppServer::start(self.command(&self.program, settings));
        let (listed, trusted) = app_server.trust_hooks(self.dir());
        record(
            "hooks-list.json",
            &serde_json::to_string_pretty(&listed).expect("JSON"),
        );
        drop(app_server);
        let before = fs::read(&config).expect("config.toml");
        self.set_up();
        assert_eq!(
            fs::read(&config).expect("config.toml"),
            before,
            "the setup changed config.toml after Codex trusted its hooks"
        );
        let mut app_server = AppServer::start(self.command(&self.program, settings));
        let untrusted = app_server.untrusted_hooks(self.dir());
        assert!(untrusted.is_empty(), "hooks not trusted: {untrusted:?}");
        trusted
    }

    /// The agent the session's `skald mcp` and hooks act as: the default name of a Codex session
    /// in its folder.
    pub fn agent(&self) -> String {
        let solo = &self.codex.web.solo;
        let device = reference(MessageId::from_bytes(*solo.talk.ada.device.as_bytes()));
        solo.short(&format!(
            "{}-{ADA_HOST}-{}",
            codex_client(),
            &device[..DEVICE_REFERENCE]
        ))
    }

    /// Waits for a session's first turn, the requests from the `from`th on, to end on the stub;
    /// gives whether its `inbox` call and `skald devices` succeeded.
    pub fn first_turn(&self, from: usize) -> (bool, bool) {
        let ended = self.await_turn(from, |body| played(body) >= 2);
        record("turn.json", &ended.to_string());
        let inbox = output_of(&ended, 0).unwrap_or_default();
        let cli = output_of(&ended, 1).unwrap_or_default();
        (inbox_answered(&inbox), cli_succeeded(&cli))
    }

    /// The first request from the `from`th on for which `done` holds; fails after [`WAIT`].
    pub fn await_turn(&self, from: usize, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            let turns = self.stub.turns();
            if let Some(body) = turns.iter().skip(from).find(|body| done(body)) {
                return body.clone();
            }
            assert!(
                Instant::now() < deadline,
                "the session did not get there within {WAIT:?}: {} requests",
                turns.len()
            );
            thread::sleep(Duration::from_millis(200));
        }
    }

    /// Once the session's turn has ended and it is idle, api sends its agent `text`; gives how long
    /// after that a turn with the signal reached the stub, if one did within [`WAIT`], and whether
    /// no request held the text.
    pub fn wake(&self, text: &str) -> (Option<Duration>, bool) {
        thread::sleep(SETTLE);
        let from = self.stub.turns().len();
        let sent = Instant::now();
        api_sends(&self.codex.web.solo, &self.agent(), text);
        let deadline = sent + WAIT;
        let mut woke = None;
        while woke.is_none() && Instant::now() < deadline {
            let turns = self.stub.turns();
            if let Some(body) = turns.iter().skip(from).find(|body| woken(body)) {
                woke = Some(sent.elapsed());
                record("woken.json", &body.to_string());
            }
            thread::sleep(Duration::from_millis(200));
        }
        let kept_out = !(self.stub.turns().iter()).any(|body| body.to_string().contains(text));
        (woke, kept_out)
    }

    /// The whole check of a session whose first turn starts from the `from`th request: its turn,
    /// then a wake by `text`.
    pub fn results(&self, from: usize, text: &str) -> Results {
        let (mcp, cli) = self.first_turn(from);
        let (woke, text_stayed_out) = self.wake(text);
        self.stub.assert_no_foreign_credential();
        Results {
            mcp,
            cli,
            woke,
            text_stayed_out,
        }
    }
}

/// Whether `output`, what Codex gave the model of an `inbox` call, is the tool's answer for this
/// device: not an error of Codex's, such as an unknown tool, nor the server's of a machine with no
/// device.
pub fn inbox_answered(output: &str) -> bool {
    output.contains(NO_MESSAGES)
}

/// Whether `output`, what Codex gave the model of a shell command, says `skald devices` succeeded
/// and listed this device.
pub fn cli_succeeded(output: &str) -> bool {
    output.contains(EXITED_0) && output.contains(THIS_DEVICE)
}

/// Whether the request `body` is of a turn the signal started: its last user message holds it.
pub fn woken(body: &Value) -> bool {
    let input = (body["input"].as_array().map(Vec::as_slice)).unwrap_or_default();
    (input.iter().rev())
        .find(|item| item["type"] == "message" && item["role"] == "user")
        .is_some_and(|item| item["content"].to_string().contains(SIGNAL))
}

/// A turn's last request, recorded from Codex 0.162.1 in Herdr: `inbox` answered, and
/// `skald devices` succeeded.
#[test]
fn a_recorded_turn_shows_the_server_and_the_cli_worked() {
    let body: Value = serde_json::from_str(include_str!("recorded/turn.json")).expect("JSON");
    assert_eq!(played(&body), 2);
    assert!(inbox_answered(
        &output_of(&body, 0).expect("inbox's output")
    ));
    assert!(cli_succeeded(
        &output_of(&body, 1).expect("the command's output")
    ));
    assert!(!woken(&body));
    assert!(!inbox_answered("unsupported call: inbox"));
    let no_device = "skald: this machine has no device yet: sign in with skald sign-in";
    assert!(!inbox_answered(no_device));
    assert!(!cli_succeeded(
        "Process exited with code 1\nOutput:\nskald: not signed in"
    ));
}

/// The woken turn's request, recorded from Codex 0.162.1 in Herdr: its last user message is the
/// signal.
#[test]
fn a_recorded_woken_turn_starts_with_the_signal() {
    let body: Value = serde_json::from_str(include_str!("recorded/woken.json")).expect("JSON");
    assert!(woken(&body));
}

/// A file a host wrote names the check's config home; as a fixture, Ada's.
#[test]
fn a_written_file_names_adas_config_home_as_a_fixture() {
    let home = Path::new("/tmp/skald-test-1-0/home/.codex");
    let written = "bash '/tmp/skald-test-1-0/home/.codex/herdr-agent-state.sh' session";
    assert_eq!(
        as_fixture(written, home),
        "bash '/home/ada/.codex/herdr-agent-state.sh' session"
    );
}

/// The result line says what each part of the check gave.
#[test]
fn the_result_line_says_each_part() {
    let results = Results {
        mcp: true,
        cli: true,
        woke: Some(Duration::from_millis(9_300)),
        text_stayed_out: true,
    };
    assert_eq!(
        results.line("Herdr 0.9.5, Codex 0.162.1"),
        "Herdr 0.9.5, Codex 0.162.1: MCP server yes; CLI yes; idle wake by the hooks' registration and codex queue yes, 9.3 s; message text kept out yes"
    );
    assert!(
        Results::default()
            .line("x")
            .contains("idle wake by the hooks' registration and codex queue no")
    );
}
