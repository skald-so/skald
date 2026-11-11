# Edit comparison prototype

The edit comparison prototype ([roadmap.md, "Preparation"](roadmap.md#preparation-before-mvp)): machines learn that they edit the same thing from a server that sees only HMAC fingerprints.

- Code: [`prototypes/edits/`](../../prototypes/edits), a cargo workspace apart from the product crates. `edit-overlap` holds the fingerprints, the hunks, the client, the server stand-in, the exact check, the stand-in for end-to-end encrypted messages and the scenario harness.
- Harness: a test repository in a bare upstream; each machine is a local clone of it. Machines edit their working trees, commit and push branches; a maintainer's clone merges a branch with a merge commit. Everything runs through the `git` CLI. Its environment holds only `PATH`, a fixed identity and switches that turn off the global and system config and the system attributes; without `HOME` and `XDG_CONFIG_HOME` git finds no global ignore or attributes file. No credentials or config of the parent's environment reach it.
- Server stand-in: in-process, without the space key. It keeps each machine's latest fingerprints and tells a machine which other machines share fingerprints with it, and which; the machine maps them back to its paths and changed lines.
- Messages stand-in: in-process inboxes for the end-to-end encrypted messages between machines; they bypass the server stand-in.
- Run: `cargo test --manifest-path prototypes/edits/Cargo.toml`; it needs `git` ([CONTRIBUTING.md](../../CONTRIBUTING.md), "Requirements"). `script/test` runs it.

## Scenarios
| Scenario | Checks |
|---|---|
| `edit_set_holds_committed_uncommitted_and_untracked_changes` | The edit set holds a committed edit, a staged and then changed edit, both paths of an uncommitted rename and an untracked file; not an ignored file, nor another machine's change merged upstream and fetched but not merged here |
| `machines_editing_one_file_overlap` | Two machines edit one file, one committed and one not, and other files each: each learns the other and only that file |
| `machines_editing_different_files_do_not_overlap` | Machines editing different files learn of no overlap |
| `rename_overlaps_an_edit_of_either_path` | A committed rename overlaps an edit of its old path on one machine and a new file at its new path on another; those two do not overlap |
| `deletion_overlaps_an_edit_of_its_path` | A deletion overlaps an edit of the deleted path |
| `machine_whose_changes_are_merged_leaves_the_overlap_once_it_pulls` | A machine pushes a branch that a maintainer merges upstream: the overlap stays until the machine pulls, then its edit set is empty and the overlap is gone |
| `same_path_in_another_repository_or_under_another_key_does_not_overlap` | An edit of one path on two clones of one repository overlaps; the same edit in another repository with the same paths, or under another space key, overlaps nothing |
| `server_receives_and_keeps_no_path` | What the machines send and what the server keeps, the latest publication of each, hold no path of the scenario and no file or directory name |
| `snapshot_commits_the_working_tree_and_leaves_the_repository_as_it_is` | The snapshot holds committed, staged and then changed, and untracked changes and a deletion, not an ignored file; its parent is `HEAD`; the status, index, refs and `.git` entries stay as they were |
| `edits_to_the_same_lines_conflict_with_their_files_and_lines` | A committed edit and an uncommitted one of the same line, and a file both add, one committed and one untracked: both machines get the two files and their own conflicting lines, shifted on one by a clean change above |
| `conflicting_lines_are_found_whatever_the_repository_attributes_and_config` | The same-lines scenario in a repository whose `.gitattributes` sizes the conflict markers of Rust files to 32 characters and makes `git diff` show files as binary, with `diff.interHunkContext` set on one machine and the other's clean changes 2 lines above and below the conflict: both machines get their own conflicting lines |
| `conflicting_lines_are_found_after_an_unstaged_deletion_of_the_attributes` | A `.gitattributes` that sizes the conflict markers of Rust files to 32 characters, deleted on one machine without staging the deletion: that machine gets its conflicting lines from markers of 7 |
| `edits_to_distant_lines_of_one_file_merge_cleanly` | Edits of the first and last function of one file overlap on the server; the check finds a clean merge |
| `modify_delete_conflict_is_found` | A deletion and an edit of one file: a modify/delete conflict on both machines |
| `rename_conflicts_are_found` | A committed rename against another rename of the same file, its deletion and an edit of its old path: rename/rename, rename/delete and a clean merge |
| `machines_on_different_bases_get_the_right_result` | One machine pulled lines added on top and the other's merged change and edits it again; the other has not pulled. An edit both make conflicts at each one's own line; the merged change and the lines on top merge cleanly; without that edit the merge is clean |
| `a_request_with_a_base_that_is_not_an_object_id_is_refused` | A base that is not an object ID never reaches the `git bundle` command line |
| `a_bundle_with_another_reference_is_refused` | A bundle whose one reference is not the sender's changes reference is refused |
| `a_bundle_with_two_references_is_refused` | A bundle with the changes reference and another one is refused |
| `the_parent_environment_does_not_reach_git` | A fake sentinel variable, `HOME`, `XDG_CONFIG_HOME` and config injected through `GIT_CONFIG_COUNT` in the test's environment do not reach git; a directory added to its `PATH`, the fixed identity and the config, attributes and prompt switches do |
| `server_receives_and_keeps_nothing_of_the_exchanged_changes` | During the check of both machines the server receives nothing and keeps what it kept; nothing it received or keeps holds a path, file or directory name or line of the changes |
| `edits_to_the_same_lines_overlap_at_hunk_level_and_conflict` | Two lines edited on both machines, committed on one and uncommitted on the other, which also edits a distant line: each learns the other at hunk level with its two lines, the lines the check finds conflicting |
| `edits_next_to_each_other_or_at_one_place_overlap_at_hunk_level_and_conflict` | An edit of a line, an edit of the next line, and two different lines inserted between them: every pair overlaps at hunk level and conflicts |
| `edits_to_different_hunks_of_one_file_overlap_at_file_level_only` | Edits of lines 7 and 9, one unchanged line apart, and of line 2: every pair overlaps at file level only and merges cleanly |
| `bases_that_differ_by_lines_added_above_the_hunk_still_match` | Lines added on top upstream, pulled by one machine only, and an edit of one line on both: they overlap at hunk level, each at its own line, and conflict there |
| `hunks_follow_the_diff_of_git_merge_tree_whatever_the_repository_attributes_and_config` | Edits that the histogram and Myers diffs, or diffs without and with the indent heuristic, place differently, in a repository whose `.gitattributes` makes files binary or text-converted, with an external diff and no context lines in one machine's config and empty context lines without their space in the other's: the hunks overlap in each file where `git merge-tree` conflicts |
| `hunk_fingerprints_match_without_a_conflict_on_common_lines_and_identical_edits` | Lines inserted between `}` and an empty line after two different functions, and the same edit on two machines: they overlap at hunk level and merge cleanly |
| `conflicts_with_upstream_commits_only_one_machine_has_do_not_match_at_hunk_level` | A line changed upstream and pulled by one machine, which edits another line, and an edit of that line on the other: they overlap at file level only; the check finds the conflict |
| `server_receives_no_line_of_the_test_repository` | What the machines send holds no line of the test repository or of their edits |

## Questions
Open questions of edit comparison, and how the prototype answers them:
| Question | How |
|---|---|
| Data source: diff, hooks or declarations | Open; the prototype uses the diff so far |
| When to warn | Warnings at each level against `git merge-tree` conflicts |
| Reservation TTL | How long each side of a merge lived |
| How to catch edits made via Bash and generators | Open |
| Path normalization | Open |
| Noise from lockfiles and migrations | Open |
| Whether intents like "replacing / extending" are needed | Whether both sides of a warning only add lines |

## File fingerprints
- **Edit set:** `git diff --no-renames --name-only <merge base>` against the working tree, and `git ls-files --others --exclude-standard`.
  - The merge base is of `HEAD` and `refs/remotes/origin/HEAD`, the upstream's default branch that `git clone` records.
  - A machine leaves an overlap only once it fetches the merge of its changes; until then the server keeps its old fingerprints.
  - `--no-renames` lists a rename as a deletion and an addition, so both paths count without rename detection and its similarity threshold.
  - Ignored files, such as build output, are not edits.
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit path"`, the scope and the path, each after its length as a big-endian `u64`; 32 bytes per path. The path is as git gives it: relative to the root, with `/` separators, not normalized.
- **Scope:** the least root commit of the upstream's default branch.
  - Every clone has it without setup. It goes only into the HMAC; the server never sees it.
  - Rejected: the remote URL, which differs between HTTPS and SSH and for mirrors; a random ID per repository, which needs a step on each machine.
  - Forks and mirrors share it, so their machines overlap.
  - Repositories with byte-identical root commits share it: the same tree, message, author and time.
  - A shallow clone takes its shallow boundary for the root, so it gets another scope than a full clone. Open: a scope for shallow clones.
- **Publication:** each machine sends the fingerprints of its whole edit set and its [hunks](#hunk-fingerprints), which replace its earlier ones. The server sees the machine, how many fingerprints it sends and when that changes, and which machines share fingerprints.

## Exact check
- **Exchange:** on an overlap, a machine asks the other for its changes over an end-to-end encrypted message; the request carries the asker's merge base with the upstream's default branch.
- **What the other machine sends:** a git bundle of a snapshot of its working tree, a commit on top of `HEAD`, with its history since the asker's base.
  - The snapshot holds committed, uncommitted and untracked changes, not ignored files: `git add --all` into a copy of the index, `write-tree`, `commit-tree -p HEAD`. The index, the working tree and the refs stay as they are; its objects stay unreferenced until `git gc`.
  - A sender that lacks the asker's base, which has pulled newer upstream commits, bundles since its own base, an ancestor of the asker's. A sender whose base is newer also sends the upstream commits between the two.
  - The sender checks that the asker's base is an object ID before it goes into a `git bundle` command line.
  - Git writes the bundle to its standard output; it never reaches a file. The asker writes it to a temporary file only its user can read.
  - The asker stores the bundle's objects with `git bundle unbundle`. That fails when the asker lacks the bundle's base, such as after a force-push to the default branch.
  - Size: bundles are thin packs, so changed files go as deltas against the base. In a repository of 40 files of 28 KB: a one-line uncommitted edit, 454 bytes; one line in each of 10 files, committed, 1.5 KB; with a new untracked 109 KB file, 45 KB: new files go whole, compressed.
- **Check:** `git merge-tree --write-tree -z --messages` on the asker's snapshot and the other's, from their merge base. Git 2.39 is the first whose `-z` messages give each conflict's type and paths.
  - Result: whether the merge is clean, by the exit status; each `CONFLICT` message's type and paths; each conflicted file with the conflicting ranges of the asker's own lines.
  - Lines: the asker's side of each conflict marker in the merged file. Outside the conflicts the merged file has the other side's clean changes too; a `git diff` from the asker's version to the merged file resolved to the asker's side maps the ranges back.
  - Markers are as long as the file's `conflict-marker-size` attribute sets, 7 by default. Each conflict's size is the run of `<` that starts it in the merged file: `git check-attr` falls back to the index's `.gitattributes` when the working tree has none, `git merge-tree` does not. The `git diff` options fix what attributes and config could change: `--text`, `--inter-hunk-context=0`, `--indent-heuristic`.
  - Types: content conflicts, a file both sides add included, are `CONFLICT (contents)`; also `modify/delete`, `rename/rename` and `rename/delete`. Conflicts without markers have no lines.
  - A rename and an edit of its old path merge cleanly: the file-level overlap is a false alarm the check clears.
- **Different bases:** the snapshots merge from the latest commit both have.
  - Upstream commits only one machine has pulled count as its changes. Lines they add shift the conflicts; each machine gets them in its own lines.
  - A machine's commits merged upstream with a merge commit are in the other's history once it pulls, so they conflict with nothing. After a squash or rebase merge they are not: an edit of the same lines on the other machine conflicts until the first drops its commits.
  - A conflict between one machine's edits and upstream commits only the other has pulled shows as a conflict between the machines.
- **Server:** receives nothing during the check; the check takes no server stand-in.
- **A shared git remote instead:** it could carry the changes: each machine pushes its snapshot to a ref of its own, such as `refs/skald/<machine>`, and the asker fetches it.
  - For: the other machine need not be online; fetch negotiation sends only missing objects.
  - Against: uncommitted work reaches the git host, which end-to-end encryption keeps from the server; every machine needs push rights; snapshot refs need cleanup; forks and mirrors share a scope but not a remote.

## Hunk fingerprints
- **Hunks:** `git diff --unified=1` from the merge base to the snapshot of the exact check, for each path of the edit set.
  - The diff is the one `git merge-tree` uses: histogram, without the indent heuristic. `git diff` defaults to Myers with the indent heuristic; both place some hunks elsewhere.
  - Options fix what attributes and config could change: `--text`, `--no-textconv`, `--no-ext-diff`, `--unified=1`. Empty context lines without their space, which `diff.suppressBlankEmpty` writes, are read as context.
- **Boundaries:** a hunk's fingerprints stand for the boundaries between lines of the merge base it touches.
  - An insertion touches the boundary it goes into; a change of `n` lines touches the `n + 1` boundaries around and between them.
  - Two hunks share a boundary when they change the same or adjacent lines, or insert at one place: where `git merge-tree` conflicts. Changes with one unchanged line between them merge cleanly and share none.
  - An added file touches the one boundary of an empty file, so two machines adding one path match; a deleted file touches all of its boundaries.
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit hunk"`, the scope, the path, and the lines on either side of the boundary with their line endings, each after its length as a big-endian `u64`; empty at the start or the end of the file. No line numbers: lines added elsewhere in the file keep the fingerprint.
- **Publication:** in one set with the path fingerprints. The server compares them alike and cannot tell them apart; a hunk match comes with a path match, since the path is in the HMAC. Each changed line of the merge base adds a fingerprint, so the server learns the size of the edits.
- **Mapping back:** a machine maps shared hunk fingerprints to its own changed lines. On the same-lines scenarios these are the lines the exact check finds conflicting.
- **False matches:**
  - Common lines: the lines on either side of a boundary recur in the file, such as `}` before an empty line between functions; insertions after two functions match.
  - Identical edits on both machines: `git merge-tree` merges them cleanly.
- **Misses:** a conflict with upstream commits only one machine has pulled, on the conflicting lines or next to them. Each machine fingerprints its own changes since its own base; the file still matches. The conflict is with upstream, and the other machine meets it when it pulls.
- **Cost:** each publication takes a snapshot, whose changed files stay as loose objects until `git gc`, and one `git diff` per path. Cost on real repositories: open.
