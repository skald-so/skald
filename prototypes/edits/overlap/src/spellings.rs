//! The check of how git and the fingerprints spell paths and line endings on the file system of
//! this machine's temporary directory: a case-only rename under each `core.ignorecase` setting,
//! file names in NFC and NFD under each `core.precomposeunicode` setting, the paths an agent's
//! tools give, and line endings under each `core.autocrlf` setting. Its command: `spellings`.
//! Findings: `docs/dev/edit-prototype.md`, "Path normalization".

use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use unicode_normalization::{is_nfc, is_nfd};

use crate::harness::{Machine, Upstream};
use crate::markdown::{list, table};
use crate::{Client, Edits, Result, fingerprint, git, paths, replay};

/// `café.md` with its `é` precomposed, NFC, and decomposed, NFD.
pub const NFC: &str = "caf\u{e9}.md";
pub const NFD: &str = "cafe\u{301}.md";

/// What the file system of a directory takes for one name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSystem {
    /// Names that differ only in case name one file.
    pub case_insensitive: bool,
    /// Names that differ only in Unicode normalization name one file.
    pub normalization_insensitive: bool,
}

/// The file system of `dir`, a new directory: files created there as `probe` and as [`NFC`],
/// then looked up as `PROBE` and as [`NFD`].
pub fn file_system(dir: &Path) -> Result<FileSystem> {
    fs::create_dir(dir)?;
    fs::write(dir.join("probe"), "")?;
    fs::write(dir.join(NFC), "")?;
    Ok(FileSystem {
        case_insensitive: dir.join("PROBE").exists(),
        normalization_insensitive: dir.join(NFD).exists(),
    })
}

/// The settings `git init` writes to a new repository in `dir`, a new directory:
/// `core.ignorecase` and `core.precomposeunicode`, none for one it does not write.
pub fn init_settings(dir: &Path) -> Result<[Option<String>; 2]> {
    fs::create_dir(dir)?;
    git::run(dir, &["init", "--quiet"])?;
    let config = String::from_utf8(git::run(dir, &["config", "--local", "--list"])?)?;
    let setting = |name: &str| {
        let line = config
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='));
        line.map(str::to_owned)
    };
    Ok([
        setting("core.ignorecase"),
        setting("core.precomposeunicode"),
    ])
}

/// How a case-only rename is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rename {
    /// `git mv`.
    Git,
    /// A rename of the file, as a shell's `mv` or an editor makes it.
    FileSystem,
}

impl Rename {
    pub const ALL: [Self; 2] = [Self::Git, Self::FileSystem];

    pub fn name(self) -> &'static str {
        match self {
            Self::Git => "git mv",
            Self::FileSystem => "file system",
        }
    }
}

/// What a machine publishes, in a clone in the new directory `dir` with `core.ignorecase` set to
/// `ignorecase`, after a case-only rename of `README.md` to `Readme.md` made `how`: its edit set
/// and its number of path fingerprints; none when the rename fails.
pub fn case_only_rename(
    dir: &Path,
    ignorecase: bool,
    how: Rename,
) -> Result<Option<(BTreeSet<String>, usize)>> {
    let upstream = Upstream::create(dir, &[("README.md", "# Inventory\n")])?;
    let config = format!("--config=core.ignorecase={ignorecase}");
    let machine = upstream.clone_with("machine", &[&config])?;
    match how {
        Rename::Git => {
            if machine.rename("README.md", "Readme.md").is_err() {
                return Ok(None);
            }
        }
        Rename::FileSystem => fs::rename(
            machine.dir().join("README.md"),
            machine.dir().join("Readme.md"),
        )?,
    }
    let edits = edits(&machine)?;
    Ok(Some((owned(edits.paths()), edits.sizes()[0])))
}

/// What a machine publishes for its working tree, with the space key of the measurement.
fn edits(machine: &Machine) -> Result<Edits> {
    Client::new("machine", replay::key()).edits(machine.dir())
}

fn owned(paths: BTreeSet<&str>) -> BTreeSet<String> {
    paths.into_iter().map(str::to_owned).collect()
}

