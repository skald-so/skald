//! A Codex config home as these tests read it: the bytes of each file in it; `config.toml` as TOML
//! and `hooks.json` as JSON, both as JSON values; Skald's entries in them; and both without Skald's
//! entries, to compare with what a fixture held.
//!
//! Skald's entries are the MCP server `skald` of `config.toml` and the hooks that run Skald's hook
//! handler for Codex, `skald hook codex`, in either file: Codex takes hooks from both, each under
//! `hooks.<event>` as a list of groups, a group's `hooks` a list of hooks whose `command` (and, on
//! Windows, `commandWindows`) Codex runs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Codex's config file and its hooks file in a config home.
pub const CONFIG: &str = "config.toml";
pub const HOOKS: &str = "hooks.json";

/// The name of Skald's MCP server in `config.toml`.
pub const SERVER: &str = "skald";

/// The events whose hooks Skald's handler for Codex serves.
pub const EVENTS: [&str; 5] = [
    "SessionStart",
    "PostToolUse",
    "UserPromptSubmit",
    "Stop",
    "PermissionRequest",
];

/// The fields of a hook that hold a command.
const COMMANDS: [&str; 3] = ["command", "commandWindows", "command_windows"];

/// Matchers that take every tool, and so every hook run of the event.
const EVERY_TOOL: [&str; 3] = ["", "*", ".*"];

/// The fixtures, in `homes/`: their names, with `empty`, a config home with no file.
pub const FIXTURES: [&str; 6] = ["empty", "others", "paseo", "herdr", "hosts", "older"];

/// The fixture whose `config.toml` and `hooks.json` hold an older Skald's entries.
pub const OLDER: &str = "older";

/// What only the older Skald's entries of [`OLDER`] hold.
pub const OLDER_MARK: &str = "/opt/skald-0.1/";

/// Each file under a directory, by its path relative to it, with its bytes.
pub type Files = BTreeMap<PathBuf, Vec<u8>>;

/// The fixture `name`'s directory.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/codex_setup/homes")
        .join(name)
}

/// The files of the fixture `name`; none for `empty`.
pub fn fixture_files(name: &str) -> Files {
    files(&fixture(name))
}

/// Every file under `dir`, recursively; none if `dir` does not exist.
pub fn files(dir: &Path) -> Files {
    let mut found = Files::new();
    let mut left = vec![dir.to_owned()];
    while let Some(next) = left.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("read a directory entry").path();
            if path.is_dir() {
                left.push(path);
            } else {
                let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {path:?}: {err}"));
                let relative = path.strip_prefix(dir).expect("under the directory");
                found.insert(relative.to_owned(), bytes);
            }
        }
    }
    found
}

/// Writes `files` into `dir`, which it makes.
pub fn write(dir: &Path, files: &Files) {
    fs::create_dir_all(dir).expect("make the config home");
    for (relative, bytes) in files {
        let path = dir.join(relative);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("make a directory");
        fs::write(&path, bytes).expect("write a fixture file");
    }
}

/// The text of `file` in `files`, if there.
pub fn text<'a>(files: &'a Files, file: &str) -> Option<&'a str> {
    (files.get(Path::new(file))).map(|bytes| std::str::from_utf8(bytes).expect("UTF-8"))
}

/// `config.toml` and `hooks.json` of a config home, as JSON values; [`Value::Null`] for one that
/// is not there.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub config: Value,
    pub hooks: Value,
}

impl Read {
    /// The two files of `files`; fails if one does not parse.
    #[track_caller]
    pub fn of(files: &Files) -> Self {
        let config = text(files, CONFIG).map_or(Value::Null, |text| {
            let table: toml::Table = toml::from_str(text)
                .unwrap_or_else(|err| panic!("{CONFIG} is not TOML ({err}):\n{text}"));
            serde_json::to_value(table).expect("TOML as JSON")
        });
        let hooks = text(files, HOOKS).map_or(Value::Null, |text| {
            serde_json::from_str(text)
                .unwrap_or_else(|err| panic!("{HOOKS} is not JSON ({err}):\n{text}"))
        });
        Read { config, hooks }
    }

    /// The two files in the config home `home`.
    #[track_caller]
    pub fn home(home: &Path) -> Self {
        Read::of(&files(home))
    }

    /// Skald's MCP server, if any.
    pub fn server(&self) -> Option<&Value> {
        self.config.get("mcp_servers")?.get(SERVER)
    }

    /// Skald's MCP server's program and arguments; fails unless there is a server with them.
    #[track_caller]
    pub fn server_command(&self) -> (String, Vec<String>) {
        let server = (self.server()).unwrap_or_else(|| panic!("no MCP server {SERVER}: {self:#?}"));
        let program = (server["command"].as_str())
            .unwrap_or_else(|| panic!("no command: {server}"))
            .to_owned();
        let args = (server["args"].as_array().map(Vec::as_slice))
            .unwrap_or_default()
            .iter()
            .map(|arg| arg.as_str().expect("an argument is a string").to_owned())
            .collect();
        (program, args)
    }

