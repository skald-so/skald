//! A machine's changed hunks: the lines each one changes, and the boundaries between lines of the
//! merge base it touches. Two machines' hunks that touch one boundary change the same or adjacent
//! lines of the merge base, or insert lines at one place: where `git merge-tree` conflicts.

use std::ops::Range;
use std::path::Path;

use crate::{Result, git};

/// A change of a file since the merge base: a run of changed lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The change's lines in this machine's version of the file, 1-based and end-exclusive. A
    /// change that only deletes lines has an empty range before the line that follows them.
    pub lines: Range<usize>,
    /// The lines of the merge base the change deletes or replaces, 1-based and end-exclusive. A
    /// change that only inserts lines has an empty range before the line that follows them.
    pub old: Range<usize>,
    /// Each boundary between two lines of the merge base that the change touches, in order: the
    /// lines on either side of it, each with its line ending, or empty at the start or the end of
    /// the file. An insertion touches the one boundary it goes into; a change of `n` lines of the
    /// merge base touches the `n + 1` boundaries around and between them.
    pub boundaries: Vec<(Vec<u8>, Vec<u8>)>,
}

/// The changes of `path` from the commit `base` to the commit `commit`, both in the repository at
/// `dir`, as `git diff` finds them with the diff algorithm of `git merge-tree`, histogram without
/// the indent heuristic. The options fix what the repository's attributes and config could change:
/// a binary diff without hunks, a text conversion, an external diff, the diff algorithm, colors, a
/// submodule's log in place of its `Subproject commit` line, and a hidden submodule.
/// `--literal-pathspecs` takes `path` as it is, not as a pattern that matches other paths. Hunks
/// that `diff.interHunkContext` joins read as the same changes, and a diff of one path finds no
/// rename.
pub fn changes(dir: &Path, base: &str, commit: &str, path: &str) -> Result<Vec<Change>> {
    let diff = git::run(
        dir,
        &[
            "--literal-pathspecs",
            "diff",
            "--unified=1",
            "--diff-algorithm=histogram",
            "--no-indent-heuristic",
            "--text",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--submodule=short",
            "--ignore-submodules=none",
            base,
            commit,
            "--",
            path,
        ],
    )?;
    parse(&diff)
}

/// The changes of a `--unified=1` diff. With one line of context, the line before a change is in
/// its hunk unless the change starts the file, and the line after it unless it ends the file.
fn parse(diff: &[u8]) -> Result<Vec<Change>> {
    let mut changes = Vec::new();
    let mut lines = diff.split_inclusive(|&byte| byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        let Some(text) = line.strip_prefix(b"@@ -") else {
            continue;
        };
        let Header {
            old,
            old_lines,
            new,
            new_lines,
        } = header(text)?;
        // The hunk's lines: each one's side, ` `, `-` or `+`, and its text with its line ending.
        let mut body: Vec<(u8, Vec<u8>)> = Vec::new();
        let (mut old_left, mut new_left) = (old_lines, new_lines);
        while old_left + new_left > 0 || lines.peek().is_some_and(|line| line[0] == b'\\') {
            let line = lines.next().ok_or("a hunk shorter than its header")?;
            let (side, text) = match line[0] {
                // `\ No newline at end of file`: the line before it has no line ending.
                b'\\' => {
                    let (_, text) = body.last_mut().ok_or("a hunk that starts with `\\`")?;
                    text.pop_if(|byte| *byte == b'\n');
                    continue;
                }
                // An empty line of context, as `diff.suppressBlankEmpty` writes it.
                b'\n' => (b' ', line),
                side @ (b' ' | b'-' | b'+') => (side, &line[1..]),
                _ => return Err(format!("a hunk line {:?}", String::from_utf8_lossy(line)).into()),
            };
            let short = "a hunk longer than its header";
            if side != b'+' {
                old_left = old_left.checked_sub(1).ok_or(short)?;
            }
            if side != b'-' {
                new_left = new_left.checked_sub(1).ok_or(short)?;
            }
            body.push((side, text.to_vec()));
        }
        // A side without lines comes after its line `old` or `new`.
        let mut next_old = if old_lines == 0 { old + 1 } else { old };
        let mut next = if new_lines == 0 { new + 1 } else { new };
        // Runs of context lines and runs of changed lines, in turn.
        let runs: Vec<_> = body
            .chunk_by(|a, b| (a.0 == b' ') == (b.0 == b' '))
            .collect();
        let mut before: &[u8] = b"";
        for (i, run) in runs.iter().enumerate() {
            if run[0].0 == b' ' {
                before = &run[run.len() - 1].1;
                next_old += run.len();
                next += run.len();
                continue;
            }
            let (start_old, start) = (next_old, next);
            let mut base = vec![before];
            for (side, text) in *run {
                match side {
                    b'-' => {
                        base.push(text);
                        next_old += 1;
                    }
                    _ => next += 1,
                }
            }
            base.push(runs.get(i + 1).map_or(&b""[..], |context| &context[0].1));
            changes.push(Change {
                lines: start..next,
                old: start_old..next_old,
                boundaries: base
                    .windows(2)
                    .map(|pair| (pair[0].to_vec(), pair[1].to_vec()))
                    .collect(),
            });
        }
    }
    Ok(changes)
}

