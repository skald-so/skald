# Edit comparison prototype

The edit comparison prototype ([roadmap.md, "Preparation"](roadmap.md#preparation-before-mvp)): machines learn that they edit the same thing from a server that sees only HMAC fingerprints.

- Code: [`prototypes/edits/`](../../prototypes/edits), a cargo workspace apart from the product crates. `edit-overlap` holds the fingerprints, the hunks, the symbols, the client, the server stand-in, the exact check, the stand-in for end-to-end encrypted messages, the scenario harness, the [measurement](#measurement)'s replay of real merges and classes of files, with its command, `measure`, the [path normalization](#path-normalization) with its check of spellings, `spellings`, and the [scripted sessions](#edits-via-bash-and-generators) with their pinned tools, with their command, `sessions`.
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
| `measure_fetches_a_repository_at_its_commit_and_replays_its_merges` | The fixture's upstream fetched at the merge before its last: the merges replayed are those of that commit, its 3 largest sides are timed, and the paths of its tree are those `git ls-tree` gives; asking for one merge more than its history has fails |
| `report_gives_the_paths_that_share_a_fingerprint` | A tree with `README.md` and `Readme.md`, and a side that changes `docs/guide.md` and `Docs/Guide.md`, and `docs/café.md` in NFC and in NFD: the table lists each group with how its paths differ |
| `files_of_each_class_are_classed_by_what_recognizes_them` | The noise fixture: the test repository with a lockfile, a migration, a script its `.gitattributes` marks `linguist-generated`, a schema with a marker and a snapshot, changed on both sides of merges, then the schema renamed on one side and edited on the other: each file's class and what recognized it; the rename's merge reads the marker from the second parent |
| `each_way_of_handling_a_class_gives_the_expected_counts` | On the noise fixture: the counts of each class at each level, the files each signal recognized, the counts of each way of handling each class, and the merges whose sides add migrations: different ones of one number, different numbers, one number in different directories, the same one; edits of existing migrations add none |
| `attributes_of_the_given_commit_mark_files_generated` | `linguist-generated`, set or `true`, `binary` and `-diff` mark a file generated, `linguist-generated=false` and `diff` do not; the attributes are those of the given commit, not of a later one without them |
| `the_header_names_the_machine_and_the_git_version` | The lines before the tables: the operating system, the architecture and the number of CPUs, on macOS the CPU that `sysctl` names, and `git version` |
| `the_file_system_and_the_settings_git_init_writes_are_found` | The [check of spellings](#path-normalization) finds the file system case-insensitive and normalization-insensitive where a directory holds one file once it is written under two names; `git init` writes `core.ignorecase` on a case-insensitive file system and `core.precomposeunicode` on macOS only |
| `a_case_only_rename_gets_one_path_fingerprint` | `README.md` renamed to `Readme.md` with `git mv` and outside git, under each `core.ignorecase` setting: the edit set and its path fingerprints, at most one, by the file system's case sensitivity |
| `names_in_nfc_and_nfd_get_one_path_fingerprint` | `café.md` written in NFC, written in NFD, and checked out from a commit that holds it in NFD, under each `core.precomposeunicode` setting: the forms of the names of the edit set and of the index, and one path fingerprint, by whether git precomposes names, on macOS with the setting on |
| `every_spelling_of_one_path_gives_one_fingerprint` | Paths an agent's tools give for a clone's files: relative, with `.` and `..`, with backslashes, absolute as the temporary directory spells it and as the file system resolves it, through a symbolic link, from another worktree, in another case, with the file name in NFD, a deleted file; on Windows also with a lowercase drive letter and with forward slashes. Each maps to the file's path and gets its fingerprint, but backslashes outside Windows, which name another file; a file outside the repository or in another repository maps to none |
| `paths_past_the_last_existing_component_stay_as_given` | `..` after a missing directory drops it; the root of the worktree, and a path that leads back to it, map to no file |
| `line_endings_of_one_edit_give_its_fingerprints_under_each_autocrlf_setting` | One line edited on a clone without `core.autocrlf` with LF, and on a clone with each setting with the whole file written with CRLF and committed: the line endings of the checkout and of the stored file, and the fingerprints of both edits and those they share |
| `the_check_prints_each_part` | The output of the check: the header, the file system, the settings of `git init`, and the rows of its tables |
| `lf_and_crlf_versions_of_one_edit_give_the_same_hunk_and_symbol_fingerprints` | A Rust file stored with CRLF, an `impl` block naming its type over three lines; upstream stores it with LF. A machine on each base edits one line of a method, with its base's line endings: both publish the same fingerprints and overlap at hunk and symbol level |
| `machines_with_macos_windows_and_linux_spellings_of_one_file_overlap` | Three machines each add one new file, named in NFC, in NFD with `core.precomposeunicode` off, and in another case, and edit one line of a tracked file, one with CRLF and `core.autocrlf`: each overlaps the other two on both files, at file and hunk level, under its own spelling |
| `checkout_fetches_the_repository_at_its_commit_with_an_empty_edit_set` | The [scripted sessions](#edits-via-bash-and-generators)' fixture: the test repository with a file to move and one to delete, fetched at its commit; its edit set is empty |
| `each_step_gives_the_files_it_changes_and_the_files_it_names` | The fixture's session: edit tool calls; a restore with `git checkout`; `sed -i` on named files and on a glob; `mv` in a subdirectory; `rm`; a generator, a script outside the worktree, that writes a tracked file and an ignored one; an edit tool call on the moved file; a new file. Each step's kind, changed files and named files, and the files changed at the end |
| `the_diff_reports_every_changed_file_the_generators_included` | On the fixture's session, the diff on its schedule and after each step reports the files changed at the end, the generator's tracked file among them, not its ignored one |
| `a_file_changed_and_restored_is_not_reported_by_the_diff` | A file an edit tool call changes and `git checkout` restores: the diff after each step reports it until the restore, neither diff at the end; the hooks and the declarations do |
| `each_source_reports_exactly_the_files_it_can_see` | On the fixture's session, each source's report at the end |
| `each_source_learns_of_each_change_when_it_can` | On the fixture's session, for each kind of command and for any: each changed file, its first change and when each source learns of it, and the files each source misses |
| `the_diff_on_its_schedule_scans_every_3_steps_and_after_the_last` | The steps after which the diff on its schedule scans: the 3rd, 6th and 9th of 10, and the last |
| `report_gives_the_tables_of_the_sessions` | The fixture's tables: its steps, the files each source misses by kind, and when each learns of them |
| `a_pipe_in_a_command_is_escaped_in_the_table` | A command line with a pipe gets an escaped one in the table of steps |
| `a_setup_that_changes_the_worktree_fails` | A setup that leaves an untracked file fails; one that leaves only an ignored file does not |
| `an_edit_of_text_the_file_does_not_hold_once_or_a_failing_command_fails` | An edit tool call of text its file holds twice or not at all, and a command line that fails, fail the session; with text the file holds once, it changes the file |
| `run_sets_up_the_pinned_tools_and_plays_each_session_on_its_repository` | With this machine's toolchain, `node`, `npm` and `python3` pinned at their versions, a fixture generator crate that `cargo install` builds, an npm package and a wheel installed offline, and a session on the fixture, whose toolchain file names a toolchain rustup cannot install: the tools' versions; the generator's step; in the clone, the pinned versions of `rustc`, `cargo` and `rustfmt`; the `CARGO_HOME` the generator's build script got, the npm cache and configs the package's install script got, and pip's cache, all the command's own; the environment of the shell commands: only `PATH`, of the toolchain, the installed tools, the linked ones and the system's directories, and the variables of `tools::isolation`, besides what the shell sets |
| `a_tool_of_another_version_is_refused` | Another version pinned for `rustc`, `cargo`, `rustfmt`, `node`, `npm`, `python3`, or a tool cargo, npm or pip installs: the command fails |
| `a_session_on_a_repository_that_is_not_measured_fails` | A session that names no measured repository fails before anything is set up |

`<language>`: `rust`, `typescript` and `python`, each on a file in that language with a function, and a type with two methods, the first named as the function.

## Questions
Open questions of edit comparison, and how the prototype answers them:
| Question | How |
|---|---|
| Data source: diff, hooks or declarations | [Edits via Bash and generators](#edits-via-bash-and-generators): the changed files each source misses and when it learns of them, on scripted sessions |
| When to warn | [Measurement](#measurement): warnings at each level against `git merge-tree` conflicts |
| Reservation TTL | [Measurement](#measurement): how long each side of a merge lived |
| How to catch edits made via Bash and generators | [Edits via Bash and generators](#edits-via-bash-and-generators): which source catches the files shell commands, a formatter, a lockfile rewrite and a code generator change |
| Path normalization | [Path normalization](#path-normalization): one form of a path and of line endings, the paths of the measured repositories that share it, and what git does on macOS and Windows |
| Noise from lockfiles and migrations | [Measurement](#measurement): warnings and conflicts by class of file, and ways of handling each class |
| Whether intents like "replacing / extending" are needed | [Measurement](#measurement): whether both sides of a warning only add lines |

## File fingerprints
- **Edit set:** `git diff --no-renames --ignore-submodules=dirty --name-only <merge base>` against the working tree, and `git ls-files --others --exclude-standard`.
  - The merge base is of `HEAD` and `refs/remotes/origin/HEAD`, the upstream's default branch that `git clone` records.
  - A machine leaves an overlap only once it fetches the merge of its changes; until then the server keeps its old fingerprints.
  - `--no-renames` lists a rename as a deletion and an addition, so both paths count without rename detection and its similarity threshold.
  - `--ignore-submodules=dirty`: a submodule counts when its commit changes, whatever the config and `.gitmodules` say, not when its work tree does; the snapshot records its commit only.
  - Ignored files, such as build output, are not edits.
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit path"`, the scope and the path in its [normal form](#path-normalization), each after its length as a big-endian `u64`; 32 bytes per path.
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
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit hunk"`, the scope, the path in its [normal form](#path-normalization), and the lines on either side of the boundary with their line endings, LF in place of CRLF, each after its length as a big-endian `u64`; empty at the start or the end of the file. No line numbers: lines added elsewhere in the file keep the fingerprint.
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
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit symbol"`, the scope, the path in its [normal form](#path-normalization), and the node kind and the name of each symbol from the outermost, LF in place of each CRLF of a name that spans lines, each after its length as a big-endian `u64`. No line numbers or content: a symbol moved within its file keeps its fingerprint.
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

## Path normalization
One file gets one fingerprint on every machine, whatever its system's spelling of its path.

- **Form:** the path relative to the repository root with `/` separators, as git gives it, each character lowercased with Rust's `char::to_lowercase`, in Unicode NFC. Hunks take the lines on either side of a boundary, and symbols their names, with LF in place of each CRLF: [`fingerprint.rs`](../../prototypes/edits/overlap/src/fingerprint.rs), [`paths.rs`](../../prototypes/edits/overlap/src/paths.rs).
  - Git gives a tracked file's path as the index holds it, the same on every clone. Spellings differ for new files, whose names git reads from the file system, and for the paths an agent's tools give.
  - Line endings matter where two machines' bases store a file's lines with different endings, such as before and after a commit that renormalizes it: the boundaries' lines come from each machine's base.
- **Case:** folded. A case-insensitive file system, the default on macOS and Windows, holds one file where a case-sensitive one can hold two; with case kept, one file that two machines add in two cases would get two fingerprints, where macOS and Windows hold one file. Folded, two files of one repository that differ only in case share a fingerprint: a false match where a case-sensitive file system holds both. No path of the measured repositories shares a fingerprint with another: none in the tree at each one's commit or among the paths the replayed sides change ([Results](#results), the last table).
- **Paths an agent's tools give:** `paths::relative` maps a path to the path of its file relative to the root of the repository's worktree that holds it, with `/` separators.
  - A relative path is taken from the machine's worktree. `Path::components` splits it at its separators: `/`, and on Windows also `\`. Each component that exists is resolved with `std::fs::canonicalize`: symbolic links, drive letters on Windows, and the case the directory holds on macOS and Windows. Components from the first that does not resolve on, such as a deleted file's name, stay as given. `..` drops the last component, resolved or not.
  - The worktree is the one `git rev-parse --show-toplevel` gives for the deepest directory that exists; it must share the machine's `git rev-parse --git-common-dir`, so another worktree of the repository maps, another repository does not.
- **Check:** `cargo run --manifest-path prototypes/edits/Cargo.toml --bin spellings`; it needs `git`. It creates its repositories in a temporary directory, deleted at the end, and prints, in Markdown, the machine, the git version, the file system of the temporary directory and the settings `git init` writes, then:
  - a case-only rename of `README.md` to `Readme.md`, with `git mv` and outside git, under each `core.ignorecase` setting: the edit set and its path fingerprints;
  - `café.md` written in NFC, in NFD, and checked out from a commit that holds it in NFD, under each `core.precomposeunicode` setting: the forms of the names in the edit set and in the index, and the path fingerprints;
  - the spellings of the paths of a clone's files that an agent's tools give: what each maps to and whether it gets the file's fingerprint;
  - a one-line edit written with LF on a clone without `core.autocrlf`, and written with CRLF over the whole file and committed on a clone with each setting: the line endings of the checkout and of the stored file, and the fingerprints of both edits and those they share.
  - Git runs as in the [harness](#edit-comparison-prototype): without the global and system config, so `core.autocrlf` is unset unless the check sets it.
  - The tests of [`spellings.rs`](../../prototypes/edits/overlap/tests/spellings.rs) run each part on the file system they run on and check what it gives there; `script/test` runs them, not the command.
- **macOS:** the check on this Mac's own volume, APFS, case-insensitive:

Machine: macos aarch64, Apple M3 Max, 14 CPUs
Git: git version 2.50.1 (Apple Git-155)

File system: case-insensitive: yes; Unicode normalization-insensitive: yes
`git init` sets: `core.ignorecase` true, `core.precomposeunicode` true

Case-only rename of `README.md` to `Readme.md`

| `core.ignorecase` | Rename | Edit set | Path fingerprints |
|---|---|---|---|
| true | git mv | `README.md`, `Readme.md` | 1 |
| true | file system | none | 0 |
| false | git mv | the rename fails | - |
| false | file system | `Readme.md` | 1 |

File names in NFC and NFD: `café.md`, by the form of each name

| `core.precomposeunicode` | File | Edit set | Index | Path fingerprints |
|---|---|---|---|---|
| true | written in NFC | NFC | NFC | 1 |
| true | written in NFD | NFC | NFC | 1 |
| true | checked out from a commit in NFD | NFC | NFD | 1 |
| false | written in NFC | NFC | NFC | 1 |
| false | written in NFD | NFD | NFD | 1 |
| false | checked out from a commit in NFD | none | NFD | 0 |

Paths an agent's tools give

| Spelling | File | Maps to | The file's path fingerprint |
|---|---|---|---|
| relative | `src/inventory.rs` | `src/inventory.rs` | yes |
| relative, with `.` and `..` | `src/inventory.rs` | `src/inventory.rs` | yes |
| relative, with backslashes | `src/inventory.rs` | `src\inventory.rs` | no |
| absolute, as the temporary directory spells it | `src/inventory.rs` | `src/inventory.rs` | yes |
| absolute, resolved by the file system | `src/inventory.rs` | `src/inventory.rs` | yes |
| through a symbolic link to the clone | `src/inventory.rs` | `src/inventory.rs` | yes |
| from another worktree | `src/inventory.rs` | `src/inventory.rs` | yes |
| in another case | `src/inventory.rs` | `src/inventory.rs` | yes |
| with the file name in NFD | `docs/café.md` | `docs/café.md` | yes |
| a deleted file | `docs/guide.md` | `docs/guide.md` | yes |
| outside the repository | `src/inventory.rs` | no worktree of the repository | no |
| in another repository | `src/inventory.rs` | no worktree of the repository | no |

Line endings of one edit

| `core.autocrlf` | Checkout of an LF file | Stored once written with CRLF | Fingerprints of an edit written with LF | Of the edit written with CRLF | Shared |
|---|---|---|---|---|---|
| true | CRLF | LF | 4 | 4 | 4 |
| input | LF | LF | 4 | 4 | 4 |
| false | LF | CRLF | 4 | 13 | 4 |

- **Windows:** the `Windows` job of [`.github/workflows/edits-windows.yml`](../../.github/workflows/edits-windows.yml), on `windows-latest`, runs the tests of `spellings.rs` and the check ([infrastructure.md](infrastructure.md), "PR checks"). Its check in run [43152461016](https://github.com/skald-so/skald/actions/runs/43152461016):

Machine: windows x86_64, 4 CPUs
Git: git version 2.56.0.windows.4

File system: case-insensitive: yes; Unicode normalization-insensitive: no
`git init` sets: `core.ignorecase` true, `core.precomposeunicode` unset

Case-only rename of `README.md` to `Readme.md`

| `core.ignorecase` | Rename | Edit set | Path fingerprints |
|---|---|---|---|
| true | git mv | `README.md`, `Readme.md` | 1 |
| true | file system | none | 0 |
| false | git mv | the rename fails | - |
| false | file system | `Readme.md` | 1 |

File names in NFC and NFD: `café.md`, by the form of each name

| `core.precomposeunicode` | File | Edit set | Index | Path fingerprints |
|---|---|---|---|---|
| true | written in NFC | NFC | NFC | 1 |
| true | written in NFD | NFD | NFD | 1 |
| true | checked out from a commit in NFD | none | NFD | 0 |
| false | written in NFC | NFC | NFC | 1 |
| false | written in NFD | NFD | NFD | 1 |
| false | checked out from a commit in NFD | none | NFD | 0 |

Paths an agent's tools give

| Spelling | File | Maps to | The file's path fingerprint |
|---|---|---|---|
| relative | `src/inventory.rs` | `src/inventory.rs` | yes |
| relative, with `.` and `..` | `src/inventory.rs` | `src/inventory.rs` | yes |
| relative, with backslashes | `src/inventory.rs` | `src/inventory.rs` | yes |
| absolute, as the temporary directory spells it | `src/inventory.rs` | `src/inventory.rs` | yes |
| absolute, resolved by the file system | `src/inventory.rs` | `src/inventory.rs` | yes |
| through a symbolic link to the clone | `src/inventory.rs` | `src/inventory.rs` | yes |
| from another worktree | `src/inventory.rs` | `src/inventory.rs` | yes |
| in another case | `src/inventory.rs` | `src/inventory.rs` | yes |
| with the file name in NFD | `docs/café.md` | `docs/café.md`, NFD | yes |
| absolute, with a lowercase drive letter | `src/inventory.rs` | `src/inventory.rs` | yes |
| absolute, with forward slashes | `src/inventory.rs` | `src/inventory.rs` | yes |
| a deleted file | `docs/guide.md` | `docs/guide.md` | yes |
| outside the repository | `src/inventory.rs` | no worktree of the repository | no |
| in another repository | `src/inventory.rs` | no worktree of the repository | no |

Line endings of one edit

| `core.autocrlf` | Checkout of an LF file | Stored once written with CRLF | Fingerprints of an edit written with LF | Of the edit written with CRLF | Shared |
|---|---|---|---|---|---|
| true | CRLF | LF | 4 | 4 | 4 |
| input | LF | LF | 4 | 4 | 4 |
| false | LF | CRLF | 4 | 13 | 4 |

- **Findings,** from the two outputs above:
  - Case-only rename: on macOS and Windows, with `core.ignorecase` true, as `git init` sets it there, `git mv` lists both paths, which share one path fingerprint; a rename outside git, as `mv` or an editor makes it, changes nothing git lists, so the machine publishes nothing for it. With `core.ignorecase` false, `git mv` fails, and a rename outside git lists the new name, with the old name's fingerprint.
  - On a case-sensitive file system, as the `Rust` job checks on Linux with `a_case_only_rename_gets_one_path_fingerprint`: with `core.ignorecase` true a rename outside git lists only the old name, as deleted; otherwise both names. One path fingerprint each time.
  - NFC and NFD: on macOS with `core.precomposeunicode` true, as `git init` sets it there, git lists and stores a new name in NFC, whatever form it is written in. A name a commit holds in NFD, checked out there, lists as a new file in NFC; normalized, it gets the fingerprint of the file, so it matches every other machine's edit of it. With the setting false, and on Windows under either setting, git keeps names as written.
  - Windows takes NFC and NFD for two names, as a case-sensitive file system takes two cases: normalized, two such files of one repository share a fingerprint.
  - Tool paths: every spelling maps to the file's path and gets its fingerprint on both systems, but backslashes outside Windows, which name another file there. With the file name in NFD, macOS maps to the name in NFC, as the directory holds it; Windows keeps the NFD name.
  - Line endings: the same on both systems. With `core.autocrlf` true or input, git stores the file written with CRLF with LF: the edit gets the same 4 fingerprints as the one written with LF. With false, it stores CRLF and every line changes: 13 fingerprints, the LF edit's 4 among them, so it matches every edit of the file at hunk level.
  - Windows inputs: the job makes each one on the runner: the repositories and files on its volume, the spellings of their paths, the name in NFD, and the line endings under each `core.autocrlf` setting; the spellings with a lowercase drive letter and with forward slashes are the real path rewritten. It cannot make git precompose names, which only macOS does, or a normalization-insensitive file system: the macOS check covers them. Not checked: network shares, whose paths have no drive letter, and directories set case-sensitive.
- **Open:**
  - `char::to_lowercase` keeps some letters apart that a case-insensitive file system may take for one, such as Greek `ς` and `σ`, whose uppercase is one letter, `Σ`. Not checked against the case tables of APFS and NTFS.
  - A rename outside git on a case-insensitive file system with `core.ignorecase` true: the diff never sees it, and the [hooks](#edits-via-bash-and-generators) see no shell command. Not measured: a declaration of an `mv` of it.
  - A name a commit holds in NFD, checked out on macOS: its NFC copy warns against every machine that edits the file.
  - A path whose last component is a symbolic link the repository tracks maps to the link's target, not to the link. Not checked.
  - A path in a submodule maps to none: the submodule is another repository.

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
- **Classes:** each file of a merge's file-level warnings and conflicts gets the class of the first rule it matches, read at the merge's first parent. The cases checked: the unit tests of [`noise.rs`](../../prototypes/edits/overlap/src/noise.rs).
  - Lockfile, by name: `Cargo.lock`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`, `poetry.lock`, `Pipfile.lock`, `uv.lock`, `Gemfile.lock`, `composer.lock`, `go.sum`.
  - Migration, by name: below the last directory named `migrations` of its path, the first file or directory whose name starts with a digit, such as Django's `<app>/migrations/0001_initial.py` and the diesel repository's `migrations/<backend>/<version>_<name>/up.sql`; drizzle-kit's `meta/_journal.json` and `meta/<number>_snapshot.json`. Its number: the digits and dashes its name starts with.
  - Generated, by `.gitattributes`: the first parent's attributes set `linguist-generated`, or set it to `true`, or unset `diff`, as `binary` does. Read with `git check-attr --source`, git 2.40.
  - Generated, by a marker: one of its first 5 lines holds `@generated`, `do not edit`, `autogenerated`, `auto-generated`, `automatically generated`, `code generated` or `generated by`, in any case. The file is read at the first parent, or the second when the first lacks it.
  - Generated, by name: the extension `.snap`, `.mo` or `.map`; a name ending with `.min.js` or `.min.css`; a directory named `dist` in its path.
  - Other: the rest.
  - A conflict has the class of the first file `git merge-tree` names.
- **Ways of handling a class,** one class at a time, the others as they are; compare with "Warnings and conflicts, in files":
  - Left out: its files publish no fingerprints.
  - File level only: its files publish path fingerprints only, and a path match warns at every level.
  - Warned apart: its warnings go to a list of their own. A warning without a conflict counts in the main list only; a conflict has a warning when either list warns about one of its files.
- **Migrations both sides add:** each side adds a migration, a path absent from the merge base, to one directory; different migrations of one number: two of them with one number and different names. A file-level warning on a migration is one on any migration of the merge.
- **Paths that share a fingerprint:** the groups of paths, of the tree at the repository's commit and of all the paths the replayed sides change, that share their [normal form](#path-normalization), with how they differ: in case, in Unicode normalization, or both.

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
| diesel | `bb9ad76bf82b` | 2 | 1230 | 20052 | 4983 | 840480 | 17811 |
| diesel | `99d16581814d` | 2 | 1088 | 16944 | 4563 | 723040 | 15649 |
| diesel | `490b14383695` | 1 | 1071 | 16558 | 4443 | 706304 | 15430 |
| drizzle-orm | `3cd1d0194bba` | 2 | 857 | 51338 | 3881 | 1794432 | 14874 |
| drizzle-orm | `bdea1eee33ee` | 2 | 531 | 47777 | 1938 | 1607872 | 9230 |
| drizzle-orm | `240dd1c51cc3` | 2 | 279 | 40173 | 622 | 1314368 | 5301 |
| netbox | `d883be9e56e9` | 1 | 135 | 107898 | 181 | 3462848 | 2526 |
| netbox | `0b002c1b6e72` | 2 | 383 | 105425 | 1957 | 3448480 | 7165 |
| netbox | `6f2ba5c75c31` | 1 | 502 | 98168 | 2147 | 3226144 | 9791 |

Warnings and conflicts by class, in files

| Repository | Class | Warnings, file | Warnings, hunk | Warnings, symbol | Without a conflict, file | Without a conflict, hunk | Without a conflict, symbol | Conflicts | Without a warning, file | Without a warning, hunk | Without a warning, symbol |
|---|---|---|---|---|---|---|---|---|---|---|---|
| diesel | lockfile | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| diesel | migration | 4 | 4 | 0 | 4 | 4 | 0 | 0 | 0 | 0 | 0 |
| diesel | generated | 39 | 30 | 0 | 23 | 14 | 0 | 16 | 0 | 0 | 16 |
| diesel | other | 636 | 433 | 180 | 452 | 249 | 119 | 187 | 3 | 3 | 126 |
| drizzle-orm | lockfile | 19 | 17 | 0 | 9 | 7 | 0 | 10 | 0 | 0 | 10 |
| drizzle-orm | migration | 6 | 6 | 0 | 6 | 6 | 0 | 0 | 0 | 0 | 0 |
| drizzle-orm | generated | 3 | 3 | 3 | 3 | 3 | 3 | 0 | 0 | 0 | 0 |
| drizzle-orm | other | 1237 | 1081 | 787 | 1076 | 920 | 692 | 164 | 3 | 3 | 69 |
| netbox | lockfile | 5 | 5 | 0 | 0 | 0 | 0 | 5 | 0 | 0 | 5 |
| netbox | migration | 9 | 9 | 9 | 9 | 9 | 9 | 0 | 0 | 0 | 0 |
| netbox | generated | 81 | 81 | 0 | 1 | 1 | 0 | 125 | 0 | 0 | 125 |
| netbox | other | 1024 | 598 | 356 | 716 | 290 | 243 | 308 | 0 | 0 | 195 |

Files of warnings at file level and of conflicts by what recognized their class, once per merge

| Repository | Class | By name | By `.gitattributes` | By a marker |
|---|---|---|---|---|
| diesel | lockfile | 0 | 0 | 0 |
| diesel | migration | 4 | 0 | 0 |
| diesel | generated | 11 | 0 | 28 |
| drizzle-orm | lockfile | 19 | 0 | 0 |
| drizzle-orm | migration | 6 | 0 | 0 |
| drizzle-orm | generated | 0 | 0 | 3 |
| netbox | lockfile | 5 | 0 | 0 |
| netbox | migration | 9 | 0 | 0 |
| netbox | generated | 45 | 36 | 0 |

Ways of handling a class, in files

| Repository | Class | Way | Warnings without a conflict, file | Warnings without a conflict, hunk | Warnings without a conflict, symbol | Conflicts without a warning, file | Conflicts without a warning, hunk | Conflicts without a warning, symbol |
|---|---|---|---|---|---|---|---|---|
| diesel | lockfile | left out | 479 | 267 | 119 | 3 | 3 | 142 |
| diesel | lockfile | file level only | 479 | 267 | 119 | 3 | 3 | 142 |
| diesel | lockfile | warned apart | 479 | 267 | 119 | 3 | 3 | 142 |
| diesel | migration | left out | 475 | 263 | 119 | 3 | 3 | 142 |
| diesel | migration | file level only | 479 | 267 | 123 | 3 | 3 | 142 |
| diesel | migration | warned apart | 475 | 263 | 119 | 3 | 3 | 142 |
| diesel | generated | left out | 456 | 253 | 119 | 19 | 19 | 142 |
| diesel | generated | file level only | 479 | 276 | 142 | 3 | 3 | 126 |
| diesel | generated | warned apart | 456 | 253 | 119 | 3 | 3 | 142 |
| drizzle-orm | lockfile | left out | 1085 | 929 | 695 | 13 | 13 | 79 |
| drizzle-orm | lockfile | file level only | 1094 | 938 | 704 | 3 | 3 | 69 |
| drizzle-orm | lockfile | warned apart | 1085 | 929 | 695 | 3 | 3 | 79 |
| drizzle-orm | migration | left out | 1088 | 930 | 695 | 3 | 3 | 79 |
| drizzle-orm | migration | file level only | 1094 | 936 | 701 | 3 | 3 | 79 |
| drizzle-orm | migration | warned apart | 1088 | 930 | 695 | 3 | 3 | 79 |
| drizzle-orm | generated | left out | 1091 | 933 | 692 | 3 | 3 | 79 |
| drizzle-orm | generated | file level only | 1094 | 936 | 695 | 3 | 3 | 79 |
| drizzle-orm | generated | warned apart | 1091 | 933 | 692 | 3 | 3 | 79 |
| netbox | lockfile | left out | 726 | 300 | 252 | 5 | 5 | 325 |
| netbox | lockfile | file level only | 726 | 300 | 252 | 0 | 0 | 320 |
| netbox | lockfile | warned apart | 726 | 300 | 252 | 0 | 0 | 325 |
| netbox | migration | left out | 717 | 291 | 243 | 0 | 0 | 325 |
| netbox | migration | file level only | 726 | 300 | 252 | 0 | 0 | 325 |
| netbox | migration | warned apart | 717 | 291 | 243 | 0 | 0 | 325 |
| netbox | generated | left out | 725 | 299 | 252 | 125 | 125 | 325 |
| netbox | generated | file level only | 726 | 300 | 253 | 0 | 0 | 200 |
| netbox | generated | warned apart | 725 | 299 | 252 | 0 | 0 | 325 |

Migrations both sides add, in merges

| Repository | Both sides add to one directory | Of them without a file-level warning on a migration | Different migrations of one number | Of them without a file-level warning on a migration |
|---|---|---|---|---|
| diesel | 1 | 0 | 0 | 0 |
| drizzle-orm | 2 | 0 | 0 | 0 |
| netbox | 15 | 12 | 14 | 12 |

Paths that share a fingerprint once case is folded and Unicode normalized

| Repository | Paths of | Differ in | Paths |
|---|---|---|---|

### Findings
- **When to warn**, in merges:
  - Hunk level: no merge that `git merge-tree` does not merge cleanly is without a warning; 3, 9 and 6 warned merges merge cleanly.
  - File level: no conflicted merge without a warning either; 73, 39 and 28 warned merges merge cleanly.
  - Symbol level: 24, 14 and 5 conflicted merges without a warning. Symbol fingerprints cover only lines inside a symbol of a Rust, TypeScript or Python file.
- **In files:** 267, 936 and 300 hunk-level warnings without a conflict. 166, 866 and 162 hunk-level warnings are on files both sides changed the same way, a false match of [hunk fingerprints](#hunk-fingerprints).
- **Conflicts without a file-level warning:** each listed one is in a merge where one side renamed the conflict's file or its directory, by `git diff -M` from the merge base, or in the merge with two merge bases. The edit sets list a rename by both of its paths ([File fingerprints](#file-fingerprints)) and do not follow it; `git merge-tree` follows renames and names the paths after them. Each of these merges has a file-level warning on another file.
- **Reservation TTL:** a TTL shorter than the median, 61.8, 116.7 and 51.7 hours, ends before at least half of the sides are merged.
- **Intents:** at hunk level, 33 of 58, 33 of 505 and 14 of 102 warnings where both sides only add lines conflict: insertions at one place can conflict, so an intent of only adding lines does not rule a conflict out.
- **Cost:** on each of the largest sides, hunk fingerprints are most of the fingerprints; the side with the most hunk fingerprints, 107898, publishes 3462848 bytes. Computing the fingerprints of these sides took 2526 to 17811 ms on this Mac.
- **Noise by class,** at file level: other files hold 452 of diesel's 479 warnings without a conflict, 1076 of drizzle-orm's 1094 and 716 of netbox's 726; lockfiles, migrations and generated code hold the rest.
- **Lockfiles:** diesel has no lockfile warning; drizzle-orm has 9 of 19 without a conflict, netbox 0 of 5. Left out, they hide conflicts: conflicts without a file-level warning rise from 3 to 13 on drizzle-orm and from 0 to 5 on netbox.
- **Migrations:** 4, 6 and 9 warnings, none of them with a conflict; no conflict of the measured merges is of a migration. Left out, they hide no conflict at any level.
  - In 14 netbox merges the sides add different migrations of one number to one directory, 12 of them without a file-level warning on a migration; none in diesel or drizzle-orm. Such migrations clash without sharing a file, so no fingerprint of theirs matches.
- **Generated code:** diesel has 23 of 39 warnings without a conflict, drizzle-orm 3 of 3, netbox 1 of 81. Left out, it hides conflicts: conflicts without a file-level warning rise from 3 to 19 on diesel and from 0 to 125 on netbox.
  - At file level only, conflicts without a symbol-level warning fall from 142 to 126 on diesel and from 325 to 200 on netbox; symbol-level warnings without a conflict rise from 119 to 142 and from 252 to 253.
- **Recognition,** in files of file-level warnings and of conflicts, once per merge:
  - `.gitattributes`: 36 on netbox, in `netbox/project-static/dist/`, which it marks `binary`; none in diesel or drizzle-orm.
  - Markers: 28 on diesel, such as the schema snapshots of Diesel CLI's tests, whose `// @generated` follows the snapshot's header on line 5; 3 on drizzle-orm. At diesel's measured commit, Diesel CLI's `print_schema.rs` holds the marker in a string on line 13: the 5-line limit keeps the generator itself other.
  - Names: 11 on diesel, such as snapshots without a marker; 45 on netbox, such as its `.mo` catalogs.
- **Handling:** warned apart, lockfiles and generated code leave the main list without their warnings and miss no more conflicts than as they are; left out, they hide conflicts. Migrations left out or warned apart hide no conflict on these merges.
- **Open:**
  - A machine publishes while it works; a replay sees each side only at the merge, so it shows neither how early a warning comes nor warnings on intermediate states.
  - Warnings on renamed paths: whether to fingerprint a rename's new path for the old one, or to leave the renamed file to the exact check.
  - Merges with several merge bases: the fingerprints take the first one.
  - Migrations of one number in one directory: a fingerprint of the directory and the number would match them; not measured.
  - drizzle-kit's SQL migrations, `<dir>/<number>_<name>.sql` next to `<dir>/meta/`, such as `integration-tests/drizzle2/pg/0000_puzzling_flatman.sql`, match no migration rule and are other. 3 drizzle-orm file-level warnings, once per merge, are on them: `integration-tests/drizzle2/singlestore/0000_nostalgic_carnage.sql` in 2 merges, `integration-tests/tests/sqlite/durable-objects/drizzle/0000_cuddly_black_bolt.sql` in 1; none with a conflict. A numbered `.sql` file next to a `meta/_journal.json` could be classed a migration.
  - Generated files no rule recognizes are other, such as JSON, which holds no comment for a marker: netbox's `contrib/generated_schema.json`. So are lists of pinned dependencies without a lockfile's name, such as netbox's `requirements.txt`.
  - A file is classed at the merge's first parent; a machine would class its own files.

## Edits via Bash and generators
Which source of a machine's edits catches the files an agent changes with shell commands and generators, and when.

- **Command:** `cargo run --release --manifest-path prototypes/edits/Cargo.toml --bin sessions`; it needs `git`, a C compiler, the pinned Rust toolchain with `rustfmt`, Node.js with npm, and Python 3, and HTTPS access to GitHub, crates.io, the npm registry, PyPI and dprint's plugins, no account. In a temporary directory, deleted at the end, also on an error, it sets up the tools, fetches each [measured repository](#measurement) at its commit, plays its session and prints the tables below in Markdown, with the machine and the git version; progress goes to the standard error. A rerun gives the same tables. `script/test` does not run it; it plays a session on a fixture repository ([Scenarios](#scenarios)).
- **Sessions:** one per measured repository, scripted in [`sessions.rs`](../../prototypes/edits/overlap/src/bin/sessions.rs); no agent CLI runs. The table of steps gives each one. A step is one of:
  - an edit tool call, which hooks see: an edit, which replaces text its file holds once, or a write of a whole file;
  - a shell command line, run by `/bin/sh -c` in a directory of the worktree, as an agent's shell tool runs it: in-place edits with `sed -i`, moves with `mv`, deletions with `rm`, the formatter, the lockfile rewrite and the code generator;
  - a restore with `git checkout --`.
  - Formatters, lockfile rewrites and code generators, each the repository's own where it has one:

    | Repository | Formatter | Lockfile rewrite | Code generator |
    |---|---|---|---|
    | diesel | `rustfmt` | `cargo update --offline` of `diesel_compile_tests/Cargo.lock` | Diesel CLI: a migration, then the schema |
    | drizzle-orm | `dprint fmt`, its `lint:fix` | `pnpm install --lockfile-only --offline` | drizzle-kit: a migration from a new schema |
    | netbox | `ruff check --fix`, its pre-commit linter with fixes | `yarn remove --offline` in `netbox/project-static` | its `yarn bundle:scripts`: `dist/` with esbuild |
- **Tools** ([`tools.rs`](../../prototypes/edits/overlap/src/tools.rs)); the table of tools gives each version:
  - Pinned, each by a line its `--version` prints; another version fails the command: the Rust toolchain, Node.js, npm, Python 3, and the tools cargo, npm and pip install.
  - Recorded only: the shell commands, `/bin/sh` and the commands of `/usr/bin` and `/bin`, the system's, by the system's name and release, since macOS's `sed`, `mv` and `rm` print no version; and git, which runs the restores, by the version in the header.
  - Rust toolchain: `rustc`, `cargo` and `rustfmt` run from the toolchain's own directory, which `rustc --print sysroot` gives in the temporary directory: diesel's `rust-toolchain` file names another version, which a rustup proxy started in its clone switches to.
  - Node.js, npm and Python 3: those this process's `PATH` gives, linked into the temporary directory; nothing else of their directories is on the sessions' `PATH`.
  - Installed into the temporary directory: Diesel CLI by `cargo install --locked`, from crates.io with the lockfile it is published with, for SQLite only, with SQLite built from its bundled C source; pnpm, yarn, dprint, drizzle-kit and drizzle-orm by `npm install --prefix`; ruff by `pip` in a virtual environment. Their versions are those the repositories use: drizzle-orm's and netbox's `packageManager`, the `dprint` of drizzle-orm's lockfile, the versions drizzle-orm's own drizzle-kit and drizzle-orm packages have at its commit, and the ruff of netbox's `pyproject.toml`.
  - Homes, configs and caches: in the temporary directory, never the machine's: a `HOME` with its XDG directories, `CARGO_HOME`, npm's cache and empty user and global configs, pnpm's store, the caches of yarn, dprint, pip and ruff, no pip config, and `NODE_PATH`, the npm installs' modules, from which drizzle-kit's schema imports drizzle-orm (`tools::isolation`). The installs fill them over HTTPS, and so do the sessions' setups: diesel's `cargo fetch --locked` at the versions of `diesel_compile_tests/Cargo.lock`; drizzle-orm's `pnpm install --resolution-only`, which fills pnpm's metadata, with its lockfile put back, and dprint's plugins; netbox's `yarn install --frozen-lockfile`, whose `node_modules` netbox ignores. The lockfile rewrites run offline from them.
  - Each shell command gets only `PATH`, of the toolchain, the installed tools, the linked ones and the system's directories, and those variables.
- **Changes:** after each step, a snapshot of the working tree, as the [exact check](#exact-check) takes it; a step's changes are the files that differ between the snapshots before and after it. A changed file: one that differs from the commit at the end of the session. Ignored files are no changes, as in the [edit set](#file-fingerprints).
- **Sources:**
  - Diff, every 3 steps: the edit set, scanned after every third step and after the last, as on a timer.
  - Diff, after each step: the edit set, scanned after each step, as a hook on every tool call could start it.
  - Hooks: the path of each edit tool call, at the call. A shell command gives none.
  - Declarations: before each step, the files it names: the edit tool's path, or each word of the command line that names a file of the worktree before or after the step, taken from the step's directory with `paths::relative`. A glob, a pipe's output and a name a command makes up name none.
- **When:** a source learns of a step's change of a file before the edit, when it reports the file as the step starts; at the edit, when it reports it once the step ends; at the next scan of its schedule; at a later tool call that names or changes the file; or never, when its report at the end does not hold it.
- **Counts:** by kind of command, each changed file once, at the first step of the kind that changes it; `any`: at its first change. Missed: never learned of. Reported, not changed at the end: files a source reports at the end that are not changed files.

- **Results,** on this Mac:

Machine: macos aarch64, Apple M3 Max, 14 CPUs
Git: git version 2.50.1 (Apple Git-155)


Tools

| Tool | Version |
|---|---|
| rustc | rustc 1.99.1 (58cf8f9c3 2026-10-13) |
| cargo | cargo 1.99.1 (5f94df478 2026-08-27) |
| rustfmt | rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13) |
| node | v26.13.0 |
| npm | 11.20.0 |
| python3 | Python 3.13.2 |
| diesel | Version: 2.3.14 |
| pnpm | 10.6.3 |
| yarn | 1.22.22 |
| dprint | dprint 0.46.3 |
| drizzle-kit | drizzle-kit: v0.31.11 |
| drizzle-kit | drizzle-orm: v0.45.4 |
| pip | pip 24.3.1 |
| ruff | ruff 0.17.4 |
| the shell and its commands | the system's: Darwin 25.5.0 |

Steps

| Repository | Step | Kind | Command | Changes | Names |
|---|---|---|---|---|---|
| diesel | 1 | code generator | in `examples/sqlite/getting_started_step_1`: `diesel migration generate --version 20261112000000 add_published_at` | `examples/sqlite/getting_started_step_1/migrations/.diesel_lock`, `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/down.sql`, `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/up.sql` | none |
| diesel | 2 | edit tool | write `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/up.sql` | `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/up.sql` | `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/up.sql` |
| diesel | 3 | edit tool | write `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/down.sql` | `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/down.sql` | `examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/down.sql` |
| diesel | 4 | code generator | in `examples/sqlite/getting_started_step_1`: `diesel migration run --database-url {scratch}/posts.db` | `examples/sqlite/getting_started_step_1/src/schema.rs` | none |
| diesel | 5 | in-place edit | `sed -i.bak -e 's/    pub published: bool,/    pub published: bool, pub published_at: Option<String>,/' examples/sqlite/getting_started_step_1/src/models.rs && rm examples/sqlite/getting_started_step_1/src/models.rs.bak` | `examples/sqlite/getting_started_step_1/src/models.rs` | `examples/sqlite/getting_started_step_1/src/models.rs` |
| diesel | 6 | formatter | `rustfmt --edition 2024 examples/sqlite/getting_started_step_1/src/lib.rs` | `examples/sqlite/getting_started_step_1/src/models.rs` | `examples/sqlite/getting_started_step_1/src/lib.rs` |
| diesel | 7 | move | in `examples/sqlite/getting_started_step_1`: `mv src/bin/show_posts.rs src/bin/list_posts.rs` | `examples/sqlite/getting_started_step_1/src/bin/list_posts.rs`, `examples/sqlite/getting_started_step_1/src/bin/show_posts.rs` | `examples/sqlite/getting_started_step_1/src/bin/list_posts.rs`, `examples/sqlite/getting_started_step_1/src/bin/show_posts.rs` |
| diesel | 8 | edit tool | edit `examples/sqlite/getting_started_step_1/Cargo.toml` | `examples/sqlite/getting_started_step_1/Cargo.toml` | `examples/sqlite/getting_started_step_1/Cargo.toml` |
| diesel | 9 | deletion | in `examples/sqlite/getting_started_step_1`: `rm tests/step_1.rs` | `examples/sqlite/getting_started_step_1/tests/step_1.rs` | `examples/sqlite/getting_started_step_1/tests/step_1.rs` |
| diesel | 10 | edit tool | edit `diesel_compile_tests/Cargo.toml` | `diesel_compile_tests/Cargo.toml` | `diesel_compile_tests/Cargo.toml` |
| diesel | 11 | lockfile rewrite | `cargo update --offline --quiet --workspace --manifest-path diesel_compile_tests/Cargo.toml` | `diesel_compile_tests/Cargo.lock` | `diesel_compile_tests/Cargo.toml` |
| diesel | 12 | edit tool | edit `examples/sqlite/getting_started_step_1/README.md` | `examples/sqlite/getting_started_step_1/README.md` | `examples/sqlite/getting_started_step_1/README.md` |
| diesel | 13 | restore | `git checkout -- examples/sqlite/getting_started_step_1/README.md` | `examples/sqlite/getting_started_step_1/README.md` | `examples/sqlite/getting_started_step_1/README.md` |
| drizzle-orm | 1 | edit tool | edit `drizzle-orm/src/logger.ts` | `drizzle-orm/src/logger.ts` | `drizzle-orm/src/logger.ts` |
| drizzle-orm | 2 | in-place edit | `sed -i.bak -e "s/'DrizzleError'/\"DrizzleOrmError\"/" drizzle-orm/src/errors.ts && rm drizzle-orm/src/errors.ts.bak` | `drizzle-orm/src/errors.ts` | `drizzle-orm/src/errors.ts` |
| drizzle-orm | 3 | in-place edit | `sed -i.bak -e 's/DrizzleError/DrizzleOrmError/g' drizzle-orm/src/*-core/dialect.ts && rm drizzle-orm/src/*-core/dialect.ts.bak` | `drizzle-orm/src/gel-core/dialect.ts`, `drizzle-orm/src/mysql-core/dialect.ts`, `drizzle-orm/src/pg-core/dialect.ts`, `drizzle-orm/src/singlestore-core/dialect.ts`, `drizzle-orm/src/sqlite-core/dialect.ts` | none |
| drizzle-orm | 4 | move | in `drizzle-orm/src`: `mv casing.ts casing-cache.ts` | `drizzle-orm/src/casing-cache.ts`, `drizzle-orm/src/casing.ts` | `drizzle-orm/src/casing-cache.ts`, `drizzle-orm/src/casing.ts` |
| drizzle-orm | 5 | edit tool | edit `drizzle-orm/src/casing-cache.ts` | `drizzle-orm/src/casing-cache.ts` | `drizzle-orm/src/casing-cache.ts` |
| drizzle-orm | 6 | formatter | `dprint fmt` | `drizzle-orm/src/errors.ts`, `drizzle-orm/src/logger.ts` | none |
| drizzle-orm | 7 | deletion | `rm changelogs/drizzle-orm/0.30.5.md` | `changelogs/drizzle-orm/0.30.5.md` | `changelogs/drizzle-orm/0.30.5.md` |
| drizzle-orm | 8 | edit tool | edit `package.json` | `package.json` | `package.json` |
| drizzle-orm | 9 | lockfile rewrite | `pnpm install --lockfile-only --offline --ignore-scripts --silent` | `pnpm-lock.yaml` | none |
| drizzle-orm | 10 | edit tool | write `integration-tests/session/schema.ts` | `integration-tests/session/schema.ts` | `integration-tests/session/schema.ts` |
| drizzle-orm | 11 | code generator | `drizzle-kit generate --dialect sqlite --schema integration-tests/session/schema.ts --out integration-tests/session/drizzle --name init > /dev/null` | `integration-tests/session/drizzle/0000_init.sql`, `integration-tests/session/drizzle/meta/0000_snapshot.json`, `integration-tests/session/drizzle/meta/_journal.json` | `integration-tests/session/schema.ts` |
| drizzle-orm | 12 | edit tool | edit `drizzle-orm/src/utils.ts` | `drizzle-orm/src/utils.ts` | `drizzle-orm/src/utils.ts` |
| drizzle-orm | 13 | restore | `git checkout -- drizzle-orm/src/utils.ts` | `drizzle-orm/src/utils.ts` | `drizzle-orm/src/utils.ts` |
| netbox | 1 | edit tool | edit `netbox/utilities/string.py` | `netbox/utilities/string.py` | `netbox/utilities/string.py` |
| netbox | 2 | in-place edit | `grep -rl remove_linebreaks netbox \| xargs sed -i.bak -e 's/remove_linebreaks/strip_linebreaks/g' && find netbox -name '*.bak' -delete` | `netbox/account/views.py`, `netbox/utilities/string.py` | none |
| netbox | 3 | edit tool | write `netbox/utilities/tests/test_linebreaks.py` | `netbox/utilities/tests/test_linebreaks.py` | `netbox/utilities/tests/test_linebreaks.py` |
| netbox | 4 | move | in `netbox/utilities`: `mv socks.py proxy_socks.py` | `netbox/utilities/proxy_socks.py`, `netbox/utilities/socks.py` | `netbox/utilities/proxy_socks.py`, `netbox/utilities/socks.py` |
| netbox | 5 | in-place edit | `sed -i.bak -e 's/trailing_slash/with_trailing_slash/g' netbox/netbox/settings.py && rm netbox/netbox/settings.py.bak` | `netbox/netbox/settings.py` | `netbox/netbox/settings.py` |
| netbox | 6 | formatter | `ruff check --fix --quiet netbox/` | `netbox/utilities/tests/test_linebreaks.py` | none |
| netbox | 7 | deletion | `rm netbox/utilities/tests/test_string.py` | `netbox/utilities/tests/test_string.py` | `netbox/utilities/tests/test_string.py` |
| netbox | 8 | lockfile rewrite | in `netbox/project-static`: `yarn remove -W typeface-roboto-mono --offline --ignore-scripts --non-interactive --silent` | `netbox/project-static/package.json`, `netbox/project-static/yarn.lock` | none |
| netbox | 9 | edit tool | edit `netbox/project-static/src/clipboard.ts` | `netbox/project-static/src/clipboard.ts` | `netbox/project-static/src/clipboard.ts` |
| netbox | 10 | code generator | in `netbox/project-static`: `yarn --silent bundle:scripts > /dev/null` | `netbox/project-static/dist/netbox.js`, `netbox/project-static/dist/netbox.js.map` | none |
| netbox | 11 | edit tool | edit `README.md` | `README.md` | `README.md` |
| netbox | 12 | restore | `git checkout -- README.md` | `README.md` | `README.md` |

Changed files each source misses, by kind of command, in files and % of the files

| Repository | Kind | Files | diff, every 3 steps | diff, after each step | hooks | declarations |
|---|---|---|---|---|---|---|
| diesel | edit tool | 4 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| diesel | in-place edit | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 0 (0%) |
| diesel | move | 2 | 0 (0%) | 0 (0%) | 2 (100%) | 0 (0%) |
| diesel | deletion | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 0 (0%) |
| diesel | formatter | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 0 (0%) |
| diesel | lockfile rewrite | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 1 (100%) |
| diesel | code generator | 4 | 0 (0%) | 0 (0%) | 2 (50%) | 2 (50%) |
| diesel | restore | 0 | - | - | - | - |
| diesel | any | 11 | 0 (0%) | 0 (0%) | 7 (64%) | 3 (27%) |
| drizzle-orm | edit tool | 4 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| drizzle-orm | in-place edit | 6 | 0 (0%) | 0 (0%) | 6 (100%) | 5 (83%) |
| drizzle-orm | move | 2 | 0 (0%) | 0 (0%) | 1 (50%) | 0 (0%) |
| drizzle-orm | deletion | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 0 (0%) |
| drizzle-orm | formatter | 2 | 0 (0%) | 0 (0%) | 1 (50%) | 0 (0%) |
| drizzle-orm | lockfile rewrite | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 1 (100%) |
| drizzle-orm | code generator | 3 | 0 (0%) | 0 (0%) | 3 (100%) | 3 (100%) |
| drizzle-orm | restore | 0 | - | - | - | - |
| drizzle-orm | any | 16 | 0 (0%) | 0 (0%) | 12 (75%) | 9 (56%) |
| netbox | edit tool | 3 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| netbox | in-place edit | 3 | 0 (0%) | 0 (0%) | 2 (67%) | 1 (33%) |
| netbox | move | 2 | 0 (0%) | 0 (0%) | 2 (100%) | 0 (0%) |
| netbox | deletion | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 0 (0%) |
| netbox | formatter | 1 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| netbox | lockfile rewrite | 2 | 0 (0%) | 0 (0%) | 2 (100%) | 2 (100%) |
| netbox | code generator | 2 | 0 (0%) | 0 (0%) | 2 (100%) | 2 (100%) |
| netbox | restore | 0 | - | - | - | - |
| netbox | any | 12 | 0 (0%) | 0 (0%) | 9 (75%) | 5 (42%) |

When each source learns of a changed file, from its first change, in files

| Repository | Source | Files | Before the edit | At the edit | At the next scan | At a later tool call | Never | Reported, not changed at the end |
|---|---|---|---|---|---|---|---|---|
| diesel | diff, every 3 steps | 11 | 0 | 0 | 11 | 0 | 0 | 0 |
| diesel | diff, after each step | 11 | 0 | 11 | 0 | 0 | 0 | 0 |
| diesel | hooks | 11 | 0 | 2 | 0 | 2 | 7 | 1 |
| diesel | declarations | 11 | 6 | 0 | 0 | 2 | 3 | 2 |
| drizzle-orm | diff, every 3 steps | 16 | 0 | 0 | 16 | 0 | 0 | 0 |
| drizzle-orm | diff, after each step | 16 | 0 | 16 | 0 | 0 | 0 | 0 |
| drizzle-orm | hooks | 16 | 0 | 3 | 0 | 1 | 12 | 1 |
| drizzle-orm | declarations | 16 | 7 | 0 | 0 | 0 | 9 | 1 |
| netbox | diff, every 3 steps | 12 | 0 | 0 | 12 | 0 | 0 | 0 |
| netbox | diff, after each step | 12 | 0 | 12 | 0 | 0 | 0 | 0 |
| netbox | hooks | 12 | 0 | 3 | 0 | 0 | 9 | 1 |
| netbox | declarations | 12 | 7 | 0 | 0 | 0 | 5 | 1 |

- **Findings,** from the output above:
  - Diff: it misses no changed file in any session or kind of command, the code generators' included: 4 on diesel, 3 on drizzle-orm, 2 on netbox. On its schedule it learns of each one at the next scan, 11, 16 and 12 files; scanned after each step, at the edit. It reports no file that is not changed at the end, such as a restored one.
  - Hooks: they miss 7 of 11 changed files on diesel, 12 of 16 on drizzle-orm and 9 of 12 on netbox: those only shell commands change. They learn of a file a command changes only when an edit tool call names it too, before or after the command: at a later tool call, diesel's `up.sql` and `down.sql`, which Diesel CLI creates and edit tool calls then write, and drizzle-orm's `casing-cache.ts`, edited after its move; earlier, netbox's `string.py`, edited before `sed` changes it, and the files of edit tool calls that the formatters change, drizzle-orm's `logger.ts` and netbox's `test_linebreaks.py`. Each session's restored file stays in their report.
  - Declarations: they miss 3 of 11, 9 of 16 and 5 of 12: the files no word of a command line names. The lockfiles the package managers rewrite, `Cargo.lock`, `pnpm-lock.yaml` and `yarn.lock`, and netbox's `package.json`, which `yarn remove` rewrites too; the files the code generators write, but diesel's `up.sql` and `down.sql`, which later edit tool calls name; the 5 `dialect.ts` files of a glob on drizzle-orm; `account/views.py`, from `grep | xargs`, on netbox. They learn of 6, 7 and 7 files before the edit, and of diesel's `up.sql` and `down.sql` at a later tool call. They also report each session's restored file, and on diesel `lib.rs`: `rustfmt`, named with `lib.rs`, changed only `models.rs`, a module `lib.rs` declares.
  - Formatters: each changed only files that earlier steps of its session had changed, though `dprint fmt` and `ruff check --fix` were given the whole repository and `netbox/`.
  - Diesel CLI's `migration generate` leaves `migrations/.diesel_lock`, an untracked file that is not ignored, in the edit set to the end of the session.
  - Answer on these sessions: only the diff learns of every changed file; scanned after each step, at the edit. Hooks learn only of the files edit tool calls name; declarations, of the files each step names, most of them before the edit.
- **Open:**
  - Steps stand for tool calls; time is not measured: what a scan after each tool call costs on a large worktree, and how often a timer scans.
  - A hook on the shell tool could take the files its command line names, as the declarations here do, at the edit.
  - Edits outside an agent's tool calls, such as a file watcher or a build script that writes files, are not in the sessions.
  - The declarations follow a rule, the files each step names; what an agent declares before it edits is not measured.
  - The npm installs pin the tools, not their dependencies: drizzle-kit's, such as `tsx`, `esbuild` and `@drizzle-team/brocli`, resolve to the newest matching versions on each run, so a later rerun may run other code.
