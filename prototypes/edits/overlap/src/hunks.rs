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
    /// Each boundary between two lines of the merge base that the change touches, in order: the
    /// lines on either side of it, each with its line ending, or empty at the start or the end of
    /// the file. An insertion touches the one boundary it goes into; a change of `n` lines of the
    /// merge base touches the `n + 1` boundaries around and between them.
    pub boundaries: Vec<(Vec<u8>, Vec<u8>)>,
}

/// The changes of `path` from the commit `base` to the commit `commit`, both in the repository at
/// `dir`, as `git diff` finds them with the diff algorithm of `git merge-tree`, histogram without
/// the indent heuristic. The options fix what the repository's attributes and config could change:
/// a binary diff without hunks, a text conversion, an external diff and the diff algorithm.
pub fn changes(dir: &Path, base: &str, commit: &str, path: &str) -> Result<Vec<Change>> {
    let diff = git::run(
        dir,
        &[
            "--literal-pathspecs",
            "diff",
            "--unified=1",
            "--inter-hunk-context=0",
            "--no-renames",
            "--diff-algorithm=histogram",
            "--no-indent-heuristic",
            "--text",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
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
        let Some(header) = line.strip_prefix(b"@@ -") else {
            continue;
        };
        let (old_lines, new, new_lines) = header_counts(header)?;
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
        // A hunk without new lines comes after its line `new`.
        let mut next = if new_lines == 0 { new + 1 } else { new };
        let mut before: &[u8] = b"";
        let mut rest = &body[..];
        while let Some(((side, text), tail)) = rest.split_first() {
            if *side == b' ' {
                before = text;
                next += 1;
                rest = tail;
                continue;
            }
            let end = rest
                .iter()
                .position(|(side, _)| *side == b' ')
                .unwrap_or(rest.len());
            let (run, tail) = rest.split_at(end);
            let start = next;
            let mut base = vec![before];
            for (side, text) in run {
                match side {
                    b'-' => base.push(text),
                    _ => next += 1,
                }
            }
            base.push(tail.first().map_or(&b""[..], |(_, text)| text));
            changes.push(Change {
                lines: start..next,
                boundaries: base
                    .windows(2)
                    .map(|pair| (pair[0].to_vec(), pair[1].to_vec()))
                    .collect(),
            });
            rest = tail;
        }
    }
    Ok(changes)
}

/// The number of old lines, the first new line and the number of new lines of a hunk header,
/// `@@ -<old>[,<lines>] +<new>[,<lines>] @@`, without its `@@ -`.
fn header_counts(header: &[u8]) -> Result<(usize, usize, usize)> {
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
    let ((_, old_lines), (new, new_lines)) = (side(old)?, side(new)?);
    Ok((old_lines, new, new_lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(lines: Range<usize>, boundaries: &[(&str, &str)]) -> Change {
        Change {
            lines,
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
                change(2..4, &[("1\n", "2\n"), ("2\n", "3\n")]),
                change(6..7, &[("4\n", "5\n")]),
                change(8..8, &[("5\n", "6\n"), ("6\n", "7\n")]),
                change(9..10, &[("7\n", "8\n"), ("8\n", "9\n"), ("9\n", "")]),
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
                change(1..2, &[("", "1\n")]),
                change(1..1, &[("", "1\n"), ("1\n", "2\n")]),
                change(1..3, &[("", "")]),
                change(1..1, &[("", "x\n"), ("x\n", "y\n"), ("y\n", "")]),
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
                change(3..4, &[("\n", "a\n"), ("a\n", "b")]),
                change(6..7, &[("x\n", "y"), ("y", "")]),
            ]
        );
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
