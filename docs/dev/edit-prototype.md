# Edit comparison prototype

A prototype of edit coordination ([features.md §11](features.md#11-cross-machine-edit-coordination); Story 5 in [task.md](../../task.md)): machines learn that they edit the same thing from a server that sees only HMAC fingerprints.

- Code: [`prototypes/edits/`](../../prototypes/edits), a cargo workspace apart from the product crates. `edit-overlap` holds the fingerprints, the client, the server stand-in, the exact check, the stand-in for end-to-end encrypted messages and the scenario harness.
- Harness: a test repository in a bare upstream; each machine is a local clone of it. Machines edit their working trees, commit and push branches; a maintainer's clone merges a branch with a merge commit. Everything runs through the `git` CLI, without the user's global or system git config.
- Server stand-in: in-process, without the space key. It keeps each machine's latest fingerprints and tells a machine which other machines share fingerprints with it, and which; the machine maps them back to its paths.
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
| `edits_to_distant_lines_of_one_file_merge_cleanly` | Edits of the first and last function of one file overlap on the server; the check finds a clean merge |
| `modify_delete_conflict_is_found` | A deletion and an edit of one file: a modify/delete conflict on both machines |
| `rename_conflicts_are_found` | A committed rename against another rename of the same file, its deletion and an edit of its old path: rename/rename, rename/delete and a clean merge |
| `machines_on_different_bases_get_the_right_result` | One machine pulled lines added on top and the other's merged change and edits it again; the other has not pulled. An edit both make conflicts at each one's own line; the merged change and the lines on top merge cleanly; without that edit the merge is clean |
| `a_request_with_a_base_that_is_not_an_object_id_is_refused` | A base that is not an object ID never reaches the `git bundle` command line |
| `server_receives_and_keeps_nothing_of_the_exchanged_changes` | During the check of both machines the server receives nothing and keeps what it kept; nothing it received or keeps holds a path, file or directory name or line of the changes |

## Questions
The prototype answers these questions of [features.md §11](features.md#11-cross-machine-edit-coordination), "To decide", in the iteration given:
| Question | Iteration |
|---|---|
| Data source: diff, hooks or declarations | 5.8; the earlier ones use the diff |
| When to warn | 5.5: warnings at each level against `git merge-tree` conflicts |
| Reservation TTL | 5.5: how long each side of a merge lived |
| How to catch edits made via Bash and generators | 5.8 |
| Path normalization | 5.7 |
| Noise from lockfiles and migrations | 5.6 |
| Whether intents like "replacing / extending" are needed | 5.5: whether both sides of a warning only add lines |

## File fingerprints
- **Edit set:** `git diff --no-renames --name-only <merge base>` against the working tree, and `git ls-files --others --exclude-standard`.
  - The merge base is of `HEAD` and `refs/remotes/origin/HEAD`, the upstream's default branch that `git clone` records.
  - A machine leaves an overlap only once it fetches the merge of its changes; until then the server keeps its old fingerprints.
  - `--no-renames` lists a rename as a deletion and an addition, so both paths count without rename detection and its similarity threshold.
  - Ignored files, such as build output, are not edits.
- **Fingerprint:** HMAC-SHA256 with the space key over `"skald edit path"`, the scope and the path, each after its length as a big-endian `u64`; 32 bytes per path. The path is as git gives it: relative to the root, with `/` separators, not normalized (5.7).
- **Scope:** the least root commit of the upstream's default branch.
  - Every clone has it without setup. It goes only into the HMAC; the server never sees it.
  - Rejected: the remote URL, which differs between HTTPS and SSH and for mirrors; a random ID per repository, which needs a step on each machine.
  - Forks and mirrors share it, so their machines overlap.
  - Repositories with byte-identical root commits share it: the same tree, message, author and time.
  - A shallow clone takes its shallow boundary for the root, so it gets another scope than a full clone. Open: a scope for shallow clones.
- **Publication:** each machine sends the fingerprints of its whole edit set, which replace its earlier ones. The server sees the machine, how many paths it edits and when that changes, and which machines share fingerprints.

## Exact check
- **Exchange:** on an overlap, a machine asks the other for its changes over an end-to-end encrypted message; the request carries the asker's merge base with the upstream's default branch.
- **What the other machine sends:** a git bundle of a snapshot of its working tree, a commit on top of `HEAD`, with its history since the asker's base.
  - The snapshot holds committed, uncommitted and untracked changes, not ignored files: `git add --all` into a copy of the index, `write-tree`, `commit-tree -p HEAD`. The index, the working tree and the refs stay as they are; its objects stay unreferenced until `git gc`.
  - A sender that lacks the asker's base, which has pulled newer upstream commits, bundles since its own base, an ancestor of the asker's. A sender whose base is newer also sends the upstream commits between the two.
  - The sender checks that the asker's base is an object ID before it goes into a `git bundle` command line.
  - The asker stores the bundle's objects with `git bundle unbundle`. That fails when the asker lacks the bundle's base, such as after a force-push to the default branch.
  - Size: bundles are thin packs, so changed files go as deltas against the base. In a repository of 40 files of 28 KB: a one-line uncommitted edit, 454 bytes; one line in each of 10 files, committed, 1.5 KB; with a new untracked 109 KB file, 45 KB: new files go whole, compressed.
- **Check:** `git merge-tree --write-tree -z --messages` on the asker's snapshot and the other's, from their merge base. Git 2.39 is the first whose `-z` messages give each conflict's type and paths.
  - Result: whether the merge is clean, by the exit status; each `CONFLICT` message's type and paths; each conflicted file with the conflicting ranges of the asker's own lines.
  - Lines: the asker's side of each conflict marker in the merged file. Outside the conflicts the merged file has the other side's clean changes too; a `git diff` from the asker's version to the merged file resolved to the asker's side maps the ranges back.
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