/// A hunk header, `@@ -<old>[,<lines>] +<new>[,<lines>] @@`: the first line and the number of lines
/// on each side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Header {
    pub old: usize,
    pub old_lines: usize,
    pub new: usize,
    pub new_lines: usize,
}

/// The hunk header `header`, without its `@@ -`.
pub(crate) fn header(header: &[u8]) -> Result<Header> {
    let header = String::from_utf8_lossy(header);
    let side = |range: &str| -> Result<(usize, usize)> {
        Ok(match range.split_once(',') {
            Some((first, lines)) => (first.parse()?, lines.parse()?),
            None => (range.parse()?, 1),
        })
    };
    let (old, rest) = header
        .split_once(" +")
        .ok_or_else(|| format!("a hunk header without a new side: {header}"))?;
    let (new, _) = rest
        .split_once(" @@")
        .ok_or_else(|| format!("a hunk header without an end: {header}"))?;
    let ((old, old_lines), (new, new_lines)) = (side(old)?, side(new)?);
    Ok(Header {
        old,
        old_lines,
        new,
        new_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(lines: Range<usize>, old: Range<usize>, boundaries: &[(&str, &str)]) -> Change {
        Change {
            lines,
            old,
            boundaries: boundaries
                .iter()
                .map(|(before, after)| (before.as_bytes().to_vec(), after.as_bytes().to_vec()))
                .collect(),
        }
    }

    #[test]
    fn parse_reads_each_change_with_its_boundaries() {
        // Lines 1 to 9 of the merge base, `1` to `9`: line 2 replaced by two, a line inserted
        // after line 4, line 6 deleted, lines 8 and 9 replaced by one.
        let diff = b"diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n\
            @@ -1,7 +1,8 @@\n 1\n-2\n+a\n+b\n 3\n 4\n+c\n 5\n-6\n 7\n\
            @@ -7,3 +8,2 @@\n 7\n-8\n-9\n+d\n";
        assert_eq!(
            parse(diff).unwrap(),
            [
                change(2..4, 2..3, &[("1\n", "2\n"), ("2\n", "3\n")]),
                change(6..7, 5..5, &[("4\n", "5\n")]),
                change(8..8, 6..7, &[("5\n", "6\n"), ("6\n", "7\n")]),
                change(9..10, 8..10, &[("7\n", "8\n"), ("8\n", "9\n"), ("9\n", "")]),
            ]
        );
    }

    #[test]
    fn parse_reads_the_edges_of_the_file() {
        // A line inserted on top; the first line deleted; a file added; a file deleted.
        let diff = b"@@ -1 +1,2 @@\n+0\n 1\n@@ -1,2 +1 @@\n-1\n 2\n\
            @@ -0,0 +1,2 @@\n+x\n+y\n\
            @@ -1,2 +0,0 @@\n-x\n-y\n";
        assert_eq!(
            parse(diff).unwrap(),
            [
                change(1..2, 1..1, &[("", "1\n")]),
                change(1..1, 1..2, &[("", "1\n"), ("1\n", "2\n")]),
                change(1..3, 1..1, &[("", "")]),
                change(1..1, 1..3, &[("", "x\n"), ("x\n", "y\n"), ("y\n", "")]),
            ]
        );
    }

    #[test]
    fn parse_reads_lines_without_a_line_ending_and_empty_context() {
        // The last line loses its line ending; an empty line of context without its space.
        let diff = b"@@ -2,3 +2,3 @@\n\n-a\n+a\n b\n\\ No newline at end of file\n\
            @@ -5,2 +5,2 @@\n x\n-y\n\\ No newline at end of file\n+y\n";
        assert_eq!(
            parse(diff).unwrap(),
            [
                change(3..4, 3..4, &[("\n", "a\n"), ("a\n", "b")]),
                change(6..7, 6..7, &[("x\n", "y"), ("y", "")]),
            ]
        );
    }

    #[test]
    fn parse_reads_joined_hunks_as_the_hunks_they_join() {
        // Lines 2 and 6 changed: two hunks, or one that `diff.interHunkContext` joins.
        let apart = b"@@ -1,3 +1,3 @@\n 1\n-2\n+b\n 3\n@@ -5,3 +5,3 @@\n 5\n-6\n+f\n 7\n";
        let joined = b"@@ -1,7 +1,7 @@\n 1\n-2\n+b\n 3\n 4\n 5\n-6\n+f\n 7\n";
        assert_eq!(parse(joined).unwrap(), parse(apart).unwrap());
        assert_eq!(parse(apart).unwrap().len(), 2);
    }

    #[test]
    fn parse_rejects_malformed_hunks() {
        for diff in [
            &b"@@ -1 +1 @@\n-a\n"[..],
            b"@@ -1 +1 @@\n-a\n-b\n+c\n",
            b"@@ -1 +1 @@\n-a\n*b\n",
            b"@@ -1,0 +1 @@\n\\ No newline at end of file\n+a\n",
            b"@@ -1 +1\n",
            b"@@ -1 @@\n",
            b"@@ -x +1 @@\n",
        ] {
            assert!(parse(diff).is_err(), "{}", String::from_utf8_lossy(diff));
        }
    }
}