/// How a file named `café.md` comes into a clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// Written in the working tree with its name in NFC.
    WrittenNfc,
    /// Written in the working tree with its name in NFD.
    WrittenNfd,
    /// Checked out from a commit that holds it with its name in NFD.
    CheckedOutNfd,
}

impl Arrival {
    pub const ALL: [Self; 3] = [Self::WrittenNfc, Self::WrittenNfd, Self::CheckedOutNfd];

    pub fn name(self) -> &'static str {
        match self {
            Self::WrittenNfc => "written in NFC",
            Self::WrittenNfd => "written in NFD",
            Self::CheckedOutNfd => "checked out from a commit in NFD",
        }
    }
}

/// What git and the fingerprints make of a file named `café.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// The [form] of each name of the machine's edit set.
    pub edit_set: Vec<&'static str>,
    /// The form of each name of the index once the machine adds the file, or after the checkout.
    pub index: Vec<&'static str>,
    /// The machine's number of path fingerprints.
    pub fingerprints: usize,
}

/// The Unicode normalization form of `name`: `NFC`, `NFD`, or `mixed` for neither.
pub fn form(name: &str) -> &'static str {
    match (is_nfc(name), is_nfd(name)) {
        (true, _) => "NFC",
        (false, true) => "NFD",
        (false, false) => "mixed",
    }
}

/// What a machine in a clone in the new directory `dir`, with `core.precomposeunicode` set to
/// `precompose`, makes of `café.md` that comes as `arrival`.
pub fn names(dir: &Path, precompose: bool, arrival: Arrival) -> Result<Names> {
    let upstream = Upstream::create(dir, &[("README.md", "# Inventory\n")])?;
    if arrival == Arrival::CheckedOutNfd {
        // A clone that keeps the NFD name in its index commits it so.
        let writer = upstream.clone_with("writer", &["--config=core.precomposeunicode=false"])?;
        writer.write(NFD, "café\n")?;
        writer.commit("add café")?;
        writer.git(&["push", "--quiet", "origin", "HEAD"])?;
    }
    let config = format!("--config=core.precomposeunicode={precompose}");
    let machine = upstream.clone_with("machine", &[&config])?;
    match arrival {
        Arrival::WrittenNfc => machine.write(NFC, "café\n")?,
        Arrival::WrittenNfd => machine.write(NFD, "café\n")?,
        Arrival::CheckedOutNfd => {}
    }
    let edits = edits(&machine)?;
    if arrival != Arrival::CheckedOutNfd {
        machine.git(&["add", "--all"])?;
    }
    let index = machine.git(&["ls-files", "-z"])?;
    let index = git::nul_separated(&index)?;
    let forms = |names: BTreeSet<&str>| {
        let other = names.into_iter().filter(|name| *name != "README.md");
        other.map(form).collect()
    };
    Ok(Names {
        edit_set: forms(edits.paths()),
        index: forms(index.iter().map(String::as_str).collect()),
        fingerprints: edits.sizes()[0],
    })
}

/// A spelling of a path that an agent's tool can give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spelling {
    /// What kind of spelling it is.
    pub name: &'static str,
    /// The path of the repository's file it names.
    pub file: &'static str,
    /// The path [`paths::relative`] maps it to, none when no worktree of the repository holds it;
    /// none at all when this system cannot make the spelling.
    pub maps_to: Option<Option<String>>,
    /// Whether the path it maps to gets the fingerprint of `file`.
    pub same: bool,
}

/// The files of the repository of [`tool_paths`].
const TOOL_FILES: [(&str, &str); 3] = [
    ("src/inventory.rs", "pub fn count() -> u32 {\n    0\n}\n"),
    ("docs/caf\u{e9}.md", "# Café\n"),
    ("docs/guide.md", "# Guide\n"),
];

