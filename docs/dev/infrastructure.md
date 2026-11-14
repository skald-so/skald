# Infrastructure and releases

## Flow
```
PR (agent or contributor) → CI checks → dev → staging (automatic, :edge image)
dev → a Release PR stays open (version + CHANGELOG)
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
  - Not required: `Client on Windows`, `Windows` (the edit comparison prototype's job) and the jobs of the release dry run.
  - Squash merges only; the PR title becomes the commit title.
- `main` — the latest stable release; only CI moves it ("Release", "Main").
  - Its ruleset blocks updates and deletion; only the release pipeline's deploy key, kept only in CI secrets, bypasses it.
- `v*` tags — the releases themselves: `vX.Y.Z` and `vX.Y.Z-beta.N`.
  - Only the GitHub App `skald-release` creates them: ruleset "Release tags: only skald-release creates", the App its only bypass.
  - No one updates, moves or deletes them, the App and admins included: ruleset "Release tags: immutable", with no bypass.
- PR titles: rule 3 in [rules.md](../../rules.md).
- Branch rules are GitHub rulesets; no person bypasses them, admins included.

## PR checks
All checks except e2e, the PR title, the CLA, the Windows jobs and the release dry run run via `script/test`, the same script agents and people use.
- **PR title:** rule 3 in [rules.md](../../rules.md), checked by `script/check-pr-title`.
- **CLA:** CLA Assistant checks that every commit author signed the CLA: [CONTRIBUTING.md](../../CONTRIBUTING.md); it exempts bots, `*[bot]`.
- **Rust:** `fmt`, `clippy`, tests; integration tests with PostgreSQL in a container, those of `script/check-github-sign-in` among them, some also in headless Chrome ([CONTRIBUTING.md](../../CONTRIBUTING.md#requirements)); `skald-crypto`'s tests, `skald-proto`'s of ID generation and the MLS prototypes' natively and in WASM, in Node.js (`script/wasm-test` for the crates), with `clippy` of `skald-proto` and `skald-crypto` in WASM; the delivery prototypes' and the edit comparison prototype's tests.
- **Frontend:** lint, tests, build; kit page screenshots against their references ([web/README.md](../../web/README.md#reference-screenshots)).
- **Server image:** `crates/server/Dockerfile` is built and a container from it is tested with PostgreSQL by `script/image-test`, in the Rust part.
- **`cargo-deny`:** dependency licenses and vulnerabilities of every workspace crate, run by `script/deny`.
- **Tooling:** tests of the scripts in `script/`, the staging deploy's against a local sshd and the staging status check's on recorded GitHub API responses, of the CI workflow's job `Client on Windows` (`script/ci-workflow-test`), of the edge image and staging deploy workflow, of the Release PR workflow (`script/release-pr-workflow-test`, `script/release-tag-workflow-test`, `script/release-tag-step-test`) and of `script/install-git-cliff` and of the release workflow (`script/release-workflow-test`, `script/release-jobs-test`, `script/release-step-test`, `script/release-attest-test`, `script/release-image-test`, `script/release-npm-workflow-test`, `script/move-main-workflow-test`), and the move of `main` on fixture repositories (`script/move-main-test`); the installers and the npm package against a stand-in release on the machine that runs them (`script/installers-test`, `script/npm-package-test`); Renovate config validation.
- **Release PR and tag:** in the Rust and the Tooling parts, on fixture repositories: `script/release-pr-test` and `script/release-tag-test`. `script/release-pr-test` also checks the section git-cliff writes for `v0.1.0` against `CHANGELOG.md`, so these jobs check out the whole history and the tags.
- **Copied code:** `script/duplicates` fails on a block copied in Rust, TypeScript or shell code, found by jscpd, unless `script/duplicates-known` lists it.
- **Secrets:** PR checks need none. Live checks of agent CLIs take keys from an untracked `.env`, named in [`.env.example`](../../.env.example); deploys, from GitHub environment secrets.
  - The Release PR workflow takes the ID and private key of the GitHub App `skald-release` from `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`, secrets of the environment `release-plz`. Only `dev` deploys from it; only that workflow's jobs use it.
  - The App: installed only on `skald-so/skald`; contents and pull requests read-write, metadata read; webhook off; it bypasses no ruleset but the creation of `v*` tags ("Branches").
  - The release workflow's job `publish-npm` runs in the environment `npm`, which holds no secret ("Release", "npm"). Only `v*` tags deploy from it; only that job uses it.
  - The release workflow's job `move-main` takes the deploy key that moves `main` from `MAIN_DEPLOY_KEY`, the only secret of the environment `move-main`. Only `v*` tags deploy from it; only that job uses it, and only its step that runs `script/move-main` reads the key. The key has write access and is the repository's only deploy key.
- **Path filters:** CI runs only the parts of `script/test` that a PR changes, chosen by `script/changed-parts`; locally `script/test` runs all of them.
- **Release dry run:** `.github/workflows/release.yml` without a release tag: it builds `skald`, installs it and runs it through `npx skald-so` on every runner, and builds the `skald-server` image, as for a release, and publishes nothing. It also runs the tests of `skald update` on macOS and Windows; `script/test` runs them on Linux. A PR that changes the workflow, dist's settings (`dist-workspace.toml`, `Cargo.toml`, `crates/client/Cargo.toml`), `installers/`, `npm/`, the scripts the workflow runs or `skald update` and its tests starts it, and so does a manual run (`workflow_dispatch`).
- **Client on Windows:** all of the client's tests on `windows-2025`, with the runner image's PostgreSQL, OpenSSL and Google Chrome, in the Rust part, outside `script/test`: `.github/workflows/ci.yml`.
  - Windows runs only the tests of code whose behavior depends on the OS: today the client's. The other crates' logic does not depend on it, and the server runs only on Linux.
  - Only this job sets `SKALD_TEST_REAL_KEY_STORAGE=1`: a test then keeps a device's keys in the runner's Credential Manager, under a keyring service of its own, and deletes them. Without it, `script/test` included, no test touches the system's key storage.
  - When a test fails, the job prints the failures in PostgreSQL's log.
  - After the tests, also when one fails, a `skald-update-*` or `skald-client-*` directory left in the temporary directory fails the job.
- **Windows:** the edit comparison prototype's path normalization tests and its `spellings` check on `windows-latest`, outside `script/test`: `.github/workflows/edits-windows.yml`. A PR that changes `prototypes/edits/` or the workflow starts it, and so does a manual run (`workflow_dispatch`). Findings: [edit-prototype.md](edit-prototype.md#path-normalization).
- **Daily:** e2e with real Claude Code and Codex.
- **Renovate** opens PRs to `dev` for Cargo, npm and GitHub Actions dependencies and the Docker base images, with Conventional Commits titles; they are reviewed and merged by hand; config in `renovate.json`.

## Release
- **Tools:** git-cliff (the next version and the CHANGELOG section, `cliff.toml`), our scripts (the Release PR and the tag) and dist (the archives of `skald` and a checksum of each).
  - `script/install-git-cliff` installs the git-cliff pinned in `script/lib/release.sh` from its official release archive, checked against its SHA-256 there. The Release PR job runs it before it creates any token; the tests run it too.
- **Release PR:** on every push to `dev`, `.github/workflows/release-pr.yml` runs `script/release-pr` on the latest `dev` and its tags.
  - `script/release-update` writes the next version into `Cargo.toml` and `Cargo.lock` and its section into `CHANGELOG.md`.
  - `script/release-pr` commits them with the GraphQL mutation `createCommitOnBranch` on a new branch from `dev`, then opens the Release PR from it, or moves the open Release PR's branch to that commit and updates its title and description.
  - With nothing to release it opens none and closes an open one.
  - Its token is the App's, narrowed to contents and pull requests, so the PR checks run on the Release PR.
- **Release tag:** on the push of a Release PR's merge, the same workflow first tags that commit `vX.Y.Z`, or `vX.Y.Z-beta.N` for a beta, the workspace version, with `script/release-tag`: an annotated tag created through the GitHub API. Its token is the App's, narrowed to contents, so the workflows that run on tags start.
  - A Release PR is one the App opened from a branch of `skald-so/skald`; `script/release-pr-merged` checks it through the GitHub API before `script/release-tag` runs.
  - No other push gets a tag, whatever its PR's branch is named, and a version already tagged gets none.
  - It creates no GitHub Release.
- **GitHub Release:** on a push of a `v*` tag, `.github/workflows/release.yml` builds `skald` with dist (`dist-workspace.toml`) for macOS, Linux and Windows on x86_64 and arm64, each on a runner of its own; each runner installs it with its installer from the release's files; then one `gh release create` publishes the tag's release with all its files: the archives, a `.sha256` checksum of each, `install.sh` and `install.ps1`.
  - A pushed tag other than `v` and the workspace version fails the run before it builds.
  - The notes are the version's CHANGELOG section, by `script/release-notes`; a beta's release is a prerelease.
  - GitHub Releases hold `skald` only; `skald-server` ships as the image.
  - Linux binaries are built on Ubuntu 22.04: they need glibc 2.35 or later.
  - Before it publishes, the job that creates the release attests every file of the release, the installers and checksums included, with `actions/attest`; a dry run attests nothing.
  - That job writes contents, and attestations with an OIDC token.
- **Image:** on a push of a `v*` tag, a job of the same workflow builds the `skald-server` image at the tag's commit from `crates/server/Dockerfile`, the definition `script/image-test` tests, with no cache, and pushes it to GHCR as `:X.Y.Z`, or `:X.Y.Z-beta.N` for a beta. It does not retag `:edge`.
  - A version GHCR already has is not pushed again: the job fails before it builds. It moves no tag, and there is no `:latest`.
  - It signs the pushed digest with cosign, keyless, and attests its build provenance with `actions/attest`; a dry run builds the image and pushes, signs and attests nothing.
  - This job writes packages, and attestations with an OIDC token. No other job writes packages.
- **npm:** on a push of a `v*` tag, once the GitHub Release is published, a job of the same workflow publishes the package `skald-so`, `npm/` packed by `script/release-npm` with the release's version, under the dist-tag `latest`, or `next` for a beta.
  - Before the GitHub Release is published, each build runner runs `skald` from the release's files through the packed package (`script/npm-package-test`); a dry run publishes nothing.
  - It publishes with provenance; npm accepts the job's OIDC token only from the package's trusted publisher, `skald-so/skald`, `release.yml` and the environment `npm`. The package's publishing access requires 2FA and disallows tokens.
  - The job's token writes nothing but `id-token`.
- **Main:** on a push of a stable release's tag, `vX.Y.Z`, once the GitHub Release, the npm package and the image are published, a job of the same workflow moves `main` to the tag's commit with `script/move-main` and the deploy key ("PR checks", "Secrets"). A beta, or a release whose publishing failed, leaves `main` where it is.
  - `main` only moves forward: a tag whose commit does not descend from `main` as `origin` has it is refused, and `main` stays.
  - The job's token writes nothing, and it requests no OIDC token.
- **npm package:** `npx skald-so` downloads the release's archive for the machine's OS and architecture, checks it against its `.sha256` and runs `skald`; on a mismatch or a missing or empty checksum it stops and runs nothing. It unpacks the archive into the package's directory once, with a file `installed-by-npm` beside `skald`. `SKALD_DOWNLOAD_URL` sets where the release's files are, as for the installers.
  - It needs Node.js 22.11 or later; behind a proxy, 24.14 or later, or 25.4 or later: its downloads go through `HTTPS_PROXY` ([Network](architecture.md#network)) by Node.js's own proxy support, and on an older Node.js with a proxy set it stops and runs nothing.
- **Installers:** `installers/install.sh` (macOS, Linux) and `installers/install.ps1` (Windows), with the release tag set by `script/release-installers`, download the release's archive for the machine's OS and architecture, check it against its `.sha256` and install `skald` so a new shell runs it. On a mismatch or a missing or empty checksum they stop and install nothing. dist's own installers are not used.
  - `install.sh`: into `~/.local/bin`. Unless that directory is on the running shell's `PATH` already, it adds it to `PATH` in `~/.profile`, in `~/.bash_profile` if there is one, in `~/.bashrc` if bash is installed, and in `.zshenv` if zsh is installed.
  - `install.ps1`: into `%LOCALAPPDATA%\Programs\skald`, added to the user's `PATH`.
  - `SKALD_INSTALL_DIR` sets the directory; `SKALD_DOWNLOAD_URL`, where the release's files are instead of GitHub.
- **One version for everything:** client, server, plugin; for the crates, `[workspace.package]` in `Cargo.toml`. Before 1.0: `0.x` versions.
- **Last release:** the last `vX.Y.Z` tag; beta tags do not count.
- **Next version,** by `script/release-update`:
  - A version in `Cargo.toml` newer than the last release, set by a PR, stays; so does the version in `Cargo.toml` with no release tag yet. One older than the last release is an error.
  - Otherwise git-cliff bumps the last release by the PRs since it. In `0.x`: a breaking change (`!`, or a `BREAKING CHANGE:` footer) bumps the minor version, any other PR the patch version. From `1.0`: a breaking change the major, a `feat` the minor, any other the patch.
  - Nothing to release, so no Release PR: no PR merged since the last release, or the version is tagged already.
- **CHANGELOG:** `CHANGELOG.md` in the root gets a section per release under `## [Unreleased]`, in the groups of Keep a Changelog: the titles of every PR merged since the last release, whatever it changes, newest first; not the Release PRs' commits, `chore: release v…`.
- **The protocol** has its own version number: [protocol spec](../protocol/README.md#protocol-version).
- **Cadence:** we release when ready, aiming for every 1–2 weeks. Fixes ship at any time.
- **Betas:** `vX.Y.Z-beta.N`.
  - A PR sets a beta's version in `[workspace.package]` of `Cargo.toml`; the Release PR keeps it. Once the beta is tagged, there is no Release PR until a PR sets the next beta's or the release's version.
  - As beta tags do not count, the CHANGELOG section of a beta or a release lists the PRs of the betas before it too.
- **Updates for users:** `skald update` and plugin auto-update from the marketplace.
  - `skald update` looks up the latest stable release through GitHub's API. If it is newer than the running `skald`, it downloads the release's archive for the machine's OS and architecture, checks it against its `.sha256` and replaces the running binary in place. It trusts the checksum because releases are immutable ("Trust in builds").
  - It never installs a beta or an older version. On a failed download or a mismatched or missing checksum it exits with an error, and the old binary stays.
  - A `skald` with a file `installed-by-npm` beside it is from npm: `skald update` leaves it to npm.

## Channels
| What | Where |
|---|---|
| `skald` binary: macOS, Linux, Windows; x86_64 and arm64 | GitHub Releases + checksums. Linux artifacts carry libdbus's license, `vendor/dbus/COPYING` of libdbus-sys |
| `install.sh`, `install.ps1` | GitHub Releases. The command with a code on the website downloads them and runs `skald connect` |
| `npx skald-so` | npm |
| `skald-server` image | GHCR. Each release publishes `:X.Y.Z`, a beta `:X.Y.Z-beta.N` ("Release", "Image"); every push to `dev` publishes `:edge` and a tag with its commit: `.github/workflows/edge.yml` |
| Claude Code plugin | Monorepo: `/plugin marketplace add skald-so/skald` |
| Website and docs | GitHub Pages, built from `docs/` |

## Trust in builds
- Release files get **GitHub Artifact Attestations** (Sigstore build provenance) from the release workflow at the release tag ("Release").
- Verify a release file, here of `vX.Y.Z`, in the directory with it and its `.sha256`:
  - checksum: `sha256sum -c skald-client-<target>.tar.gz.sha256` (macOS: `shasum -a 256 -c …`; Windows: compare `(Get-FileHash skald-client-<target>.zip).Hash` with the `.sha256`);
  - attestation, with gh 2.97.0 or later: `gh attestation verify <file> --repo skald-so/skald --signer-workflow skald-so/skald/.github/workflows/release.yml --source-ref refs/tags/vX.Y.Z`. An attestation made on any other ref, or by another workflow or repository, fails it.
- Release images are signed with cosign, keyless, and get a build provenance attestation from the release workflow at the release tag ("Release", "Image"). `:edge` and the commit tags of `dev` are not signed or attested.
- GHCR tags can be moved, so a self-hoster verifies an image by its digest and signature, and pulls it by that digest: `docker buildx imagetools inspect ghcr.io/skald-so/skald-server:X.Y.Z` prints the digest; then, here for `vX.Y.Z`:
  - signature: `cosign verify ghcr.io/skald-so/skald-server@sha256:<digest> --certificate-identity https://github.com/skald-so/skald/.github/workflows/release.yml@refs/tags/vX.Y.Z --certificate-oidc-issuer https://token.actions.githubusercontent.com`;
  - attestation, as a release file's: `gh attestation verify oci://ghcr.io/skald-so/skald-server@sha256:<digest> --repo skald-so/skald --signer-workflow skald-so/skald/.github/workflows/release.yml --source-ref refs/tags/vX.Y.Z`.
- The npm package's provenance, here of `vX.Y.Z`, with gh 2.97.0 or later: `npm pack skald-so@X.Y.Z`; `curl -fsSL https://registry.npmjs.org/-/npm/v1/attestations/skald-so@X.Y.Z | jq -c '.attestations[] | select(.predicateType == "https://slsa.dev/provenance/v1") | .bundle' >skald-so.jsonl`; then `gh attestation verify skald-so-X.Y.Z.tgz --bundle skald-so.jsonl --digest-alg sha512 --repo skald-so/skald --signer-workflow skald-so/skald/.github/workflows/release.yml --source-ref refs/tags/vX.Y.Z`. Provenance made on any other ref, or by another workflow or repository, fails it.
- Workflows pin actions to a full commit SHA, and a repository setting requires it; the tools they install with `taiki-e/install-action` are pinned to exact versions and checked against checksums; git-cliff is installed from its release archive, checked against a SHA-256 ("Release").
- dist is built from the commit of its release tag, each dependency checked against its `Cargo.lock`.
- Releases are immutable, a repository setting: a published release's files and checksums never change.
- We publish to npm and crates.io via **trusted publishing** (OIDC); there are no long-lived tokens in CI.
- The repository has `SECURITY.md`; private vulnerability reporting is enabled on GitHub.
- The web UI build is reproducible: a rebuild from the release tag matches the files `app.skald.so` serves.
- Later: reproducible builds of the binaries.

## Cloud
- **The same image that is published for self-hosting.**
- **Staging:** `dev.skald.so` on an existing VPS shared with other projects; one Caddy on it serves all sites. `script/staging-status` tells whether staging runs the latest commit on `dev` and how long after its merge it got there, from public data only: the head of `dev`, its CI runs and the health check.
  - Its GitHub sign-in uses an OAuth App of its own under `skald-so`; `script/check-github-sign-in https://dev.skald.so <client ID>` checks it without a person.
  - Every merge into `dev` is deployed to it by this repository's CI: after `.github/workflows/edge.yml` publishes `:edge`, its deploy job deploys that digest over SSH with `script/deploy-staging`: one deploy at a time, only of the head of `dev`, so staging never moves back.
  - The job runs in the environment `staging`: only `dev` deploys from it, only this job uses it. Its secrets are the connection `script/deploy-staging` reads, described there: `STAGING_SSH_KEY`, `STAGING_SSH_HOST_KEY`, `STAGING_SSH_DESTINATION`. The key runs only the deploy on the VPS.
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
