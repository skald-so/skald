//! The live checks of the subscription limits each CLI shows the user, read by the `agent-limits`
//! prototype: its arguments, the Claude Code session whose status line it is, what it printed, and
//! the checks' verdicts. They run off the model stub: the stub has no subscription. The binary
//! starts the processes.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agent_limits::{Limit, claude_status_line};
use serde_json::{Value, json};

use crate::channel::Files;
use crate::{Agent, Checks, Kind, Usage, app_server, pretty, queue, shell_quote, verdict};

/// The prototype's binary, next to `live-check`.
pub const LIMITS_BIN: &str = "agent-limits";

/// Each status line window with the `/usage` row of the same limit.
const SAME: [(&str, &str); 2] = [("five_hour", "session"), ("seven_day", "weekly_all")];

/// The arguments of `agent-limits claude`, which writes the CLI's output to `log`.
pub fn claude(log: &Path) -> Vec<String> {
    vec![
        "claude".to_owned(),
        "--log".to_owned(),
        log.to_string_lossy().into_owned(),
    ]
}

/// The arguments of `agent-limits codex`, which writes its exchange with the app-server to `log`.
/// The app-server gets [`quiet_app_server`]'s options for `config`, the user's Codex config.
pub fn codex(log: &Path, config: &str) -> Vec<String> {
    let mut args = vec![
        "codex".to_owned(),
        "--log".to_owned(),
        log.to_string_lossy().into_owned(),
        "--".to_owned(),
    ];
    args.extend(quiet_app_server(config));
    args
}

/// The options of an app-server that starts no thread: those of the app-server check, with no
/// model, and each MCP server of `config`, the user's Codex config, turned off. The app-server has
/// no option that ignores the user's config.
pub fn quiet_app_server(config: &str) -> Vec<String> {
    let mut args = app_server::options(None, None);
    for name in mcp_servers(config) {
        args.extend(["-c".to_owned(), format!("mcp_servers.{name}.enabled=false")]);
    }
    args
}