/// The spellings of the paths of a clone's files that an agent's tools can give, in the new
/// directory `dir`, each with what it maps to: relative to the clone, with `.` and `..`, with
/// backslashes; absolute, as `dir` spells it and as the file system resolves it, through a symbolic
/// link to the clone, from another worktree, in another case, with the file name in NFD; on Windows
/// with a lowercase drive letter and with forward slashes; a deleted file; a file outside the
/// repository; a file of another repository.
pub fn tool_paths(dir: &Path) -> Result<Vec<Spelling>> {
    fs::create_dir(dir)?;
    let upstream = Upstream::create(&dir.join("repo"), &TOOL_FILES)?;
    let machine = upstream.clone_as("machine")?;
    machine.delete("docs/guide.md")?;
    let worktree = dir.join("worktree");
    let worktree_arg = worktree.to_str().ok_or("a path that is not UTF-8")?;
    machine.git(&["worktree", "add", "--quiet", "--detach", worktree_arg])?;
    let mut other_files = TOOL_FILES;
    other_files[0].1 = "pub fn count() -> u64 {\n    0\n}\n";
    let other = Upstream::create(&dir.join("other"), &other_files)?.clone_as("machine")?;
    fs::write(dir.join("outside.rs"), "")?;
    let link = dir.join("link");
    let linked = symlink_dir(machine.dir(), &link).is_ok();

    let file = |root: &Path, path: &str| -> PathBuf {
        path.split('/')
            .fold(root.to_path_buf(), |dir, name| dir.join(name))
    };
    let root = machine.dir();
    let inventory = "src/inventory.rs";
    let absolute = file(root, inventory);
    let text = absolute.to_str().ok_or("a path that is not UTF-8")?;
    let mut spellings: Vec<(&'static str, &'static str, Option<PathBuf>)> = vec![
        ("relative", inventory, Some(inventory.into())),
        (
            "relative, with `.` and `..`",
            inventory,
            Some("./docs/../src/inventory.rs".into()),
        ),
        (
            "relative, with backslashes",
            inventory,
            Some("src\\inventory.rs".into()),
        ),
        (
            "absolute, as the temporary directory spells it",
            inventory,
            Some(absolute.clone()),
        ),
        (
            "absolute, resolved by the file system",
            inventory,
            Some(fs::canonicalize(&absolute)?),
        ),
        (
            "through a symbolic link to the clone",
            inventory,
            linked.then(|| file(&link, inventory)),
        ),
        (
            "from another worktree",
            inventory,
            Some(file(&worktree, inventory)),
        ),
        (
            "in another case",
            inventory,
            Some(file(root, "SRC/Inventory.RS")),
        ),
        (
            "with the file name in NFD",
            TOOL_FILES[1].0,
            Some(file(root, "docs").join(NFD)),
        ),
    ];
    if cfg!(windows) {
        let windows = windows_spellings(text);
        spellings.extend(windows.map(|(name, path)| (name, inventory, path.map(PathBuf::from))));
    }
    spellings.extend([
        (
            "a deleted file",
            TOOL_FILES[2].0,
            Some(file(root, TOOL_FILES[2].0)),
        ),
        (
            "outside the repository",
            inventory,
            Some(dir.join("outside.rs")),
        ),
        (
            "in another repository",
            inventory,
            Some(file(other.dir(), inventory)),
        ),
    ]);

    let scope = git::scope(root)?;
    let key = replay::key();
    spellings
        .into_iter()
        .map(|(name, path, given)| {
            let maps_to = given
                .map(|given| paths::relative(root, &given))
                .transpose()?;
            let same = maps_to.iter().flatten().any(|mapped| {
                fingerprint::path(&key, &scope, mapped) == fingerprint::path(&key, &scope, path)
            });
            Ok(Spelling {
                name,
                file: path,
                maps_to,
                same,
            })
        })
        .collect()
}

/// The spellings of `path`, an absolute Windows path, that only Windows takes for it: with a
/// lowercase drive letter, none when it has no drive letter, and with forward slashes.
pub fn windows_spellings(path: &str) -> [(&'static str, Option<String>); 2] {
    let lowercase = match path.as_bytes() {
        [drive, b':', ..] => Some(format!(
            "{}{}",
            drive.to_ascii_lowercase() as char,
            &path[1..]
        )),
        _ => None,
    };
    [
        ("absolute, with a lowercase drive letter", lowercase),
        (
            "absolute, with forward slashes",
            Some(path.replace('\\', "/")),
        ),
    ]
}

/// `path` in backquotes, followed by its Unicode normalization form when it is not NFC.
pub fn shown(path: &str) -> String {
    match form(path) {
        "NFC" => format!("`{path}`"),
        form => format!("`{path}`, {form}"),
    }
}

/// Creates `link`, a symbolic link to the directory `target`.
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    return std::os::windows::fs::symlink_dir(target, link);
}

