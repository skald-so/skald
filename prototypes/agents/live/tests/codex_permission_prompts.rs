//! The live checks of the paths that tell that a Codex session waits on Codex's own permission
//! prompt, and their report in `docs/dev/agent-delivery.md`: a section of its own that "Checks
//! before implementation" links to. The checks are the `live-check <check> codex` commands that
//! section records; each runs here against a stand-in of the CLI that records how the check
//! starts it and exits at once, as a Codex that never asks.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agent_testing::{snapshot, write_executable};

/// The app-server request a candidate path reads.
const REQUEST_APPROVAL: &str = "item/commandExecution/requestApproval";

/// The `live-check <check> codex` commands that came before these checks; a mention of one in the
/// section is not one of its checks.
const EARLIER: [&str; 14] = [
    "inbox",
    "post-tool-use",
    "stop",
    "user-prompt-submit",
    "queue",
    "app-server",
    "ack",
    "transcript",
    "limits",
    "signal",
    "own-text",
    "executor-read-only",
    "executor-full",
    "executor-clean",
];

/// The version the stand-in reports.
const VERSION: &str = "9.8.7";

/// A variable of the parent's environment that no session on the stub may get.
const PARENT_ONLY: (&str, &str) = ("SKALD_PARENT_ONLY", "parent-only-value");

/// A key that would sign Codex in other than on the stub.
const PARENT_KEY: (&str, &str) = ("OPENAI_API_KEY", "sk-parent-key");

/// The longest a check may run against a stand-in that exits at once.
const RUN_LIMIT: Duration = Duration::from_secs(240);

/// The document of the live checks' results.
fn doc() -> String {
    let doc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/dev/agent-delivery.md");
    fs::read_to_string(doc).unwrap()
}

/// A heading of the document: its level, its text and its anchor.
struct Heading {
    line: usize,
    level: usize,
    text: String,
    anchor: String,
}

/// The anchor GitHub gives a heading's text, before any `-<n>` that tells repeats apart.
fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// The document's headings outside code blocks, in order.
fn headings(lines: &[&str]) -> Vec<Heading> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut fenced = false;
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        if fenced || level == 0 || !line[level..].starts_with(' ') {
            continue;
        }
        let text = line[level..].trim().to_owned();
        let base = slug(&text);
        let count = seen.entry(base.clone()).or_insert(0);
        let anchor = match *count {
            0 => base,
            n => format!("{base}-{n}"),
        };
        *count += 1;
        found.push(Heading {
            line: at,
            level,
            text,
            anchor,
        });
    }
    found
}

/// The text of the section under `headings[at]`: to the next heading of its level or higher.
fn section(lines: &[&str], headings: &[Heading], at: usize) -> String {
    let start = headings[at].line;
    let end = headings[at + 1..]
        .iter()
        .find(|next| next.level <= headings[at].level)
        .map_or(lines.len(), |next| next.line);
    lines[start..end].join("\n")
}

/// The report: each section that "Checks before implementation" links to, is about permission
/// prompts by its heading or one above it, and names Codex.
fn report() -> String {
    let doc = doc();
    let lines: Vec<&str> = doc.lines().collect();
    let headings = headings(&lines);
    let checks = headings
        .iter()
        .position(|heading| heading.level == 2 && heading.text == "Checks before implementation")
        .expect("the section \"Checks before implementation\"");
    let links = section(&lines, &headings, checks);
    let mut report = Vec::new();
    for (at, heading) in headings.iter().enumerate() {
        if !links.contains(&format!("](#{})", heading.anchor)) {
            continue;
        }
        let about_permission = headings[..=at]
            .iter()
            .rev()
            .scan(usize::MAX, |level, above| {
                let shown = above.level < *level;
                *level = (*level).min(above.level);
                Some((shown, above))
            })
            .any(|(shown, above)| shown && above.text.to_lowercase().contains("permission"));
        let text = section(&lines, &headings, at);
        if about_permission && text.contains("Codex") {
            report.push(text);
        }
    }
    assert!(
        !report.is_empty(),
        "\"Checks before implementation\" links to no section on Codex permission prompts"
    );
    report.join("\n")
}

