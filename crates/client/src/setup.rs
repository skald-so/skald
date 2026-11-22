//! `skald setup codex`: Skald's entries in the config home Codex uses, [`CODEX_HOME`] or
//! `~/.codex` (`docs/agent-sessions.md`, "What you need to do").
//!
//! - Skald's entries, all in `config.toml`: the MCP server [`SERVER`], this `skald` run as
//!   `skald mcp`, whose tools Codex calls without asking; and a group of one command hook running
//!   `skald hook codex` for each event of [`EVENTS`].
//! - It edits `config.toml` keeping every other entry, comment and line as it was, and adds each
//!   group after the event's other groups, so the trust Codex keeps for another hook, by its
//!   position, stays. Run again, it changes no byte, so Codex asks for no new trust; it replaces
//!   Skald's entries that differ, as an older `skald` wrote them, in `hooks.json` too.
//! - `--remove` takes out Skald's entries and nothing else; a file that held only them goes.
//! - It reads both files before it writes either: one it cannot read, or whose shape it cannot
//!   edit, stays as it is, and the run fails, naming it.
//!
//! A hook is Skald's when its command runs a `skald` with `hook codex` and nothing more
//! ([`is_skalds`]); a hook with `--agent` is the person's own.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike};

use crate::hook::CODEX_HOME;

/// The agent CLIs `skald setup` sets Skald up in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
pub enum Cli {
    /// Codex.
    Codex,
}

/// Codex's config file and hooks file in its config home.
const CONFIG: &str = "config.toml";
const HOOKS: &str = "hooks.json";

/// The name of Skald's MCP server in `config.toml`.
const SERVER: &str = "skald";

/// The events whose hooks run Skald's handler for Codex ([`crate::hook::Cli::handles`]).
pub const EVENTS: [&str; 5] = [
    "SessionStart",
    "PostToolUse",
    "UserPromptSubmit",
    "Stop",
    "PermissionRequest",
];

/// The variables Codex gives Skald's MCP server besides its own defaults, such as `HOME` and
/// `PATH`: those `skald mcp` reads, for its state directory and key storage, the Secret Service's
/// session bus on Linux, one-shot mode, the proxy (`docs/dev/architecture.md`, "Network") and the
/// root certificates ([`crate::tls`]).
pub const SERVER_VARIABLES: [&str; 12] = [
    "SKALD_STATE_DIR",
    "SKALD_KEY_STORAGE",
    "SKALD_ONE_SHOT",
    "XDG_STATE_HOME",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "HTTPS_PROXY",
    "https_proxy",
    "NO_PROXY",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// How long Codex lets Skald's hook run, in seconds: the handler gives up after
/// [`crate::codex::PATIENCE`].
const HOOK_TIMEOUT: i64 = 10;

/// The approval mode under which Codex calls a server's tools without asking.
const APPROVE: &str = "approve";

/// `skald setup <cli>`, or with `remove`, `skald setup <cli> --remove`.
pub fn run(cli: Cli, remove: bool) -> Result<(), String> {
    match cli {
        Cli::Codex => codex(remove),
    }
}

/// Sets Skald up in Codex's config home, or removes its entries.
fn codex(remove: bool) -> Result<(), String> {
    let home = codex_home()?;
    let exe = crate::helper::installed()
        .map(|exe| crate::plain_path::plain(&exe))
        .ok_or("cannot tell where this skald is installed")?;
    let exe = exe
        .to_str()
        .ok_or_else(|| format!("the path of this skald, {}, is not UTF-8", exe.display()))?;
    let config = home.join(CONFIG);
    let hooks = home.join(HOOKS);
    let read = Files {
        config: read(&config)?,
        hooks: read(&hooks)?,
    };
    let wanted = (!remove).then(|| Entries::of(exe, cfg!(windows)));
    let changes = edit(&read, wanted.as_ref(), (&config, &hooks))?;
    if changes.is_empty() {
        println!(
            "{} {}",
            if remove {
                "Codex's config holds no Skald entries:"
            } else {
                "Skald is already set up in Codex:"
            },
            config.display()
        );
        return Ok(());
    }
    if !remove {
        // On Unix readable only by its owner, as it holds Codex's sign-in.
        crate::state::create_dir(&home)
            .map_err(|err| format!("cannot create {}: {err}", home.display()))?;
    }
    for (path, change) in [(&config, changes.config), (&hooks, changes.hooks)] {
        let done = match change {
            Change::Keep => Ok(()),
            Change::Write(text) => fs::write(path, text),
            Change::Delete => fs::remove_file(path),
        };
        done.map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    }
    if remove {
        println!(
            "Skald's entries removed from Codex's config: {}",
            home.display()
        );
    } else {
        println!(
            "Skald is set up in Codex: {}. Trust its hooks in Codex: /hooks in the terminal, \
             Review hooks in the app and VS Code.",
            config.display()
        );
    }
    Ok(())
}

/// The config home Codex uses: [`CODEX_HOME`], else `.codex` in the home folder.
fn codex_home() -> Result<PathBuf, String> {
    let var = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(home) = var(CODEX_HOME) {
        return Ok(PathBuf::from(home));
    }
    #[cfg(windows)]
    let home = var("USERPROFILE");
    #[cfg(not(windows))]
    let home = var("HOME");
    home.map(|home| PathBuf::from(home).join(".codex"))
        .ok_or_else(|| format!("no home folder: set {CODEX_HOME} to Codex's config home"))
}

/// The text of the file at `path`; `None` if there is none.
fn read(path: &Path) -> Result<Option<String>, String> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("cannot read {}: it is not UTF-8", path.display())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("cannot read {}: {err}", path.display())),
    }
}