/// The names of the MCP servers that a Codex `config.toml`, `config`, sets up: those of its
/// `[mcp_servers.<name>]` tables and their subtables. Only the tables' headers are read, never a
/// value.
pub fn mcp_servers(config: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in config.lines() {
        let Some(table) = line.trim().strip_prefix("[mcp_servers.") else {
            continue;
        };
        let name = match table.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next(),
            None => table.split(['.', ']']).next(),
        };
        if let Some(name) = name.filter(|name| !name.is_empty())
            && !names.iter().any(|known| known == name)
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// The user's Codex config file: `config.toml` in `codex_home`, the value of `CODEX_HOME`, or else
/// in `.codex` in `home`.
pub fn codex_config_file(codex_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let dir = match codex_home {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(home?).join(".codex"),
    };
    Some(dir.join("config.toml"))
}

/// The file to which the status line command of the session in `files` appends its inputs.
pub fn status_line_log(files: &Files) -> PathBuf {
    files.dir.join("status-line.jsonl")
}

/// Writes the settings of the status line session in `files` and returns the arguments of an
/// interactive Claude Code session with one short prompt, with `model` if given. Only the settings
/// file and the arguments change Claude Code's config: no project or user settings, no MCP server,
/// skills or tools, the lowest effort, no prompt suggestions, which would cost a model request; the
/// status line command `agent_limits status-line --log <log>`, with `agent_limits` the prototype;
/// and a `Stop` hook that tells when the turn ended.
pub fn status_line_session(
    files: &Files,
    agent_limits: &Path,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(&files.project)?;
    let command = format!(
        "{} status-line --log {}",
        shell_quote(&agent_limits.to_string_lossy()),
        shell_quote(&status_line_log(files).to_string_lossy())
    );
    let settings = files.dir.join("settings.json");
    fs::write(
        &settings,
        pretty(&json!({
            "promptSuggestionEnabled": false,
            "statusLine": { "type": "command", "command": command },
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": files.record() }] }] },
        })),
    )?;
    let mut args = vec![queue::prompt(Kind::Idle)];
    args.extend(
        [
            "--setting-sources",
            "local",
            "--strict-mcp-config",
            "--settings",
            &settings.to_string_lossy(),
            "--disable-slash-commands",
            "--tools",
            "",
            "--effort",
            "low",
        ]
        .map(String::from),
    );
    if let Some(model) = model {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    Ok(args)
}

/// The limits of the last input in `log`, the status line log's text, that has any, and how many
/// inputs the log holds.
pub fn status_line_limits(log: &str) -> (usize, Vec<Limit>) {
    let inputs: Vec<&str> = log.lines().collect();
    let limits = inputs
        .iter()
        .rev()
        .map(|input| claude_status_line(input))
        .find(|limits| !limits.is_empty())
        .unwrap_or_default();
    (inputs.len(), limits)
}

/// The tokens the session used, as the last input in `log`, the status line log's text, gives them
/// in `context_window`: the session's input and output tokens, and the cached input tokens of its
/// last request.
pub fn status_line_usage(log: &str) -> Option<Usage> {
    let input: Value = serde_json::from_str(log.lines().last()?).ok()?;
    let window = &input["context_window"];
    Some(Usage {
        input: window["total_input_tokens"].as_u64()?,
        cached: window["current_usage"]["cache_read_input_tokens"]
            .as_u64()
            .unwrap_or(0),
        output: window["total_output_tokens"].as_u64()?,
    })
}

/// What `agent-limits` printed.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Read {
    pub limits: Vec<Limit>,
    /// Codex: the account's kind, `none` when signed out.
    pub account: Option<String>,
    /// Codex: why the app-server gave no limits.
    pub refused: Option<String>,
}

impl Read {
    /// What the JSON lines of `stdout` say.
    pub fn parse(stdout: &str) -> Self {
        let mut read = Self::default();
        for line in stdout.lines() {
            let Ok(line) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(limit) = Limit::from_json(&line) {
                read.limits.push(limit);
            } else if let Some(account) = line.get("account") {
                read.account = Some(account.as_str().unwrap_or("none").to_owned());
            } else if let Some(reason) = line["refused"].as_str() {
                read.refused = Some(reason.to_owned());
            }
        }
        read
    }
}

/// A limit as Claude Code's `/usage` shows it: `<label>: <percent>% used · resets <time>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub label: String,
    pub percent: f64,
    /// The reset time as shown, if it is.
    pub resets: Option<String>,
}

/// The limits the text of `/usage` shows, in its order.
pub fn shown(text: &str) -> Vec<Shown> {
    text.lines()
        .filter_map(|line| {
            let (label, rest) = line.split_once(": ")?;
            let (percent, rest) = rest.split_once("% used")?;
            Some(Shown {
                label: label.to_owned(),
                percent: percent.parse().ok()?,
                resets: rest
                    .split_once("resets ")
                    .map(|(_, time)| time.trim().to_owned()),
            })
        })
        .collect()
}

/// The text of the `/usage` result in `claude`'s stream-json `output`; empty without one.
pub fn usage_text(output: &str) -> String {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["type"] == "result")
        .and_then(|line| line["result"].as_str().map(String::from))
        .unwrap_or_default()
}

/// `limits`, each with its usage and when it resets, from `now_s` in seconds since the Unix epoch.
fn describe(limits: &[Limit], now_s: i64) -> String {
    if limits.is_empty() {
        return "no limits".to_owned();
    }
    let limits: Vec<String> = limits
        .iter()
        .map(|limit| {
            let resets = limit.resets_at.map_or_else(
                || "no reset time".to_owned(),
                |at| format!("resets in {:.1} h", (at - now_s) as f64 / 3600.0),
            );
            format!("{} {}% used, {resets}", limit.name, limit.used_percent)
        })
        .collect();
    limits.join("; ")
}

/// The limits checks: each returns its line in the output.
#[derive(Debug, Default)]
pub struct LimitsCheck {
    checks: Checks,
}

