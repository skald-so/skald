//! Scripted sessions that stand in for agents: each edits a clone of a measured repository with
//! edit tool writes, which hooks see, and shell commands, which they do not. Each source of a
//! machine's edits is compared with the files the session changes: the working-tree diff on a
//! schedule and after each step, hooks on the edit tool calls, and declarations of the files each
//! step names, made before it. Its command: `sessions`. Findings: `docs/dev/edit-prototype.md`,
//! "Edits via Bash and generators".

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::noise::table;
use crate::replay::{self, Repository};
use crate::tools::{self, Environment, Toolset};
use crate::{Result, git, paths};

/// A kind of step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    EditTool,
    InPlaceEdit,
    Move,
    Deletion,
    Formatter,
    Lockfile,
    Generator,
    Restore,
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::EditTool,
        Self::InPlaceEdit,
        Self::Move,
        Self::Deletion,
        Self::Formatter,
        Self::Lockfile,
        Self::Generator,
        Self::Restore,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::EditTool => "edit tool",
            Self::InPlaceEdit => "in-place edit",
            Self::Move => "move",
            Self::Deletion => "deletion",
            Self::Formatter => "formatter",
            Self::Lockfile => "lockfile rewrite",
            Self::Generator => "code generator",
            Self::Restore => "restore",
        }
    }
}

/// The text a shell command line holds in place of the session's scratch directory, outside the
/// worktree.
pub const SCRATCH: &str = "{scratch}";

/// A step of a session. Paths are relative to the root of the worktree, with `/` separators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The edit tool replaces the one occurrence of `old` in the file `path` with `new`.
    Edit {
        path: &'static str,
        old: &'static str,
        new: &'static str,
    },
    /// The edit tool writes `contents` to the file `path`, creating it and its directories.
    Write {
        path: &'static str,
        contents: &'static str,
    },
    /// The shell command line `line`, run by `/bin/sh -c` in the directory `dir` with the tools'
    /// environment; [`SCRATCH`] in it stands for the scratch directory.
    Shell {
        kind: Kind,
        dir: &'static str,
        line: &'static str,
    },
    /// `git` with `args` at the root, as [`git::run`] runs it.
    Git {
        kind: Kind,
        args: &'static [&'static str],
    },
}

impl Step {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Edit { .. } | Self::Write { .. } => Kind::EditTool,
            Self::Shell { kind, .. } | Self::Git { kind, .. } => *kind,
        }
    }

    /// The step as the report shows it.
    pub fn shown(&self) -> String {
        match self {
            Self::Edit { path, .. } => format!("edit `{path}`"),
            Self::Write { path, .. } => format!("write `{path}`"),
            Self::Shell { dir: "", line, .. } => format!("`{line}`"),
            Self::Shell { dir, line, .. } => format!("in `{dir}`: `{line}`"),
            Self::Git { args, .. } => format!("`git {}`", args.join(" ")),
        }
    }
}

/// A scripted session on a repository: shell command lines that set it up, such as filling the
/// cargo cache over the network, which must leave the worktree as it is, then its steps.
pub struct Session {
    pub repository: &'static str,
    pub setup: &'static [&'static str],
    pub steps: &'static [Step],
}

/// The words of the shell command line `line`: split at blanks and at the operators `|`, `&`, `;`,
/// `<`, `>`, `(` and `)` outside quotes, without their quotes and backslashes. Globs, variables and
/// command substitutions stay as written.
pub fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut chars = line.chars();
    let mut quote = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some('"') | None, '\\') => word.extend(chars.next()),
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, c) if c.is_whitespace() || "|&;<>()".contains(c) => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (None, c) => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// What a step did.
#[derive(Debug, Clone)]
pub struct Ran {
    pub step: Step,
    /// The files whose contents or existence the step changed, ignored files aside.
    pub changes: BTreeSet<String>,
    /// The files the step names: the edit tool's path, or the words of its command line that name
    /// a file of the worktree before or after it.
    pub named: BTreeSet<String>,
    /// The [edit set](git::edit_set) of the worktree once the step ended.
    pub edit_set: BTreeSet<String>,
}

/// A session played on a repository.
#[derive(Debug)]
pub struct Played {
    pub repository: &'static str,
    pub steps: Vec<Ran>,
    /// The files that differ from the commit once the session ends, ignored files aside.
    pub changed: BTreeSet<String>,
}

