//! The exact check: `git merge-tree` merges another machine's changes into this machine's and
//! gives the files and lines where they conflict.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use crate::{Result, git};

/// What `git merge-tree` finds when it merges another machine's changes into this machine's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Whether the changes merge cleanly.
    pub clean: bool,
    /// Each conflict `git merge-tree` reports, in its order.
    pub conflicts: Vec<Conflict>,
    /// Each file with conflicts, by its path in the merge, with the conflicting ranges of this
    /// machine's lines, 1-based and end-exclusive. An empty range stands before a line this
    /// machine deleted where the other changed it. A conflict without conflict markers, such as
    /// modify/delete, has no ranges.
    pub files: BTreeMap<String, Vec<Range<usize>>>,
}

/// A conflict as `git merge-tree` reports it: its type, such as `CONFLICT (contents)` or
/// `CONFLICT (modify/delete)`, and the paths it involves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub kind: String,
    pub paths: Vec<String>,
}

/// Merges the commit `theirs` into the commit `ours`, both in the repository at `dir`, with
/// `git merge-tree` from their merge base.
pub fn merge(dir: &Path, ours: &str, theirs: &str) -> Result<Check> {
    let args = [
        "merge-tree",
        "--write-tree",
        "-z",
        "--messages",
        ours,
        theirs,
    ];
    let output = git::command(dir, &args).output()?;
    let clean = match output.status.code() {
        Some(0) => true,
        Some(1) => false,
        _ => {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git merge-tree in {}: {}", dir.display(), error.trim()).into());
        }
    };
    let merged = parse(&output.stdout)?;
    let mut files = BTreeMap::new();
    for (path, stages) in &merged.files {
        let lines = match (&stages[1], &stages[2]) {
            (Some(own), Some(_)) => {
                let file = git::run(
                    dir,
                    &["cat-file", "blob", &format!("{}:{path}", merged.tree)],
                )?;
                let size = marker_size(dir, path)?;
                own_lines(dir, own, &file, size, ours, theirs)?
            }
            _ => Vec::new(),
        };
        files.insert(path.clone(), lines);
    }
    Ok(Check {
        clean,
        conflicts: merged.conflicts,
        files,
    })
}

/// The output of `git merge-tree --write-tree -z --messages`.
#[derive(Debug, PartialEq, Eq)]
struct Merged {
    tree: String,
    /// Each conflicted file with the blob of its stages 1, the merge base, 2, ours, and 3, theirs.
    files: BTreeMap<String, [Option<String>; 3]>,
    conflicts: Vec<Conflict>,
}

fn parse(stdout: &[u8]) -> Result<Merged> {
    let mut fields = std::str::from_utf8(stdout)?.split('\0');
    let tree = fields
        .next()
        .ok_or("git merge-tree gave no tree")?
        .to_owned();
    let mut files = BTreeMap::<String, [Option<String>; 3]>::new();
    // `<mode> <object> <stage>\t<path>` for each conflicted stage, then an empty field.
    for field in fields.by_ref().take_while(|field| !field.is_empty()) {
        let (info, path) = field
            .split_once('\t')
            .ok_or("git merge-tree gave a stage without a path")?;
        let [_mode, object, stage] = info
            .split(' ')
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| format!("git merge-tree gave an unknown stage: {info}"))?;
        let stage: usize = stage.parse()?;
        let slot = files
            .entry(path.to_owned())
            .or_default()
            .get_mut(stage.wrapping_sub(1))
            .ok_or_else(|| format!("git merge-tree gave stage {stage}"))?;
        *slot = Some(object.to_owned());
    }
    // `<number of paths>`, the paths, the type and the message of each record.
    let mut conflicts = Vec::new();
    while let Some(count) = fields.next().filter(|field| !field.is_empty()) {
        let count: usize = count.parse()?;
        let paths: Vec<String> = fields.by_ref().take(count).map(str::to_owned).collect();
        let (Some(kind), Some(_message)) = (fields.next(), fields.next()) else {
            return Err("git merge-tree gave a message without its type".into());
        };
        if kind.starts_with("CONFLICT") {
            conflicts.push(Conflict {
                kind: kind.to_owned(),
                paths,
            });
        }
    }
    Ok(Merged {
        tree,
        files,
        conflicts,
    })
}