/// The settings of `core.autocrlf`.
pub const AUTOCRLF: [&str; 3] = ["true", "input", "false"];

/// What git and the fingerprints make of line endings under a `core.autocrlf` setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEndings {
    /// The line endings of a file stored with LF, once checked out: `LF` or `CRLF`.
    pub checkout: &'static str,
    /// The line endings git stores for the file once written with CRLF and committed.
    pub committed: &'static str,
    /// The numbers of fingerprints of one edit of the file written with LF on a clone without the
    /// setting, of the same edit written with CRLF on the clone with it, and of those they share.
    pub fingerprints: [usize; 3],
}

/// `src/store.rs` of [`line_endings`], with LF line endings.
const STORE: &str = "pub fn count() -> u32 {\n    let items = 0;\n    items\n}\n\n\
    pub fn reserve() -> u32 {\n    let reserved = 1;\n    reserved\n}\n";

/// What two clones in the new directory `dir` make of one edit of `src/store.rs`, a line of
/// `reserve`: one without `core.autocrlf` writes it with LF; the other, with `core.autocrlf` set
/// to `autocrlf`, writes the whole file with CRLF, as a Windows editor does, and commits it.
pub fn line_endings(dir: &Path, autocrlf: &str) -> Result<LineEndings> {
    let path = "src/store.rs";
    let upstream = Upstream::create(dir, &[(path, STORE)])?;
    let edited = STORE.replace("reserved = 1", "reserved = 2");
    let lf = upstream.clone_as("lf")?;
    lf.write(path, &edited)?;
    let config = format!("--config=core.autocrlf={autocrlf}");
    let crlf = upstream.clone_with("crlf", &[&config])?;
    let endings = |text: &[u8]| match text.windows(2).any(|pair| pair == b"\r\n") {
        true => "CRLF",
        false => "LF",
    };
    let checkout = endings(&fs::read(crlf.dir().join(path))?);
    crlf.write(path, &edited.replace('\n', "\r\n"))?;
    crlf.commit("reserve two")?;
    let committed = endings(&git::blob(crlf.dir(), "HEAD", path)?);
    let [lf, crlf] = [edits(&lf)?, edits(&crlf)?].map(|edits| edits.publication.fingerprints);
    Ok(LineEndings {
        checkout,
        committed,
        fingerprints: [lf.len(), crlf.len(), lf.intersection(&crlf).count()],
    })
}