/// The texts of `config.toml` and `hooks.json`, each if there.
#[derive(Debug, Default)]
struct Files {
    config: Option<String>,
    hooks: Option<String>,
}

/// What a run does to a file.
#[derive(Debug, PartialEq)]
enum Change {
    Keep,
    Write(String),
    Delete,
}

/// What a run does to `config.toml` and `hooks.json`.
#[derive(Debug, PartialEq)]
struct Changes {
    config: Change,
    hooks: Change,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.config == Change::Keep && self.hooks == Change::Keep
    }
}

/// Skald's entries, as JSON values: its MCP server's table and the group of its hook.
#[derive(Debug)]
struct Entries {
    server: Value,
    group: Value,
}

impl Entries {
    /// The entries of the `skald` at `exe`, quoted for Windows' shell if `windows`, else for `sh`.
    fn of(exe: &str, windows: bool) -> Self {
        let command = format!("{} hook codex", quoted(exe, windows));
        Entries {
            server: json!({
                "command": exe,
                "args": ["mcp"],
                "env_vars": SERVER_VARIABLES,
                "default_tools_approval_mode": APPROVE,
            }),
            group: json!({
                "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT }],
            }),
        }
    }
}

/// `path` as one word of a command line: in double quotes for Windows' shell; for `sh`, as it is
/// if nothing in it is special, else in single quotes.
fn quoted(path: &str, windows: bool) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@%".contains(c);
    if windows {
        format!("\"{path}\"")
    } else if !path.is_empty() && path.chars().all(plain) {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}

