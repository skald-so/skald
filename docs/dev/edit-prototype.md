# Edit comparison prototype

The edit comparison prototype ([roadmap.md, "Preparation"](roadmap.md#preparation-before-mvp)): machines learn that they edit the same thing from a server that sees only HMAC fingerprints.

- Code: [`prototypes/edits/`](../../prototypes/edits), a cargo workspace apart from the product crates. `edit-overlap` holds the fingerprints, the hunks, the symbols, the client, the server stand-in, the exact check, the stand-in for end-to-end encrypted messages, the scenario harness and the [measurement](#measurement)'s replay of real merges, with its command, `measure`.
- Harness: a test repository in a bare upstream; each machine is a local clone of it. Machines edit their working trees, commit and push branches; a maintainer's clone merges a branch with a merge commit. The harness's commits carry a fixed date, so they get the same hashes on every run; the client's snapshot commits keep the current time. Everything runs through the `git` CLI. Its environment holds only `PATH`, a fixed identity, for the harness's commands the fixed date, and switches that turn off the global and system config and the system attributes; without `HOME` and `XDG_CONFIG_HOME` git finds no global ignore or attributes file. No credentials or config of the parent's environment reach it.
- Server stand-in: in-process, without the space key. It keeps each machine's latest fingerprints and tells a machine which other machines share fingerprints with it, and which; the machine maps them back to its paths, changed lines and symbols.
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
| `machines_on_different_upstream_commits_share_the_scope` | Upstream commits until one sorts before the root commit; one machine pulls them and the other does not: an edit of one path on both overlaps |
| `edit_set_holds_a_submodule_when_its_commit_moves_not_when_its_files_change` | A submodule checked out at its commit with a tracked file changed is not in the edit set; once its commit moves, it is |
| `server_receives_and_keeps_no_path` | What the machines send and what the server keeps, the latest publication of each, hold no path of the scenario and no file or directory name |
| `snapshot_commits_the_working_tree_and_leaves_the_repository_as_it_is` | The snapshot holds committed, staged and then changed, and untracked changes and a deletion, not an ignored file; its parent is `HEAD`; the status, index, refs and `.git` entries stay as they were |
| `snapshot_takes_an_edit_that_keeps_the_size_and_time_of_the_file` | An edit of the same size whose file and index keep the old modification time the index holds for it: the snapshot holds the edit |
| `edits_to_the_same_lines_conflict_with_their_files_and_lines` | A committed edit and an uncommitted one of the same line, and a file both add, one committed and one untracked: both machines get the two files and their own conflicting lines, shifted on one by a clean change above |
| `conflicting_lines_are_found_whatever_the_repository_attributes_and_config` | The same-lines scenario in a repository whose `.gitattributes` sizes the conflict markers of Rust files to 32 characters and makes `git diff` show files as binary. One machine sets `diff.interHunkContext`, `color.diff=always`, a failing `diff.external` and `diff.algorithm=histogram`, keeps the default indent heuristic and adds a line on top; the other has `git diff` convert every file with `sort`, adds a line 2 lines above the conflict and changes one 2 lines below it. More conflicts sit on repeated lines, which the histogram diff or the indent heuristic maps to another copy, and on an empty side between clean changes 2 lines apart, which a wider inter-hunk context joins: both machines get their own conflicting lines |
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
| `edits_to_different_hunks_of_one_file_do_not_overlap_at_hunk_level` | Edits of lines 7 and 9 of one function, one unchanged line apart, and of line 2 of another: every pair overlaps at file level, not at hunk level, and merges cleanly; the first two overlap at symbol level |
| `bases_that_differ_by_lines_added_above_the_hunk_still_match` | Lines added on top upstream, pulled by one machine only, and an edit of one line on both: they overlap at hunk level, each at its own line, and conflict there |
| `hunks_follow_the_diff_of_git_merge_tree_whatever_the_repository_attributes_and_config` | Edits that the histogram diff places differently from the Myers or patience diff, or diffs without and with the indent heuristic place differently, and edits of `src/[id].rs` and of `src/i.rs`, which it matches as a glob, in a repository whose `.gitattributes` makes files binary or text-converted, with an external diff, no context lines and `color.diff=always` in one machine's config and empty context lines without their space in the other's: the hunks overlap in each file where `git merge-tree` conflicts, and only there |
| `submodule_moves_overlap_at_hunk_level_whatever_the_repository_config` | A submodule that `.gitmodules` sets to `ignore = all`, moved to different commits on two machines, committed on one with `diff.submodule=log` and staged on the other with `diff.ignoreSubmodules=all`: each sees the other at hunk level on the submodule's line, and `git merge-tree` reports `CONFLICT (submodule)` |
| `hunk_fingerprints_match_without_a_conflict_on_common_lines_and_identical_edits` | Lines inserted between `}` and an empty line after two different functions, and the same edit on two machines: they overlap at hunk level and merge cleanly |
| `rename_matches_an_edit_of_its_old_path_at_hunk_level_and_merges_cleanly` | A committed rename and an edit of the last line of its old path: the editor sees the renamer at hunk level on that line, the renamer sees the editor on its deletion; they merge cleanly |
| `conflicts_with_upstream_commits_only_one_machine_has_do_not_match_at_hunk_level` | A line changed upstream and pulled by one machine, which edits another line, and an edit of that line on the other: they overlap at file level only; the check finds the conflict |
| `server_receives_no_line_of_the_test_repository` | What the machines send holds no line of the test repository or of their edits |
| `<language>::edits_to_different_functions_of_one_file_overlap_at_file_level_only` | A line of the function and one of the method of its name: they overlap at file level only |
| `<language>::edits_to_distant_lines_of_one_function_overlap_at_symbol_level_and_merge_cleanly` | The first and the last line of a method's body: they overlap at symbol level, on the method named with its type, and merge cleanly |
| `<language>::edits_to_two_methods_of_one_type_do_not_overlap_at_symbol_level` | A line of each method: they overlap at file level only |
| `<language>::a_symbol_moved_within_its_file_keeps_its_fingerprint` | The function moved to the end of its file upstream, pulled by one machine only, and a line of it edited on both: they overlap at symbol level |
| `<language>::a_file_in_another_language_gets_no_symbol_fingerprints` | Distant lines of one function edited in the file and in a Go file: only the first has symbols; the machines share the fingerprints of the two paths and of the method, nothing of the Go function |
| `<language>::server_receives_no_symbol_name` | What the machines send holds no kind or name of their changed symbols |
| `insertions_deletions_and_added_and_deleted_files_get_symbol_fingerprints` | In Rust, a line inserted into a method's body and a deleted file on one machine, a line deleted from it and an added file on the other: each gets the symbols of its changes, the deleted ones from the merge base; they overlap at symbol level on the method and merge cleanly |
| `merges_are_those_of_two_parents_newest_first` | The [measurement](#measurement)'s fixture: an octopus merge, then 12 merges of two parents, each of a branch with one commit. The merges listed are the 12, not the octopus merge; with a limit of 1, the last |
| `replay_gives_the_expected_counts_at_each_level` | The fixture's pairs of branches, each merged in turn, the second resolved to the default branch's side: different files; the same line of a function; distant lines of a function; different functions of one file and distant lines of another; different lines inserted at one place in a function; the same line outside every function. Replayed on 3 threads, in the order of the merges: the counts of each level |
| `replay_gives_each_merges_warnings_conflicts_and_sides` | On the fixture: each merge's warned files at each level, its conflicts, and each side's lifetime from the author date of its first commit to the merge |
| `report_gives_the_tables_of_the_replays` | The fixture's tables: its range of merges, the counts, no conflict without a file-level warning, the lifetimes, and the sizes of its 3 largest sides with the times of a timing run |
| `merges_with_several_merge_bases_are_counted` | Two branches that each merge the other before both are merged: the last merge has two merge bases, the replay takes the first that `git merge-base --all` gives and warns about the file both sides change the same way from it, and the table counts the merge |
| `replaying_a_commit_that_is_not_a_merge_fails` | A commit of one parent among the merges to replay: the replay fails |
| `report_gives_a_repository_without_merges_and_lifetimes` | A repository without merges: its tables give no range, lifetimes or largest sides |
| `a_conflict_in_a_renamed_file_has_no_file_level_warning` | One side renames a file and edits a line of it, the other edits that line at the old path: the warning is on the old path, the conflict on the new one, and the table of conflicts without a file-level warning lists it |
| `the_same_change_on_both_sides_is_counted` | Both sides make one change to a function and different changes to distant lines of another file: the merge is clean, and the function's file is counted as the same change at each level |
| `measure_fetches_a_repository_at_its_commit_and_replays_its_merges` | The fixture's upstream fetched at the merge before its last: the merges replayed are those of that commit, and its 3 largest sides are timed; asking for one merge more than its history has fails |
| `the_header_names_the_machine_and_the_git_version` | The lines before the tables: the operating system, the architecture and the number of CPUs, on macOS the CPU that `sysctl` names, and `git version` |

`<language>`: `rust`, `typescript` and `python`, each on a file in that language with a function, and a type with two methods, the first named as the function.

## Questions
Open questions of edit comparison, and how the prototype answers them:
| Question | How |
|---|---|
| Data source: diff, hooks or declarations | Open; the prototype uses the diff so far |
| When to warn | [Measurement](#measurement): warnings at each level against `git merge-tree` conflicts |
| Reservation TTL | [Measurement](#measurement): how long each side of a merge lived |
| How to catch edits made via Bash and generators | Open |
| Path normalization | Open |
| Noise from lockfiles and migrations | Open |
| Whether intents like "replacing / extending" are needed | [Measurement](#measurement): whether both sides of a warning only add lines |

## File fingerprints
- **Edit set:** `git diff --no-renames --ignore-submodules=dirty --name-only <merge base>` against the working tree, and `git ls-files --others --exclude-standard`.
  - The merge base is of `HEAD` and `refs/remotes/origin/HEAD`, the upstream's default branch that `git clone` records.
  - A machine leaves an overlap only once it fetches the merge of its changes; until then the server keeps its old fingerprints.
  - `--no-renames` lists a rename as a deletion and an addition, so both paths count without rename detection and its similarity threshold.
  - `--ignore-submodules=dirty`: a submodule counts when its commit changes, whatever the config and `.gitmodules` say, not when its work tree does; the snapshot records its commit only.
  - Ignored files, such as build output, are not edits.
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit path"`, the scope and the path, each after its length as a big-endian `u64`; 32 bytes per path. The path is as git gives it: relative to the root, with `/` separators, not normalized.
- **Scope:** the least root commit of the upstream's default branch.
  - Every clone has it without setup. It goes only into the HMAC; the server never sees it.
  - Rejected: the remote URL, which differs between HTTPS and SSH and for mirrors; a random ID per repository, which needs a step on each machine.
  - Forks and mirrors share it, so their machines overlap.
  - Repositories with byte-identical root commits share it: the same tree, message, author and time.
  - A shallow clone takes its shallow boundary for the root, so it gets another scope than a full clone. Open: a scope for shallow clones.
- **Publication:** each machine sends the fingerprints of its whole edit set, its [hunks](#hunk-fingerprints) and its [symbols](#symbol-fingerprints), which replace its earlier ones. The server sees the machine, how many fingerprints it sends and when that changes, and which machines share fingerprints.

## Exact check
- **Exchange:** on an overlap, a machine asks the other for its changes over an end-to-end encrypted message; the request carries the asker's merge base with the upstream's default branch.
- **What the other machine sends:** a git bundle of a snapshot of its working tree, a commit on top of `HEAD`, with its history since the asker's base.
  - The snapshot holds committed, uncommitted and untracked changes, not ignored files: `git add --all` into a copy of the index, `write-tree`, `commit-tree -p HEAD`. The index, the working tree and the refs stay as they are; its objects stay unreferenced until `git gc`.
    - The copy keeps the index's modification time. A newer copy, as `std::fs::copy` makes on Linux, makes git trust the stat data of an edit of the same size made within the timestamp's resolution after the index was written, and the snapshot misses it.
  - A sender that lacks the asker's base, which has pulled newer upstream commits, bundles since its own base, an ancestor of the asker's. A sender whose base is newer also sends the upstream commits between the two.
  - The sender checks that the asker's base is an object ID before it goes into a `git bundle` command line.
  - Git writes the bundle to its standard output; it never reaches a file. The asker writes it to a temporary file only its user can read.
  - The asker stores the bundle's objects with `git bundle unbundle`. That fails when the asker lacks the bundle's base, such as after a force-push to the default branch.
  - Size: bundles are thin packs, so changed files go as deltas against the base. In a repository of 40 files of 28 KB: a one-line uncommitted edit, 454 bytes; one line in each of 10 files, committed, 1.5 KB; with a new untracked 109 KB file, 45 KB: new files go whole, compressed.
- **Check:** `git merge-tree --write-tree -z --messages` on the asker's snapshot and the other's, from their merge base. Git 2.39 is the first whose `-z` messages give each conflict's type and paths.
  - Result: whether the merge is clean, by the exit status; each `CONFLICT` message's type and paths; each conflicted file with the conflicting ranges of the asker's own lines.
  - Lines: the asker's side of each conflict marker in the merged file. Outside the conflicts the merged file has the other side's clean changes too; a `git diff` from the asker's version to the merged file resolved to the asker's side maps the ranges back.
  - Markers are as long as the file's `conflict-marker-size` attribute sets, 7 by default. Each conflict's size is the run of `<` that starts it in the merged file: `git check-attr` falls back to the index's `.gitattributes` when the working tree has none, `git merge-tree` does not. The `git diff` options fix what attributes, which reach a diff of two blobs through patterns such as `*`, and config could change: `--text`, `--no-textconv`, `--no-ext-diff`, `--inter-hunk-context=0`, `--no-color`, `--diff-algorithm=myers`, `--no-indent-heuristic`. On repeated lines the algorithm and the indent heuristic decide which copy a hunk takes; Myers without the indent heuristic maps the scenarios' conflicts to the lines `git merge-tree` merged, histogram or the indent heuristic does not.
  - Types: content conflicts, a file both sides add included, are `CONFLICT (contents)`; also `modify/delete`, `rename/rename` and `rename/delete`. Conflicts without markers have no lines.
  - Submodules: a `CONFLICT (submodule)` fails the check, which reads each conflicted path as a file with `git cat-file blob`. Open.
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
  - Options fix what attributes and config could change: `--text`, `--no-textconv`, `--no-ext-diff`, `--unified=1`, `--no-color`, `--submodule=short`, `--ignore-submodules=none`. Empty context lines without their space, which `diff.suppressBlankEmpty` writes, are read as context. Hunks that `diff.interHunkContext` joins read as the same changes, so it needs no option.
  - `--literal-pathspecs`: a path with glob characters, such as `src/[id].rs`, gets only its own hunks, not those of the paths it matches as a glob.
  - Other config changes nothing the parser reads: prefixes and `core.quotePath` change only the lines before the hunks, `diff.relative` nothing at the root, `diff.orderFile` and `diff.renames` nothing in a diff of one path.
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
  - Renames: a rename deletes its old path, which touches each of its boundaries, so it matches any edit of the old path. `git merge-tree` carries the edit into the renamed file and merges cleanly.
- **Misses:** a conflict with upstream commits only one machine has pulled, on the conflicting lines or next to them. Each machine fingerprints its own changes since its own base; the file still matches. The conflict is with upstream, and the other machine meets it when it pulls.
- **Cost:** each publication takes a snapshot, whose changed files stay as loose objects until `git gc`, and one `git diff` per path. Cost on real repositories: [Measurement](#measurement).

## Symbol fingerprints
- **Symbols:** functions, methods and types, found with a [tree-sitter](https://tree-sitter.github.io) grammar of the file's language, which its extension gives. A symbol is a syntax node of a kind its language lists, with its name; it is named with the symbols that enclose it, outermost first, such as `impl_item Store > function_item reserve`.
- **Changed lines:** each line a hunk adds or changes in the snapshot, and each line it deletes or changes in the merge base, gets the innermost symbol enclosing it in that version of the file. Lines outside every symbol get none.
- **Languages:**

  | Language | Extension | Grammar | Symbols |
  |---|---|---|---|
  | Rust | `.rs` | `tree-sitter-rust` | Functions, trait method declarations, structs, enums, unions, type aliases, traits, modules; `impl` blocks, named by their trait and type, such as `fmt::Display for Box<T>` |
  | TypeScript | `.ts` | `tree-sitter-typescript` | Functions, generators, classes, abstract classes, methods, abstract and interface method signatures, interfaces, enums, type aliases, namespaces; variables and class fields that hold an arrow function, a function or a generator expression |
  | Python | `.py` | `tree-sitter-python` | Functions, classes |

  - Each language needs its grammar, compiled from C at build time, and the list of node kinds that are symbols with the field that names each.
  - Other files, `.tsx` and `.go` among them, get path and hunk fingerprints only.
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit symbol"`, the scope, the path, and the node kind and the name of each symbol from the outermost, each after its length as a big-endian `u64`. No line numbers or content: a symbol moved within its file keeps its fingerprint.
- **Publication:** in one set with the path and hunk fingerprints. The server cannot tell them apart: each changed symbol adds to the total it sees.
- **Matches without a conflict:**
  - Edits to distant lines of one function, which `git merge-tree` merges cleanly: what this level is for.
  - Symbols of one name and kind in one file under the same enclosing symbols, such as a TypeScript getter and setter, share a fingerprint.
- **Misses:**
  - Lines written above an item, such as outer doc comments, outer attributes and decorators, are part of its symbol only where the grammar puts them in the item's node; otherwise they count towards the enclosing symbol, or none. The cases checked for each language are the unit tests of [`symbols.rs`](../../prototypes/edits/overlap/src/symbols.rs).
  - Closures, lambdas and callbacks are not symbols: their lines count towards the symbol around them. A TypeScript test file of `describe` and `it` callbacks has none.
  - Edits outside every symbol, such as imports and module-level constants, match at file and hunk level only.
  - A type's fields and its methods are different symbols; so are a Rust struct and its `impl` block.
- **Open:** a symbol moved to another file, which gets another fingerprint; files with syntax errors, as during an edit; a line that holds two symbols, which counts towards the last; more languages and extensions, such as `.tsx` and JavaScript.
- **Cost:** at most two parses of each changed file in a supported language, one of each version, the merge base and the snapshot, that holds changed lines. Cost on real repositories: [Measurement](#measurement).

## Measurement
How often the warnings of each level match the conflicts of real merges.

- **Command:** `cargo run --release --manifest-path prototypes/edits/Cargo.toml --bin measure`; it needs `git` and HTTPS access to GitHub, no account. It fetches each repository at its commit into a temporary directory, deleted at the end, also on an error; replays its merges on every CPU; then times its largest sides one after another. It prints the tables below in Markdown, with the machine and the git version; progress goes to the standard error. A rerun gives the same tables, times aside. `script/test` does not run it; it runs the replay on a fixture repository ([Scenarios](#scenarios)).
- **Repositories:** public ones that keep merge commits, one per language with symbol fingerprints:
  - `diesel`, Rust: `Cargo.lock` files, the SQL migrations of its examples and tests, and schema code that Diesel CLI generates, checked in as its examples' `schema.rs` and as test snapshots.
  - `drizzle-orm`, TypeScript: `pnpm-lock.yaml`, and the SQL migrations and snapshots drizzle-kit generates, checked in for its tests.
  - `netbox`, Python: Django migrations, `yarn.lock`, and the bundled scripts and styles of `netbox/project-static/dist/`.
- **Merges:** the 300 first merges of two parents that `git rev-list --merges` lists from the repository's commit: every merge of its history, not only those on the default branch's first-parent line, such as merges of the default branch into a topic branch. Octopus merges are left out.
- **Replay:** each parent is a machine; its edit set is the paths `git diff --no-renames` finds from the parents' merge base to the parent, and its fingerprints are those of the client, with the parent's commit for the snapshot. Both publish to a server stand-in. `git merge-tree --write-tree` merges the two parents.
  - The merge base is the first one `git merge-base --all` gives; `git merge-tree` merges from all of them.
- **Counts:**
  - A warning: a file both sides publish a fingerprint of the level of.
  - A conflict: a `CONFLICT` message of `git merge-tree`, with the files it names. It has a warning when the level warns about one of them.
  - A warning has a conflict when its file is in a conflict of the merge.
  - In merges: a merge with a warning, and one that `git merge-tree` does not merge cleanly.
- **Lifetimes:** of each side with commits after the merge base, from the author date of its first commit after the merge base along its first-parent history to the merge's committer date.
- **Only additions:** a warning where neither side deletes or replaces a line of the merge base in its file.
- **Same change:** a warning whose file is the same in both parents.
- **Cost:** the time to compute a side's fingerprints from its commits, without the snapshot a machine takes, and their size in bytes, for the 3 sides with the most fingerprints of each repository.

### Results
Machine: macos aarch64, Apple M3 Max, 14 CPUs
Git: git version 2.50.1 (Apple Git-155)

Repositories

| Repository | URL | Commit | Merges | Newest | Oldest | Both sides changed | Several merge bases |
|---|---|---|---|---|---|---|---|
| diesel | https://github.com/diesel-rs/diesel | `31afd783aeddb2471bba2bece823816fe4925fed` | 300 | `31afd783aedd` 2026-11-11 | `141e61d4c6c6` 2026-05-19 | 199 | 0 |
| drizzle-orm | https://github.com/drizzle-team/drizzle-orm | `bea7dd3623279ec2a12ca6c838cf5c8b17d3de42` | 300 | `bea7dd362327` 2026-11-10 | `b779e8bcc086` 2024-06-04 | 175 | 6 |
| netbox | https://github.com/netbox-community/netbox | `cf6d3f0319fade2fa0f1c51155f1c11330e58ff3` | 300 | `9b5d26700dd1` 2026-11-09 | `cd10087b2b61` 2025-03-19 | 256 | 0 |

Warnings and conflicts, in files

| Repository | Level | Warnings | Conflicts | Warnings without a conflict | Conflicts without a warning | Warnings where both sides only add | Of them with a conflict | Warnings where both sides made the same change |
|---|---|---|---|---|---|---|---|---|
| diesel | file | 679 | 203 | 479 | 3 | 113 | 33 | 166 |
| diesel | hunk | 467 | 203 | 267 | 3 | 58 | 33 | 166 |
| diesel | symbol | 180 | 203 | 119 | 142 | 7 | 3 | 49 |
| drizzle-orm | file | 1265 | 174 | 1094 | 3 | 526 | 33 | 866 |
| drizzle-orm | hunk | 1107 | 174 | 936 | 3 | 505 | 33 | 866 |
| drizzle-orm | symbol | 790 | 174 | 695 | 79 | 303 | 19 | 592 |
| netbox | file | 1119 | 438 | 726 | 0 | 128 | 14 | 162 |
| netbox | hunk | 693 | 438 | 300 | 0 | 102 | 14 | 162 |
| netbox | symbol | 365 | 438 | 252 | 325 | 54 | 5 | 110 |

Conflicts without a file-level warning

| Repository | Merge | Merge bases | Conflict | Files |
|---|---|---|---|---|
| diesel | `bb9ad76bf82b` | 1 | CONFLICT (contents) | `diesel/src/mysql_like/types/json.rs` |
| diesel | `99d16581814d` | 1 | CONFLICT (contents) | `diesel/src/mysql_like/value.rs` |
| diesel | `f3d8f4ea4167` | 1 | CONFLICT (directory rename suggested) | `diesel/src/mysql_like/query_builder/batch_update.rs`, `diesel/src/mysql/query_builder/batch_update.rs` |
| drizzle-orm | `6cb70911457b` | 1 | CONFLICT (contents) | `integration-tests/tests/mysql/tidb-serverless.test.ts` |
| drizzle-orm | `e78f396b2ea3` | 2 | CONFLICT (contents) | `changelogs/drizzle-orm/0.30.5.md` |
| drizzle-orm | `e78f396b2ea3` | 2 | CONFLICT (contents) | `drizzle-orm/package.json` |

Warnings and conflicts, in merges

| Repository | Level | With a warning | With a conflict | Warned without a conflict | Conflicted without a warning |
|---|---|---|---|---|---|
| diesel | file | 123 | 50 | 73 | 0 |
| diesel | hunk | 53 | 50 | 3 | 0 |
| diesel | symbol | 32 | 50 | 6 | 24 |
| drizzle-orm | file | 78 | 39 | 39 | 0 |
| drizzle-orm | hunk | 48 | 39 | 9 | 0 |
| drizzle-orm | symbol | 42 | 39 | 17 | 14 |
| netbox | file | 61 | 33 | 28 | 0 |
| netbox | hunk | 39 | 33 | 6 | 0 |
| netbox | symbol | 33 | 33 | 5 | 5 |

Side lifetimes, in hours

| Repository | Sides | Median | 90th percentile | Longest |
|---|---|---|---|---|
| diesel | 501 | 61.8 | 839.8 | 9358.5 |
| drizzle-orm | 489 | 116.7 | 1116.3 | 7455.8 |
| netbox | 556 | 51.7 | 310.8 | 2960.4 |

Cost on the largest sides; times are not compared on a rerun

| Repository | Merge | Parent | File | Hunk | Symbol | Bytes | Time, ms |
|---|---|---|---|---|---|---|---|
| diesel | `bb9ad76bf82b` | 2 | 1230 | 20052 | 4983 | 840480 | 19983 |
| diesel | `99d16581814d` | 2 | 1088 | 16944 | 4563 | 723040 | 15468 |
| diesel | `490b14383695` | 1 | 1071 | 16558 | 4443 | 706304 | 15078 |
| drizzle-orm | `3cd1d0194bba` | 2 | 857 | 51338 | 3881 | 1794432 | 15259 |
| drizzle-orm | `bdea1eee33ee` | 2 | 531 | 47777 | 1938 | 1607872 | 8664 |
| drizzle-orm | `240dd1c51cc3` | 2 | 279 | 40173 | 622 | 1314368 | 4904 |
| netbox | `d883be9e56e9` | 1 | 135 | 107898 | 181 | 3462848 | 2341 |
| netbox | `0b002c1b6e72` | 2 | 383 | 105425 | 1957 | 3448480 | 7583 |
| netbox | `6f2ba5c75c31` | 1 | 502 | 98168 | 2147 | 3226144 | 9621 |

### Findings
- **When to warn**, in merges:
  - Hunk level: no merge that `git merge-tree` does not merge cleanly is without a warning; 3, 9 and 6 warned merges merge cleanly.
  - File level: no conflicted merge without a warning either; 73, 39 and 28 warned merges merge cleanly.
  - Symbol level: 24, 14 and 5 conflicted merges without a warning. Symbol fingerprints cover only lines inside a symbol of a Rust, TypeScript or Python file.
- **In files:** 267, 936 and 300 hunk-level warnings without a conflict. 166, 866 and 162 hunk-level warnings are on files both sides changed the same way, a false match of [hunk fingerprints](#hunk-fingerprints).
- **Conflicts without a file-level warning:** each listed one is in a merge where one side renamed the conflict's file or its directory, by `git diff -M` from the merge base, or in the merge with two merge bases. The edit sets list a rename by both of its paths ([File fingerprints](#file-fingerprints)) and do not follow it; `git merge-tree` follows renames and names the paths after them. Each of these merges has a file-level warning on another file.
- **Reservation TTL:** a TTL shorter than the median, 61.8, 116.7 and 51.7 hours, ends before at least half of the sides are merged.
- **Intents:** at hunk level, 33 of 58, 33 of 505 and 14 of 102 warnings where both sides only add lines conflict: insertions at one place can conflict, so an intent of only adding lines does not rule a conflict out.
- **Cost:** on each of the largest sides, hunk fingerprints are most of the fingerprints; the side with the most hunk fingerprints, 107898, publishes 3462848 bytes. Computing the fingerprints of these sides took 2341 to 19983 ms on this Mac.
- **Open:**
  - A machine publishes while it works; a replay sees each side only at the merge, so it shows neither how early a warning comes nor warnings on intermediate states.
  - Warnings on renamed paths: whether to fingerprint a rename's new path for the old one, or to leave the renamed file to the exact check.
  - Merges with several merge bases: the fingerprints take the first one.
  - Lockfiles, migrations and generated code among the warnings without a conflict.