    /// The groups of hooks of `event` in both files, each with its hooks.
    pub fn groups(&self, event: &str) -> Vec<&Value> {
        [&self.config, &self.hooks]
            .into_iter()
            .filter_map(|file| file.get("hooks")?.get(event)?.as_array())
            .flatten()
            .collect()
    }

    /// Skald's hooks of `event`, each with the matcher of its group.
    pub fn skald_hooks(&self, event: &str) -> Vec<(Option<&str>, &Value)> {
        let mut found = Vec::new();
        for group in self.groups(event) {
            let matcher = group.get("matcher").and_then(Value::as_str);
            let hooks = (group.get("hooks").and_then(Value::as_array)).map(Vec::as_slice);
            for hook in hooks.unwrap_or_default() {
                if is_skalds(hook) {
                    found.push((matcher, hook));
                }
            }
        }
        found
    }

    /// The command of Skald's only hook of `event`, as Codex runs it on this OS; fails unless
    /// there is exactly one such hook, a command hook whose group takes every tool.
    #[track_caller]
    pub fn hook_command(&self, event: &str) -> String {
        let found = self.skald_hooks(event);
        let [(matcher, hook)] = found.as_slice() else {
            panic!("not one Skald hook of {event}: {found:#?}");
        };
        assert!(
            matcher.is_none_or(|matcher| EVERY_TOOL.contains(&matcher)),
            "the group of Skald's hook of {event} does not take every tool: {matcher:?}"
        );
        assert_eq!(hook["type"].as_str(), Some("command"), "{hook}");
        let windows = (hook
            .get("commandWindows")
            .or_else(|| hook.get("command_windows")))
        .and_then(Value::as_str);
        let command = if cfg!(windows) {
            windows.or_else(|| hook["command"].as_str())
        } else {
            hook["command"].as_str()
        };
        (command.unwrap_or_else(|| panic!("no command: {hook}"))).to_owned()
    }

    /// Both files without Skald's entries: its MCP server and its hooks gone, and a table, a list
    /// or a file that held only them gone with them.
    pub fn without_skald(&self) -> Read {
        let mut config = self.config.clone();
        if let Some(root) = config.as_object_mut() {
            if let Some(servers) = root.get_mut("mcp_servers").and_then(Value::as_object_mut) {
                servers.remove(SERVER);
                if servers.is_empty() {
                    root.remove("mcp_servers");
                }
            }
            drop_skald_hooks(root);
        }
        let mut hooks = self.hooks.clone();
        if let Some(root) = hooks.as_object_mut() {
            drop_skald_hooks(root);
        }
        Read {
            config: emptied(config),
            hooks: emptied(hooks),
        }
    }

    /// Whether either file holds any of Skald's entries.
    pub fn has_skald(&self) -> bool {
        self.server().is_some()
            || [&self.config, &self.hooks].into_iter().any(|file| {
                (file.get("hooks").and_then(Value::as_object)).is_some_and(|events| {
                    events
                        .keys()
                        .any(|event| !self.skald_hooks(event).is_empty())
                })
            })
    }
}

/// Whether `hook` runs Skald's hook handler for Codex.
fn is_skalds(hook: &Value) -> bool {
    COMMANDS.iter().any(|field| {
        (hook.get(*field).and_then(Value::as_str))
            .is_some_and(|command| command.contains("skald") && command.contains("hook codex"))
    })
}

/// Drops Skald's hooks from `root`'s `hooks`, then each group, event and `hooks` table they
/// emptied.
fn drop_skald_hooks(root: &mut Map<String, Value>) {
    let Some(events) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    events.retain(|_, groups| {
        let Some(groups) = groups.as_array_mut() else {
            return true;
        };
        groups.retain_mut(|group| {
            let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let had = hooks.len();
            hooks.retain(|hook| !is_skalds(hook));
            had == 0 || !hooks.is_empty()
        });
        !groups.is_empty()
    });
    if events.is_empty() {
        root.remove("hooks");
    }
}

/// [`Value::Null`] for an empty object: a file with nothing in it counts as none.
fn emptied(value: Value) -> Value {
    match value {
        Value::Object(map) if map.is_empty() => Value::Null,
        other => other,
    }
}

/// Fails unless the lines of `before` are, in order, among those of `after`: each line of
/// `before`, comments included, stays as it was.
#[track_caller]
pub fn assert_lines_kept(before: &str, after: &str, what: &str) {
    let mut after_lines = after.lines();
    for line in before.lines() {
        assert!(
            after_lines.any(|kept| kept == line),
            "{what}: the line {line:?} is gone or moved:\n{after}"
        );
    }
}
