//! What the tests share: the page's marked blocks and README's install command, the GitHub Actions
//! step as the tests read it, their substitutions, and the talk with the agent `ci/deploy-bot`
//! whose messages Ada's machine reads.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::kit::{RECV, received};
use crate::machine::Finished;
use crate::member::{JSON, bot_address, messages, talk_with_bot};
use crate::service_kit::Secrets;
use crate::talk::Talk;
use crate::world::run;

/// The page, from the repository root.
pub const PAGE: &str = "docs/service-agents.md";

/// The threat model's section on a leaked CI secret, as a link from the page names it.
pub const LEAKED_CI_SECRET: &str = "threat-model.md#a-leaked-ci-secret";

/// The word of a marked block's info string, before its step.
const MARKER: &str = "service-agent=";

/// The steps: the GitHub Actions step, the Docker example's Dockerfile and its commands.
pub const ACTIONS: &str = "github-actions";
pub const DOCKERFILE: &str = "dockerfile";
pub const DOCKER: &str = "docker";
pub const STEPS: [&str; 3] = [ACTIONS, DOCKERFILE, DOCKER];

/// What the tests substitute in every block: the channel.
pub const CHANNEL_PLACEHOLDER: &str = "<channel>";

/// README's heading of the install command for macOS and Linux.
const INSTALL_HEADING: &str = "#### macOS and Linux";

/// The keys of the step the tests run.
const STEP_KEYS: [&str; 4] = ["name", "env", "run", "shell"];

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The text of `path`, from the repository root, its line ends `\n`.
#[track_caller]
pub fn read(path: &str) -> String {
    let text = fs::read_to_string(root().join(path))
        .unwrap_or_else(|err| panic!("cannot read {path}: {err}"));
    text.replace("\r\n", "\n")
}

/// A marked block of the page: its step and its text, its fence's indentation dropped.
#[derive(Debug, Clone)]
pub struct Block {
    pub step: String,
    pub text: String,
}

/// The marked blocks of `page`, in its order. A block whose info string has the marker without a
/// step has the step `?`.
pub fn blocks_of(page: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines = page.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let fence = if trimmed.starts_with("```") {
            "```"
        } else if trimmed.starts_with("~~~") {
            "~~~"
        } else {
            continue;
        };
        let info = trimmed.trim_start_matches(&fence[..1]);
        let step = info
            .split_whitespace()
            .find_map(|word| word.strip_prefix(MARKER));
        let mut text = String::new();
        for inner in lines.by_ref() {
            let closing = inner.trim();
            if closing.starts_with(fence) && closing.trim_start_matches(&fence[..1]).is_empty() {
                break;
            }
            let dropped = inner.len() - inner.trim_start().len();
            text.push_str(&inner[dropped.min(indent)..]);
            text.push('\n');
        }
        if let Some(step) = step {
            let step = if step.is_empty() { "?" } else { step };
            blocks.push(Block {
                step: step.to_owned(),
                text,
            });
        }
    }
    blocks
}

/// The page's one block of `step`; fails unless it has exactly one.
#[track_caller]
pub fn block(step: &str) -> String {
    let blocks: Vec<Block> = blocks_of(&read(PAGE))
        .into_iter()
        .filter(|block| block.step == step)
        .collect();
    match blocks.as_slice() {
        [one] => one.text.clone(),
        [] => panic!("{PAGE} has no block marked {MARKER}{step}"),
        _ => panic!("{PAGE} has {} blocks marked {MARKER}{step}", blocks.len()),
    }
}

/// README's install command for macOS and Linux: the first line of the first code block after its
/// heading in "Install the client".
#[track_caller]
pub fn install_command() -> String {
    let readme = read("README.md");
    let after = readme
        .split_once(INSTALL_HEADING)
        .unwrap_or_else(|| panic!("README.md has no heading {INSTALL_HEADING:?}"))
        .1;
    let mut in_block = false;
    for line in after.lines() {
        if line.trim_start().starts_with("```") {
            assert!(
                !in_block,
                "README's first block after {INSTALL_HEADING:?} is empty"
            );
            in_block = true;
        } else if in_block && !line.trim().is_empty() {
            return line.trim().to_owned();
        }
    }
    panic!("README.md has no code block after {INSTALL_HEADING:?}")
}

