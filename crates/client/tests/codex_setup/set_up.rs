//! The setup on fixture config homes: Skald's entries are added and every other entry and comment
//! stays; a second run changes no byte; an older Skald's entries are replaced; a removal gives back
//! the fixture and takes out only Skald's entries; a config it cannot read stays as it is, and the
//! run says why; it follows `CODEX_HOME`, and `~/.codex` without it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{Value, json};

use crate::codex::{CODEX, CODEX_HOME};
use crate::config_home::{
    CONFIG, EVENTS, FIXTURES, Files, HOOKS, OLDER, OLDER_MARK, Read, assert_lines_kept, files,
    fixture_files, text, write,
};
use crate::machine::{Finished, Machine, Run};
use crate::{REMOVE, SETUP};

/// The approval mode under which Codex calls a server's tools without asking.
const APPROVE: &str = "approve";

/// A machine of its own whose config home, `codex-home` under its directory, holds `files`.
struct Home {
    machine: Machine,
    dir: PathBuf,
}

impl Home {
    fn new(files: &Files) -> Self {
        let machine = Machine::new();
        let dir = machine.root().join("codex-home");
        write(&dir, files);
        Home { machine, dir }
    }

    /// A config home that holds the fixture `name`.
    fn of(name: &str) -> Self {
        Home::new(&fixture_files(name))
    }

    /// Runs `skald <args>` on the machine with `CODEX_HOME` the config home.
    fn run(&self, args: &[&str]) -> Finished {
        let mut command = self.machine.command_in(self.machine.root(), args);
        command.env(CODEX_HOME, &self.dir).stdin(Stdio::null());
        Run::spawn(&mut command).finish()
    }

    /// `skald setup codex`; fails unless it succeeds.
    #[track_caller]
    fn set_up(&self, what: &str) {
        let done = self.run(&[SETUP, CODEX]);
        assert!(done.success, "{what}: the setup failed: {done:?}");
    }

    /// `skald setup codex --remove`; fails unless it succeeds.
    #[track_caller]
    fn remove(&self, what: &str) {
        let done = self.run(&[SETUP, CODEX, REMOVE]);
        assert!(done.success, "{what}: the removal failed: {done:?}");
    }

    fn files(&self) -> Files {
        files(&self.dir)
    }
}

/// Fails unless `read` holds Skald's entries, as the setup adds them: the MCP server `skald`, run
/// as `skald mcp`, whose tools Codex calls without asking, and exactly one hook of Skald's handler
/// for each of its events, in a group that takes every tool.
#[track_caller]
fn assert_set_up(read: &Read, what: &str) {
    let server =
        (read.server()).unwrap_or_else(|| panic!("{what}: no MCP server skald: {read:#?}"));
    assert_eq!(
        server["default_tools_approval_mode"].as_str(),
        Some(APPROVE),
        "{what}: Codex would ask to approve Skald's tools: {server}"
    );
    let (program, args) = read.server_command();
    let file = Path::new(&program)
        .file_stem()
        .and_then(|stem| stem.to_str());
    assert_eq!(file, Some("skald"), "{what}: the server runs {program}");
    assert_eq!(
        args.last().map(String::as_str),
        Some("mcp"),
        "{what}: {args:?}"
    );
    for event in EVENTS {
        read.hook_command(event);
    }
}

/// On each fixture: the setup succeeds; Skald's MCP server and hooks are there; without them both
/// files hold what the fixture held, other servers, hooks, trust and settings included; each line
/// of the fixture's `config.toml`, comments included, stays as it was; every other file stays byte
/// for byte. Of [`OLDER`], only its older Skald entries go.
#[test]
fn the_setup_adds_skald_and_keeps_every_other_entry() {
    for name in FIXTURES {
        let fixture = fixture_files(name);
        let home = Home::new(&fixture);

        home.set_up(name);

        let after = home.files();
        let read = Read::of(&after);
        assert_set_up(&read, name);
        assert_eq!(
            read.without_skald(),
            Read::of(&fixture).without_skald(),
            "{name}: other entries changed"
        );
        if name != OLDER
            && let Some(before) = text(&fixture, CONFIG)
        {
            let now = text(&after, CONFIG).expect("config.toml is still there");
            assert_lines_kept(before, now, name);
        }
        for (file, bytes) in &fixture {
            if file != Path::new(CONFIG) && file != Path::new(HOOKS) {
                assert_eq!(after.get(file), Some(bytes), "{name}: {file:?} changed");
            }
        }
    }
}