/// The check in `tmp`, an empty directory, as Markdown: the [header](replay::header), the file
/// system and the settings `git init` writes, then a table for each part of the check. The same
/// on every run on one machine.
pub fn check(tmp: &Path) -> Result<String> {
    let mut out = replay::header(tmp)?;
    let yes = |value: bool| if value { "yes" } else { "no" };
    let system = file_system(&tmp.join("probe"))?;
    writeln!(
        out,
        "File system: case-insensitive: {}; Unicode normalization-insensitive: {}",
        yes(system.case_insensitive),
        yes(system.normalization_insensitive)
    )?;
    let [ignorecase, precompose] = init_settings(&tmp.join("init"))?;
    let unset = |value: Option<String>| value.unwrap_or_else(|| "unset".into());
    writeln!(
        out,
        "`git init` sets: `core.ignorecase` {}, `core.precomposeunicode` {}",
        unset(ignorecase),
        unset(precompose)
    )?;

    let mut rows = Vec::new();
    for ignorecase in [true, false] {
        for how in Rename::ALL {
            let dir = tmp.join(format!("rename-{ignorecase}-{how:?}"));
            let (edit_set, fingerprints) = match case_only_rename(&dir, ignorecase, how)? {
                Some((paths, fingerprints)) => {
                    let paths = paths.iter().map(|path| format!("`{path}`"));
                    (list(paths), fingerprints.to_string())
                }
                None => ("the rename fails".into(), "-".into()),
            };
            rows.push(vec![
                ignorecase.to_string(),
                how.name().into(),
                edit_set,
                fingerprints,
            ]);
        }
    }
    let header = [
        "`core.ignorecase`",
        "Rename",
        "Edit set",
        "Path fingerprints",
    ];
    let title = "Case-only rename of `README.md` to `Readme.md`";
    table(&mut out, title, &header.map(str::to_owned), |row| {
        rows.into_iter().for_each(row)
    });

    let mut rows = Vec::new();
    for precompose in [true, false] {
        for arrival in Arrival::ALL {
            let dir = tmp.join(format!("names-{precompose}-{arrival:?}"));
            let names = names(&dir, precompose, arrival)?;
            rows.push(vec![
                precompose.to_string(),
                arrival.name().into(),
                list(&names.edit_set),
                list(&names.index),
                names.fingerprints.to_string(),
            ]);
        }
    }
    let header = [
        "`core.precomposeunicode`",
        "File",
        "Edit set",
        "Index",
        "Path fingerprints",
    ];
    let title = "File names in NFC and NFD: `café.md`, by the form of each name";
    table(&mut out, title, &header.map(str::to_owned), |row| {
        rows.into_iter().for_each(row)
    });

    let rows = tool_paths(&tmp.join("tools"))?.into_iter().map(|spelling| {
        let maps_to = match spelling.maps_to {
            None => "cannot be made here".into(),
            Some(None) => "no worktree of the repository".into(),
            Some(Some(path)) => shown(&path),
        };
        vec![
            spelling.name.into(),
            format!("`{}`", spelling.file),
            maps_to,
            yes(spelling.same).into(),
        ]
    });
    let header = ["Spelling", "File", "Maps to", "The file's path fingerprint"];
    let title = "Paths an agent's tools give";
    table(&mut out, title, &header.map(str::to_owned), |row| {
        rows.for_each(row)
    });

    let mut rows = Vec::new();
    for autocrlf in AUTOCRLF {
        let endings = line_endings(&tmp.join(format!("autocrlf-{autocrlf}")), autocrlf)?;
        let mut cells = vec![
            autocrlf.into(),
            endings.checkout.into(),
            endings.committed.into(),
        ];
        cells.extend(endings.fingerprints.map(|count| count.to_string()));
        rows.push(cells);
    }
    let header = [
        "`core.autocrlf`",
        "Checkout of an LF file",
        "Stored once written with CRLF",
        "Fingerprints of an edit written with LF",
        "Of the edit written with CRLF",
        "Shared",
    ];
    let title = "Line endings of one edit";
    table(&mut out, title, &header.map(str::to_owned), |row| {
        rows.into_iter().for_each(row)
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_are_told_apart_and_shown_when_not_nfc() {
        assert_eq!(form("src/lib.rs"), "NFC");
        assert_eq!(form(NFC), "NFC");
        assert_eq!(form(NFD), "NFD");
        assert_eq!(form(&format!("{NFC}/{NFD}")), "mixed");
        assert_eq!(shown(NFC), format!("`{NFC}`"));
        assert_eq!(shown(NFD), format!("`{NFD}`, NFD"));
    }

    #[test]
    fn windows_spellings_lowercase_the_drive_letter_and_turn_backslashes_forward() {
        assert_eq!(
            windows_spellings("C:\\Users\\RUNNER~1\\src\\inventory.rs"),
            [
                (
                    "absolute, with a lowercase drive letter",
                    Some("c:\\Users\\RUNNER~1\\src\\inventory.rs".into())
                ),
                (
                    "absolute, with forward slashes",
                    Some("C:/Users/RUNNER~1/src/inventory.rs".into())
                ),
            ]
        );
        let [lowercase, forward] = windows_spellings("\\\\server\\share\\a.rs");
        assert_eq!(lowercase.1, None);
        assert_eq!(forward.1.as_deref(), Some("//server/share/a.rs"));
    }
}