/// Whether `command`, a hook's command line, runs Skald's hook handler for Codex: a program whose
/// file is `skald`, alone or in quotes, then `hook codex` and nothing more.
fn is_skalds(command: &str) -> bool {
    let Some(program) = command.trim().strip_suffix(" hook codex") else {
        return false;
    };
    let program = program.trim_end();
    let unquoted = ['\'', '"']
        .into_iter()
        .find_map(|quote| program.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(program);
    if unquoted.is_empty() || (unquoted == program && program.contains(char::is_whitespace)) {
        return false;
    }
    let file = unquoted.rsplit(['/', '\\']).next().unwrap_or(unquoted);
    let file = file.to_ascii_lowercase();
    file.strip_suffix(".exe").unwrap_or(&file) == SERVER
}

/// The fields of a hook that hold a command line.
const COMMANDS: [&str; 3] = ["command", "commandWindows", "command_windows"];

/// Whether `hook`, a hook's table, runs Skald's handler.
fn hook_is_skalds(hook: &dyn TableLike) -> bool {
    COMMANDS.iter().any(|field| {
        hook.get(field)
            .and_then(Item::as_str)
            .is_some_and(is_skalds)
    })
}

/// Whether `hook`, a hook of `hooks.json`, runs Skald's handler.
fn json_hook_is_skalds(hook: &Value) -> bool {
    COMMANDS.iter().any(|field| {
        hook.get(field)
            .and_then(Value::as_str)
            .is_some_and(is_skalds)
    })
}

/// What a run on `files` changes: with `wanted`, Skald's entries set up as they are; without,
/// Skald's entries removed. `paths`, of `config.toml` and `hooks.json`, name a file it cannot read
/// or edit.
fn edit(files: &Files, wanted: Option<&Entries>, paths: (&Path, &Path)) -> Result<Changes, String> {
    let (config_path, hooks_path) = paths;
    let refused = |path: &Path, why: &dyn std::fmt::Display| {
        format!("cannot read {}: {why}; nothing changed", path.display())
    };
    let text = files.config.as_deref().unwrap_or_default();
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|err: toml_edit::TomlError| refused(config_path, &err.message().trim_end()))?;
    let config: Value = toml::from_str::<toml::Table>(text)
        .ok()
        .and_then(|table| serde_json::to_value(table).ok())
        .ok_or_else(|| refused(config_path, &"it is not TOML"))?;
    let mut hooks: Option<Value> = (files.hooks.as_deref())
        .map(serde_json::from_str)
        .transpose()
        .map_err(|err| refused(hooks_path, &err))?;

    let older_in_hooks = hooks.as_ref().is_some_and(json_has_skalds);
    let set_up = wanted.is_some_and(|wanted| {
        config
            .get("mcp_servers")
            .and_then(|servers| servers.get(SERVER))
            == Some(&wanted.server)
            && skald_groups(&config) == EVENTS.map(|event| (event, vec![&wanted.group]))
    });
    let config_change = match wanted {
        Some(_) if set_up && !toml_has_other_skalds(&config) => Change::Keep,
        _ => {
            remove_from_toml(&mut doc).map_err(|why| refused(config_path, &why))?;
            if let Some(wanted) = wanted {
                add_to_toml(&mut doc, wanted).map_err(|why| refused(config_path, &why))?;
            }
            let edited = doc.to_string();
            if edited == text {
                Change::Keep
            } else if edited.trim().is_empty() {
                Change::Delete
            } else {
                Change::Write(edited)
            }
        }
    };
    let hooks_change = match hooks.as_mut() {
        Some(hooks) if older_in_hooks => {
            remove_from_json(hooks);
            if hooks.as_object().is_some_and(serde_json::Map::is_empty) {
                Change::Delete
            } else {
                let pretty = serde_json::to_string_pretty(hooks).map_err(|err| err.to_string())?;
                Change::Write(format!("{pretty}\n"))
            }
        }
        _ => Change::Keep,
    };
    Ok(Changes {
        config: config_change,
        hooks: hooks_change,
    })
}