/// The size of the conflict markers `git merge-tree` writes in `path`: its `conflict-marker-size`
/// attribute.
fn marker_size(dir: &Path, path: &str) -> Result<usize> {
    // `<path>\0conflict-marker-size\0<value>\0`
    let output = git::run(
        dir,
        &["check-attr", "-z", "conflict-marker-size", "--", path],
    )?;
    let value = String::from_utf8(output)?
        .split('\0')
        .nth(2)
        .ok_or("git check-attr gave no value")?
        .to_owned();
    Ok(parse_marker_size(&value))
}

/// The conflict marker size for the value of the `conflict-marker-size` attribute, as git reads
/// it: its leading number if it is positive, otherwise 7. `set`, `unset` and `unspecified` give 7.
fn parse_marker_size(value: &str) -> usize {
    let digits = value.strip_prefix('+').unwrap_or(value);
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    match digits[..end].parse() {
        Ok(size) if size > 0 => size,
        _ => 7,
    }
}

/// The conflicting ranges, in this machine's version `own` of a file, of `file`, the file as
/// `git merge-tree` writes it with conflict markers of `size` characters labelled `ours` and
/// `theirs`.
fn own_lines(
    dir: &Path,
    own: &str,
    file: &[u8],
    size: usize,
    ours: &str,
    theirs: &str,
) -> Result<Vec<Range<usize>>> {
    let (kept, ranges) = keep_ours(file, size, ours, theirs);
    if ranges.is_empty() {
        return Ok(ranges);
    }
    // Outside the conflicts, the file holds the other machine's clean changes too; a diff from
    // this machine's version maps the ranges back to it. The options fix what the repository's
    // attributes and config could change: a binary diff without hunks, hunks joined across the
    // lines between them, and where a hunk slides.
    let kept = git::run_with_input(dir, &["hash-object", "-w", "--stdin"], &kept)?;
    let kept = String::from_utf8(kept)?;
    let diff = git::run(
        dir,
        &[
            "diff",
            "--unified=0",
            "--inter-hunk-context=0",
            "--text",
            "--indent-heuristic",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--diff-algorithm=myers",
            own,
            kept.trim_end(),
        ],
    )?;
    let hunks = hunks(&diff)?;
    Ok(ranges
        .into_iter()
        .map(|range| map_back(&hunks, range))
        .collect())
}

/// `file` with each conflict resolved to this machine's side, and the range of that side in it.
/// Conflict markers are `size` characters long: with 7, they start with `<<<<<<< <ours>` and end
/// with `>>>>>>> <theirs>`; the merge base's side, with the `diff3` and `zdiff3` styles, starts
/// with `|||||||`.
fn keep_ours(file: &[u8], size: usize, ours: &str, theirs: &str) -> (Vec<u8>, Vec<Range<usize>>) {
    enum Side {
        Outside,
        Ours,
        Base,
        Theirs,
    }
    let start = format!("{} {ours}", "<".repeat(size));
    let base = "|".repeat(size);
    let middle = "=".repeat(size);
    let end = format!("{} {theirs}", ">".repeat(size));
    let mut kept = Vec::new();
    let mut ranges = Vec::new();
    let mut side = Side::Outside;
    // The number of the next line of `kept`, and of the first line of this machine's side.
    let mut next = 1;
    let mut first = 1;
    for line in file.split_inclusive(|&byte| byte == b'\n') {
        let text = line.strip_suffix(b"\n").unwrap_or(line);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        match side {
            Side::Outside if text.starts_with(start.as_bytes()) => {
                side = Side::Ours;
                first = next;
            }
            Side::Ours
                if text
                    .strip_prefix(base.as_bytes())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(b" ")) =>
            {
                side = Side::Base
            }
            Side::Ours | Side::Base if text == middle.as_bytes() => {
                ranges.push(first..next);
                side = Side::Theirs;
            }
            Side::Theirs if text.starts_with(end.as_bytes()) => side = Side::Outside,
            Side::Outside | Side::Ours => {
                kept.extend_from_slice(line);
                next += 1;
            }
            Side::Base | Side::Theirs => {}
        }
    }
    (kept, ranges)
}