/// `text` with every `<channel>` replaced by `channel`.
pub fn with_channel(text: &str, channel: &str) -> String {
    text.replace(CHANNEL_PLACEHOLDER, channel)
}

/// `text` with every `deploy-bot` that no `/` comes before replaced by `name`.
pub fn with_names(text: &str, name: &str) -> String {
    const AGENT: &str = "deploy-bot";
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(AGENT) {
        let (before, after) = rest.split_at(at);
        out.push_str(before);
        let slashed = before.ends_with('/') || (before.is_empty() && out.ends_with('/'));
        out.push_str(if slashed { AGENT } else { name });
        rest = &after[AGENT.len()..];
    }
    out.push_str(rest);
    out
}

/// The GitHub Actions step, as the tests read it.
#[derive(Debug, Default)]
pub struct Step {
    /// Its `env`, in its order, each value with surrounding quotes dropped.
    pub env: Vec<(String, String)>,
    /// Its script.
    pub run: String,
    /// Its `shell`, if set.
    pub shell: Option<String>,
}

/// `value` without one pair of surrounding quotes, single or double.
fn unquoted(value: &str) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.to_owned();
        }
    }
    value.to_owned()
}

/// The indentation of `line`, in spaces.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Whether `line` holds nothing but spaces or a comment.
fn empty_or_comment(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

/// The step of `yaml`, the block of [`ACTIONS`]: a `steps:` item, `- <key>: …`, or its mapping;
/// fails on a key other than [`STEP_KEYS`], on a `run` that is neither a literal block nor a line,
/// or on a second item.
#[track_caller]
pub fn step_of(yaml: &str) -> Step {
    let mut lines: Vec<String> = yaml.lines().map(str::to_owned).collect();
    if let Some(first) = lines.iter_mut().find(|line| !empty_or_comment(line)) {
        let indent = indent_of(first);
        if first[indent..].starts_with("- ") {
            first.replace_range(indent..indent + 2, "  ");
        }
    }
    let base = lines
        .iter()
        .find(|line| !empty_or_comment(line))
        .map(|line| indent_of(line))
        .unwrap_or_else(|| panic!("the block {ACTIONS} is empty"));
    let mut step = Step::default();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        i += 1;
        if empty_or_comment(line) {
            continue;
        }
        let indent = indent_of(line);
        assert!(
            indent == base,
            "the step {ACTIONS} has a line the tests do not read: {line:?}\n{yaml}"
        );
        assert!(
            !line[indent..].starts_with("- "),
            "the block {ACTIONS} has more than one step:\n{yaml}"
        );
        let (key, value) = line[indent..]
            .split_once(':')
            .unwrap_or_else(|| panic!("the step {ACTIONS} has a line without a key: {line:?}"));
        let (key, value) = (key.trim(), value.trim());
        assert!(
            STEP_KEYS.contains(&key),
            "the step {ACTIONS} has the key {key:?}, which the tests do not run; they run {STEP_KEYS:?}"
        );
        // The lines indented under the key.
        let mut nested = Vec::new();
        while i < lines.len() && (lines[i].trim().is_empty() || indent_of(&lines[i]) > base) {
            nested.push(lines[i].clone());
            i += 1;
        }
        match key {
            "env" => {
                assert!(
                    value.is_empty(),
                    "the step's env is not a mapping: {value:?}"
                );
                for entry in nested.iter().filter(|line| !empty_or_comment(line)) {
                    let (name, value) = entry.trim().split_once(':').unwrap_or_else(|| {
                        panic!("the step's env has a line without a name: {entry:?}")
                    });
                    step.env.push((name.trim().to_owned(), unquoted(value)));
                }
            }
            "run" if value.starts_with('|') => step.run = literal_block(&nested),
            "run" => {
                assert!(
                    !value.is_empty()
                        && !value.starts_with('>')
                        && nested.iter().all(|line| line.trim().is_empty()),
                    "the step's run is neither a literal block nor a line: {value:?}"
                );
                step.run = format!("{}\n", unquoted(value));
            }
            "shell" => step.shell = Some(unquoted(value)),
            _ => {}
        }
    }
    assert!(
        !step.run.trim().is_empty(),
        "the step {ACTIONS} has no run:\n{yaml}"
    );
    step
}

