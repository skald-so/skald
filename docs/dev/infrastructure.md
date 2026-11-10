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
- Product, binary and Homebrew formula: `skald`; domain: `skald.so`.
- Where `skald` is taken: `skald-so` — the GitHub organization, npm, crates.io, PyPI.

## Repositories
| Repository | Access | Contents |
|---|---|---|
| `skald-so/skald` | Public | Monorepo: code, docs, `docker-compose.yml` for self-hosting, Claude Code plugin marketplace |
| `skald-so/skald-cloud` | Private | Production configs, deployment, anti-abuse rules, runbooks, approved UI designs (`design/`) |
| `skald-so/homebrew-tap` | Public, later | Homebrew formula, updated by dist |

## Branches
- `dev` — the main and default branch; PRs from people and agents go here.
  - Changes only through PRs with all required checks green; no approvals required; a PR need not be up to date with `dev`.
  - Required checks: `Changed parts`, `Rust`, `Web`, `Tooling`, `Conventional Commits` (accepted only from GitHub Actions), `license/cla`. A job skipped by path filters counts as passed; `Changed parts` is required because its failure skips the jobs after it.
  - Squash merges only; the PR title becomes the commit title.
- `main` — the latest release; only CI moves it.
  - Its ruleset blocks updates and deletion; only deploy keys bypass it.
  - No deploy key exists yet; the release pipeline adds one, kept only in CI secrets.
- `vX.Y.Z` tags — the releases themselves.
- PR titles: rule 3 in [rules.md](../../rules.md).
- Branch rules are GitHub rulesets; no person bypasses them, admins included.

## PR checks
All checks except e2e, the PR title and the CLA run via `script/test`, the same script agents and people use.
- **PR title:** rule 3 in [rules.md](../../rules.md), checked by `script/check-pr-title`.
- **CLA:** CLA Assistant checks that every commit author signed the CLA: [CONTRIBUTING.md](../../CONTRIBUTING.md).
- **Rust:** `fmt`, `clippy`, tests; integration tests with PostgreSQL in a container.
- **Frontend:** lint, tests, build.
- **`cargo-deny`:** dependency licenses and vulnerabilities.
- **Tooling:** tests of the scripts in `script/`, Renovate config validation.
- **Path filters:** CI runs only the parts of `script/test` that a PR changes, chosen by `script/changed-parts`; locally `script/test` runs all of them.
- **Daily:** e2e with real Claude Code and Codex.
- **Renovate** opens PRs to `dev` for Cargo, npm and GitHub Actions dependencies, with Conventional Commits titles; config in `renovate.json`.

## Release
- **Tools:** release-plz (version, CHANGELOG, tag) and dist (build, installers, publishing).
- **One version for everything:** client, server, plugin. Before 1.0: `0.x` versions.
- **The protocol** has its own version number. The server supports the current and previous versions and tells a too-old client "please update".
- **Cadence:** we release when ready, aiming for every 1–2 weeks. Fixes ship at any time.
- **Betas:** `vX.Y.Z-beta.N`, on npm with the `next` tag.
- **Updates for users:** `skald update` and plugin auto-update from the marketplace.

## Channels
| What | Where |
|---|---|
| `skald` binary: macOS, Linux, Windows; x86_64 and arm64 | GitHub Releases + checksums |
| `install.sh`, `install.ps1` | GitHub Releases. The command with a code on the website downloads them and runs `skald connect` |
| `npx skald-so` | npm |
| `skald-server` image | GHCR |
| Claude Code plugin | Monorepo: `/plugin marketplace add skald-so/skald` |
| Website and docs | GitHub Pages, built from `docs/` |
| Homebrew, winget, crates.io, Helm | Later |

## Trust in builds
- Builds get **GitHub Artifact Attestations** (Sigstore); verify with `gh attestation verify`.
- Images are signed with cosign, without stored keys.
- We publish to npm and crates.io via **trusted publishing** (OIDC); there are no long-lived tokens in CI.
- The installer verifies the checksum.
- The repository has `SECURITY.md`; private vulnerability reporting is enabled on GitHub.
- Later: reproducible builds.

## Cloud
- **The same image that is published for self-hosting.** `ee/` features are enabled by a license key.
- **Staging:** an existing VPS shared with other projects; one Caddy on it serves all sites; every merge into `dev` is deployed to it automatically.
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
- Helm later.