/// For each event of [`EVENTS`], the groups of `config`, `config.toml` as JSON, that hold a hook
/// of Skald's.
fn skald_groups(config: &Value) -> [(&'static str, Vec<&Value>); EVENTS.len()] {
    EVENTS.map(|event| {
        let groups = (config.get("hooks").and_then(|hooks| hooks.get(event)))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        (
            event,
            groups
                .iter()
                .filter(|group| group_has_skalds(group))
                .collect(),
        )
    })
}

/// Whether `group`, a group of hooks as JSON, holds a hook of Skald's.
fn group_has_skalds(group: &Value) -> bool {
    (group.get("hooks").and_then(Value::as_array))
        .is_some_and(|hooks| hooks.iter().any(json_hook_is_skalds))
}

/// Whether `config`, `config.toml` as JSON, holds a hook of Skald's on an event not in [`EVENTS`].
fn toml_has_other_skalds(config: &Value) -> bool {
    (config.get("hooks").and_then(Value::as_object)).is_some_and(|events| {
        events.iter().any(|(event, groups)| {
            !EVENTS.contains(&event.as_str())
                && (groups.as_array()).is_some_and(|groups| groups.iter().any(group_has_skalds))
        })
    })
}

/// Whether `hooks`, `hooks.json`, holds a hook of Skald's.
fn json_has_skalds(hooks: &Value) -> bool {
    (hooks.get("hooks").and_then(Value::as_object)).is_some_and(|events| {
        (events.values().filter_map(Value::as_array))
            .any(|groups| groups.iter().any(group_has_skalds))
    })
}

/// Keeps the tables of `item`, an array of tables or an inline array, for which `keep` says so;
/// anything else in it stays. Gives how many tables it held and how many it keeps.
fn retain_tables(
    item: &mut Item,
    mut keep: impl FnMut(&mut dyn TableLike) -> bool,
) -> (usize, usize) {
    match item {
        Item::ArrayOfTables(tables) => {
            let had = tables.len();
            for index in (0..had).rev() {
                if tables.get_mut(index).is_some_and(|table| !keep(table)) {
                    tables.remove(index);
                }
            }
            (had, tables.len())
        }
        Item::Value(toml_edit::Value::Array(values)) => {
            let (mut had, mut kept) = (0, 0);
            for index in (0..values.len()).rev() {
                if let Some(toml_edit::Value::InlineTable(table)) = values.get_mut(index) {
                    had += 1;
                    if keep(table) {
                        kept += 1;
                    } else {
                        values.remove(index);
                    }
                }
            }
            (had, kept)
        }
        _ => (0, 0),
    }
}

/// Whether `item`, once empty, goes: an inline table does; a table stays, as one the person wrote a
/// header for keeps it, and one of dotted keys or with no header of its own shows nothing.
fn goes_when_empty(item: &Item) -> bool {
    matches!(item, Item::Value(_))
}

/// Takes Skald's entries out of `doc`, `config.toml`, and each table and list that held only them.
fn remove_from_toml(doc: &mut DocumentMut) -> Result<(), String> {
    if let Some(servers) = doc.get_mut("mcp_servers")
        && let Some(table) = servers.as_table_like_mut()
    {
        table.remove(SERVER);
        if table.is_empty() && goes_when_empty(servers) {
            doc.remove("mcp_servers");
        }
    }
    let Some(hooks) = doc.get_mut("hooks") else {
        return Ok(());
    };
    let Some(events) = hooks.as_table_like_mut() else {
        return Err("hooks is not a table".to_owned());
    };
    let mut emptied = Vec::new();
    for (event, groups) in events.iter_mut() {
        let (had, kept) = retain_tables(groups, |group| {
            let Some(inner) = group.get_mut("hooks") else {
                return true;
            };
            let (had, kept) = retain_tables(inner, |hook| !hook_is_skalds(hook));
            had == 0 || kept > 0
        });
        if had > 0 && kept == 0 {
            emptied.push(event.get().to_owned());
        }
    }
    for event in emptied {
        events.remove(&event);
    }
    if events.is_empty() && goes_when_empty(hooks) {
        doc.remove("hooks");
    }
    Ok(())
}

/// Adds Skald's entries, `wanted`, to `doc`, `config.toml`: its server into `mcp_servers`, and its
/// group after each event's other groups. Where the file has no table of them, each goes in as a
/// dotted key of the root, before any table, so a key the person adds at the file's end stays at
/// its root.
fn add_to_toml(doc: &mut DocumentMut, wanted: &Entries) -> Result<(), String> {
    let servers = doc.entry("mcp_servers").or_insert_with(dotted);
    match servers {
        Item::Table(table) if table.is_dotted() => {
            table.insert(SERVER, toml_edit::value(inline_of(&wanted.server)));
        }
        Item::Table(table) => {
            table.insert(SERVER, Item::Table(table_of(&wanted.server)));
        }
        Item::Value(toml_edit::Value::InlineTable(table)) => {
            table.insert(SERVER, inline_of(&wanted.server).into());
        }
        _ => return Err("mcp_servers is not a table".to_owned()),
    }
    let hooks = doc.entry("hooks").or_insert_with(dotted);
    let headers = matches!(hooks, Item::Table(table) if !table.is_dotted());
    let Some(events) = hooks.as_table_like_mut() else {
        return Err("hooks is not a table".to_owned());
    };
    for event in EVENTS {
        let groups = events.entry(event).or_insert_with(|| {
            if headers {
                Item::ArrayOfTables(ArrayOfTables::new())
            } else {
                toml_edit::value(Array::new())
            }
        });
        match groups {
            Item::ArrayOfTables(groups) => groups.push(table_of(&wanted.group)),
            Item::Value(toml_edit::Value::Array(groups)) => groups.push(inline_of(&wanted.group)),
            _ => return Err(format!("hooks.{event} is not a list")),
        }
    }
    Ok(())
}

/// A table of dotted keys of the root, as `a.b = 1` makes `a`.
fn dotted() -> Item {
    let mut table = Table::new();
    table.set_dotted(true);
    Item::Table(table)
}

/// The order of the keys of Skald's entries.
const KEYS: [&str; 7] = [
    "type",
    "command",
    "args",
    "env_vars",
    "default_tools_approval_mode",
    "timeout",
    "hooks",
];

/// The keys and values of `value`, a JSON object, in the order of [`KEYS`].
fn ordered(value: &Value) -> Vec<(&str, &Value)> {
    let mut pairs: Vec<_> = (value.as_object().into_iter().flatten())
        .map(|(key, value)| (key.as_str(), value))
        .collect();
    pairs.sort_by_key(|(key, _)| KEYS.iter().position(|known| known == key));
    pairs
}

/// `value`, a JSON object of strings, integers and lists of them or of such objects, as a table.
fn table_of(value: &Value) -> Table {
    let mut table = Table::new();
    for (key, value) in ordered(value) {
        table.insert(key, Item::Value(value_of(value)));
    }
    table
}

/// `value`, as [`table_of`] takes it, as an inline table.
fn inline_of(value: &Value) -> InlineTable {
    let mut table = InlineTable::new();
    for (key, value) in ordered(value) {
        table.insert(key, value_of(value));
    }
    table
}

/// `value`, a string, an integer, or a list or an object of them, as a TOML value.
fn value_of(value: &Value) -> toml_edit::Value {
    match value {
        Value::String(text) => text.as_str().into(),
        Value::Number(number) => number.as_i64().unwrap_or_default().into(),
        Value::Array(values) => values.iter().map(value_of).collect::<Array>().into(),
        Value::Object(_) => inline_of(value).into(),
        Value::Bool(yes) => (*yes).into(),
        Value::Null => "".into(),
    }
}

/// Takes Skald's hooks out of `hooks`, `hooks.json`, and each group, event and table that held
/// only them.
fn remove_from_json(hooks: &mut Value) {
    let Some(events) = hooks.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    let emptied: Vec<String> = (events.iter_mut())
        .filter_map(|(event, groups)| {
            let groups = groups.as_array_mut()?;
            let had = groups.len();
            *groups = groups.drain(..).filter_map(without_skalds).collect();
            (had > 0 && groups.is_empty()).then(|| event.clone())
        })
        .collect();
    for event in emptied {
        events.remove(&event);
    }
    if events.is_empty()
        && let Some(root) = hooks.as_object_mut()
    {
        root.remove("hooks");
    }
}

/// `group`, a group of `hooks.json`, without Skald's hooks; none if it held only them.
fn without_skalds(mut group: Value) -> Option<Value> {
    if let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) {
        let had = list.len();
        list.retain(|hook| !json_hook_is_skalds(hook));
        if had > 0 && list.is_empty() {
            return None;
        }
    }
    Some(group)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/home/ada/.local/bin/skald";

    fn paths() -> (&'static Path, &'static Path) {
        (Path::new("/c/config.toml"), Path::new("/c/hooks.json"))
    }

    fn files(config: Option<&str>, hooks: Option<&str>) -> Files {
        Files {
            config: config.map(str::to_owned),
            hooks: hooks.map(str::to_owned),
        }
    }

    fn set_up(files: &Files) -> Changes {
        edit(files, Some(&Entries::of(EXE, false)), paths()).expect("it edits")
    }

    fn removed(files: &Files) -> Changes {
        edit(files, None, paths()).expect("it edits")
    }

    fn parsed(text: &str) -> toml::Table {
        toml::from_str(text).unwrap_or_else(|err| panic!("not TOML ({err}):\n{text}"))
    }

    fn groups<'t>(table: &'t toml::Table, event: &str) -> &'t Vec<toml::Value> {
        (table["hooks"][event].as_array()).unwrap_or_else(|| panic!("no groups of {event}"))
    }

    fn written(change: Change) -> String {
        match change {
            Change::Write(text) => text,
            other => panic!("not written: {other:?}"),
        }
    }

    #[test]
    fn a_path_is_quoted_only_when_the_shell_needs_it() {
        assert_eq!(quoted(EXE, false), EXE);
        assert_eq!(quoted("/Users/a b/skald", false), "'/Users/a b/skald'");
        assert_eq!(quoted("/it's/skald", false), r"'/it'\''s/skald'");
        assert_eq!(quoted("", false), "''");
        assert_eq!(
            quoted(r"C:\Users\ada\skald.exe", true),
            r#""C:\Users\ada\skald.exe""#
        );
    }

    #[test]
    fn a_hook_is_skalds_when_it_runs_skald_hook_codex_and_nothing_more() {
        for command in [
            "skald hook codex",
            "/opt/skald-0.1/bin/skald hook codex",
            "'/Users/a b/skald' hook codex",
            r"'/it'\''s/skald' hook codex",
            r#""C:\Program Files\Skald\skald.exe" hook codex"#,
            r"C:\skald\SKALD.EXE hook codex",
            "  skald hook codex  ",
        ] {
            assert!(is_skalds(command), "{command}");
        }
        for command in [
            "skald hook claude",
            "skald --agent bob hook codex",
            "skald hook codex --json",
            "env A=1 skald hook codex",
            "env /opt/skald hook codex",
            "/opt/skaldx hook codex",
            "/opt/skald/run hook codex",
            "'' hook codex",
            " hook codex",
            "paseo hooks codex Stop",
        ] {
            assert!(!is_skalds(command), "{command}");
        }
    }

    #[test]
    fn the_setup_of_an_empty_config_home_writes_only_config_toml() {
        let changes = set_up(&files(None, None));
        assert_eq!(changes.hooks, Change::Keep);
        let text = written(changes.config);
        let expected = format!(
            "mcp_servers.skald = {{ command = \"{EXE}\", args = [\"mcp\"], env_vars = [{}], \
             default_tools_approval_mode = \"approve\" }}\n",
            SERVER_VARIABLES
                .map(|name| format!("\"{name}\""))
                .join(", ")
        );
        assert!(text.starts_with(&expected), "{text}");
        for event in EVENTS {
            let group = format!(
                "\nhooks.{event} = [{{ hooks = [{{ type = \"command\", command = \"{EXE} hook codex\", timeout = 10 }}] }}]\n"
            );
            assert!(text.contains(&group), "{event}:\n{text}");
        }
        assert_eq!(
            set_up(&files(Some(&text), None)),
            Changes {
                config: Change::Keep,
                hooks: Change::Keep
            }
        );
        assert_eq!(removed(&files(Some(&text), None)).config, Change::Delete);
    }

    #[test]
    fn its_group_goes_after_the_events_other_groups() {
        let config = "[[hooks.Stop]]\nhooks = [{ type = \"command\", command = \"say done\" }]\n";
        let text = written(set_up(&files(Some(config), None)).config);
        let table = parsed(&text);
        let stop = groups(&table, "Stop");
        assert_eq!(stop.len(), 2, "{text}");
        assert_eq!(stop[0]["hooks"][0]["command"].as_str(), Some("say done"));
        assert_eq!(
            removed(&files(Some(&text), None)).config,
            Change::Write(config.to_owned())
        );
    }

    #[test]
    fn inline_tables_take_inline_entries() {
        let config = "mcp_servers = { docs = { command = \"npx\" } }\n\
                      hooks = { Stop = [{ hooks = [{ type = \"command\", command = \"say done\" }] }] }\n";
        let text = written(set_up(&files(Some(config), None)).config);
        let table = parsed(&text);
        assert_eq!(
            table["mcp_servers"]["skald"]["args"][0].as_str(),
            Some("mcp")
        );
        assert_eq!(
            table["mcp_servers"]["docs"]["command"].as_str(),
            Some("npx")
        );
        for event in EVENTS {
            let last = groups(&table, event).last().expect("a group");
            assert_eq!(
                last["hooks"][0]["command"].as_str(),
                Some(&*format!("{EXE} hook codex"))
            );
        }
        assert_eq!(groups(&table, "Stop").len(), 2);
        let back = written(removed(&files(Some(&text), None)).config);
        assert_eq!(parsed(&back), parsed(config));
    }

    #[test]
    fn a_skald_hook_beside_others_in_a_group_goes_alone() {
        let config = "[[hooks.Stop]]\nhooks = [{ type = \"command\", command = \"say done\" }, \
                      { type = \"command\", command = \"skald hook codex\" }]\n\
                      [[hooks.Notify]]\nhooks = [{ type = \"command\", command = \"skald hook codex\" }]\n";
        let text = written(removed(&files(Some(config), None)).config);
        let table = parsed(&text);
        let stop = groups(&table, "Stop");
        assert_eq!(stop.len(), 1, "{text}");
        assert_eq!(stop[0]["hooks"].as_array().map(Vec::len), Some(1), "{text}");
        assert!(table["hooks"].get("Notify").is_none(), "{text}");
        let again = written(set_up(&files(Some(config), None)).config);
        let table = parsed(&again);
        assert!(table["hooks"].get("Notify").is_none(), "{again}");
    }

    #[test]
    fn a_skald_hook_on_another_event_goes() {
        let other = "\n[[hooks.Notify]]\nhooks = [{ type = \"command\", command = \"skald hook codex\" }]\n";
        let set = written(set_up(&files(None, None)).config);
        let text = written(set_up(&files(Some(&format!("{set}{other}")), None)).config);
        assert!(parsed(&text)["hooks"].get("Notify").is_none(), "{text}");
        assert_eq!(removed(&files(Some(other), None)).config, Change::Delete);
    }

    #[test]
    fn a_changed_entry_is_replaced() {
        let set = written(set_up(&files(None, None)).config);
        let older = set.replace(" timeout = 10 ", " timeout = 30 ");
        let text = written(set_up(&files(Some(&older), None)).config);
        for event in EVENTS {
            assert_eq!(groups(&parsed(&text), event).len(), 1, "{text}");
        }
        assert!(!text.contains("timeout = 30"), "{text}");
        let older = set.replace("[\"mcp\"]", "[\"mcp\", \"--old\"]");
        let text = written(set_up(&files(Some(&older), None)).config);
        assert!(!text.contains("--old"), "{text}");
    }

    #[test]
    fn an_inline_table_that_held_only_skalds_entries_goes() {
        let config = "model = \"m\"\nmcp_servers = { skald = { command = \"skald\" } }\n\
                      hooks = { Stop = [{ hooks = [{ command = \"skald hook codex\" }] }] }\n";
        let text = written(removed(&files(Some(config), None)).config);
        assert_eq!(text, "model = \"m\"\n");
    }

    #[test]
    fn changes_are_empty_only_when_both_files_are_kept() {
        let changes = |config, hooks| Changes { config, hooks }.is_empty();
        assert!(changes(Change::Keep, Change::Keep));
        assert!(!changes(Change::Delete, Change::Keep));
        assert!(!changes(Change::Keep, Change::Write(String::new())));
    }

    #[test]
    fn a_file_it_cannot_read_is_named() {
        let dir = crate::test_dir::TestDir::new("setup-read");
        assert_eq!(read(&dir.join(CONFIG)), Ok(None));
        let err = read(&dir).expect_err("a directory is no file");
        assert!(err.contains(&dir.display().to_string()), "{err}");
        let file = dir.join(HOOKS);
        fs::write(&file, b"\xff").expect("write");
        let err = read(&file).expect_err("not UTF-8");
        assert!(err.contains(HOOKS), "{err}");
    }

    #[test]
    fn its_entries_set_up_anywhere_change_nothing() {
        let config = "[[hooks.Stop]]\nhooks = [{ type = \"command\", command = \"say done\" }]\n";
        let set = written(set_up(&files(Some(config), None)).config);
        let moved = format!(
            "{set}\n[[hooks.Stop]]\nhooks = [{{ type = \"command\", command = \"say again\" }}]\n"
        );
        assert_eq!(set_up(&files(Some(&moved), None)).config, Change::Keep);
    }

    #[test]
    fn a_header_the_person_wrote_stays() {
        let config = "[mcp_servers]\n\n[hooks]\n";
        let text = written(set_up(&files(Some(config), None)).config);
        assert_eq!(
            removed(&files(Some(&text), None)).config,
            Change::Write(config.to_owned())
        );
    }

    #[test]
    fn a_shape_it_cannot_edit_is_refused() {
        for config in ["mcp_servers = 1\n", "hooks = 1\n", "hooks.Stop = 1\n"] {
            let err = edit(
                &files(Some(config), None),
                Some(&Entries::of(EXE, false)),
                paths(),
            )
            .expect_err(config);
            assert!(err.contains("/c/config.toml"), "{err}");
        }
    }

    #[test]
    fn hooks_json_loses_only_skalds_hooks() {
        let hooks = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"skald hook codex"}]},
            {"hooks":[{"type":"command","command":"say done"}]}],
            "SessionStart":[{"hooks":[{"type":"command","command":"skald hook codex"}]}],
            "PreToolUse":[], "PostToolUse":[{"hooks":[]}]}}"#;
        let text = written(removed(&files(None, Some(hooks))).hooks);
        let value: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(
            value,
            json!({"hooks": {
                "Stop": [{"hooks": [{"type": "command", "command": "say done"}]}],
                "PreToolUse": [],
                "PostToolUse": [{"hooks": []}],
            }})
        );
        let only = r#"{"hooks":{"Stop":[{"hooks":[{"command":"skald hook codex"}]}]}}"#;
        assert_eq!(removed(&files(None, Some(only))).hooks, Change::Delete);
        let other = r#"{"hooks":{"Stop":[{"hooks":[{"command":"say done"}]}]}}"#;
        assert_eq!(removed(&files(None, Some(other))).hooks, Change::Keep);
    }
}
