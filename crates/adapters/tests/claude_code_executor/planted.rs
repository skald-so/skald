//! The user's config, planted in a config home where Claude Code reads it: an MCP server, hooks,
//! a plugin with a hook and a skill, a skill, an instruction file and the folder's project memory.
//! The MCP server and the hooks are a witness server's URLs, so each that loads leaves a request
//! there; the skills, the instructions and the memory each hold a token of their own.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::fixtures::token;

/// The witness's path the MCP server, the hooks and the plugin's hook each request.
pub const MCP_PATH: &str = "/planted-mcp";
pub const HOOK_PATH: &str = "/planted-hook";
pub const PLUGIN_HOOK_PATH: &str = "/planted-plugin-hook";

/// The events the planted hooks run on.
const EVENTS: [&str; 5] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
];

/// The tokens of what was planted.
pub struct Planted {
    pub skill: String,
    pub plugin_skill: String,
    pub instructions: String,
    pub memory: String,
}

impl Planted {
    /// Each token with what holds it.
    pub fn tokens(&self) -> [(&'static str, &str); 4] {
        [
            ("the skill", &self.skill),
            ("the plugin's skill", &self.plugin_skill),
            ("the instruction file", &self.instructions),
            ("the project memory", &self.memory),
        ]
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("a file in a directory")).expect("make its directory");
    fs::write(path, text).expect("plant a file");
}

fn write_json(path: &Path, value: &Value) {
    write(
        path,
        &serde_json::to_string_pretty(value).expect("JSON encodes"),
    );
}

/// Hooks on each of [`EVENTS`] that post to `url`.
fn hooks_to(url: &str) -> Value {
    let posting = json!([{ "hooks": [{ "type": "http", "url": url }] }]);
    let each = EVENTS
        .iter()
        .map(|event| ((*event).to_owned(), posting.clone()));
    Value::Object(each.collect())
}

fn skill_file(name: &str, holds: &str) -> String {
    format!("---\nname: {name}\ndescription: Planted; it holds {holds}.\n---\n\nSay {holds}.\n")
}

/// Claude Code's key of a project in `folder`.
fn project(folder: &Path) -> String {
    let path = folder.to_string_lossy();
    let key = path.chars().map(|c| match c.is_ascii_alphanumeric() {
        true => c,
        false => '-',
    });
    key.collect()
}

/// Plants the user's config in the config home `config` of sessions in `folder`; the MCP server and
/// the hooks post to the witness at `witness`, its URL without a path.
pub fn plant(config: &Path, folder: &Path, witness: &str) -> Planted {
    let planted = Planted {
        skill: token("skill"),
        plugin_skill: token("plugin-skill"),
        instructions: token("instructions"),
        memory: token("memory"),
    };

    let server = json!({ "type": "http", "url": format!("{witness}{MCP_PATH}") });
    write_json(
        &config.join(".claude.json"),
        &json!({ "hasCompletedOnboarding": true, "mcpServers": { "planted": server } }),
    );
    write_json(
        &config.join("settings.json"),
        &json!({
            "hooks": hooks_to(&format!("{witness}{HOOK_PATH}")),
            "enabledPlugins": { "witnessed@planted-market": true },
        }),
    );

    // The plugin `witnessed` from the marketplace `planted-market`, installed for the user.
    let market = config
        .join("plugins")
        .join("marketplaces")
        .join("planted-market");
    let installed = (config.join("plugins").join("cache"))
        .join("planted-market")
        .join("witnessed")
        .join("0.1.0");
    let manifest = json!({ "name": "witnessed", "version": "0.1.0", "description": "Planted." });
    let plugin_hooks = json!({ "hooks": hooks_to(&format!("{witness}{PLUGIN_HOOK_PATH}")) });
    for plugin in [market.join("witnessed"), installed.clone()] {
        write_json(
            &plugin.join(".claude-plugin").join("plugin.json"),
            &manifest,
        );
        write_json(&plugin.join("hooks").join("hooks.json"), &plugin_hooks);
        write(
            &plugin
                .join("skills")
                .join("witnessed-skill")
                .join("SKILL.md"),
            &skill_file("witnessed-skill", &planted.plugin_skill),
        );
    }
    let entry = json!({ "name": "witnessed", "source": "./witnessed", "version": "0.1.0" });
    write_json(
        &market.join(".claude-plugin").join("marketplace.json"),
        &json!({ "name": "planted-market", "owner": { "name": "tests" }, "plugins": [entry] }),
    );
    let when = "2026-01-01T00:00:00.000Z";
    let source = json!({ "source": "directory", "path": market });
    write_json(
        &config.join("plugins").join("known_marketplaces.json"),
        &json!({ "planted-market": { "source": source, "installLocation": market, "lastUpdated": when } }),
    );
    let install = json!({
        "scope": "user", "installPath": installed, "version": "0.1.0",
        "installedAt": when, "lastUpdated": when,
    });
    write_json(
        &config.join("plugins").join("installed_plugins.json"),
        &json!({ "version": 2, "plugins": { "witnessed@planted-market": [install] } }),
    );

    write(
        &config.join("skills").join("planted-skill").join("SKILL.md"),
        &skill_file("planted-skill", &planted.skill),
    );
    write(
        &config.join("CLAUDE.md"),
        &format!(
            "# Instructions\n\nAlways mention {}.\n",
            planted.instructions
        ),
    );
    let memory = config.join("projects").join(project(folder)).join("memory");
    write(
        &memory.join("MEMORY.md"),
        &format!("- [Planted](planted.md) — remember {}\n", planted.memory),
    );
    write(
        &memory.join("planted.md"),
        &format!(
            "---\nname: planted\ndescription: remember {}\ntype: project\n---\n\nPlanted.\n",
            planted.memory
        ),
    );
    planted
}