/// Plays `session` in the worktree at `dir`, a clone at its commit as [`checkout`] makes it, with
/// `scratch`, a directory outside it, for [`SCRATCH`], and the shell commands' environment `env`;
/// calls `progress` with the number of steps played after each.
pub fn play(
    dir: &Path,
    scratch: &Path,
    session: &Session,
    env: &Environment,
    progress: impl Fn(usize),
) -> Result<Played> {
    let scratch = scratch.to_str().ok_or("a scratch path that is not UTF-8")?;
    let shell = |cwd: &Path, line: &str| {
        let mut command = env.command("/bin/sh", cwd);
        command.args(["-c", &line.replace(SCRATCH, scratch)]);
        tools::output(command, line)
    };
    for line in session.setup {
        shell(dir, line)?;
    }
    if !git::edit_set(dir)?.is_empty() {
        return Err(format!("the setup of {} changed its worktree", session.repository).into());
    }
    let commit = git::line(dir, &["rev-parse", "HEAD"])?;
    let mut before = commit.clone();
    let mut steps = Vec::new();
    for step in session.steps {
        let (cwd, words) = match step {
            Step::Shell { dir: sub, line, .. } => {
                (dir.join(sub), words(&line.replace(SCRATCH, scratch)))
            }
            Step::Git { args, .. } => {
                (dir.to_owned(), args.iter().map(|a| a.to_string()).collect())
            }
            Step::Edit { path, .. } | Step::Write { path, .. } => {
                (dir.to_owned(), vec![path.to_string()])
            }
        };
        let mut candidates = BTreeSet::new();
        for word in &words {
            candidates.extend(paths::relative(dir, &cwd.join(word))?);
        }
        let is_file = |path: &String| dir.join(path).is_file();
        let mut named: BTreeSet<String> =
            candidates.iter().filter(|p| is_file(p)).cloned().collect();
        match step {
            Step::Edit { path, old, new } => {
                let file = dir.join(path);
                let text = fs::read_to_string(&file)?;
                if text.matches(old).count() != 1 {
                    return Err(format!("{path} does not hold {old:?} once").into());
                }
                fs::write(file, text.replacen(old, new, 1))?;
            }
            Step::Write { path, contents } => {
                let file = dir.join(path);
                fs::create_dir_all(file.parent().ok_or("a path without a directory")?)?;
                fs::write(file, contents)?;
            }
            Step::Shell { line, .. } => {
                shell(&cwd, line)?;
            }
            Step::Git { args, .. } => {
                git::run(dir, args)?;
            }
        }
        named.extend(candidates.into_iter().filter(is_file));
        let after = git::snapshot(dir)?;
        steps.push(Ran {
            step: *step,
            changes: git::changed(dir, &before, &after)?,
            named,
            edit_set: git::edit_set(dir)?,
        });
        before = after;
        progress(steps.len());
    }
    Ok(Played {
        repository: session.repository,
        changed: git::changed(dir, &commit, &before)?,
        steps,
    })
}

/// Fetches `repository` at its commit into a new worktree `dir`, with the commit for the
/// upstream's default branch: its edit set holds what a session changes.
pub fn checkout(dir: &Path, repository: &Repository) -> Result<()> {
    fs::create_dir(dir)?;
    git::run(dir, &["init", "--quiet"])?;
    let (url, commit) = (repository.url, repository.commit);
    git::run(
        dir,
        &["fetch", "--quiet", "--no-tags", "--depth=1", url, commit],
    )?;
    git::run(dir, &["checkout", "--quiet", "--detach", commit])?;
    git::run(dir, &["update-ref", git::UPSTREAM_HEAD, commit])?;
    Ok(())
}

/// The steps after which the diff scans on its schedule: every this many, and the last one.
pub const SCAN_EVERY: usize = 3;

/// A source of a machine's edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The edit set, scanned after every [`SCAN_EVERY`] steps and after the last.
    Diff,
    /// The edit set, scanned after each step, as a hook on each tool call could start it.
    DiffEachStep,
    /// The paths of the edit tool's calls, at each call.
    Hooks,
    /// The files each step names, declared before it.
    Declarations,
}

impl Source {
    pub const ALL: [Self; 4] = [
        Self::Diff,
        Self::DiffEachStep,
        Self::Hooks,
        Self::Declarations,
    ];

    pub fn name(self) -> String {
        match self {
            Self::Diff => format!("diff, every {SCAN_EVERY} steps"),
            Self::DiffEachStep => "diff, after each step".into(),
            Self::Hooks => "hooks".into(),
            Self::Declarations => "declarations".into(),
        }
    }
}

