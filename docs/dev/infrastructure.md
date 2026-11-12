# Infrastructure and releases

## Flow
```
PR (agent or contributor) → CI checks → dev → staging (automatic, :edge image)
dev → release-plz keeps a Release PR open (version + CHANGELOG)
merge Release PR → tag vX.Y.Z → dist builds → GitHub Releases, npm, GHCR
                              → main moves to the tag
                              → production (same image, one click)
```
A human takes one action per release: merging the Release PR.

## Names
- Product and binary: `skald`; domain: `skald.so`.
- Where `skald` is taken: `skald-so` — the GitHub organization, npm, crates.io, PyPI.

## Repositories
| Repository | Access | Contents |
|---|---|---|
| `skald-so/skald` | Public | Monorepo: code, docs, `docker-compose.yml` for self-hosting, Claude Code plugin marketplace |
| `skald-so/skald-cloud` | Private | Production and staging configs, deployment, anti-abuse rules, runbooks, approved UI designs (`design/`) |

## Branches
- `dev` — the main and default branch; PRs from people and agents go here.
  - Changes only through PRs with all required checks green; no approvals required; a PR need not be up to date with `dev`.
  - Required checks: `Changed parts`, `Rust`, `Web`, `Tooling`, `Conventional Commits` (accepted only from GitHub Actions), `license/cla`. A job skipped by path filters counts as passed; `Changed parts` is required because its failure skips the jobs after it.
  - `Windows`, the edit comparison prototype's job, and the jobs of the release dry run are not required.
  - Squash merges only; the PR title becomes the commit title.
- `main` — the latest release; only CI moves it.
  - Its ruleset blocks updates and deletion; only the release pipeline's deploy key, kept only in CI secrets, bypasses it.
- `v*` tags — the releases themselves: `vX.Y.Z` and `vX.Y.Z-beta.N`.
  - Only the GitHub App `skald-release` creates them: ruleset "Release tags: only skald-release creates", the App its only bypass.
  - No one updates, moves or deletes them, the App and admins included: ruleset "Release tags: immutable", with no bypass.
- PR titles: rule 3 in [rules.md](../../rules.md).
- Branch rules are GitHub rulesets; no person bypasses them, admins included.

