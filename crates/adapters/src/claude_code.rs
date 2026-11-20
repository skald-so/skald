//! Claude Code's executor sessions (`docs/protocol/queues.md`, "Running a task"), by the flags the
//! live checks confirmed (`docs/dev/agent-delivery.md`, "Checks before implementation").

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;

use serde_json::Value;

use crate::executor::{Mode, Outcome, Refused};

/// The Claude Code version the live checks of `read-only` ran on: an older one gets no `read-only`.
pub const CHECKED: [u64; 3] = [2, 1, 328];

/// The flags that leave a session none of the user's MCP servers, hooks, plugins, skills,
/// instruction files and memory, and keep no record of it.
const CLEAN: [&str; 6] = [
    "--safe-mode",
    "--setting-sources",
    "local",
    "--strict-mcp-config",
    "--disable-slash-commands",
    "--no-session-persistence",
];

/// The variable that keeps the project's memory out.
const NO_AUTO_MEMORY: (&str, &str) = ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");

/// The variables of the device's environment a session keeps: what Claude Code needs to run, on
/// Unix and on Windows, and to sign in. Names on Windows match in any case.
const KEPT: [&str; 31] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "CLAUDE_CONFIG_DIR",
    "PATHEXT",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "CLAUDE_CODE_GIT_BASH_PATH",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
];

/// Runs prompts in new Claude Code sessions with no person.
#[derive(Debug, Clone)]
pub struct Executor {
    claude: PathBuf,
    environment: Option<Vec<(OsString, OsString)>>,
}

impl Executor {
    /// An executor of the Claude Code binary at `claude`, on this process's environment.
    pub fn new(claude: impl Into<PathBuf>) -> Self {
        Executor {
            claude: claude.into(),
            environment: None,
        }
    }

    /// Starts sessions from `vars`, the device's environment, in place of this process's.
    pub fn environment<K, V>(mut self, vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let vars = vars
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()));
        self.environment = Some(vars.collect());
        self
    }

    /// Runs `prompt` in a new session in `folder` at `mode` and waits for it to end. Refuses
    /// `read-only` on a Claude Code older than [`CHECKED`] or of unknown version.
    pub fn run(&self, folder: &Path, mode: Mode, prompt: &str) -> Result<Outcome, Refused> {
        if mode == Mode::ReadOnly {
            let output = self.command(folder).arg("--version").output();
            let output = output.map_err(|error| self.not_run(error))?;
            read_only_runs_on(&output)?;
        }
        let mut child = (self.command(folder).args(arguments(mode)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| self.not_run(error))?;
        // The prompt goes in on stdin, which then closes: no person answers the session.
        let stdin = child.stdin.take();
        let prompt = prompt.to_owned();
        let writer = thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                // A session that ends without reading it closes the pipe.
                let _ = stdin.write_all(prompt.as_bytes());
            }
        });
        let output = child.wait_with_output();
        let _ = writer.join();
        Ok(outcome(&output.map_err(|error| self.not_run(error))?))
    }

    /// Claude Code in `folder` with only the [`KEPT`] variables of the device's environment and
    /// auto-memory off.
    fn command(&self, folder: &Path) -> Command {
        let device = match &self.environment {
            Some(vars) => vars.clone(),
            None => env::vars_os().collect(),
        };
        let mut command = Command::new(&self.claude);
        command
            .current_dir(folder)
            .env_clear()
            .envs(device.into_iter().filter(|(name, _)| kept(name)))
            .env(NO_AUTO_MEMORY.0, NO_AUTO_MEMORY.1)
            .stdin(Stdio::null());
        command
    }

    fn not_run(&self, error: std::io::Error) -> Refused {
        Refused(format!(
            "cannot run Claude Code at {}: {error}",
            self.claude.display()
        ))
    }
}

/// Whether a session keeps the variable `name`.
fn kept(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    KEPT.iter().any(|kept| match cfg!(windows) {
        true => kept.eq_ignore_ascii_case(name),
        false => *kept == name,
    })
}

/// The arguments of a session at `mode`, its prompt on stdin: no person, its result as JSON, clean,
/// and `read-only` as reading tools that ask nothing, `full` as every tool without prompts.
pub fn arguments(mode: Mode) -> Vec<&'static str> {
    let mut args = vec!["-p", "--output-format", "json"];
    args.extend(CLEAN);
    match mode {
        Mode::ReadOnly => {
            args.extend(["--tools", "Read,Glob,Grep", "--permission-mode", "dontAsk"])
        }
        Mode::Full => args.push("--dangerously-skip-permissions"),
    }
    args
}

/// The numbers of the version `claude --version` printed, such as `2.1.328 (Claude Code)`.
fn version(printed: &str) -> Option<Vec<u64>> {
    let word = printed.split_whitespace().next()?;
    word.split('.').map(|part| part.parse().ok()).collect()
}

/// Whether `read-only` runs on the Claude Code whose `claude --version` gave `output`: only on
/// [`CHECKED`] or later.
fn read_only_runs_on(output: &Output) -> Result<(), Refused> {
    let checked = CHECKED.map(|part| part.to_string()).join(".");
    let printed = String::from_utf8_lossy(&output.stdout);
    let found = version(&printed).filter(|_| output.status.success());
    let Some(found) = found else {
        return Err(Refused(format!(
            "read-only needs Claude Code {checked} or later, and its version is unknown: \
             `claude --version` printed {:?} and {}",
            printed.trim(),
            output.status
        )));
    };
    if found.as_slice() < CHECKED.as_slice() {
        let found = found
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".");
        return Err(Refused(format!(
            "read-only needs Claude Code {checked} or later, which enforces it; this is {found}"
        )));
    }
    Ok(())
}

