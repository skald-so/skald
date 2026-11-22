//! Codex's executor sessions (`docs/protocol/queues.md`, "Running a task"): `codex exec` with the
//! flags and environment the live checks found clean (`docs/dev/agent-delivery.md`, "Session
//! config").

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

use crate::executor::{self, Device, Mode, Outcome, Refused, Request, Run, Scratch};

/// The flags that leave a session none of the user's config, with its MCP servers, plugins and
/// hooks, rules and skills' instructions, and save no session.
const CLEAN: [&str; 10] = [
    "--ephemeral",
    "--ignore-user-config",
    "--ignore-rules",
    "--disable",
    "plugins",
    "--disable",
    "hooks",
    "-c",
    "skills.include_instructions=false",
    "--skip-git-repo-check",
];

/// The sign-in file a run's config home holds a copy of, and nothing else.
const AUTH: &str = "auth.json";

/// Why `read-only` is refused on this OS: Codex's `read-only` sandbox enforces only some of its
/// limits on each OS the live checks ran on.
const READ_ONLY_REFUSED: &str = if cfg!(windows) {
    "on Windows, Codex 0.154.0's read-only sandbox lets a session run commands, read files \
     outside its folder and reach the local network"
} else if cfg!(any(target_os = "macos", target_os = "linux")) {
    "Codex 0.162.1's read-only sandbox lets a session run commands and read files outside its \
     folder"
} else {
    "Codex's read-only sandbox is not checked on this OS"
};

/// The variables of the device's environment a session keeps besides those every agent does:
/// a proxy's certificate authority.
const KEPT: [&str; 2] = ["SSL_CERT_FILE", "CODEX_CA_CERTIFICATE"];

/// Whether Codex signs in with the variable `name`, in upper case: through OpenAI's API or Azure
/// OpenAI, or a provider's secret Codex names.
fn signs_in(name: &str) -> bool {
    let secret = ["KEY", "TOKEN", "AUTH", "SECRET", "PROVIDER"];
    name.starts_with("OPENAI_")
        || name.starts_with("AZURE_OPENAI_")
        || (name.starts_with("CODEX_") && secret.iter().any(|word| name.contains(word)))
}

/// Whether a session keeps the variable `name`, as executors name it: one Codex needs to run or
/// sign in.
fn kept(name: &str) -> bool {
    KEPT.contains(&name) || signs_in(name)
}

/// Runs requests in new Codex sessions with no person.
#[derive(Debug, Clone)]
pub struct Executor {
    program: PathBuf,
    device: Device,
}

impl Executor {
    executor::builders!("Codex");

    /// Starts `request` in a new session in `folder` at `mode`, in a config home of its own that
    /// holds only a copy of the user's sign-in. Refuses `read-only`, which Codex does not enforce
    /// here, before anything runs.
    pub fn start(&self, folder: &Path, mode: Mode, request: &Request) -> Result<Run, Refused> {
        if mode == Mode::ReadOnly {
            return Err(Refused(format!(
                "Codex runs no read-only session: {READ_ONLY_REFUSED}"
            )));
        }
        let home = self.own_home().map_err(|error| {
            Refused(format!(
                "cannot make a config home for the Codex session: {error}"
            ))
        })?;
        let mut command = Command::new(&self.program);
        command
            .current_dir(folder)
            .env_clear()
            .envs(self.device.kept(kept))
            .env("CODEX_HOME", home.path())
            .args(arguments(folder, mode));
        Run::start(command, request.labelled(), outcome, Some(home)).map_err(|error| {
            Refused(format!(
                "cannot run Codex at {}: {error}",
                self.program.display()
            ))
        })
    }

    /// The user's Codex config home: the device's `CODEX_HOME`, else `.codex` in its home.
    fn user_home(&self) -> Option<PathBuf> {
        if let Some(home) = self.device.var("CODEX_HOME") {
            return Some(home.into());
        }
        let home = match cfg!(windows) {
            true => self.device.var("USERPROFILE"),
            false => None,
        };
        let home = home.or_else(|| self.device.var("HOME"))?;
        Some(Path::new(&home).join(".codex"))
    }