/// A hunk of a `--unified=0` diff: the first line and the number of lines on each side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hunk {
    old: usize,
    old_lines: usize,
    new: usize,
    new_lines: usize,
}

fn hunks(diff: &[u8]) -> Result<Vec<Hunk>> {
    let side = |range: &str| -> Result<(usize, usize)> {
        Ok(match range.split_once(',') {
            Some((first, lines)) => (first.parse()?, lines.parse()?),
            None => (range.parse()?, 1),
        })
    };
    diff.split(|&byte| byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"@@ -"))
        .map(|header| {
            let header = String::from_utf8_lossy(header);
            let (old, rest) = header
                .split_once(" +")
                .ok_or_else(|| format!("a hunk header without a new side: {header}"))?;
            let (new, _) = rest
                .split_once(" @@")
                .ok_or_else(|| format!("a hunk header without an end: {header}"))?;
            let ((old, old_lines), (new, new_lines)) = (side(old)?, side(new)?);
            Ok(Hunk {
                old,
                old_lines,
                new,
                new_lines,
            })
        })
        .collect()
}

/// Maps `range`, lines of the new side of `hunks` that no hunk changes, to the old side.
fn map_back(hunks: &[Hunk], range: Range<usize>) -> Range<usize> {
    let mut start = range.start;
    for hunk in hunks {
        // The first new line after the hunk; a hunk without new lines comes after its line `new`.
        let after = hunk.new + hunk.new_lines.max(1);
        if after > range.start {
            break;
        }
        start = start + hunk.old_lines - hunk.new_lines;
    }
    start..start + range.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_conflicted_stages_and_the_conflicts() {
        let stdout = b"tree1\0\
            100644 base1 1\tf\x00100644 ours1 2\tf\x00100644 theirs1 3\tf\0\
            100644 base2 1\tg\x00100644 ours2 2\tg\0\
            100644 ours3 2\th1\x00100644 theirs3 3\th2\0\0\
            1\0f\0Auto-merging\0Auto-merging f\n\0\
            1\0f\0CONFLICT (contents)\0CONFLICT (content): Merge conflict in f\n\0\
            3\0h\0h1\0h2\0CONFLICT (rename/rename)\0h renamed to h1 and to h2\n\0";
        let some = |oid: &str| Some(oid.to_owned());
        assert_eq!(
            parse(stdout).unwrap(),
            Merged {
                tree: "tree1".into(),
                files: BTreeMap::from([
                    ("f".into(), [some("base1"), some("ours1"), some("theirs1")]),
                    ("g".into(), [some("base2"), some("ours2"), None]),
                    ("h1".into(), [None, some("ours3"), None]),
                    ("h2".into(), [None, None, some("theirs3")]),
                ]),
                conflicts: vec![
                    Conflict {
                        kind: "CONFLICT (contents)".into(),
                        paths: vec!["f".into()],
                    },
                    Conflict {
                        kind: "CONFLICT (rename/rename)".into(),
                        paths: vec!["h".into(), "h1".into(), "h2".into()],
                    },
                ],
            }
        );
    }

    #[test]
    fn parse_reads_a_clean_merge() {
        let clean = Merged {
            tree: "tree1".into(),
            files: BTreeMap::new(),
            conflicts: Vec::new(),
        };
        assert_eq!(parse(b"tree1\0\0").unwrap(), clean);
        assert_eq!(
            parse(b"tree1\0\x001\0f\0Auto-merging\0Auto-merging f\n\0").unwrap(),
            clean
        );
    }

    #[test]
    fn parse_rejects_malformed_output() {
        for stdout in [
            &b"tree1\x00100644 ours1 2 f\0\0"[..],
            b"tree1\0ours1 2\tf\0\0",
            b"tree1\x00100644 ours1 4\tf\0\0",
            b"tree1\x00100644 ours1 0\tf\0\0",
            b"tree1\0\x002\0f\0",
            b"tree1\0\x002\0f\0CONFLICT (contents)\0",
            b"tree1\0\x00x\0f\0Auto-merging\0Auto-merging f\0",
        ] {
            assert!(
                parse(stdout).is_err(),
                "{}",
                String::from_utf8_lossy(stdout)
            );
        }
    }

    #[test]
    fn keep_ours_keeps_this_machines_side_of_each_conflict() {
        let file = b"a\n\
            <<<<<<< ours1:old.rs\nb\nc\n=======\nB\n>>>>>>> theirs1\n\
            d\n\
            <<<<<<< ours1\r\n||||||| base1\r\nx\r\n=======\r\nX\r\n>>>>>>> theirs1\r\n\
            <<<<<<< other\n=======\n>>>>>>> other\n\
            e";
        let (kept, ranges) = keep_ours(file, 7, "ours1", "theirs1");
        assert_eq!(
            String::from_utf8(kept).unwrap(),
            "a\nb\nc\nd\n<<<<<<< other\n=======\n>>>>>>> other\ne"
        );
        assert_eq!(ranges, [2..4, 5..5]);
    }

    #[test]
    fn keep_ours_reads_markers_of_the_given_size_only() {
        let file = b"a\n\
            <<<<<<<<< ours1\n<<<<<<< ours1\n|||||||\n=======\n>>>>>>> theirs1\n\
            |||||||||x\n|||||||||\nx\n=========\nX\n>>>>>>>>> theirs1\n\
            <<<<<<<<< ours1\n||||||||| base1\n=========\n>>>>>>>>> theirs1\n\
            e\n";
        let (kept, ranges) = keep_ours(file, 9, "ours1", "theirs1");
        assert_eq!(
            String::from_utf8(kept).unwrap(),
            "a\n<<<<<<< ours1\n|||||||\n=======\n>>>>>>> theirs1\n|||||||||x\ne\n"
        );
        assert_eq!(ranges, [2..7, 7..7]);
    }

    #[test]
    fn parse_marker_size_reads_the_attribute_as_git_does() {
        for (value, size) in [
            ("9", 9),
            ("32", 32),
            ("+12", 12),
            ("12abc", 12),
            ("0", 7),
            ("-3", 7),
            ("set", 7),
            ("unset", 7),
            ("unspecified", 7),
            ("", 7),
        ] {
            assert_eq!(parse_marker_size(value), size, "{value}");
        }
    }

    #[test]
    fn hunks_reads_each_hunk_header() {
        let diff = b"diff --git a/1 b/2\nindex 1..2 100644\n--- a/1\n+++ b/2\n\
            @@ -0,0 +1,2 @@\n+x\n+y\n\
            @@ -3 +5 @@ fn main() {\n-a\n+b\n\
            @@ -7,2 +8,0 @@\n-c\n-d\n";
        assert_eq!(
            hunks(diff).unwrap(),
            [
                Hunk {
                    old: 0,
                    old_lines: 0,
                    new: 1,
                    new_lines: 2
                },
                Hunk {
                    old: 3,
                    old_lines: 1,
                    new: 5,
                    new_lines: 1
                },
                Hunk {
                    old: 7,
                    old_lines: 2,
                    new: 8,
                    new_lines: 0
                },
            ]
        );
        assert!(hunks(b"@@ -1 @@\n").is_err());
        assert!(hunks(b"@@ -1 +1\n").is_err());
        assert!(hunks(b"@@ -x +1 @@\n").is_err());
    }

    #[test]
    fn map_back_shifts_a_range_by_the_hunks_before_it() {
        let hunk = |old, old_lines, new, new_lines| Hunk {
            old,
            old_lines,
            new,
            new_lines,
        };
        // Two lines added above line 1 of the old side, one replaced by two at old line 5, two
        // deleted after new line 10.
        let hunks = [hunk(0, 0, 1, 2), hunk(5, 1, 7, 2), hunk(8, 2, 10, 0)];
        assert_eq!(map_back(&hunks, 3..5), 1..3);
        assert_eq!(map_back(&hunks, 6..6), 4..4);
        assert_eq!(map_back(&hunks, 9..10), 6..7);
        assert_eq!(map_back(&hunks, 10..11), 7..8);
        assert_eq!(map_back(&hunks, 11..12), 10..11);
        assert_eq!(map_back(&[], 3..5), 3..5);
    }
}