impl LimitsCheck {
    /// Claude Code's `/usage`, at `now_s`: `agent-limits` printed `read`, and the command's text
    /// showed `shown`. Passes when it read each limit the text shows, in its order, with the same
    /// usage and a reset time.
    pub fn usage(&mut self, read: &Read, shown: &[Shown], now_s: i64) -> String {
        let pass = !shown.is_empty()
            && read.limits.len() == shown.len()
            && read.limits.iter().zip(shown).all(|(limit, shown)| {
                (limit.used_percent - shown.percent).abs() < 1.0
                    && limit.resets_at.is_some()
                    && shown.resets.is_some()
            });
        let shown: Vec<String> = shown
            .iter()
            .map(|shown| format!("{} {}%", shown.label, shown.percent))
            .collect();
        let details = format!(
            "read {}; /usage shows {}",
            describe(&read.limits, now_s),
            if shown.is_empty() {
                "no limits".to_owned()
            } else {
                shown.join("; ")
            }
        );
        self.checks.report("/usage", pass, &details)
    }

    /// Claude Code's status line, at `now_s`, in the session that `summary` describes: its log held
    /// `inputs` inputs, the last with limits gave `status`, and `/usage` read `usage` after the
    /// turn. Passes when the status line gave limits, each the same as the `/usage` row of its
    /// window: usage within a point, reset within a minute.
    pub fn status_line(
        &mut self,
        summary: &str,
        inputs: usize,
        status: &[Limit],
        usage: &[Limit],
        now_s: i64,
    ) -> String {
        let row = |window: &str| {
            let (_, row) = SAME.iter().find(|(same, _)| *same == window)?;
            usage.iter().find(|limit| limit.name == *row)
        };
        let pass = !status.is_empty()
            && status.iter().all(|limit| {
                row(&limit.name).is_some_and(|row| {
                    (row.used_percent - limit.used_percent).abs() <= 1.0
                        && row
                            .resets_at
                            .zip(limit.resets_at)
                            .is_some_and(|(row, limit)| (row - limit).abs() <= 60)
                })
            });
        let lacks: Vec<&str> = usage
            .iter()
            .filter(|limit| {
                !status
                    .iter()
                    .any(|window| row(&window.name).is_some_and(|row| row.name == limit.name))
            })
            .map(|limit| limit.name.as_str())
            .collect();
        let details = format!(
            "{summary}; {inputs} status line inputs, the last with limits: {}; /usage after the \
             turn: {}; the status line lacks {lacks:?}",
            describe(status, now_s),
            describe(usage, now_s)
        );
        self.checks.report("status line", pass, &details)
    }

