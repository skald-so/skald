//! Reads the subscription limits Claude Code and Codex show the user, only from what each CLI gives
//! a local program: the output of Claude Code's `/usage` command, the input Claude Code gives its
//! status line command, and the Codex app-server's `account/rateLimits/read`. It reads no
//! credential and calls no vendor API: the CLI does, on its own sign-in.

use std::io::{self, Write};
use std::process::{Command, Stdio};

use agent_app_server::{Client, Transport};
use serde_json::{Value, json};

/// The arguments of the `claude` command that prints the `/usage` report, with no model turn: print
/// mode, stream-json so that the report also comes structured, no project or user settings, no MCP
/// server, and no session saved.
pub const CLAUDE_USAGE: [&str; 9] = [
    "-p",
    "/usage",
    "--output-format",
    "stream-json",
    "--verbose",
    "--setting-sources",
    "local",
    "--strict-mcp-config",
    "--no-session-persistence",
];

/// A subscription limit: how much of a window is used, and when the window resets.
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    /// The CLI's name of the limit, such as `session`, `weekly_scoped Fable`, `five_hour` or
    /// `codex primary 300 min`.
    pub name: String,
    /// The share of the window used, from 0 to 100.
    pub used_percent: f64,
    /// When the window resets, in seconds since the Unix epoch.
    pub resets_at: Option<i64>,
}

impl Limit {
    /// The limit as a line of `agent-limits`' output.
    pub fn to_json(&self) -> Value {
        json!({ "limit": self.name, "usedPercent": self.used_percent, "resetsAt": self.resets_at })
    }

    /// The limit in a line of `agent-limits`' output; `None` for another line.
    pub fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            name: value["limit"].as_str()?.to_owned(),
            used_percent: value["usedPercent"].as_f64()?,
            resets_at: value["resetsAt"].as_i64(),
        })
    }
}