/// On each fixture, the setup run a second time succeeds and changes no byte in the config home, so
/// Codex asks for no new trust.
#[test]
fn a_second_run_changes_no_byte() {
    for name in FIXTURES {
        let home = Home::of(name);
        home.set_up(name);
        let once = home.files();

        home.set_up(name);

        assert_eq!(
            home.files(),
            once,
            "{name}: the second run changed the config home"
        );
    }
}

/// An older Skald set up its server without the approval mode and its hooks on two events, by
/// another path and timeout, beside Paseo's hooks. The setup replaces them: no file holds the older
/// path; each event has one Skald hook; Paseo's hooks and the other server stay.
#[test]
fn an_older_skald_entry_is_replaced() {
    let fixture = fixture_files(OLDER);
    let home = Home::new(&fixture);

    home.set_up(OLDER);

    let after = home.files();
    for (file, bytes) in &after {
        let held = String::from_utf8_lossy(bytes);
        assert!(
            !held.contains(OLDER_MARK),
            "{file:?} keeps the older entry:\n{held}"
        );
    }
    let read = Read::of(&after);
    assert_set_up(&read, OLDER);
    assert_eq!(read.without_skald(), Read::of(&fixture).without_skald());
}

/// On each fixture but [`OLDER`]: a removal with nothing set up changes no byte; after the setup, a
/// removal gives back the fixture byte for byte, with no file more or less.
#[test]
fn a_removal_gives_back_the_fixture() {
    for name in FIXTURES.into_iter().filter(|name| *name != OLDER) {
        let fixture = fixture_files(name);
        let home = Home::new(&fixture);
        home.remove(&format!("{name}, with nothing set up"));
        assert_eq!(
            home.files(),
            fixture,
            "{name}: a removal with nothing set up changed it"
        );

        home.set_up(name);
        home.remove(name);

        assert_eq!(
            home.files(),
            fixture,
            "{name}: the removal did not give back the fixture"
        );
    }
}

/// The hooks Paseo adds to `hooks.json` for its events, as it writes them.
fn paseo_groups() -> Vec<(&'static str, Value)> {
    ["UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop"]
        .into_iter()
        .map(|event| {
            let command = format!(
                "if [ -n \"$PASEO_TERMINAL_ID\" ]; then \"${{PASEO_HOOK_CLI:-paseo}}\" hooks codex {event}; fi"
            );
            let group = json!({
                "matcher": "",
                "hooks": [{ "type": "command", "command": command, "timeout": 10 }],
            });
            (event, group)
        })
        .collect()
}

