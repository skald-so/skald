# Edit comparison prototype

A prototype of edit coordination ([features.md §11](features.md#11-cross-machine-edit-coordination); Story 5 in [task.md](../../task.md)): machines learn that they edit the same thing from a server that sees only HMAC fingerprints.

- Code: [`prototypes/edits/`](../../prototypes/edits), a cargo workspace apart from the product crates. `edit-overlap` holds the fingerprints, the client, the server stand-in and the scenario harness.
- Harness: a test repository in a bare upstream; each machine is a local clone of it. Machines edit their working trees, commit and push branches; a maintainer's clone merges a branch with a merge commit. Everything runs through the `git` CLI, without the user's global or system git config.
- Server stand-in: in-process, without the space key. It keeps each machine's latest fingerprints and tells a machine which other machines share fingerprints with it, and which; the machine maps them back to its paths.
- Run: `cargo test --manifest-path prototypes/edits/Cargo.toml`; it needs `git`. `script/test` runs it.

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