## PR checks
All checks except e2e, the PR title, the CLA, the edit comparison prototype's Windows job and the release dry run run via `script/test`, the same script agents and people use.
- **PR title:** rule 3 in [rules.md](../../rules.md), checked by `script/check-pr-title`.
- **CLA:** CLA Assistant checks that every commit author signed the CLA: [CONTRIBUTING.md](../../CONTRIBUTING.md); it exempts bots, `*[bot]`.
- **Rust:** `fmt`, `clippy`, tests; integration tests with PostgreSQL in a container, those of the server's GitHub sign-in and of the client's sign-in also in headless Chrome; the MLS prototypes' tests natively and in WASM, in Node.js; the delivery prototypes' and the edit comparison prototype's tests.
- **Frontend:** lint, tests, build; kit page screenshots against their references ([web/README.md](../../web/README.md#reference-screenshots)).
- **Server image:** `crates/server/Dockerfile` is built and a container from it is tested with PostgreSQL by `script/image-test`, in the Rust part.
- **`cargo-deny`:** dependency licenses and vulnerabilities of every workspace crate, run by `script/deny`.
- **Tooling:** tests of the scripts in `script/`, the staging deploy's against a local sshd and the staging status check's on recorded GitHub API responses, of the edge image and staging deploy workflow, of the release-plz workflow (`script/release-plz-workflow-test`, `script/release-tag-workflow-test`, `script/release-tag-step-test`) and of the release workflow (`script/release-workflow-test`, `script/release-jobs-test`, `script/release-step-test`, `script/release-attest-test`); the installers against a stand-in release on the machine that runs them (`script/installers-test`); Renovate config validation.
- **Release PR and tag:** in the Rust and the Tooling parts, release-plz on fixture repositories: `script/release-plz-test` and `script/release-tag-test`, and `script/release-plz-config-test` for `release-plz.toml` against this repository.
- **Copied code:** `script/duplicates` fails on a block copied in Rust, TypeScript or shell code, found by jscpd, unless `script/duplicates-known` lists it.
- **Secrets:** PR checks need none. Live checks of agent CLIs take keys from an untracked `.env`, named in [`.env.example`](../../.env.example); deploys, from GitHub environment secrets.
  - The release-plz workflow takes the ID and private key of the GitHub App `skald-release` from `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`, secrets of the environment `release-plz`. Only `dev` deploys from it; only the release-plz jobs use it.
  - The App: installed only on `skald-so/skald`; contents and pull requests read-write, metadata read; webhook off; it bypasses no ruleset but the creation of `v*` tags ("Branches").
- **Path filters:** CI runs only the parts of `script/test` that a PR changes, chosen by `script/changed-parts`; locally `script/test` runs all of them.
- **Release dry run:** `.github/workflows/release.yml` without a release tag: it builds `skald` and installs it on every runner, as for a release, and publishes nothing. A PR that changes the workflow, dist's settings (`dist-workspace.toml`, `Cargo.toml`, `crates/client/Cargo.toml`), `installers/` or the scripts the workflow runs starts it, and so does a manual run (`workflow_dispatch`).
- **Windows:** the edit comparison prototype's path normalization tests and its `spellings` check on `windows-latest`, outside `script/test`: `.github/workflows/edits-windows.yml`. A PR that changes `prototypes/edits/` or the workflow starts it, and so does a manual run (`workflow_dispatch`). Findings: [edit-prototype.md](edit-prototype.md#path-normalization).
- **Daily:** e2e with real Claude Code and Codex.
- **Renovate** opens PRs to `dev` for Cargo, npm and GitHub Actions dependencies and the Docker base images, with Conventional Commits titles; they are reviewed and merged by hand; config in `renovate.json`.

## Release
- **Tools:** release-plz (version, CHANGELOG, tag) and dist (the archives of `skald` and a checksum of each).
- **Release PR:** on every push to `dev`, `.github/workflows/release-plz.yml` runs release-plz with `release-plz.toml` on the latest `dev` and its tags; it opens the Release PR or updates the open one. Its token is the App's, narrowed to contents and pull requests, so the PR checks run on the Release PR.
- **Release tag:** on the push of a Release PR's merge, the same workflow first tags that commit `vX.Y.Z`, or `vX.Y.Z-beta.N` for a beta, with `release-plz release`. Its token is the App's, narrowed to contents, so the workflows that run on tags start.
  - A Release PR is one the App opened from a branch of `skald-so/skald`; `script/release-pr-merged` checks it through the GitHub API before release-plz runs.
  - No other push gets a tag, whatever its PR's branch is named, and a version already tagged gets none.
  - It publishes no crate to crates.io and creates no GitHub Release.
- **GitHub Release:** on a push of a `v*` tag, `.github/workflows/release.yml` builds `skald` with dist (`dist-workspace.toml`) for macOS, Linux and Windows on x86_64 and arm64, each on a runner of its own; each runner installs it with its installer from the release's files; then one `gh release create` publishes the tag's release with all its files: the archives, a `.sha256` checksum of each, `install.sh` and `install.ps1`.
  - A pushed tag other than `v` and the workspace version fails the run before it builds.
  - The notes are the version's CHANGELOG section, by `script/release-notes`; a beta's release is a prerelease.
  - GitHub Releases hold `skald` only; `skald-server` ships as the image.
  - Linux binaries are built on Ubuntu 22.04: they need glibc 2.35 or later.
  - Before it publishes, the job that creates the release attests every file of the release, the installers and checksums included, with `actions/attest`; a dry run attests nothing.
  - Only that job writes: contents, and attestations with an OIDC token.
- **Installers:** `installers/install.sh` (macOS, Linux) and `installers/install.ps1` (Windows), with the release tag set by `script/release-installers`, download the release's archive for the machine's OS and architecture, check it against its `.sha256` and install `skald` so a new shell runs it. On a mismatch or a missing or empty checksum they stop and install nothing. dist's own installers are not used.
  - `install.sh`: into `~/.local/bin`. Unless that directory is on the running shell's `PATH` already, it adds it to `PATH` in `~/.profile`, in `~/.bash_profile` if there is one, in `~/.bashrc` if bash is installed, and in `.zshenv` if zsh is installed.
  - `install.ps1`: into `%LOCALAPPDATA%\Programs\skald`, added to the user's `PATH`.
  - `SKALD_INSTALL_DIR` sets the directory; `SKALD_DOWNLOAD_URL`, where the release's files are instead of GitHub.
- **One version for everything:** client, server, plugin; for the crates, `[workspace.package]` in `Cargo.toml`. Before 1.0: `0.x` versions.
- **Bumps in `0.x`:** of the PRs the CHANGELOG lists, one whose title has `!` bumps the minor version; any other, of any type, the patch version.
- **Last release:** the last `vX.Y.Z` tag, not crates.io; beta tags do not count.
- **CHANGELOG:** `CHANGELOG.md` in the root lists the titles of the PRs merged since the last release, except those of PRs that change only `prototypes/`: prototypes ship in no release, and they belong to no package of the workspace, whose files release-plz counts.
- **First Release PR:** with no release tag yet, it keeps the version in `Cargo.toml` and lists every PR merged into `dev` so far, those that change only `prototypes/` included; the exception for them starts with the first release tag.
- **The protocol** has its own version number: [protocol spec](../protocol/README.md#protocol-version).
- **Cadence:** we release when ready, aiming for every 1–2 weeks. Fixes ship at any time.
- **Betas:** `vX.Y.Z-beta.N`, on npm with the `next` tag.
  - A PR sets a beta's version in `[workspace.package]` of `Cargo.toml`; the Release PR keeps it, after the beta too, until a PR sets the next beta's or the release's.
  - As beta tags do not count, the CHANGELOG section of a beta or a release lists the PRs of the betas before it too.
- **Updates for users:** `skald update` and plugin auto-update from the marketplace.

## Channels
| What | Where |
|---|---|
| `skald` binary: macOS, Linux, Windows; x86_64 and arm64 | GitHub Releases + checksums. Linux artifacts carry libdbus's license, `vendor/dbus/COPYING` of libdbus-sys |
| `install.sh`, `install.ps1` | GitHub Releases. The command with a code on the website downloads them and runs `skald connect` |
| `npx skald-so` | npm |
| `skald-server` image | GHCR. Every push to `dev` publishes `:edge` and a tag with its commit: `.github/workflows/edge.yml` |
| Claude Code plugin | Monorepo: `/plugin marketplace add skald-so/skald` |
| Website and docs | GitHub Pages, built from `docs/` |

## Trust in builds
- Release files get **GitHub Artifact Attestations** (Sigstore build provenance) from the release workflow at the release tag ("Release").
- Verify a release file, here of `vX.Y.Z`, in the directory with it and its `.sha256`:
  - checksum: `sha256sum -c skald-client-<target>.tar.gz.sha256` (macOS: `shasum -a 256 -c …`; Windows: compare `(Get-FileHash skald-client-<target>.zip).Hash` with the `.sha256`);
  - attestation, with gh 2.97.0 or later: `gh attestation verify <file> --repo skald-so/skald --signer-workflow skald-so/skald/.github/workflows/release.yml --source-ref refs/tags/vX.Y.Z`. An attestation made on any other ref, or by another workflow or repository, fails it.
- Images are signed with cosign, without stored keys.
- Workflows pin actions to a full commit SHA, and a repository setting requires it; the tools they install with `taiki-e/install-action` are pinned to exact versions and checked against checksums.
- dist is built from the commit of its release tag, each dependency checked against its `Cargo.lock`.
- Releases are immutable, a repository setting: a published release's files and checksums never change.
- We publish to npm and crates.io via **trusted publishing** (OIDC); there are no long-lived tokens in CI.
- The repository has `SECURITY.md`; private vulnerability reporting is enabled on GitHub.
- The web UI build is reproducible: a rebuild from the release tag matches the files `app.skald.so` serves.
- Later: reproducible builds of the binaries.

## Cloud
- **The same image that is published for self-hosting.**
- **Staging:** `dev.skald.so` on an existing VPS shared with other projects; one Caddy on it serves all sites; every merge into `dev` is deployed to it automatically. `script/staging-status` tells whether staging runs the latest commit on `dev` and how long after its merge it got there, from public data only: the head of `dev`, its CI runs and the health check.
  - The deploy runs in this repository's CI: after `.github/workflows/edge.yml` publishes `:edge`, its deploy job deploys that digest over SSH with `script/deploy-staging`: one deploy at a time, only of the head of `dev`, so staging never moves back.
  - The job runs in the environment `staging`: only `dev` deploys from it, only this job uses it. Its secrets: `STAGING_SSH_KEY`, the deploy key; `STAGING_SSH_HOST_KEY`, the host key the job pins; `STAGING_SSH_DESTINATION`, where it connects. The key runs only the deploy on the VPS.
- **Production:** dedicated Hetzner servers, a primary and a replica.
  - PostgreSQL 19 with synchronous streaming replication: `stored` is sent after the replica has the write. If the replica is down, replication switches to asynchronous and an alert fires.
  - WAL-G backups to object storage.
- **Deployment:**
  - Docker Compose + Caddy; Caddy obtains TLS automatically;
  - production rollout requires manual approval in GitHub Environments;
  - migrations are backward compatible only;
  - short downtime during a deploy is acceptable: clients reconnect and catch up from the cursor. Zero-downtime deploys (Kamal) later, if needed.
- **Monitoring:** Prometheus, Grafana, Sentry, uptime checks, status page.
- **Addresses:** `skald.so` — website, `app.` — web UI, `api.` — API, `docs.` — documentation.

## Self-host
- `docker compose up` brings up `skald-server`, PostgreSQL and Caddy. The user sets their domain in a single variable, and HTTPS works right away.
- Attachments are kept in a disk volume by default; an external S3-compatible bucket is optional.