/// When a source learns of a step's change of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// It reported the file when the step started.
    Before,
    /// It reports the file once the step ends, from the step's own call or a scan right after it.
    AtTheEdit,
    /// At a later scan of its schedule.
    NextScan,
    /// At a later step that it sees and that names or changes the file.
    LaterCall,
    /// Its report at the end of the session does not hold the file.
    Never,
}

impl When {
    pub const ALL: [Self; 5] = [
        Self::Before,
        Self::AtTheEdit,
        Self::NextScan,
        Self::LaterCall,
        Self::Never,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Before => "before the edit",
            Self::AtTheEdit => "at the edit",
            Self::NextScan => "at the next scan",
            Self::LaterCall => "at a later tool call",
            Self::Never => "never",
        }
    }
}

impl Played {
    /// What `source` reports when each step starts and once it ends.
    pub fn reports(&self, source: Source) -> Vec<[BTreeSet<&str>; 2]> {
        let mut known = BTreeSet::new();
        let last = self.steps.len().saturating_sub(1);
        let mut reports = Vec::new();
        for (i, ran) in self.steps.iter().enumerate() {
            let named = ran.named.iter().map(String::as_str);
            let edit_set = || ran.edit_set.iter().map(String::as_str).collect();
            let start = match source {
                Source::Declarations => {
                    known.extend(named.clone());
                    known.clone()
                }
                _ => known.clone(),
            };
            match source {
                Source::Diff if (i + 1) % SCAN_EVERY == 0 || i == last => known = edit_set(),
                Source::DiffEachStep => known = edit_set(),
                Source::Hooks if ran.step.kind() == Kind::EditTool => known.extend(named),
                _ => {}
            }
            reports.push([start, known.clone()]);
        }
        reports
    }

    /// What `source` reports once the session ends.
    pub fn report(&self, source: Source) -> BTreeSet<&str> {
        let reports = self.reports(source);
        reports
            .last()
            .map(|[_, end]| end.clone())
            .unwrap_or_default()
    }

    /// When `source` learns of the change of `path` at the step `step`, 0-based.
    pub fn when(&self, source: Source, step: usize, path: &str) -> When {
        let reports = self.reports(source);
        if reports[step][0].contains(path) {
            return When::Before;
        }
        let later = reports[step..]
            .iter()
            .position(|[_, end]| end.contains(path));
        match (source, later) {
            (_, None) => When::Never,
            (Source::Diff, Some(_)) => When::NextScan,
            (_, Some(0)) => When::AtTheEdit,
            (_, Some(_)) => When::LaterCall,
        }
    }

    /// For `kind`, or every kind when none, each file changed at the end that a step of it
    /// changes, with the first such step, 0-based.
    pub fn first_changes(&self, kind: Option<Kind>) -> BTreeMap<&str, usize> {
        let mut first = BTreeMap::new();
        for (i, ran) in self.steps.iter().enumerate() {
            if kind.is_some_and(|kind| kind != ran.step.kind()) {
                continue;
            }
            for path in ran.changes.intersection(&self.changed) {
                first.entry(path.as_str()).or_insert(i);
            }
        }
        first
    }

    /// The files of [`Played::first_changes`] that `source` never learns of.
    pub fn missed(&self, source: Source, kind: Option<Kind>) -> usize {
        let first = self.first_changes(kind);
        first
            .iter()
            .filter(|(path, step)| self.when(source, **step, path) == When::Never)
            .count()
    }
}

/// Sets up the tools in `tmp`, an empty directory, as [`tools::prepare`] does, plays each of
/// `sessions` on a clone in `tmp` of the one of `repositories` it names, at its commit, and
/// reports them as [`report`] does after the [header](replay::header). Calls `progress` with the
/// repository and the number of steps played after each.
pub fn run(
    tmp: &Path,
    toolset: &Toolset,
    repositories: &[Repository],
    sessions: &[Session],
    progress: impl Fn(&str, usize),
) -> Result<String> {
    let repository = |session: &Session| {
        let name = session.repository;
        let found = repositories
            .iter()
            .find(|repository| repository.name == name);
        found.ok_or_else(|| format!("{name} is not a measured repository"))
    };
    for session in sessions {
        repository(session)?;
    }
    let tools = tools::prepare(tmp, toolset)?;
    let mut played = Vec::new();
    for session in sessions {
        let repository = repository(session)?;
        let dir = tmp.join(repository.name);
        checkout(&dir, repository)?;
        let scratch = tmp.join(format!("{}-scratch", repository.name));
        fs::create_dir(&scratch)?;
        let steps = |steps| progress(repository.name, steps);
        played.push(play(&dir, &scratch, session, &tools.env, steps)?);
    }
    Ok(replay::header(tmp)? + &report(&tools.versions, &played))
}