/// The limits in the output of `claude` with [`CLAUDE_USAGE`]: the rows of the `usage_report` that
/// the `/usage` result carries, in the CLI's order. None without a report or with a report without
/// rows, such as on an API key.
pub fn claude_usage(output: &str) -> Vec<Limit> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .flat_map(|line| {
            line["usage_report"]["rate_limits"]["limits"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(usage_row)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A row of the `/usage` report: its `kind`, then the display names of its scope, if any.
fn usage_row(row: &Value) -> Option<Limit> {
    let mut name = row["kind"].as_str()?.to_owned();
    for scope in ["model", "surface"] {
        if let Some(scope) = row["scope"][scope]["display_name"].as_str() {
            name = format!("{name} {scope}");
        }
    }
    Some(Limit {
        name,
        used_percent: row["percent"].as_f64()?,
        resets_at: row["resets_at"].as_str().and_then(unix_seconds),
    })
}

/// The limits in an input of Claude Code's status line command: each window of `rate_limits`, such
/// as `five_hour`, by name. None when the input has no `rate_limits`, as before the session's first
/// model response or on an API key.
pub fn claude_status_line(input: &str) -> Vec<Limit> {
    let input: Value = serde_json::from_str(input).unwrap_or_default();
    input["rate_limits"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, window)| {
            Some(Limit {
                name: name.clone(),
                used_percent: window["used_percentage"].as_f64()?,
                resets_at: window["resets_at"].as_i64(),
            })
        })
        .collect()
}

/// The text the status line command prints: each limit's name and usage.
pub fn status_line(limits: &[Limit]) -> String {
    if limits.is_empty() {
        return "no limits".to_owned();
    }
    let limits: Vec<String> = limits
        .iter()
        .map(|limit| format!("{} {}%", limit.name, limit.used_percent))
        .collect();
    limits.join(" · ")
}

/// The limits in a message of the Codex app-server that carries them: the result of
/// `account/rateLimits/read` or the params of `account/rateLimits/updated`. Each bucket of
/// `rateLimitsByLimitId`, or else the one of `rateLimits`, gives its `primary` and `secondary`
/// windows.
pub fn codex(message: &Value) -> Vec<Limit> {
    let buckets: Vec<&Value> = match message["rateLimitsByLimitId"].as_object() {
        Some(buckets) if !buckets.is_empty() => buckets.values().collect(),
        _ => vec![&message["rateLimits"]],
    };
    buckets
        .into_iter()
        .flat_map(|bucket| {
            ["primary", "secondary"]
                .into_iter()
                .filter_map(|window| codex_window(bucket, window))
        })
        .collect()
}

/// The `window` of a Codex bucket, named after the bucket, the window and its length.
fn codex_window(bucket: &Value, window: &str) -> Option<Limit> {
    let used = &bucket[window];
    let id = bucket["limitName"]
        .as_str()
        .or(bucket["limitId"].as_str())
        .unwrap_or("codex");
    let name = match used["windowDurationMins"].as_i64() {
        Some(minutes) => format!("{id} {window} {minutes} min"),
        None => format!("{id} {window}"),
    };
    Some(Limit {
        name,
        used_percent: used["usedPercent"].as_f64()?,
        resets_at: used["resetsAt"].as_i64(),
    })
}

/// `text`, an RFC 3339 time such as `2026-11-15T14:59:59.892319+00:00`, in seconds since the Unix
/// epoch; fractions of a second are dropped.
pub fn unix_seconds(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let (time, offset) = match time.strip_suffix('Z') {
        Some(time) => (time, 0),
        None => {
            let at = time.rfind(['+', '-'])?;
            let sign = if time[at..].starts_with('-') { -1 } else { 1 };
            let [hours, minutes] = numbers(&time[at + 1..], ':')?;
            (&time[..at], sign * (hours * 3600 + minutes * 60))
        }
    };
    let [year, month, day] = numbers(date, '-')?;
    let whole = time.split('.').next()?;
    let [hours, minutes, seconds] = numbers(whole, ':')?;
    Some(
        days_from_civil(year, month, day) * 86_400 + hours * 3600 + minutes * 60 + seconds - offset,
    )
}

/// The `N` numbers of `text`, split at `separator`.
fn numbers<const N: usize>(text: &str, separator: char) -> Option<[i64; N]> {
    let parts: Vec<i64> = text
        .split(separator)
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    parts.try_into().ok()
}

/// Days from 1970-01-01 to the date in the proleptic Gregorian calendar (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Runs `claude`, a command that runs Claude Code, with [`CLAUDE_USAGE`] and returns its output.
/// Fails when the command fails; its stderr is the caller's.
pub fn run_claude(mut claude: Command) -> io::Result<String> {
    let output = claude
        .args(CLAUDE_USAGE)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "claude /usage failed with {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What the Codex app-server said of the account and its limits.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexLimits {
    /// The account's kind, `account.type` of `account/read`, such as `chatgpt` or `apiKey`; `null`
    /// when signed out.
    pub account: Value,
    /// The limits of `account/rateLimits/read`, or the app-server's reason for refusing them.
    pub limits: Result<Vec<Limit>, String>,
}

/// Asks the app-server of `client`, initialized, for the account's kind and its limits.
pub fn read_codex<T: Transport, W: Write>(client: &mut Client<T, W>) -> io::Result<CodexLimits> {
    let account = client.call("account/read", json!({}))?;
    let limits = client.request("account/rateLimits/read", Value::Null)?;
    Ok(CodexLimits {
        account: account["account"]["type"].clone(),
        limits: limits.map(|result| codex(&result)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_seconds_reads_rfc_3339_times_with_any_offset() {
        for (text, seconds) in [
            ("1970-01-01T00:00:00Z", 0),
            ("1969-12-31T23:59:59Z", -1),
            ("2026-11-15T14:59:59.892319+00:00", 1_794_754_799),
            ("2000-02-29T12:00:00Z", 951_825_600),
            ("2024-03-01T00:00:00+07:00", 1_709_226_000),
            ("2100-12-31T23:59:59-05:30", 4_134_000_599),
            ("1600-01-01T00:00:00Z", -11_676_096_000),
        ] {
            assert_eq!(unix_seconds(text), Some(seconds), "{text}");
        }
    }

    #[test]
    fn unix_seconds_refuses_what_is_not_a_time() {
        for text in [
            "",
            "2026-11-15",
            "2026-11-15T14:59:59",
            "2026-11-15T14:59Z",
            "2026-11-15T14:59:59+07",
            "2026-11T14:59:59Z",
            "2026-11-15Tnoon",
        ] {
            assert_eq!(unix_seconds(text), None, "{text}");
        }
    }

    #[test]
    fn a_limit_survives_the_output_line() {
        for limit in [
            Limit {
                name: "five_hour".to_owned(),
                used_percent: 23.5,
                resets_at: Some(1_794_432_801),
            },
            Limit {
                name: "codex primary".to_owned(),
                used_percent: 0.0,
                resets_at: None,
            },
        ] {
            assert_eq!(Limit::from_json(&limit.to_json()), Some(limit));
        }
        assert_eq!(Limit::from_json(&json!({ "refused": "no" })), None);
        assert_eq!(Limit::from_json(&json!({ "limit": "session" })), None);
    }

    fn limit(name: &str, used_percent: f64, resets_at: Option<i64>) -> Limit {
        Limit {
            name: name.to_owned(),
            used_percent,
            resets_at,
        }
    }

    #[test]
    fn usage_rows_are_named_by_kind_and_scope() {
        let report = |limits: Value| {
            json!({ "type": "assistant", "usage_report": { "rate_limits": { "limits": limits } } })
                .to_string()
        };
        let rows = json!([
            { "kind": "weekly_scoped", "percent": 5, "resets_at": null,
              "scope": { "model": { "display_name": "Opus" }, "surface": { "display_name": "Web" } } },
            { "kind": "weekly_scoped", "percent": 6.5, "resets_at": "1970-01-01T00:01:00Z",
              "scope": { "model": null, "surface": { "display_name": "Desktop" } } },
            { "kind": "session" },
            { "percent": 7 },
        ]);
        assert_eq!(
            claude_usage(&format!("not json\n{}\n", report(rows))),
            [
                limit("weekly_scoped Opus Web", 5.0, None),
                limit("weekly_scoped Desktop", 6.5, Some(60)),
            ]
        );
        assert_eq!(claude_usage(&report(Value::Null)), []);
        let no_rate_limits = json!({ "usage_report": { "rate_limits": null } });
        assert_eq!(claude_usage(&no_rate_limits.to_string()), []);
    }

    #[test]
    fn status_line_windows_need_their_usage() {
        let input = json!({ "rate_limits": {
            "five_hour": { "used_percentage": 23.5 },
            "seven_day": { "resets_at": 9 },
        } });
        assert_eq!(
            claude_status_line(&input.to_string()),
            [limit("five_hour", 23.5, None)]
        );
        assert_eq!(claude_status_line("not json"), []);
    }

    /// The example of `account/rateLimits/read` in Codex's app-server docs, shortened.
    fn docs_example() -> Value {
        let primary = |used: u32, minutes: u32, at: u32| json!({ "usedPercent": used, "windowDurationMins": minutes, "resetsAt": at });
        json!({
            "rateLimits": { "limitId": "codex", "primary": primary(25, 15, 1_730_947_200) },
            "rateLimitsByLimitId": {
                "codex": { "limitId": "codex", "limitName": null,
                           "primary": primary(25, 15, 1_730_947_200), "secondary": null },
                "codex_other": { "limitId": "codex_other", "limitName": "Other",
                                 "primary": primary(42, 60, 1_730_950_800), "secondary": null },
            },
        })
    }

    #[test]
    fn codex_takes_each_bucket_by_limit_id_or_else_the_single_one() {
        assert_eq!(
            codex(&docs_example()),
            [
                limit("codex primary 15 min", 25.0, Some(1_730_947_200)),
                limit("Other primary 60 min", 42.0, Some(1_730_950_800)),
            ]
        );
        let single = json!({
            "rateLimitsByLimitId": {},
            "rateLimits": { "primary": { "usedPercent": 3 }, "secondary": { "resetsAt": 9 } },
        });
        assert_eq!(codex(&single), [limit("codex primary", 3.0, None)]);
        assert_eq!(codex(&Value::Null), []);
    }

    #[test]
    fn the_status_line_shows_each_limit_or_none() {
        let limits = [
            Limit {
                name: "five_hour".to_owned(),
                used_percent: 23.5,
                resets_at: None,
            },
            Limit {
                name: "seven_day".to_owned(),
                used_percent: 41.0,
                resets_at: None,
            },
        ];
        assert_eq!(status_line(&limits), "five_hour 23.5% · seven_day 41%");
        assert_eq!(status_line(&[]), "no limits");
    }
}