/// Each `<a|b>` of a command's word, one word each.
fn expand(word: &str) -> Vec<String> {
    match word
        .strip_prefix('<')
        .and_then(|word| word.strip_suffix('>'))
    {
        Some(choices) => choices.split('|').map(str::to_owned).collect(),
        None => vec![word.to_owned()],
    }
}

/// The checks of `live-check <check> codex` that `report` records, other than [`EARLIER`] ones and
/// the GUI ones.
fn checks(report: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in report.match_indices("live-check ") {
        let words: Vec<&str> = report[at + "live-check ".len()..]
            .split_whitespace()
            .take(2)
            .map(|word| word.trim_end_matches(['`', ',', '.', ';', ':', ')']))
            .collect();
        let [name, agent] = words[..] else { continue };
        if !expand(agent).iter().any(|agent| agent == "codex") {
            continue;
        }
        for name in expand(name) {
            let word = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if word
                && !name.starts_with("gui")
                && !EARLIER.contains(&name.as_str())
                && !found.contains(&name)
            {
                found.push(name);
            }
        }
    }
    found
}

/// Whether `text` names a Codex version, such as `Codex 0.162.1` or `codex-cli 0.162.1`.
fn names_a_version(text: &str) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| "`*(),;:".contains(c))
                .to_lowercase()
        })
        .collect();
    words.windows(2).any(|pair| {
        let numbers = pair[1].trim_end_matches('.').split('.');
        (pair[0] == "codex" || pair[0] == "codex-cli")
            && numbers.clone().count() == 3
            && numbers
                .into_iter()
                .all(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Whether `text` gives a delay, such as `0.1 s`, `6 s` or `250 ms`.
fn names_a_delay(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len()).any(|at| {
        if !chars[at].is_ascii_digit() || (at > 0 && chars[at - 1].is_alphanumeric()) {
            return false;
        }
        let mut end = at;
        while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
            end += 1;
        }
        if chars.get(end) == Some(&' ') {
            end += 1;
        }
        let unit: String = chars[end..]
            .iter()
            .take_while(|c| c.is_alphabetic())
            .collect();
        unit == "s" || unit == "ms"
    })
}

#[test]
fn the_report_gives_each_codex_permission_candidate_its_result_delay_and_version() {
    let report = report();
    let lower = report.to_lowercase();
    assert!(
        report.contains(REQUEST_APPROVAL),
        "the report does not name the candidate {REQUEST_APPROVAL}:\n{report}"
    );
    assert!(
        report.contains("--remote") && lower.contains("background"),
        "the report does not give {REQUEST_APPROVAL} on both the app-server a terminal UI joins \
         with --remote and the background app-server:\n{report}"
    );
    assert!(
        names_a_version(&report),
        "no Codex version in the report:\n{report}"
    );
    assert!(names_a_delay(&report), "no delay in the report:\n{report}");
    assert!(
        lower.contains("stub"),
        "no result on the model stub in the report:\n{report}"
    );
    assert!(
        lower.contains("session") && lower.contains("answer"),
        "the report does not say whether the candidates name the session and tell when the \
         prompt is answered:\n{report}"
    );
    let checks = checks(&report);
    assert!(
        !checks.is_empty(),
        "the report records no live check of Codex permission prompts:\n{report}"
    );
    for check in &checks {
        let command = format!("{check} codex");
        let results: usize = report
            .lines()
            .filter(|line| line.contains(&command))
            .map(|line| line.matches("pass").count() + line.matches("fail").count())
            .sum();
        assert!(
            results >= 1,
            "no result of live-check {command} in the report:\n{report}"
        );
    }
}

#[test]
fn usage_names_each_codex_permission_check_of_the_report() {
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .env_clear()
        .output()
        .expect("live-check runs");
    assert_eq!(output.status.code(), Some(2));
    let usage = String::from_utf8_lossy(&output.stderr);
    for check in checks(&report()) {
        let named = usage.lines().any(|line| {
            let words: Vec<&str> = line
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .collect();
            words.contains(&check.as_str()) && words.contains(&"codex")
        });
        assert!(named, "no usage line names {check} with codex:\n{usage}");
    }
}

/// The user's home before a check: a Codex config of its own.
fn user_home(home: &Path) {
    fs::create_dir(home.join(".codex")).unwrap();
    fs::write(
        home.join(".codex/config.toml"),
        "approval_policy = \"never\"\n",
    )
    .unwrap();
    fs::write(home.join(".codex/hooks.json"), r#"{"hooks":{}}"#).unwrap();
    fs::write(
        home.join(".codex/AGENTS.md"),
        "The user's own instructions.\n",
    )
    .unwrap();
}

/// A stand-in of `codex` that records each run but `--version` in a folder of its own under
/// `record`: its arguments, one per line; the variables it got of those the test cares about; and
/// the config file in its config home, if any. Then it exits at once, as a Codex that fails.
fn stand_in(record: &Path) -> String {
    let record = record.display();
    format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli {VERSION}"
  exit 0
fi
dir='{record}'/$$
mkdir -p "$dir"
printf '%s\n' "$@" > "$dir/args"
{{
  printf 'HOME=%s\n' "${{HOME-<unset>}}"
  printf 'CODEX_HOME=%s\n' "${{CODEX_HOME-<unset>}}"
  printf 'SKALD_STUB_API_KEY=%s\n' "${{SKALD_STUB_API_KEY-<unset>}}"
  printf '{key}=%s\n' "${{{key}-<unset>}}"
  printf '{parent}=%s\n' "${{{parent}-<unset>}}"
}} > "$dir/env"
if [ -n "$CODEX_HOME" ] && [ -f "$CODEX_HOME/config.toml" ]; then
  cp "$CODEX_HOME/config.toml" "$dir/config.toml"
fi
exit 1
"#,
        key = PARENT_KEY.0,
        parent = PARENT_ONLY.0,
    )
}

/// One run of the stand-in as it recorded it.
struct Session {
    args: Vec<String>,
    env: BTreeMap<String, String>,
    config: String,
}

/// What a check did against the stand-in.
struct Run {
    command: String,
    code: Option<i32>,
    stdout: String,
    stderr: String,
    sessions: Vec<Session>,
    home: PathBuf,
    home_before: BTreeMap<PathBuf, Option<Vec<u8>>>,
    _dir: tempfile::TempDir,
}

impl Run {
    fn at(&self) -> String {
        format!(
            "live-check {}: exit {:?}\nstdout:\n{}\nstderr:\n{}",
            self.command, self.code, self.stdout, self.stderr
        )
    }
}

/// Runs `live-check <check> codex` with the stand-in, a user home with a Codex config, and
/// [`PARENT_ONLY`] and [`PARENT_KEY`] in its environment.
fn run(check: &str) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let [bin, record, tmp, home] = ["bin", "record", "tmp", "home"].map(|name| {
        let made = dir.path().join(name);
        fs::create_dir(&made).unwrap();
        made
    });
    user_home(&home);
    let home_before = snapshot(&home);
    write_executable(&bin.join("codex"), &stand_in(&record));
    let command = format!("{check} codex");

    // Its output goes to files, so a session it leaves behind cannot hold the test on a pipe.
    let (out, err) = (dir.path().join("stdout"), dir.path().join("stderr"));
    let mut variables = vec![
        (
            "PATH".to_owned(),
            format!("{}:/usr/bin:/bin", bin.display()),
        ),
        ("TMPDIR".to_owned(), tmp.display().to_string()),
        ("HOME".to_owned(), home.display().to_string()),
    ];
    for (name, value) in [PARENT_ONLY, PARENT_KEY] {
        variables.push((name.to_owned(), value.to_owned()));
    }
    let mut live = Command::new(env!("CARGO_BIN_EXE_live-check"));
    live.args([check, "codex"]).env_clear().envs(variables);
    live.stdin(Stdio::null())
        .stdout(fs::File::create(&out).unwrap())
        .stderr(fs::File::create(&err).unwrap());
    let mut child = live.spawn().expect("live-check runs");
    let deadline = Instant::now() + RUN_LIMIT;
    let status = loop {
        match child.try_wait().unwrap() {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "live-check {command} ran over {RUN_LIMIT:?} with a codex that exits at once"
                );
            }
        }
    };
    let read = |path: &Path| String::from_utf8_lossy(&fs::read(path).unwrap()).into_owned();

    let mut sessions = Vec::new();
    for entry in fs::read_dir(&record).unwrap() {
        let dir = entry.unwrap().path();
        let env = fs::read_to_string(dir.join("env"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        sessions.push(Session {
            args: fs::read_to_string(dir.join("args"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect(),
            env,
            config: fs::read_to_string(dir.join("config.toml")).unwrap_or_default(),
        });
    }
    let run = Run {
        command,
        code: status.code(),
        stdout: read(&out),
        stderr: read(&err),
        sessions,
        home,
        home_before,
        _dir: dir,
    };
    assert_ne!(
        run.code,
        Some(2),
        "the check printed the usage: {}",
        run.at()
    );
    run
}

/// Each check of the report, run against the stand-in.
fn runs() -> Vec<Run> {
    let checks = checks(&report());
    assert!(!checks.is_empty(), "the report records no check");
    checks.iter().map(|check| run(check)).collect()
}

#[test]
fn codex_permission_checks_print_the_codex_version_and_fail_when_no_prompt_comes() {
    for run in runs() {
        assert!(
            run.stdout.contains(VERSION),
            "the check does not print the Codex version: {}",
            run.at()
        );
        assert_eq!(
            run.code,
            Some(1),
            "the check passed, or did not fail, with a Codex that never asks: {}",
            run.at()
        );
    }
}

#[test]
fn codex_permission_checks_run_codex_on_the_stub_in_a_config_home_of_their_own() {
    for run in runs() {
        let at = run.at();
        assert!(!run.sessions.is_empty(), "the check never ran codex: {at}");
        let home = run.home.display().to_string();
        for session in &run.sessions {
            let args = session.args.join(" ");
            for name in ["HOME", "CODEX_HOME"] {
                let value = session.env.get(name).map_or("<unset>", String::as_str);
                assert!(
                    value != "<unset>" && !value.is_empty() && !value.starts_with(&home),
                    "codex {args} got {name}={value}, not a temporary one: {at}"
                );
            }
            assert_ne!(
                session.env.get("SKALD_STUB_API_KEY").map(String::as_str),
                Some("<unset>"),
                "codex {args} got no placeholder key of the stub: {at}"
            );
            for (name, _) in [PARENT_KEY, PARENT_ONLY] {
                assert_eq!(
                    session.env.get(name).map(String::as_str),
                    Some("<unset>"),
                    "codex {args} got the parent's {name}: {at}"
                );
            }
        }
        assert!(
            run.sessions.iter().any(|session| {
                let all = format!("{}\n{}", session.args.join("\n"), session.config);
                all.contains("base_url") && (all.contains("127.0.0.1") || all.contains("localhost"))
            }),
            "no codex run got the model stub as its provider: {at}"
        );
    }
}

/// Whether `value`, a `-c` value or a line of `config.toml`, sets the approval policy `never`.
fn sets_never(value: &str) -> bool {
    let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    ["\"never\"", "'never'", "never"]
        .iter()
        .any(|never| value == format!("approval_policy={never}"))
}

#[test]
fn codex_permission_checks_never_turn_off_codex_approvals() {
    for run in runs() {
        let at = run.at();
        for session in &run.sessions {
            let args = &session.args;
            let joined = args.join(" ");
            for (at_flag, flag) in args.iter().enumerate() {
                let next = args.get(at_flag + 1).map(String::as_str);
                let never = match flag.as_str() {
                    "-a" | "--ask-for-approval" => next == Some("never"),
                    "-c" | "--config" => next.is_some_and(sets_never),
                    "--dangerously-bypass-approvals-and-sandbox" | "--yolo" => true,
                    flag => {
                        flag == "--ask-for-approval=never"
                            || flag.strip_prefix("--config=").is_some_and(sets_never)
                    }
                };
                assert!(!never, "codex {joined} runs without approvals: {at}");
            }
            assert!(
                !session.config.lines().any(sets_never),
                "codex {joined} has approval_policy never in its config home: {at}"
            );
        }
    }
}

#[test]
fn codex_permission_checks_leave_the_users_codex_config_alone() {
    for run in runs() {
        assert!(
            snapshot(&run.home) == run.home_before,
            "the check changed the user's home: {}",
            run.at()
        );
    }
}