/// The tools with their versions and the findings on `played` as Markdown tables. The same on
/// every run with the same tools.
pub fn report(versions: &[(String, String)], played: &[Played]) -> String {
    let mut out = String::new();
    let list = |paths: &mut dyn Iterator<Item = &String>| {
        let paths: Vec<String> = paths.map(|path| format!("`{path}`")).collect();
        if paths.is_empty() {
            "none".to_owned()
        } else {
            paths.join(", ")
        }
    };
    let header = ["Tool", "Version"].map(str::to_owned);
    table(&mut out, "Tools", &header, |row| {
        for (tool, version) in versions {
            row(vec![tool.clone(), version.clone()]);
        }
    });

    let header = ["Repository", "Step", "Kind", "Command", "Changes", "Names"].map(str::to_owned);
    table(&mut out, "Steps", &header, |row| {
        for session in played {
            for (i, ran) in session.steps.iter().enumerate() {
                row(vec![
                    session.repository.into(),
                    (i + 1).to_string(),
                    ran.step.kind().name().into(),
                    ran.step.shown().replace('|', "\\|"),
                    list(&mut ran.changes.iter()),
                    list(&mut ran.named.iter()),
                ]);
            }
        }
    });

    let mut header = vec!["Repository".to_owned(), "Kind".into(), "Files".into()];
    header.extend(Source::ALL.map(Source::name));
    let title = "Changed files each source misses, by kind of command, in files and % of the files";
    table(&mut out, title, &header, |row| {
        for session in played {
            let kinds = Kind::ALL
                .into_iter()
                .filter(|kind| session.steps.iter().any(|ran| ran.step.kind() == *kind));
            let kinds = kinds.map(Some).chain([None]);
            for kind in kinds {
                let files = session.first_changes(kind).len();
                let mut cells = vec![
                    session.repository.into(),
                    kind.map_or("any", Kind::name).into(),
                    files.to_string(),
                ];
                for source in Source::ALL {
                    let missed = session.missed(source, kind);
                    cells.push(match files {
                        0 => "-".into(),
                        _ => format!("{missed} ({:.0}%)", missed as f64 * 100.0 / files as f64),
                    });
                }
                row(cells);
            }
        }
    });

    let mut header = vec!["Repository".to_owned(), "Source".into(), "Files".into()];
    header.extend(When::ALL.map(|when| capitalized(when.name())));
    header.push("Reported, not changed at the end".into());
    let title = "When each source learns of a changed file, from its first change, in files";
    table(&mut out, title, &header, |row| {
        for session in played {
            let first = session.first_changes(None);
            for source in Source::ALL {
                let mut cells = vec![
                    session.repository.into(),
                    source.name(),
                    first.len().to_string(),
                ];
                for when in When::ALL {
                    let count = first
                        .iter()
                        .filter(|(path, step)| session.when(source, **step, path) == when)
                        .count();
                    cells.push(count.to_string());
                }
                let report = session.report(source);
                let unchanged = report
                    .iter()
                    .filter(|path| !session.changed.contains(**path));
                cells.push(unchanged.count().to_string());
                row(cells);
            }
        }
    });
    out
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_at_blanks_and_operators_outside_quotes() {
        let line = r#"sed -i.bak -e 's/a b/c|d/' "docs/my file.md"&&rm docs/*.bak; grep -rl x src|xargs echo >out.txt 2>err (cd a\ b)"#;
        assert_eq!(
            words(line),
            [
                "sed",
                "-i.bak",
                "-e",
                "s/a b/c|d/",
                "docs/my file.md",
                "rm",
                "docs/*.bak",
                "grep",
                "-rl",
                "x",
                "src",
                "xargs",
                "echo",
                "out.txt",
                "2",
                "err",
                "cd",
                "a b",
            ]
        );
        assert_eq!(words(r#"echo "a\"b" 'c\d' ''"#), ["echo", "a\"b", "c\\d"]);
        assert!(words("  ").is_empty());
    }

    #[test]
    fn names_are_lowercase_and_capitalized_in_the_tables() {
        assert_eq!(capitalized("at the edit"), "At the edit");
        assert_eq!(capitalized(""), "");
        assert_eq!(Source::Diff.name(), "diff, every 3 steps");
    }
}