    /// The Codex app-server, at `now_s`: `agent-limits` printed `read`. Passes when it read
    /// limits, each with a reset time.
    pub fn codex(&mut self, read: &Read, now_s: i64) -> String {
        let pass =
            !read.limits.is_empty() && read.limits.iter().all(|limit| limit.resets_at.is_some());
        let account = read.account.as_deref().unwrap_or("unknown");
        let details = match &read.refused {
            Some(reason) => format!("account {account}; the app-server refused: {reason}"),
            None => format!("account {account}; read {}", describe(&read.limits, now_s)),
        };
        self.checks
            .report("account/rateLimits/read", pass, &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output: the verdict on the limits of `source` on `agent`.
    pub fn verdict(&self, source: &str, agent: Agent) -> String {
        format!(
            "limits of {source} on {}: {}",
            agent.cli(),
            verdict(self.pass())
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::read_json;

    use super::*;

    fn limit(name: &str, used_percent: f64, resets_at: Option<i64>) -> Limit {
        Limit {
            name: name.to_owned(),
            used_percent,
            resets_at,
        }
    }

    #[test]
    fn agent_limits_gets_its_log_and_codex_turns_off_each_mcp_server() {
        let log = Path::new("/w/log.jsonl");
        assert_eq!(claude(log), ["claude", "--log", "/w/log.jsonl"]);
        let args = codex(log, "[mcp_servers.alpha]\ncommand = \"a\"\n");
        let mut expected: Vec<String> = ["codex", "--log", "/w/log.jsonl", "--"]
            .map(String::from)
            .to_vec();
        expected.extend(app_server::options(None, None));
        expected.extend([
            "-c".to_owned(),
            "mcp_servers.alpha.enabled=false".to_owned(),
        ]);
        assert_eq!(args, expected);
        assert!(args.windows(2).any(|pair| pair == ["--disable", "plugins"]));
    }

    #[test]
    fn mcp_servers_are_the_tables_names_without_values() {
        let config = "model = \"gpt\"\n\
                      [mcp_servers.alpha]\n\
                      command = \"secret-value\"\n\
                      [mcp_servers.alpha.env]\n\
                      TOKEN = \"secret-token\"\n\
                       [mcp_servers.\"beta-two\"]\n\
                      [mcp_servers.gamma-3]\n\
                      [mcp_servers.]\n\
                      [profiles.mcp_servers.delta]\n";
        assert_eq!(mcp_servers(config), ["alpha", "beta-two", "gamma-3"]);
        assert_eq!(mcp_servers(""), Vec::<String>::new());
    }

    #[test]
    fn codex_config_is_in_codex_home_or_else_in_home() {
        let file = |codex_home: Option<&str>, home: Option<&str>| {
            codex_config_file(codex_home.map(OsStr::new), home.map(OsStr::new))
        };
        assert_eq!(
            file(Some("/c"), Some("/h")),
            Some(PathBuf::from("/c/config.toml"))
        );
        assert_eq!(
            file(None, Some("/h")),
            Some(PathBuf::from("/h/.codex/config.toml"))
        );
        assert_eq!(file(None, None), None);
    }

    #[test]
    fn the_status_line_session_runs_the_prototype_and_a_stop_hook_and_loads_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let files = Files::new(dir.path());
        let agent_limits = Path::new("/bin/agent limits");
        let args = status_line_session(&files, agent_limits, Some("haiku")).unwrap();
        assert!(files.project.is_dir());
        assert_eq!(
            status_line_log(&files),
            dir.path().join("status-line.jsonl")
        );
        let settings = files.dir.join("settings.json");
        let mut expected = vec!["Reply with only OK.".to_owned()];
        expected.extend(
            [
                "--setting-sources",
                "local",
                "--strict-mcp-config",
                "--settings",
                &settings.to_string_lossy(),
                "--disable-slash-commands",
                "--tools",
                "",
                "--effort",
                "low",
                "--model",
                "haiku",
            ]
            .map(String::from),
        );
        assert_eq!(args, expected);
        let without_model = status_line_session(&files, agent_limits, None).unwrap();
        assert_eq!(without_model, expected[..expected.len() - 2]);

        let settings = read_json(&settings);
        assert_eq!(settings["promptSuggestionEnabled"], false);
        assert_eq!(settings["statusLine"]["type"], "command");
        assert_eq!(
            settings["statusLine"]["command"],
            format!(
                "'/bin/agent limits' status-line --log '{}'",
                status_line_log(&files).display()
            )
        );
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0],
            json!({ "type": "command", "command": files.record() })
        );
        assert_eq!(settings.as_object().unwrap().len(), 3);
    }

    #[test]
    fn the_status_line_gives_the_last_input_with_limits() {
        let with = |used: f64| json!({ "rate_limits": { "five_hour": { "used_percentage": used, "resets_at": 9 } } });
        let log = format!("{}\n{}\n{}\n", json!({}), with(1.0), with(2.0));
        assert_eq!(
            status_line_limits(&log),
            (3, vec![limit("five_hour", 2.0, Some(9))])
        );
        let log = format!("{}\n{}\n", with(3.0), json!({ "model": {} }));
        assert_eq!(
            status_line_limits(&log),
            (2, vec![limit("five_hour", 3.0, Some(9))])
        );
        assert_eq!(status_line_limits(""), (0, Vec::new()));
    }

    #[test]
    fn the_status_line_gives_the_tokens_of_its_last_input() {
        let input = |total: u64| {
            json!({ "context_window": {
                "total_input_tokens": total, "total_output_tokens": 4,
                "current_usage": { "input_tokens": 2, "cache_read_input_tokens": 7 },
            } })
        };
        let log = format!("{}\n{}\n", input(1), input(4_770));
        assert_eq!(
            status_line_usage(&log),
            Some(Usage {
                input: 4_770,
                cached: 7,
                output: 4
            })
        );
        let uncached = json!({ "context_window": {
            "total_input_tokens": 3, "total_output_tokens": 4, "current_usage": null,
        } });
        assert_eq!(
            status_line_usage(&uncached.to_string()),
            Some(Usage {
                input: 3,
                cached: 0,
                output: 4
            })
        );
        assert_eq!(status_line_usage("{}"), None);
        assert_eq!(status_line_usage(""), None);
    }

    #[test]
    fn read_takes_the_limits_the_account_and_the_refusal() {
        let stdout = "{\"account\":\"apiKey\"}\n\
                      not json\n\
                      {\"limit\":\"codex primary\",\"usedPercent\":23.0,\"resetsAt\":9}\n\
                      {\"refused\":\"chatgpt authentication required\"}\n";
        assert_eq!(
            Read::parse(stdout),
            Read {
                limits: vec![limit("codex primary", 23.0, Some(9))],
                account: Some("apiKey".to_owned()),
                refused: Some("chatgpt authentication required".to_owned()),
            }
        );
        assert_eq!(
            Read::parse("{\"account\":null}").account.as_deref(),
            Some("none")
        );
        assert_eq!(Read::parse(""), Read::default());
    }

    /// The `/usage` text of Claude Code 2.1.322 on a Claude subscription, without its breakdown.
    const USAGE_TEXT: &str = "You are currently using your subscription to power your Claude \
         Code usage\n\nCurrent session: 29% used · resets Nov 12 at 2:19am (Asia/Bangkok)\n\
         Current week (all models): 94% used · resets Nov 15 at 9:59pm (Asia/Bangkok)\n\
         Current week (Fable): 1% used\n\nLast 24h · 9760 requests · 25 sessions\n  100%  Stop";

    #[test]
    fn shown_takes_each_limit_line_of_usage() {
        let shown = shown(USAGE_TEXT);
        let expected = [
            (
                "Current session",
                29.0,
                Some("Nov 12 at 2:19am (Asia/Bangkok)"),
            ),
            (
                "Current week (all models)",
                94.0,
                Some("Nov 15 at 9:59pm (Asia/Bangkok)"),
            ),
            ("Current week (Fable)", 1.0, None),
        ]
        .map(|(label, percent, resets)| Shown {
            label: label.to_owned(),
            percent,
            resets: resets.map(String::from),
        });
        assert_eq!(shown, expected);
        assert_eq!(super::shown("Total cost: $0.0000"), []);
    }

    #[test]
    fn usage_text_is_the_result_line() {
        let output = format!(
            "{}\nnot json\n{}\n",
            json!({ "type": "assistant", "result": "no" }),
            json!({ "type": "result", "result": "text" })
        );
        assert_eq!(usage_text(&output), "text");
        assert_eq!(usage_text(""), "");
    }

    fn shown_line(label: &str, percent: f64, resets: bool) -> Shown {
        Shown {
            label: label.to_owned(),
            percent,
            resets: resets.then(|| "Nov 15".to_owned()),
        }
    }

    #[test]
    fn usage_passes_when_each_shown_limit_was_read_with_its_reset() {
        let read = |limits: Vec<Limit>| Read {
            limits,
            ..Read::default()
        };
        let both = read(vec![
            limit("session", 29.0, Some(3_600)),
            limit("weekly_all", 94.0, Some(7_200)),
        ]);
        let shown = [
            shown_line("Current session", 29.0, true),
            shown_line("Current week (all models)", 94.0, true),
        ];
        let mut check = LimitsCheck::default();
        assert_eq!(
            check.usage(&both, &shown, 0),
            "  /usage: pass — read session 29% used, resets in 1.0 h; weekly_all 94% used, \
             resets in 2.0 h; /usage shows Current session 29%; Current week (all models) 94%"
        );
        assert!(check.pass());

        let fails = |read: &Read, shown: &[Shown]| {
            let mut check = LimitsCheck::default();
            check.usage(read, shown, 0);
            !check.pass()
        };
        assert!(fails(&read(Vec::new()), &[]));
        assert!(fails(&read(both.limits[..1].to_vec()), &shown));
        let mut other = both.clone();
        other.limits[1].used_percent = 95.0;
        assert!(fails(&other, &shown));
        other.limits[1].used_percent = 93.0;
        assert!(fails(&other, &shown));
        let mut no_reset = both.clone();
        no_reset.limits[0].resets_at = None;
        assert!(fails(&no_reset, &shown));
        let mut shown_no_reset = shown.clone();
        shown_no_reset[1].resets = None;
        assert!(fails(&both, &shown_no_reset));
        let mut check = LimitsCheck::default();
        assert!(
            check
                .usage(&read(Vec::new()), &[], 0)
                .ends_with("read no limits; /usage shows no limits")
        );
    }

    #[test]
    fn the_status_line_passes_when_each_window_matches_its_usage_row() {
        let usage = [
            limit("session", 30.0, Some(3_600)),
            limit("weekly_all", 94.0, Some(7_200)),
            limit("weekly_scoped Fable", 1.0, Some(7_200)),
        ];
        let status = [
            limit("five_hour", 29.0, Some(3_660)),
            limit("seven_day", 94.5, Some(7_140)),
        ];
        let mut check = LimitsCheck::default();
        assert_eq!(
            check.status_line("replies [\"OK\"]", 2, &status, &usage, 0),
            "  status line: pass — replies [\"OK\"]; 2 status line inputs, the last with limits: \
             five_hour 29% used, resets in 1.0 h; seven_day 94.5% used, resets in 2.0 h; /usage \
             after the turn: session 30% used, resets in 1.0 h; weekly_all 94% used, resets in \
             2.0 h; weekly_scoped Fable 1% used, resets in 2.0 h; the status line lacks \
             [\"weekly_scoped Fable\"]"
        );
        assert!(check.pass());

        let fails = |status: &[Limit]| {
            let mut check = LimitsCheck::default();
            check.status_line("", 1, status, &usage, 0);
            !check.pass()
        };
        assert!(fails(&[]));
        for changed in [
            limit("five_hour", 31.5, Some(3_600)),
            limit("five_hour", 28.5, Some(3_600)),
            limit("five_hour", 30.0, Some(3_661)),
            limit("five_hour", 30.0, Some(3_539)),
            limit("five_hour", 30.0, None),
            limit("spend_limit", 30.0, Some(3_600)),
        ] {
            assert!(fails(&[status[1].clone(), changed.clone()]), "{changed:?}");
        }
        let mut check = LimitsCheck::default();
        check.status_line("", 1, &status, &[limit("session", 29.0, None)], 0);
        assert!(!check.pass());
    }

    #[test]
    fn codex_passes_when_the_app_server_read_limits_with_their_resets() {
        let mut check = LimitsCheck::default();
        let read = Read {
            limits: vec![
                limit("codex primary 300 min", 23.0, Some(2_800)),
                limit("codex secondary 10080 min", 41.0, Some(8_200)),
            ],
            account: Some("chatgpt".to_owned()),
            refused: None,
        };
        assert_eq!(
            check.codex(&read, 1_000),
            "  account/rateLimits/read: pass — account chatgpt; read codex primary 300 min 23% \
             used, resets in 0.5 h; codex secondary 10080 min 41% used, resets in 2.0 h"
        );
        assert!(check.pass());
        assert_eq!(
            check.verdict("the app-server", Agent::Codex),
            "limits of the app-server on codex: pass"
        );

        let mut check = LimitsCheck::default();
        let refused = Read {
            limits: Vec::new(),
            account: Some("apiKey".to_owned()),
            refused: Some("chatgpt authentication required".to_owned()),
        };
        assert_eq!(
            check.codex(&refused, 0),
            "  account/rateLimits/read: fail — account apiKey; the app-server refused: chatgpt \
             authentication required"
        );
        assert_eq!(
            check.verdict("/usage", Agent::Claude),
            "limits of /usage on claude: fail"
        );

        let mut check = LimitsCheck::default();
        let no_reset = Read {
            limits: vec![limit("codex primary", 23.0, None)],
            ..Read::default()
        };
        assert!(
            check
                .codex(&no_reset, 0)
                .ends_with("account unknown; read codex primary 23% used, no reset time")
        );
        assert!(!check.pass());
    }
}