/// After the setup in an empty config home, Paseo adds its hooks to `hooks.json` and the person
/// adds a setting to `config.toml`. The removal takes out only Skald's entries: Paseo's hooks and
/// the setting stay; no Skald entry is left.
#[test]
fn a_removal_takes_out_only_skalds_entries() {
    let home = Home::new(&Files::new());
    home.set_up("an empty config home");
    let hooks_file = home.dir.join(HOOKS);
    let mut hooks: Value = fs::read_to_string(&hooks_file)
        .ok()
        .map_or(json!({}), |text| {
            serde_json::from_str(&text).expect("hooks.json is JSON")
        });
    if hooks.get("hooks").is_none() {
        hooks["hooks"] = json!({});
    }
    for (event, group) in paseo_groups() {
        let groups = hooks["hooks"]
            .as_object_mut()
            .expect("hooks is an object")
            .entry(event)
            .or_insert_with(|| json!([]));
        groups.as_array_mut().expect("a list of groups").push(group);
    }
    let pretty = serde_json::to_string_pretty(&hooks).expect("JSON");
    fs::write(&hooks_file, format!("{pretty}\n")).expect("write hooks.json as Paseo does");
    let config_file = home.dir.join(CONFIG);
    let mut config = fs::read_to_string(&config_file).unwrap_or_default();
    config.push_str("\n# Set by the person after the setup.\nmodel_verbosity = \"low\"\n");
    fs::write(&config_file, &config).expect("write config.toml");

    home.remove("after Paseo and the person");

    let read = Read::home(&home.dir);
    assert!(!read.has_skald(), "a Skald entry is left: {read:#?}");
    assert_eq!(
        read.config["model_verbosity"].as_str(),
        Some("low"),
        "{read:#?}"
    );
    for (event, group) in paseo_groups() {
        assert_eq!(read.groups(event), vec![&group], "Paseo's hook of {event}");
    }
    let config = fs::read_to_string(&config_file).expect("config.toml is still there");
    assert!(
        config.contains("# Set by the person after the setup."),
        "{config}"
    );
}

/// Fails unless `done`, a run on a config home of which `file` cannot be read, failed and named
/// `file`, and the config home still holds `before`, byte for byte.
#[track_caller]
fn assert_refused(done: &Finished, file: &str, home: &Home, before: &Files, what: &str) {
    assert!(!done.success, "{what}: it succeeded: {done:?}");
    assert!(
        done.stderr.contains(file),
        "{what}: it does not say {file}: {done:?}"
    );
    assert_eq!(&home.files(), before, "{what}: the config home changed");
}

/// With a `config.toml` that is not TOML, then with a `hooks.json` that is not JSON: the setup and
/// the removal each fail, say which file they cannot read, and change no byte of the config home.
#[test]
fn a_config_it_cannot_read_stays_unchanged_and_it_says_why() {
    for (name, file) in [("broken-toml", CONFIG), ("broken-json", HOOKS)] {
        let fixture = fixture_files(name);
        let home = Home::new(&fixture);
        for args in [&[SETUP, CODEX][..], &[SETUP, CODEX, REMOVE]] {
            let done = home.run(args);
            assert_refused(&done, file, &home, &fixture, &format!("{name}: {args:?}"));
        }
    }
}

/// With a `config.toml` this OS account may not read: the setup fails, says which file, and
/// changes no byte of the config home.
#[cfg(unix)]
#[test]
fn a_config_this_account_cannot_read_stays_unchanged() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = fixture_files("others");
    let home = Home::new(&fixture);
    let config = home.dir.join(CONFIG);
    fs::set_permissions(&config, fs::Permissions::from_mode(0o000)).expect("make it unreadable");
    if fs::read(&config).is_ok() {
        // Run as root, which reads it anyway: nothing to check.
        return;
    }

    let done = home.run(&[SETUP, CODEX]);

    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).expect("make it readable");
    assert_refused(&done, CONFIG, &home, &fixture, "unreadable");
}

/// With `CODEX_HOME` naming a config home, the setup writes there and nothing into `~/.codex`;
/// without it, it sets Skald up in `~/.codex`, which it makes if Codex has not yet.
#[test]
fn it_follows_codex_home_and_takes_the_home_folder_without_it() {
    let home = Home::new(&Files::new());
    let default = home.machine.root().join("home").join(".codex");

    home.set_up("with CODEX_HOME");

    assert_set_up(&Read::home(&home.dir), "CODEX_HOME");
    assert!(
        files(&default).is_empty(),
        "it wrote into ~/.codex: {:?}",
        files(&default)
    );

    let mut command = home
        .machine
        .command_in(home.machine.root(), &[SETUP, CODEX]);
    command.stdin(Stdio::null());
    let done = Run::spawn(&mut command).finish();
    assert!(done.success, "without CODEX_HOME: {done:?}");
    assert_set_up(&Read::home(&default), "~/.codex");
}