/// How a session that printed `output` ended: its final answer from the JSON result, which ends
/// well only on success; without a result, what it said on stderr.
fn outcome(output: &Output) -> Outcome {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = (stdout.lines().rev())
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find(|event| event["type"] == "result");
    let Some(result) = result else {
        let said = String::from_utf8_lossy(&output.stderr);
        return Outcome {
            answer: said.trim().to_owned(),
            finished: false,
        };
    };
    Outcome {
        answer: (result["result"].as_str()).unwrap_or_default().to_owned(),
        finished: output.status.success()
            && result["subtype"] == "success"
            && result["is_error"] == false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        use std::os::unix::process::ExitStatusExt;
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[cfg(windows)]
    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        use std::os::windows::process::ExitStatusExt;
        Output {
            status: std::process::ExitStatus::from_raw(code as u32),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn reads_versions_by_number() {
        assert_eq!(version("2.1.328 (Claude Code)\n"), Some(vec![2, 1, 328]));
        assert_eq!(version("2.10.0"), Some(vec![2, 10, 0]));
        assert_eq!(version("Claude Code 2.1.328"), None);
        assert_eq!(version("2.1.x"), None);
        assert_eq!(version(""), None);
    }

    #[test]
    fn read_only_runs_only_on_the_checked_version_or_later() {
        for printed in ["2.1.328 (Claude Code)", "2.1.1000", "2.2.0", "3.0.0"] {
            assert!(
                read_only_runs_on(&output(0, printed, "")).is_ok(),
                "{printed}"
            );
        }
        for printed in ["2.1.327 (Claude Code)", "2.1.32", "2.0.999", "1.9.400"] {
            let refused = read_only_runs_on(&output(0, printed, "")).unwrap_err();
            let found = printed.split_whitespace().next().unwrap();
            assert!(
                refused.0.ends_with(&format!("this is {found}")),
                "{refused}"
            );
        }
        for (code, printed) in [(0, "Claude Code"), (0, ""), (1, "2.1.328 (Claude Code)")] {
            assert!(
                read_only_runs_on(&output(code, printed, "")).is_err(),
                "{printed}"
            );
        }
    }

    #[test]
    fn a_session_is_clean_and_in_its_mode() {
        let clean = [
            "-p",
            "--output-format",
            "json",
            "--safe-mode",
            "--setting-sources",
            "local",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-session-persistence",
        ];
        for mode in [Mode::ReadOnly, Mode::Full] {
            assert_eq!(arguments(mode)[..9], clean);
        }
        assert_eq!(
            arguments(Mode::ReadOnly)[9..],
            ["--tools", "Read,Glob,Grep", "--permission-mode", "dontAsk"]
        );
        assert_eq!(
            arguments(Mode::Full)[9..],
            ["--dangerously-skip-permissions"]
        );
    }

    /// A session runs in its folder with the device's variables it keeps and auto-memory off.
    #[test]
    fn a_session_gets_its_folder_and_auto_memory_off() {
        let device = [
            ("PATH", "/bin"),
            ("ANTHROPIC_API_KEY", "key"),
            ("GITHUB_TOKEN", "token"),
            ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "0"),
        ];
        let executor = Executor::new("claude").environment(device);
        let folder = Path::new("folder");
        let command = executor.command(folder);

        assert_eq!(command.get_current_dir(), Some(folder));
        let envs: Vec<_> = (command.get_envs())
            .filter_map(|(name, value)| Some((name.to_str()?, value?.to_str()?)))
            .collect();
        assert_eq!(
            envs,
            [
                ("ANTHROPIC_API_KEY", "key"),
                ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"),
                ("PATH", "/bin"),
            ]
        );
    }

    #[test]
    fn keeps_only_what_runs_and_signs_in() {
        for name in [
            "PATH",
            "HOME",
            "CLAUDE_CONFIG_DIR",
            "ANTHROPIC_API_KEY",
            "SYSTEMROOT",
        ] {
            assert!(kept(OsStr::new(name)), "{name}");
        }
        for name in [
            "GITHUB_TOKEN",
            "SKALD_TOKEN",
            "OPENAI_API_KEY",
            "CLAUDE_CODE_SAFE_MODE",
            "",
        ] {
            assert!(!kept(OsStr::new(name)), "{name}");
        }
        assert_eq!(kept(OsStr::new("Path")), cfg!(windows));
    }

    #[test]
    fn tells_how_a_session_ended() {
        let ended = |code, stdout: &str| outcome(&output(code, stdout, "it broke\n"));
        let success = r#"{"type":"result","subtype":"success","is_error":false,"result":"Done."}"#;
        let failed = r#"{"type":"result","subtype":"success","is_error":true,"result":"No."}"#;
        let capped = r#"{"type":"result","subtype":"error_max_turns","is_error":false}"#;
        let well = |answer: &str| Outcome {
            answer: answer.to_owned(),
            finished: true,
        };
        let badly = |answer: &str| Outcome {
            answer: answer.to_owned(),
            finished: false,
        };
        assert_eq!(ended(0, &format!("noise\n{success}\n")), well("Done."));
        assert_eq!(ended(1, success), badly("Done."));
        assert_eq!(ended(0, failed), badly("No."));
        assert_eq!(ended(0, capped), badly(""));
        assert_eq!(ended(1, "not JSON"), badly("it broke"));
    }
}