    /// A config home of a run's own with a copy of the user's sign-in, if there is one.
    fn own_home(&self) -> io::Result<Scratch> {
        let home = Scratch::new("skald-codex-home")?;
        if let Some(user) = self.user_home() {
            match fs::copy(user.join(AUTH), home.path().join(AUTH)) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(home)
    }
}

/// The arguments of a session in `folder` at `mode`, its prompt on stdin: no person, its events
/// as JSON lines, the clean flags, no approval asked and Codex's sandbox of the mode; on Windows,
/// the sandbox `read-only` needs there.
pub fn arguments(folder: &Path, mode: Mode) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["exec", "--json"].map(OsString::from).to_vec();
    args.extend(CLEAN.map(OsString::from));
    args.extend(["-c", "approval_policy=\"never\""].map(OsString::from));
    if cfg!(windows) {
        args.extend(["-c", "windows.sandbox=\"unelevated\""].map(OsString::from));
    }
    let sandbox = match mode {
        Mode::ReadOnly => "read-only",
        Mode::Full => "danger-full-access",
    };
    args.extend(["--sandbox", sandbox, "-C"].map(OsString::from));
    args.push(folder.as_os_str().to_owned());
    args.push("-".into());
    args
}

/// How a session that printed `output` ended: well only if it exited with success and its turn
/// completed, then with the agent's last message; otherwise with why, from its failed turn, its
/// last error event or its stderr.
fn outcome(output: &Output) -> Outcome {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut answer = None;
    let mut completed = false;
    let mut failed = None;
    let mut error = None;
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line.trim()));
    for event in events.filter_map(Result::ok) {
        let text = |value: &Value| value.as_str().map(str::to_owned);
        match event["type"].as_str() {
            Some("item.completed") if event["item"]["type"] == "agent_message" => {
                answer = text(&event["item"]["text"]);
            }
            Some("turn.completed") => completed = true,
            Some("turn.failed") => failed = text(&event["error"]["message"]),
            Some("error") => error = text(&event["message"]),
            _ => {}
        }
    }
    if output.status.success() && completed && failed.is_none() {
        return Outcome {
            answer: answer.unwrap_or_default(),
            finished: true,
        };
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let said = failed.or(error).or((!stderr.is_empty()).then_some(stderr));
    Outcome {
        answer: said.unwrap_or_else(|| {
            format!("Codex ended with {} and said nothing of why", output.status)
        }),
        finished: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn status(code: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn status(code: i32) -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }

    fn ended(code: i32, stdout: &str, stderr: &str) -> Outcome {
        outcome(&Output {
            status: status(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    const STARTED: &str = r#"{"type":"thread.started","thread_id":"t"}
{"type":"turn.started"}"#;
    const COMPLETED: &str = r#"{"type":"turn.completed","usage":{}}"#;

    fn message(text: &str) -> String {
        format!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"{text}"}}}}"#)
    }

    fn well(answer: &str) -> Outcome {
        Outcome {
            answer: answer.to_owned(),
            finished: true,
        }
    }

    fn badly(answer: &str) -> Outcome {
        Outcome {
            answer: answer.to_owned(),
            finished: false,
        }
    }

    #[test]
    fn a_completed_turn_gives_the_last_message() {
        let command = r#"{"type":"item.completed","item":{"type":"command_execution","text":"x"}}"#;
        let events = [
            STARTED,
            &message("First."),
            command,
            &message("Done."),
            COMPLETED,
        ];
        assert_eq!(ended(0, &events.join("\n"), ""), well("Done."));
        assert_eq!(ended(0, &format!("{STARTED}\n{COMPLETED}"), ""), well(""));
    }

    #[test]
    fn a_failed_run_says_why() {
        let failed = r#"{"type":"error","message":"one"}
{"type":"turn.failed","error":{"message":"two"}}"#;
        assert_eq!(
            ended(1, &format!("{STARTED}\n{failed}"), "Error: three"),
            badly("two")
        );
        let error = r#"{"type":"error","message":"one"}"#;
        assert_eq!(
            ended(1, &format!("{STARTED}\n{error}"), "three"),
            badly("one")
        );
        assert_eq!(ended(2, "", "error: three\n"), badly("error: three"));
        let completed = [STARTED, &message("Done."), COMPLETED].join("\n");
        assert_eq!(ended(1, &completed, "broke"), badly("broke"));
        assert!(!ended(0, STARTED, "").finished);
        assert!(ended(3, "", "").answer.contains("said nothing of why"));
    }

    #[test]
    fn a_session_is_clean_and_in_its_mode() {
        let args = arguments(Path::new("folder"), Mode::Full);
        let args: Vec<&str> = args.iter().map(|arg| arg.to_str().unwrap()).collect();
        let mut expected = vec![
            "exec",
            "--json",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--disable",
            "plugins",
            "--disable",
            "hooks",
            "-c",
            "skills.include_instructions=false",
            "--skip-git-repo-check",
            "-c",
            "approval_policy=\"never\"",
        ];
        if cfg!(windows) {
            expected.extend(["-c", "windows.sandbox=\"unelevated\""]);
        }
        expected.extend(["--sandbox", "danger-full-access", "-C", "folder", "-"]);
        assert_eq!(args, expected);
        let read_only = arguments(Path::new("folder"), Mode::ReadOnly);
        let sandbox = read_only.iter().position(|arg| arg == "--sandbox").unwrap();
        assert_eq!(read_only[sandbox + 1], "read-only");
    }

    #[test]
    fn keeps_only_what_signs_in() {
        let openai = ["OPENAI_API_KEY", "OPENAI_BASE_URL", "AZURE_OPENAI_API_KEY"];
        let codex = ["CODEX_API_KEY", "CODEX_ACCESS_TOKEN", "CODEX_PROVIDER"];
        for name in openai.iter().chain(&codex).chain(&KEPT) {
            assert!(kept(name), "{name}");
        }
        for name in [
            "CODEX_HOME",
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
            "SKALD_TOKEN",
            "PATH",
        ] {
            assert!(!kept(name), "{name}");
        }
    }

    #[test]
    fn finds_the_user_s_config_home() {
        let executor = |vars: &[(&str, &str)]| Executor::new("codex").environment(vars.to_vec());
        let set = executor(&[("CODEX_HOME", "/codex"), ("HOME", "/home")]);
        assert_eq!(set.user_home(), Some(PathBuf::from("/codex")));
        let unset = executor(&[("CODEX_HOME", ""), ("HOME", "/home")]);
        assert_eq!(unset.user_home(), Some(Path::new("/home").join(".codex")));
        let profile = executor(&[("USERPROFILE", "/profile"), ("HOME", "/home")]);
        let home = if cfg!(windows) { "/profile" } else { "/home" };
        assert_eq!(profile.user_home(), Some(Path::new(home).join(".codex")));
        assert_eq!(executor(&[]).user_home(), None);
    }

    #[test]
    fn refuses_read_only_before_anything_runs() {
        let request = Request {
            from: "acme/alice/reviewer@skald.example".parse().unwrap(),
            level: skald_proto::TrustLevel::Own,
            text: "Read it.".to_owned(),
        };
        let executor = Executor::new("/nonexistent/codex");
        let refused = executor.start(Path::new("/nonexistent"), Mode::ReadOnly, &request);
        let refused = refused.unwrap_err().0;
        assert!(refused.contains("read-only sandbox"), "{refused}");
    }

    /// The run's config home, with its copy of the sign-in, is gone once the run ends.
    #[cfg(unix)]
    #[test]
    fn a_run_s_config_home_outlives_it_not() {
        use std::os::unix::fs::PermissionsExt as _;

        let scratch = Scratch::new("skald-codex-test").unwrap();
        let user = scratch.path().join("user");
        fs::create_dir(&user).unwrap();
        fs::write(user.join(AUTH), "{}").unwrap();
        let codex = scratch.path().join("codex");
        let script = r#"#!/bin/sh
cat "$CODEX_HOME/auth.json" >/dev/null || exit 1
printf '{"type":"item.completed","item":{"type":"agent_message","text":"%s"}}\n' "$CODEX_HOME"
printf '{"type":"turn.completed"}\n'
"#;
        fs::write(&codex, script).unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
        let executor = Executor::new(&codex).environment([
            ("PATH", OsString::from("/usr/bin:/bin")),
            ("CODEX_HOME", user.into_os_string()),
        ]);
        let request = Request {
            from: "acme/alice/reviewer@skald.example".parse().unwrap(),
            level: skald_proto::TrustLevel::Own,
            text: "Say done.".to_owned(),
        };

        let run = executor
            .start(scratch.path(), Mode::Full, &request)
            .unwrap();
        let outcome = run.wait();

        assert!(outcome.finished, "{outcome:?}");
        let home = PathBuf::from(&outcome.answer);
        assert!(home.is_absolute(), "{outcome:?}");
        assert!(!home.exists(), "the run's config home outlived it");
    }
}