/// The text of a literal block scalar of `lines`: each line without the first line's indentation.
fn literal_block(lines: &[String]) -> String {
    let indent = lines
        .iter()
        .find(|line| !line.trim().is_empty())
        .map(|line| indent_of(line))
        .unwrap_or(0);
    let mut text = String::new();
    for line in lines {
        text.push_str(line.get(indent..).unwrap_or(""));
        text.push('\n');
    }
    text.trim_end_matches('\n').to_owned() + "\n"
}

/// The value the step's `env` must give the secret of `name`: the repository's secret of the
/// same name.
pub fn from_secrets(name: &str) -> String {
    format!("${{{{ secrets.{name} }}}}")
}

/// `value` without spaces: an expression `${{ … }}` as GitHub Actions reads it, whatever its
/// spacing.
pub fn compact(value: &str) -> String {
    value.split_whitespace().collect()
}

/// The talk of Ada and Bob in `acme`, and the agent `ci/deploy-bot` that Ada's machine created for
/// the talk's channel, with `extra` options.
pub struct Recipe {
    pub talk: Talk,
    pub secrets: Secrets,
    /// The agent's address, `acme/ci/deploy-bot@<server>`.
    pub address: String,
}

impl Recipe {
    /// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones; Ada's
    /// machine creates `ci/deploy-bot` with `extra`, which must succeed, and takes in its mailbox.
    pub fn new(ids: [u64; 2], logins: [&str; 2], extra: &[&str]) -> Self {
        let (talk, secrets) = talk_with_bot(ids, logins, extra);
        received(&talk.ada.machine);
        Recipe {
            talk,
            secrets,
            address: bot_address(),
        }
    }

    /// The two secrets, by their variables' names.
    pub fn variables(&self) -> [(&'static str, String); 2] {
        (self.secrets.variables()).map(|(name, value)| (name, value.to_owned()))
    }

    /// The talk's channel's ID.
    pub fn channel(&self) -> String {
        self.talk.channel.to_string()
    }

    /// Ada's machine sends `text` in the channel; fails unless the send succeeds.
    #[track_caller]
    pub fn ada_says(&self, text: &str) {
        let sent = self.talk.send(text);
        assert!(sent.success, "Ada's send failed: {sent:?}");
    }

    /// Runs `skald recv --json` on Ada's machine, which must succeed; gives the messages it shows
    /// from the agent.
    #[track_caller]
    pub fn ada_reads_from_the_agent(&self) -> (Vec<Value>, Finished) {
        let got = run(&self.talk.ada.machine, &[RECV, JSON]);
        assert!(got.success, "Ada's recv failed: {got:?}");
        let from_agent = messages(&got)
            .into_iter()
            .filter(|message| message["from"] == self.address.as_str())
            .collect();
        (from_agent, got)
    }
}

/// The lines of the `RUN` instruction of `dockerfile` that holds `command`, its continuation lines
/// included, as a range of line indexes; none if no `RUN` instruction holds it.
pub fn install_instruction(dockerfile: &str, command: &str) -> Option<std::ops::Range<usize>> {
    let lines: Vec<&str> = dockerfile.lines().collect();
    let continued = |line: &str| line.trim_end().ends_with('\\');
    let at = lines.iter().position(|line| line.contains(command))?;
    let mut start = at;
    while start > 0 && continued(lines[start - 1]) {
        start -= 1;
    }
    let first = lines[start].trim_start().to_ascii_uppercase();
    if !first.starts_with("RUN ") {
        return None;
    }
    let mut end = at;
    while end + 1 < lines.len() && continued(lines[end]) {
        end += 1;
    }
    Some(start..end + 1)
}
