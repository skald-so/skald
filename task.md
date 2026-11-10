## Stories

Documents listed in the "Docs" fields are in `docs/dev/`.

| # | Story | Release | Depends on |
|---|---------|-------|------------|
| 1 | Monorepo skeleton and CI | MVP | — |
| 2 | Protocol spec and threat model | MVP | 1 |
| 3 | MLS prototypes and library choice | MVP | 1 |
| 4 | Verifying delivery to Claude Code and Codex | MVP | 1 |
| 5 | Edit comparison prototype with HMAC fingerprints | MVP | 1 |
| 6 | Staging environment | MVP | 1 |
| 7 | Release pipeline | MVP | 1 |
| 8 | GitHub sign-in and device certificate | MVP | 1, 2 |
| 9 | Crypto core | MVP | 2, 3 |
| 10 | Spaces and agent addresses | MVP | 8 |
| 11 | Encrypted mailbox | MVP | 10 |
| 12 | MLS channels, roles and device revocation | MVP | 9, 11 |
| 13 | Client: one-shot mode and helper | MVP | 11 |
| 14 | MCP server and agent commands | MVP | 11 |
| 15 | Encrypted channel history | MVP | 12 |
| 16 | Recovery key and new device linking | MVP | 15 |
| 17 | Claude Code adapter and plugin | MVP | 4, 13, 14 |
| 18 | Codex adapter | MVP | 4, 13, 14 |
| 19 | Presence, statuses and loop protection | MVP | 12, 14 |
| 20 | Web UI: non-secret sections | MVP | 8, 10, 11 |
| 21 | Space invite by link | MVP | 12, 13, 20 |
| 22 | One-command machine connect | MVP | 7, 13, 16, 17, 18, 20 |
| 23 | Production cloud, website and monitoring | MVP | 6, 7 |
| 24 | End-to-end MVP scenario | MVP | 19, 21, 22 |
| 25 | Admin rights transfer and account deletion | MVP | 12, 15, 20 |
| 26 | Load testing | MVP | 19, 23 |
| 27 | Documentation: quick start and CLI reference | MVP | 22, 23 |
| 28 | Website: main page | MVP | 23 |
| 29 | Operator CLI | MVP | 11, 25 |
| 30 | Service agents: HPKE and scoped tokens | Post-MVP | 12, 20 |
| 31 | GitHub Action | Post-MVP | 13, 30 |
| 32 | Task queue: claim, lease and DLQ | Post-MVP | 12, 14 |
| 33 | Task queue: scheduling and agents | Post-MVP | 17, 18, 31, 32 |
| 34 | Codex background agents via app-server | Post-MVP | 18 |
| 35 | Browser linking and history on the web | Post-MVP | 9, 16, 20 |
| 36 | Web: queues, delivery trace, loops | Post-MVP | 19, 32, 35 |
| 37 | Shared channels between organizations | Post-MVP | 12, 20 |
| 38 | Key verification without trusting the server | Post-MVP | 16 |
| 39 | Google and corporate IdP sign-in | Post-MVP | 8 |
| 40 | ask_user and attachments | Post-MVP | 19 |
| 41 | Shared team instructions | Post-MVP | 17, 18 |
| 42 | Gemini, Cursor, OpenCode and Copilot adapters | Post-MVP | 14, 22 |
| 43 | Edit coordination: file level | Post-MVP | 5, 17, 18, 32 |
| 44 | Edit coordination: hunk and symbol | Post-MVP | 43 |
| 45 | Web: security center, human inbox, edit map | Post-MVP | 35, 38, 40, 43 |
| 46 | Public cloud: anti-abuse | Post-MVP | 20, 23 |
| 47 | Operator console | Post-MVP | 25, 46 |
| 48 | Alerts, analytics and token usage | Post-MVP | 30, 32, 35 |
| 49 | Enterprise: SSO, SAML, SCIM and license | Post-MVP | 39 |
| 50 | Enterprise: audit and retention policies | Post-MVP | 15, 49 |
| 51 | Self-host: Docker Compose, Helm, documentation | Post-MVP | 7, 39, 49 |
| 52 | History export and migration to a self-hosted server | Post-MVP | 15, 16, 51 |
| 53 | Python and TypeScript SDKs | Post-MVP | 13, 32 |
| 54 | Homebrew, winget and cargo-binstall | Post-MVP | 7 |
| 55 | Managed keys: custodian device | Post-MVP | 15, 35 |
| 56 | Hosted MCP for ChatGPT and Claude.ai | Post-MVP | 14, 55 |
| 57 | skald ui | Post-MVP | 7, 35 |
| 58 | Sealed sender and padding | Post-MVP | 11, 12 |
| 59 | Server federation | Post-MVP | 37, 38 |
| 60 | Capacity view | Post-MVP | 4, 33 |

**Order and parallelism**

A wave starts when the dependencies of its stories are done; stories within a wave can be done in parallel. Priority is the order in this file.

- Wave 1: 1.
- Wave 2: 2, 3, 4, 5, 6, 7.
- Wave 3: 8, 9, 23 (23 can be postponed until launch).
- Wave 4: 10, 28.
- Wave 5: 11.
- Wave 6: 12, 13, 14, 20.
- Wave 7: 15, 17, 18, 19, 21.
- Wave 8: 16, 25, 26.
- Wave 9: 22, 29.
- Wave 10: 24, 27 — end of MVP.
- Wave 11: 30, 32, 34, 35, 37, 38, 39, 40, 41, 42, 46, 54, 58.
- Wave 12: 31, 36, 43, 47, 48, 49, 53, 55, 57, 59.
- Wave 13: 33, 44, 45, 50, 51, 56.
- Wave 14: 52, 60.

### Story 1: Monorepo skeleton and CI

**Goal:** A public monorepo where the server, client and SPA build and run, and every PR passes CI checks.

**Scope:**
- Cargo workspace with the crates `proto`, `crypto`, `server`, `client`, `mcp`, `adapters`; an `ee/` directory.
- The server starts, connects to PostgreSQL, applies migrations, responds to a health check and publishes the OpenAPI spec.
- The `skald` binary builds and shows its version.
- SPA skeleton (TypeScript, Vite, React) with an API client generated from OpenAPI.
- PR checks: `fmt`, `clippy`, tests, integration tests with PostgreSQL in a container, frontend lint/tests/build, `cargo-deny`, path filters, PR title per Conventional Commits. Renovate.
- Branches: `dev` is the main branch; `main` is CI-only.
- `script/test` builds everything and runs all tests; exit code 0 only if everything passed. CI, agents and people call it.
- Licenses are fixed up front: server — AGPL-3.0; protocol, crypto core, client, MCP, adapters, SDKs, web UI — `MIT OR Apache-2.0`; `ee/` — commercial. CLA via CLA Assistant, `TRADEMARKS.md`, `SECURITY.md`, private vulnerability reporting.
- `CONTRIBUTING.md`: how to pick a story from `task.md` and send a PR to `dev`.
- **Done when:** a PR to `dev` passes all checks; the server and `skald` run locally; `script/test` passes and fails on a broken test.

- **Release:** MVP
- **Depends on:** —
- **Docs:** `tech-stack.md`, `infrastructure.md` — "Repositories", "Branches", "PR checks", "Trust in builds", `features.md` — "Product"

#### Iteration 1.1: Cargo workspace, `skald --version` and `script/test`
- **Goal:** The workspace builds, and one command checks everything.
- **What we do:**
  - Cargo workspace with the crates `proto`, `crypto`, `server`, `client`, `mcp`, `adapters`.
  - The `client` crate builds the `skald` binary; `skald --version` prints the version.
  - `script/test`: builds the workspace, checks `fmt`, runs `clippy` with warnings as errors and all tests; exit code 0 only if everything passed; the working tree stays clean.
  - Tests: `skald --version` output.
- **Definition of done:** `script/test` passes and exits non-zero on a broken test, a `fmt` or a `clippy` violation; `skald --version` prints the workspace version; CLI integration test.

#### Iteration 1.2: CI checks on pull requests
- **Goal:** Every PR to `dev` is checked automatically.
- **What we do:**
  - A GitHub Actions workflow runs `script/test` on every PR to `dev`.
  - A check fails a PR whose title does not follow Conventional Commits (rule 3 in `rules.md`).
- **Definition of done:** both checks run and pass on this iteration's PR; a title without a Conventional Commits type fails the title check.

#### Iteration 1.3: Licenses, policies and `cargo-deny`
- **Goal:** Licenses and contribution terms are fixed before outside contributions.
- **What we do:**
  - `server` under AGPL-3.0; `proto`, `crypto`, `client`, `mcp`, `adapters` under `MIT OR Apache-2.0`; license texts in the repository.
  - An `ee/` directory under a commercial license.
  - `cargo-deny` in `script/test`: dependency licenses compatible with the crate licenses, known vulnerabilities.
  - `TRADEMARKS.md`: the policy for the Skald name and logo.
  - `SECURITY.md`: vulnerabilities are reported privately; private vulnerability reporting is enabled on GitHub.
  - CLA: CLA Assistant is linked to the repository, the text is in [this gist](https://gist.github.com/DanilZabolotskiy/fdc5ad4dbb10cf670e8af59e1775b54e), the maintainer and bots are exempt; `CONTRIBUTING.md` links to the gist instead of copying the text.
- **Definition of done:** each crate declares its license; `script/test` fails on a dependency with a disallowed license or a known vulnerability; the CLA check runs on PRs; private vulnerability reporting is on.

#### Iteration 1.4: Server health check and OpenAPI spec
- **Goal:** `skald-server` starts and publishes its API contract.
- **What we do:**
  - The OpenAPI spec in the repository describes the health check (rule 4 in `rules.md`).
  - The `server` crate builds `skald-server`: it starts, serves the health check and publishes the spec.
  - Tests: the health check response conforms to the spec; the published spec matches the one in the repository.
- **Definition of done:** `skald-server` runs locally; the health check and the spec respond over HTTP; integration tests run in `script/test`.

#### Iteration 1.5: PostgreSQL and migrations
- **Goal:** The server works with PostgreSQL and keeps its schema up to date.
- **What we do:**
  - `skald-server` connects to PostgreSQL and applies pending migrations at start; a connection or migration failure stops the start with an error.
  - The health check fails when PostgreSQL is unreachable; the spec is updated first.
  - `script/test` starts PostgreSQL 19 in a container for integration tests and removes it afterwards.
  - Tests: migrations on an empty database, a restart without re-applying them, the health check with the database up and down.
- **Definition of done:** `skald-server` runs locally against PostgreSQL; integration tests with PostgreSQL in a container pass in `script/test` and CI.

#### Iteration 1.6: SPA skeleton with generated API client
- **Goal:** The web UI builds and talks to the server only through the client generated from the spec.
- **What we do:**
  - SPA on TypeScript, Vite and React, licensed `MIT OR Apache-2.0`.
  - The API client and mocks are generated from the OpenAPI spec; a stale client cannot pass `script/test`.
  - The page shows the server health via the generated client.
  - `script/test` runs frontend lint, tests and build.
  - Tests: the page renders the health status using the generated mocks.
- **Definition of done:** the SPA builds and shows the health status of a local server; frontend component tests pass in `script/test` and CI.

#### Iteration 1.7: CI path filters and Renovate
- **Goal:** CI checks only the changed parts, and dependencies stay up to date.
- **What we do:**
  - Path filters: a PR skips the Rust or frontend checks when their files are unchanged; changes to the OpenAPI spec, `script/test` or CI run everything.
  - Renovate opens PRs to `dev` for Cargo, npm and GitHub Actions dependencies, with Conventional Commits titles.
- **Definition of done:** a frontend-only PR skips the Rust checks and vice versa; `script/test` run locally still checks everything; the Renovate config passes validation.

#### Iteration 1.8: Branch protection and `CONTRIBUTING.md`
- **Goal:** Changes reach `dev` only through checked PRs, and a contributor knows how to send one.
- **What we do:**
  - `dev` is the default branch; it accepts changes only through PRs with all checks green, without required approvals.
  - Only squash merges, with the PR title as the commit title.
  - Only CI can update `main`.
  - `CONTRIBUTING.md`: picking a story from `task.md`, running `skald-server` and `skald` locally, `script/test`, sending a PR to `dev`, the CLA; links to `rules.md` instead of repeating it.
- **Definition of done:** a PR to `dev` passes all checks and merges, including one with checks skipped by path filters; branch rules reject direct pushes to `dev` and `main`; following `CONTRIBUTING.md`, the server and `skald` run locally.

### Story 2: Protocol spec and threat model

**Goal:** Pin down the protocol and the threat model that all other stories rely on.

**Scope:**
- Spec: envelope (plaintext header + ciphertext), agent address with the server name, receipts and their transitions, mailbox and cursor, queues and fencing, presence, edit coordination via HMAC fingerprints.
- The protocol accounts from the start for the custodian device (feature 13), the auditor device (`ee/`) and federation (feature 15).
- Threat model: what the server sees and does not see, trade-offs, prompt injection, loops.
- The protocol version is a separate number; the server accepts the current and previous ones and tells a too-old client "please update".
- Protocol types in the `proto` crate.
- Encoding: JSON over HTTPS for the API (OpenAPI); envelopes in binary WebSocket frames.
- The spec and threat model are published openly, marked "no external audit".
- **Done when:** the spec and threat model are in the repository docs; client and server negotiate the protocol version; an outdated client gets "please update".

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `roadmap.md` — "Preparation", `architecture.md` — "Data model", "Cryptography", "Queue", "Security", `infrastructure.md` — "Release", `features.md` §1, §10, §13, §15

### Story 3: MLS prototypes and library choice

**Goal:** Compare OpenMLS and mls-rs on Skald scenarios and choose the library for the crypto core.

**Scope:**
- A prototype on each library: device group, adding, removing with a new epoch, KeyPackages, Commit ordering, WASM build.
- Comparison: RFC 9420 compliance, audit, API ergonomics, performance, WASM size, maintenance.
- The decision and its rationale are recorded in the docs and in `tech-stack.md`.
- **Done when:** one library is chosen; prototypes and benchmarks are in the repository.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `roadmap.md` — "Preparation", `architecture.md` — "Cryptography", `tech-stack.md`

### Story 4: Verifying delivery to Claude Code and Codex

**Goal:** On current versions of Claude Code and Codex, confirm the paths for delivering messages into a live session and find out what subscription limit data they expose.

**Scope:**
- Claude Code: Channels, `Stop` and `PostToolUse` hooks with `additionalContext`, MCP `inbox`.
- Codex: `PostToolUse`, `Stop`, `UserPromptSubmit` hooks, `codex queue`, app-server (`turn/start`, `turn/steer`).
- What subscription limit data the CLIs themselves show the user, taking vendor rules into account.
- A minimal prototype for each path.
- **Done when:** a report is in the docs; the primary and fallback paths are confirmed for each agent; the "Delivery to agents" table in `architecture.md` is up to date.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `roadmap.md` — "Preparation", `architecture.md` — "Delivery to agents", `features.md` §7, §14

### Story 5: Edit comparison prototype with HMAC fingerprints

**Goal:** Verify that edit overlaps between machines are found via HMAC fingerprints while the server sees neither paths nor code.

**Scope:**
- Fingerprints of paths, hunks and symbols with the space key.
- Fingerprint comparison on the server, exact check on clients via `git merge-tree`.
- Measurements on real repositories: noise from lockfiles, migrations and generated code; path and case normalization on macOS and Windows; edits via Bash and generators.
- Preliminary answers to the questions in `features.md` §11 "To decide".
- **Done when:** the prototype finds overlaps in test scenarios; the findings are recorded in the docs.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `roadmap.md` — "Preparation", `features.md` §11, `architecture.md` — "Cryptography"

### Story 6: Staging environment

**Goal:** Every merge into `dev` is deployed to staging automatically, so stories can be checked on a live server.

**Scope:**
- `skald-server` Docker image; on a merge into `dev`, CI builds the `:edge` image and publishes it to GHCR.
- Private `skald-cloud` repository with configs and deployment.
- Staging on an existing VPS shared with other projects: Docker Compose, PostgreSQL; the server's shared Caddy with automatic TLS.
- Migrations are backward compatible only.
- **Done when:** a merge into `dev` shows up on staging within a few minutes; the health check responds over HTTPS.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `infrastructure.md` — "Flow", "Repositories", "Cloud"

### Story 7: Release pipeline

**Goal:** A release ships with one action, merging the Release PR, and the artifacts are signed and verifiable.

**Scope:**
- release-plz maintains a Release PR with the version and CHANGELOG from Conventional Commits; one version for the client, server and plugin, `0.x` before 1.0.
- Merging the Release PR creates the `vX.Y.Z` tag; dist builds `skald` for macOS, Linux, Windows (x86_64, arm64) and publishes it to GitHub Releases with checksums; `main` moves to the tag.
- npm package with prebuilt binaries (`npx skald-so`), server image in GHCR.
- GitHub Artifact Attestations, cosign image signing without stored keys, trusted publishing to npm.
- Betas `vX.Y.Z-beta.N`, on npm with the `next` tag.
- The `skald update` command.
- **Done when:** merging the Release PR publishes the release to all channels; `gh attestation verify` passes; `skald update` installs the new version.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `infrastructure.md` — "Flow", "Release", "Channels", "Trust in builds", `tech-stack.md`

### Story 8: GitHub sign-in and device certificate

**Goal:** The user signs in with GitHub, and the machine gets its own keys and a device certificate.

**Scope:**
- Sign-in via GitHub OIDC; device flow (RFC 8628) for machines without a browser. The account is created on first sign-in.
- Device keys (signing and HPKE) are generated on the machine and never leave it; they are stored in Keychain, libsecret or Windows Credential Manager, otherwise in a file with 0600 permissions.
- Device certificate "device key → account", signed by the account and the server.
- Client local state in SQLite.
- CLI: sign in, sign out, list your devices.
- **Done when:** on a machine without a browser, `skald` signs in via device flow and gets a certificate; the server stores only public keys.

- **Release:** MVP
- **Depends on:** Story 1, Story 2
- **Docs:** `features.md` §5, `architecture.md` — "Device linking", "Data model", "Cryptography"

### Story 9: Crypto core

**Goal:** A `crypto` crate with MLS on the chosen library and primitives for history, working the same natively and in WASM.

**Scope:**
- MLS (RFC 9420, `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`): groups, KeyPackages, adding and removing a member, Commit, epochs.
- Channel history key: encrypting message copies, wrapping the key to device keys and the recovery key.
- Stateless HPKE (RFC 9180), Ed25519 signatures.
- **Done when:** tests confirm forward secrecy and that a removed member cannot access new messages; the crate builds for WASM.

- **Release:** MVP
- **Depends on:** Story 2, Story 3
- **Docs:** `architecture.md` — "Cryptography", `features.md` §4, §6, `tech-stack.md`

### Story 10: Spaces and agent addresses

**Goal:** A device belongs to several spaces, and an agent session automatically works in the space of its folder.

**Scope:**
- Spaces and channels on the server, a signed space roster; a device in N spaces.
- An agent is a named endpoint on a device; an address like `space/alice/claude-api@skald.example` with the server name.
- Binding a session to a space by folder: `.skald.toml` or a rule like `~/work/clientA/** → clientA`.
- Isolation: an agent of one space cannot see other spaces.
- **Done when:** a device is in three spaces; an agent in `~/work/clientA` sees only clientA.

- **Release:** MVP
- **Depends on:** Story 8
- **Docs:** `features.md` §3, §15, `architecture.md` — "Network", "Data model"

### Story 11: Encrypted mailbox

**Goal:** A message reaches the recipient device even if the sender and recipient were never online at the same time; the server stores only ciphertext.

**Scope:**
- Envelope: a plaintext header and opaque ciphertext; a separate mailbox for each recipient device.
- Recipient online: an immediate WebSocket push; offline: the message waits; after reconnecting, the client catches up from the cursor; an ack removes the message from the mailbox.
- Server instances wake each other via PostgreSQL `LISTEN/NOTIFY`.
- Receipts `stored → delivered → read → replied`, plus `expired` and `refused`; `idempotency_key` rejects duplicates.
- Cloud limits as per-account settings: spaces, devices, messages per month, text up to 64 KB, undelivered messages kept for 7 days.
- CLI: send and receive a message.
- **Done when:** A sends and goes offline, B connects 10 hours later and receives the message; a retry with the same `idempotency_key` does not create a duplicate.

- **Release:** MVP
- **Depends on:** Story 10
- **Docs:** `features.md` §1, "Product", `architecture.md` — "Network", "Data model", "Security"

### Story 12: MLS channels, roles and device revocation

**Goal:** All channel messages are MLS-encrypted between devices, and an admin can revoke a specific device.

**Scope:**
- The server as the MLS Delivery Service: stores KeyPackages, orders Commits, delivers MLS control messages through mailboxes.
- Channel = MLS group, member = device; addressing to the right session happens inside the ciphertext.
- Sending to a recipient without keys fails with an error; there is no plaintext on the server.
- Roles: admin, member, service agent; only an admin removes members.
- Device revocation is Remove + Commit, a new epoch.
- **Done when:** an automated test shows that a removed device with a full copy of its disk cannot read any message sent after the removal.

- **Release:** MVP
- **Depends on:** Story 9, Story 11
- **Docs:** `features.md` §4, `architecture.md` — "Cryptography", "Data model"

### Story 13: Client: one-shot mode and helper

**Goal:** The client works without a daemon or root: one-shot in CI and sandboxes, and through an automatic helper on a developer machine.

**Scope:**
- One-shot mode: load state → sync → run the operation → save.
- `skald helper` is started by the first client; the helper holds the WebSocket, dispatches messages to sessions over a unix socket, holds the local state lock and exits after being idle.
- Outbound HTTPS on 443 only, `HTTPS_PROXY` support; push fallbacks: SSE and long-poll.
- `skald recv --wait` for any agent with a shell.
- **Done when:** two sessions on one machine receive their own messages through a single helper; one-shot mode sends and receives behind a proxy.

- **Release:** MVP
- **Depends on:** Story 11
- **Docs:** `features.md` §2, `architecture.md` — "Components", "Network", "Delivery to agents"

### Story 14: MCP server and agent commands

**Goal:** Any agent with MCP or a shell sends and reads messages on behalf of its session.

**Scope:**
- `skald` as an MCP server (stdio): sending and `inbox`; the same operations in the CLI.
- A session registers as an agent in the space of its folder.
- Inbound content is data, not an instruction: a sender label and a prompt injection warning.
- **Done when:** two agents in different folders of one space exchange messages via MCP tools.

- **Release:** MVP
- **Depends on:** Story 11
- **Docs:** `features.md` §7, `architecture.md` — "Components", "Security", `tech-stack.md`

### Story 15: Encrypted channel history

**Goal:** The server stores the full channel history, and only devices with the history key can read it.

**Scope:**
- The channel history key is distributed inside MLS; the sender puts a copy of the message under this key on the server.
- History keys are stored on the server, wrapped to the account's devices.
- A new channel member sees the history only if an admin allowed it.
- When a member is removed, the history key is rotated.
- Reading and searching the history happens on the client (CLI).
- History retention is a space setting capped by the account limit; the cloud default is 30 days.
- **Done when:** a member's new device reads the channel history; a removed member cannot read messages sent after the removal.

- **Release:** MVP
- **Depends on:** Story 12
- **Docs:** `features.md` §6, "Product", `architecture.md` — "Cryptography", "Data model"

### Story 16: Recovery key and new device linking

**Goal:** The user links new devices and does not lose the history, even if all devices are wiped.

**Scope:**
- The recovery key is generated on the machine on first connect and shown to the user; history keys are wrapped to it as well.
- A new device is confirmed on an already linked device or with the recovery key, and then receives the history keys.
- **Done when:** all devices are wiped; the recovery key is entered on a new laptop, and the full history is there.

- **Release:** MVP
- **Depends on:** Story 15
- **Docs:** `features.md` §6, §16, `architecture.md` — "Device linking", "Cryptography"

### Story 17: Claude Code adapter and plugin

**Goal:** A message reaches a live Claude Code session; the adapter installs as a plugin.

**Scope:**
- Primary path: Channels; fallbacks: `Stop` and `PostToolUse` hooks with `additionalContext`, MCP `inbox` (based on the results of Story 4).
- A skill with rules for working with Skald.
- A Claude Code plugin with MCP, hooks and the skill in the monorepo marketplace (`/plugin marketplace add skald-so/skald`), auto-update from the marketplace.
- e2e tests against real Claude Code, nightly run in CI.
- **Done when:** a message sent to an agent appears in the context of a running Claude Code session.

- **Release:** MVP
- **Depends on:** Story 4, Story 13, Story 14
- **Docs:** `features.md` §7, §16, `architecture.md` — "Components", "Delivery to agents", `infrastructure.md` — "Channels", "Release", "PR checks"

### Story 18: Codex adapter

**Goal:** A message reaches a live Codex session without tmux.

**Scope:**
- `PostToolUse`, `Stop`, `UserPromptSubmit` hooks; fallbacks: MCP `inbox` and `codex queue`, if Story 4 confirmed it.
- e2e tests against real Codex, nightly run in CI.
- **Done when:** a message sent to an agent appears in the context of a running Codex session.

- **Release:** MVP
- **Depends on:** Story 4, Story 13, Story 14
- **Docs:** `features.md` §7, `architecture.md` — "Delivery to agents", `infrastructure.md` — "PR checks"

### Story 19: Presence, statuses and loop protection

**Goal:** Core features for parity with competitors: you can see who is online and where a message is; agents do not loop; a human controls inbound messages.

**Scope:**
- Presence directory: the agent status is encrypted; the server stores only the latest value.
- `message_status` in MCP and the CLI.
- Loop protection: a hop counter in the envelope, a client-side limit on replies without a human, a server-side rate limit.
- Inbound modes `accept / hold / refuse`, a sender allowlist; `hold` waits for human confirmation in the CLI, `refuse` produces a `refused` receipt.
- **Done when:** two agents replying to each other stop at the limit; a message in `hold` mode does not reach the agent's context until confirmed.

- **Release:** MVP
- **Depends on:** Story 12, Story 14
- **Docs:** `features.md` §8, `architecture.md` — "Data model", "Security"

### Story 20: Web UI: non-secret sections

**Goal:** On `app.skald.so`, the user signs up with GitHub and manages the account without opening any content.

**Scope:**
- Static SPA on `app.skald.so`; "Sign in with GitHub" gives one-click sign-up.
- Layout per the approved design in `../skald-cloud/design/`.
- Sections: quotas, devices, sessions, space members and roles.
- Sessions are web UI and CLI sign-ins, with revocation; the server shows only metadata: where, when, last use.
- Data comes only from the server API; the page receives neither keys nor content.
- **Done when:** a new user signs up in one click and sees their quotas, devices and space members.

- **Release:** MVP
- **Depends on:** Story 8, Story 10, Story 11
- **Docs:** `features.md` §12, §16, `architecture.md` — "Components", `tech-stack.md`

### Story 21: Space invite by link

**Goal:** An admin invites a person by link, and the newcomer's devices join the channels automatically.

**Scope:**
- An invite link is created in the web UI and CLI and accepted after sign-in.
- The admin's device adds the newcomer's devices to the space's channels by itself.
- Active invite links are visible in the web UI and can be revoked.
- Invites never reach the agent's context.
- **Done when:** a user from another account joins the space via the link, and their agents exchange messages with the admin's agents.

- **Release:** MVP
- **Depends on:** Story 12, Story 13, Story 20
- **Docs:** `features.md` §16, §12, `architecture.md` — "Security"

### Story 22: One-command machine connect

**Goal:** One command per machine and one confirmation for each additional one; after that, the agents on the machine are ready to work.

**Scope:**
- The "Connect machines" section: the website detects the OS and shows a single command with a one-time code: `curl … | sh`, `irm … | iex` or `npx skald-so connect <code>`. The code lives for a few minutes, works once and does not transfer keys.
- `install.sh` and `install.ps1` in GitHub Releases: download the binary, verify the checksum, run `skald connect`.
- `skald connect`: generates keys, registers the device with the code without a second sign-in, shows the recovery key, finds Claude Code and Codex and, after confirmation, sets up MCP and hooks, starts the helper.
- The next machine: the same command, plus confirmation on a linked device or with the recovery key.
- The status on the website updates by itself: "✓ MacBook connected: Claude Code, Codex".
- **Done when:** one-click sign-up, one command per machine; after that, Claude Code and Codex on two machines exchange messages.

- **Release:** MVP
- **Depends on:** Story 7, Story 13, Story 16, Story 17, Story 18, Story 20
- **Docs:** `features.md` §16, `architecture.md` — "Device linking", `infrastructure.md` — "Channels"

### Story 23: Production cloud, website and monitoring

**Goal:** The cloud is ready for the public MVP launch.

**Scope:**
- Production on dedicated Hetzner servers: a primary and a replica, PostgreSQL 19 with streaming replication, WAL-G backups to object storage.
- Docker Compose + Caddy with automatic TLS; the same image that is published for self-hosting.
- One-click release rollout to production with manual approval in GitHub Environments.
- Monitoring: Prometheus, Grafana, Sentry, uptime checks, status page.
- Addresses: `skald.so` — website, `app.` — web UI, `api.` — API, `docs.` — documentation; the website and docs are built from `docs/` on GitHub Pages.
- **Done when:** a release is deployed to production with one click; restore from backup has been tested; monitoring alerts arrive.

- **Release:** MVP
- **Depends on:** Story 6, Story 7
- **Docs:** `infrastructure.md` — "Cloud", "Channels"

### Story 24: End-to-end MVP scenario

**Goal:** The MVP demo passes in full, automatically, on real agents.

**Scope:**
- Demo: Claude Code on account A asks Codex on machine B under account C for a review; the sender goes offline, and the reply waits in their mailbox.
- The scenario starts with the new-user path: sign-up, one-command machine connect, invite by link.
- Daily run on staging with real Claude Code and Codex in CI.
- It checks the "Done when" criteria of features 1, 4, 6 and 16.
- **Done when:** the demo passes automatically; the run report is available in CI.

- **Release:** MVP
- **Depends on:** Story 19, Story 21, Story 22
- **Docs:** `roadmap.md` — "MVP", `features.md` §1, §4, §6, §16, `infrastructure.md` — "PR checks"

### Story 25: Admin rights transfer and account deletion

**Goal:** A space is never left without an admin, and a user can delete their account.

**Scope:**
- An admin transfers their rights to another member or appoints a second admin; the change is signed and visible to all members.
- If there is only one admin, the web UI and CLI warn about it and suggest appointing a second one.
- Only the recovery key restores admin rights; there is no server-side reset. If an admin loses both their devices and the key, members create a new space and keep the old space's history.
- Account deletion from the web UI and CLI: the account's devices are removed from all channels and history keys are rotated; the server erases the account, devices, mailboxes and wrapped keys.
- Messages in channels with other members stay with them; spaces where the account was the only member are deleted.
- The only admin of a space with other members must transfer their rights first; otherwise deletion fails.
- **Done when:** a space with a single admin shows a warning; after the transfer, the new admin removes members; after account deletion, the server has none of its data, and its former devices cannot read new messages.

- **Release:** MVP
- **Depends on:** Story 12, Story 15, Story 20
- **Docs:** `features.md` §4, §5, `architecture.md` — "Data model", "Cryptography"

### Story 26: Load testing

**Goal:** Before the public launch, know what load the server can handle and remove bottlenecks.

**Scope:**
- Scenarios: delivery and WebSocket push, cursor catch-up after a mass reconnect, MLS Commits in large channels, history writes, multiple instances with `LISTEN/NOTIFY`.
- Target numbers with headroom over the cloud limits (about 1M messages per month) are recorded in the docs.
- Runs on a production-like configuration; bottlenecks found are fixed or recorded.
- The run is repeated before releases.
- **Done when:** there is a report with the maximum load and the bottlenecks; production handles the target load with headroom.

- **Release:** MVP
- **Depends on:** Story 19, Story 23
- **Docs:** `features.md` — "Product", `architecture.md` — "Network", `infrastructure.md` — "Cloud"

### Story 27: Documentation: quick start and CLI reference

**Goal:** A new user connects and starts working from the docs, without help from the author.

**Scope:**
- Quick start: sign-up, connecting a machine, the first message between agents, inviting the team, the recovery key.
- The CLI and MCP tool reference is generated from the code and never diverges from it.
- In plain language: what the server sees, that there has been no external audit, what to do after losing a device and after losing the admin.
- Published on `docs.skald.so` together with the release.
- **Done when:** a person without context connects Claude Code and Codex on two machines using the quick start; the reference matches `skald --help`.

- **Release:** MVP
- **Depends on:** Story 22, Story 23
- **Docs:** `features.md` §16, `architecture.md` — "Cryptography", `infrastructure.md` — "Channels", "Cloud"

### Story 28: Website: main page

**Goal:** `skald.so` explains what Skald is and leads to sign-up.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- What Skald is and how it works, what the server sees and does not see, pricing (everything is free except Enterprise); links to the docs, GitHub and sign-in at `app.skald.so`.
- Built and published together with the docs from `docs/` on GitHub Pages.
- **Done when:** `skald.so` shows the main page as in the approved design, at desktop and phone width; all links resolve.

- **Release:** MVP
- **Depends on:** Story 23
- **Docs:** `features.md` — "Product", `architecture.md` — "Cryptography", `infrastructure.md` — "Channels", "Cloud"

### Story 29: Operator CLI

**Goal:** The operator runs the MVP cloud without SQL.

**Scope:**
- `skald-server admin`: list accounts with quota usage, disable and re-enable an account, delete an account.
- Metadata only, like the server itself.
- **Done when:** an operator finds an account and disables it, and the account loses access; deletion removes the data as in Story 25.

- **Release:** MVP
- **Depends on:** Story 11, Story 25
- **Docs:** `features.md` §17

### Story 30: Service agents: HPKE and scoped tokens

**Goal:** A service agent (`ci/deploy-bot`) without a persistent disk exchanges end-to-end encrypted messages using a scoped token.

**Scope:**
- A service agent with its own key, stored in an environment secret.
- A scoped token: send-only, queue-only, specific channels, expiry; token and key rotation.
- Stateless HPKE encryption for ephemeral agents.
- The "Service agents and tokens" web UI section: permissions, expiry, last use, rotation and revocation; the token value is shown once, at creation. The number of service agents is capped by the account limit.
- **Done when:** messages are exchanged with an agent in one-shot mode without a persistent disk, with only ciphertext on the server; a token used outside its permissions or after expiry is rejected.

- **Release:** Post-MVP
- **Depends on:** Story 12, Story 20
- **Docs:** `features.md` §9, "Product", `architecture.md` — "Data model", "Cryptography"

### Story 31: GitHub Action

**Goal:** A GitHub Actions workflow with a single secret sends and receives encrypted messages.

**Scope:**
- A GitHub Action built on the client's one-shot mode and a service agent key.
- The Action is published together with the release.
- **Done when:** a GitHub Action with a single secret sends and receives an encrypted message, including behind a proxy.

- **Release:** Post-MVP
- **Depends on:** Story 13, Story 30
- **Docs:** `features.md` §2, §9, §16, `architecture.md` — "Components"

### Story 32: Task queue: claim, lease and DLQ

**Goal:** A queue in which a task is never lost and never runs on two agents at once.

**Scope:**
- Named queues; a task is an encrypted payload plus plaintext scheduling fields; `idempotency_key`.
- Atomic claim; a heartbeat extends the lease; an expired lease returns the task to the queue.
- Fencing: `claim_id` with an increasing generation, extension and completion via compare-and-swap; whoever lost the lease gets `claim_lost`.
- Retries with exponential backoff, then DLQ.
- The result returns to the sender as an encrypted message; at-least-once semantics.
- CLI to enqueue, claim, extend and complete; the helper sends the heartbeat.
- Queue load test.
- **Done when:** a stopped agent gets `claim_lost`; a live but slow agent does not lose its task; failed tasks go to the DLQ.

- **Release:** Post-MVP
- **Depends on:** Story 12, Story 14
- **Docs:** `features.md` §10, `architecture.md` — "Queue", "Data model"

### Story 33: Task queue: scheduling and agents

**Goal:** Claude Code, Codex and CI work through a shared queue according to priorities and capabilities.

**Scope:**
- Priorities, capability tags (`needs: codex`), per-queue concurrency, per-task cost limit.
- Queue tools in MCP; the Claude Code and Codex adapters take and hand in tasks, CI does so via the GitHub Action.
- Queue limits are per-account settings.
- **Done when:** Claude Code, Codex and CI process 100 tasks, duplicates are rejected, failed tasks go to the DLQ.

- **Release:** Post-MVP
- **Depends on:** Story 17, Story 18, Story 31, Story 32
- **Docs:** `features.md` §10, "Product"

### Story 34: Codex background agents via app-server

**Goal:** A Codex background agent receives messages without an open interactive session and without tmux.

**Scope:**
- The adapter drives Codex via app-server: `turn/start` starts a turn, `turn/steer` delivers a message into a running turn.
- e2e tests against real Codex.
- **Done when:** a background Codex receives a message and replies without human involvement.

- **Release:** Post-MVP
- **Depends on:** Story 18
- **Docs:** `features.md` §7, `architecture.md` — "Delivery to agents"

### Story 35: Browser linking and history on the web

**Goal:** The browser becomes a linked device and shows content, decrypting it locally.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- The "Link this browser" button: a code or QR from a linked machine, or the recovery key; the browser key is in IndexedDB and can be revoked.
- A WASM build of the Rust core in the SPA; the browser reads through history keys wrapped to its key and sends actions as signed messages via HPKE.
- Sections: message texts and threads, a "which agent messages whom" graph, history search in the browser.
- SRI and reproducible builds for the JS served by the server.
- **Done when:** a linked browser shows the channel history, and the server receives only ciphertext; a revoked browser cannot read new messages.

- **Release:** Post-MVP
- **Depends on:** Story 9, Story 16, Story 20
- **Docs:** `features.md` §12, §6, `architecture.md` — "Components", "Device linking", "Cryptography"

### Story 36: Web: queues, delivery trace, loops

**Goal:** The browser shows what happens to tasks and messages and lets you control it.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- Queue panel: who holds a task, how much lease is left, `claim_lost`, retries, DLQ with "restart" and "cancel" buttons.
- Delivery trace: the `stored → delivered → read` path and the reason if a message was not delivered.
- Loop analysis: agent↔agent threads without a human, a "pause" button.
- **Done when:** a task from the DLQ is restarted from the browser, and a looping thread is paused.

- **Release:** Post-MVP
- **Depends on:** Story 19, Story 32, Story 35
- **Docs:** `features.md` §12

### Story 37: Shared channels between organizations

**Goal:** Two spaces of different organizations communicate through a shared channel with the consent of both sides.

**Scope:**
- A shared channel joins two spaces; it is created only with the consent of admins on both sides, and either side can disable it.
- Approval in the web UI and CLI.
- **Done when:** a freelancer is in three spaces, with a clientA↔clientB shared channel; after one side disables it, no messages pass through it.

- **Release:** Post-MVP
- **Depends on:** Story 12, Story 20
- **Docs:** `features.md` §3, §12, `architecture.md` — "Data model"

### Story 38: Key verification without trusting the server

**Goal:** The client detects a key swap even if the server is compromised.

**Scope:**
- The device key is signed with the user's SSH key from GitHub; the client checks the signature directly against GitHub.
- Device cross-signing, key change warnings, fingerprints for manual verification.
- Optional DNS root of trust (TXT record) for organizations with their own domain.
- **Done when:** the client detects a key swap on the server (automated test).

- **Release:** Post-MVP
- **Depends on:** Story 16
- **Docs:** `features.md` §5, `architecture.md` — "Device linking"

### Story 39: Google and corporate IdP sign-in

**Goal:** Sign-in is not limited to GitHub, and a self-hosted server works with its own IdP.

**Scope:**
- OIDC sign-in via Google and a corporate IdP.
- A self-hosted server uses its own IdP, with no calls to our cloud.
- **Done when:** a self-hosted server works with its own IdP without our cloud.

- **Release:** Post-MVP
- **Depends on:** Story 8
- **Docs:** `features.md` §5

### Story 40: ask_user and attachments

**Goal:** An agent asks a human and sends small files.

**Scope:**
- `ask_user`: the agent requests confirmation from a human and waits for the answer.
- Small encrypted text attachments up to 256 KB.
- **Done when:** the agent gets a human's answer to `ask_user`; an attachment reaches the recipient, and the server sees only ciphertext.

- **Release:** Post-MVP
- **Depends on:** Story 19
- **Docs:** `features.md` §8, `architecture.md` — "Security"

### Story 41: Shared team instructions

**Goal:** The team maintains shared instructions for agents, and they reach all machines automatically.

**Scope:**
- Versioned instructions, stored and distributed encrypted.
- Distribution to the space's machines and injection into `AGENTS.md` or `CLAUDE.md`.
- No distribution of agent configs (agent-fleet is out of scope).
- **Done when:** a new instruction version reaches all machines in the space and appears in `CLAUDE.md` and `AGENTS.md`.

- **Release:** Post-MVP
- **Depends on:** Story 17, Story 18
- **Docs:** `features.md` §8, "Out of scope"

### Story 42: Gemini, Cursor, OpenCode and Copilot adapters

**Goal:** Messages reach live Gemini CLI, Cursor, OpenCode and Copilot sessions.

**Scope:**
- Delivery via hooks and plugins; the fallback is MCP `inbox`.
- `skald connect` finds these agents and, after confirmation, connects them.
- e2e tests against real CLIs.
- **Done when:** a message reaches a live session of each of the four agents.

- **Release:** Post-MVP
- **Depends on:** Story 14, Story 22
- **Docs:** `features.md` §7, `architecture.md` — "Components", "Delivery to agents"

### Story 43: Edit coordination: file level

**Goal:** Agents of different people and machines learn before merge that they are editing the same files.

**Scope:**
- Clients publish HMAC fingerprints of changed files with the space key; the server sees only fingerprints.
- The action follows the space policy: notify, warn, ask a human, block pre-push.
- Claiming a task reserves its edit scope; reservations have a TTL.
- Decisions on the questions in `features.md` §11 "To decide", based on Story 5.
- **Done when:** two agents on different machines editing the same file get a warning before merge; the server does not know the paths.

- **Release:** Post-MVP
- **Depends on:** Story 5, Story 17, Story 18, Story 32
- **Docs:** `features.md` §11, `architecture.md` — "Cryptography"

### Story 44: Edit coordination: hunk and symbol

**Goal:** Edit overlap warnings that are more precise than file level.

**Scope:**
- Hunk and symbol fingerprints.
- Clients compute the exact overlap via `git merge-tree`.
- **Done when:** edits to different functions in one file trigger no warning; edits to the same function do.

- **Release:** Post-MVP
- **Depends on:** Story 43
- **Docs:** `features.md` §11

### Story 45: Web: security center, human inbox, edit map

**Goal:** A human manages security and approvals from the browser, including from a phone.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- Security center: devices and fingerprints, one-click revocation, key rotation, "who can read the channel".
- Human inbox: all `hold` and `ask_user` items, approval from a phone (the phone's browser is a linked device).
- Edit map.
- **Done when:** an `ask_user` is approved from a phone; a device is revoked with one click; the map shows overlapping edits.

- **Release:** Post-MVP
- **Depends on:** Story 35, Story 38, Story 40, Story 43
- **Docs:** `features.md` §12

### Story 46: Public cloud: anti-abuse

**Goal:** The cloud is protected from abuse.

**Scope:**
- Reports and bans based on reports; rules in `skald-cloud`.
- **Done when:** a banned account loses access.

- **Release:** Post-MVP
- **Depends on:** Story 20, Story 23
- **Docs:** `features.md` — "Product", §12, `architecture.md` — "Security", `infrastructure.md` — "Repositories"

### Story 47: Operator console

**Goal:** The instance operator — the cloud owner or a self-hosted server admin — runs the service from the web UI.

**Scope:**
- A web UI section visible only to instance admins.
- Layout per the approved design in `../skald-cloud/design/`.
- Accounts and organizations, quota usage and licenses.
- Abuse reports and bans; account deletion and export requests.
- System health.
- Metadata only: the operator cannot read content.
- **Done when:** an operator bans an account and handles a deletion request from the console, without SQL; a regular user does not see the section.

- **Release:** Post-MVP
- **Depends on:** Story 25, Story 46
- **Docs:** `features.md` §17, §12, `architecture.md` — "Security"

### Story 48: Alerts, analytics and token usage

**Goal:** The team learns about problems immediately and sees statistics on agent work.

**Scope:**
- Alerts to Telegram, Slack or a webhook: DLQ is growing, an agent is gone, a key changed, a token is expiring.
- Metadata statistics in the non-secret sections.
- Analytics: computed by the client, stored encrypted, shown in a linked browser.
- LLM token usage per task, only from what the CLI itself shows the user; low priority.
- **Done when:** DLQ growth and an expiring token arrive as Telegram alerts; analytics is visible only in a linked browser.

- **Release:** Post-MVP
- **Depends on:** Story 30, Story 32, Story 35
- **Docs:** `features.md` §12

### Story 49: Enterprise: SSO, SAML, SCIM and license

**Goal:** Organizations manage access through their own systems; `ee/` features are enabled by a license key.

**Scope:**
- `ee/` features under a commercial license are enabled by a license key in the same image.
- The Enterprise plan in the cloud: billing and the "Enterprise license, invoices" web UI section.
- Layout per the approved design in `../skald-cloud/design/`.
- SSO and SAML, SCIM.
- Admin access revocation.
- **Done when:** an employee removed in the organization's IdP automatically loses access; without a license key, `ee/` features are off; an organization that buys the cloud Enterprise plan gets `ee/` features right away.

- **Release:** Post-MVP
- **Depends on:** Story 39
- **Docs:** `features.md` — "Product", `infrastructure.md` — "Cloud"

### Story 50: Enterprise: audit and retention policies

**Goal:** An organization controls and retains conversations by its own rules.

**Scope:**
- Audit log.
- Auditor device: the organization admin sees all conversations; members know about it.
- History retention policies.
- **Done when:** the auditor device reads the organization's channels, and members see an audit notice; history is deleted per policy.

- **Release:** Post-MVP
- **Depends on:** Story 15, Story 49
- **Docs:** `features.md` — "Product", §12

### Story 51: Self-host: Docker Compose, Helm, documentation

**Goal:** A self-hosted server comes up with one command, following the docs.

**Scope:**
- `docker compose up` brings up `skald-server`, PostgreSQL and Caddy; the domain is set with a single variable, and HTTPS works right away.
- Helm chart.
- Self-host documentation.
- Deployment in an isolated network (ee).
- **Done when:** following the docs, a working Skald with its own domain comes up on a clean server.

- **Release:** Post-MVP
- **Depends on:** Story 7, Story 39, Story 49
- **Docs:** `infrastructure.md` — "Self-host", "Repositories", `features.md` — "Product"

### Story 52: History export and migration to a self-hosted server

**Goal:** A user takes their data and moves from the cloud to a self-hosted server without losing history.

**Scope:**
- `skald export` decrypts on the client the history available to the device and saves it as JSON or Markdown.
- Moving a space from the cloud to a self-hosted server: members, channels, history; devices reconnect to the new server.
- Account metadata export from the web UI instead of manual GDPR requests through support.
- **Done when:** a space has moved from the cloud to a self-hosted server, and members read all the previous history; `skald export` outputs the history in a readable form.

- **Release:** Post-MVP
- **Depends on:** Story 15, Story 16, Story 51
- **Docs:** `features.md` §6, `infrastructure.md` — "Self-host"

### Story 53: Python and TypeScript SDKs

**Goal:** Developers embed Skald into their Python and TypeScript services.

**Scope:**
- The Rust SDK is the foundation for the others.
- Python SDK (PyO3) and TypeScript SDK (WASM or napi): messages and the task queue.
- Packages are published together with the release.
- **Done when:** a Python example and a TypeScript example send a message and process a task from the queue.

- **Release:** Post-MVP
- **Depends on:** Story 13, Story 32
- **Docs:** `tech-stack.md`

### Story 54: Homebrew, winget and cargo-binstall

**Goal:** `skald` installs through familiar package managers.

**Scope:**
- The `skald-so/homebrew-tap` repository; dist updates the formula.
- winget package.
- Publishing to crates.io as `skald-so` via trusted publishing, and installation via `cargo binstall skald-so`.
- **Done when:** after a release, the new version installs via Homebrew, winget and `cargo binstall`.

- **Release:** Post-MVP
- **Depends on:** Story 7
- **Docs:** `features.md` §16, `infrastructure.md` — "Repositories", "Channels", "Trust in builds"

### Story 55: Managed keys: custodian device

**Goal:** A space can choose a mode in which the server reads content for the sake of convenience.

**Scope:**
- The server keeps its own custodian device in channels.
- An admin chooses the mode for the whole space; the choice is signed and visible to all members; the default is end-to-end encryption.
- In this mode, the web works without linking a device, and search runs on the server.
- **Done when:** in a managed-keys space, the web shows the history without linking the browser; members see that the mode is on.

- **Release:** Post-MVP
- **Depends on:** Story 15, Story 35
- **Docs:** `features.md` §13, `architecture.md` — "Cryptography"

### Story 56: Hosted MCP for ChatGPT and Claude.ai

**Goal:** ChatGPT and Claude.ai work with a space via remote MCP.

**Scope:**
- Remote MCP on the server, only for managed-keys spaces.
- Connecting ChatGPT and Claude.ai.
- **Done when:** ChatGPT and Claude.ai send and read the space's messages via hosted MCP.

- **Release:** Post-MVP
- **Depends on:** Story 14, Story 55
- **Docs:** `features.md` §9, §13

### Story 57: skald ui

**Goal:** A web UI without code delivered from the server, for those who do not trust that code.

**Scope:**
- `skald ui` serves the same SPA build from localhost out of a signed binary; there is no second interface.
- **Done when:** `skald ui` opens the web UI from localhost, and the content sections work without loading JS from the server.

- **Release:** Post-MVP
- **Depends on:** Story 7, Story 35
- **Docs:** `features.md` §12, `architecture.md` — "Components"

### Story 58: Sealed sender and padding

**Goal:** Reduce the metadata the server sees.

**Scope:**
- Sealed sender: the server does not see which device sent a message.
- Padding: the server does not see the exact message size.
- The threat model is updated.
- **Done when:** the envelope's plaintext header has no sender, and ciphertext sizes are evened out.

- **Release:** Post-MVP
- **Depends on:** Story 11, Story 12
- **Docs:** `architecture.md` — "Cryptography"

### Story 59: Server federation

**Goal:** Organizations on different Skald servers communicate with each other.

**Scope:**
- Delivery between servers by the server name in the agent address.
- Shared channels between spaces on different servers.
- **Done when:** agents on two different servers exchange messages through a shared channel.

- **Release:** Post-MVP
- **Depends on:** Story 37, Story 38
- **Docs:** `features.md` §15, `architecture.md` — "Network"

### Story 60: Capacity view

**Goal:** The team sees members' subscription limits, and a task waits for a limit to reset.

**Scope:**
- Only data that the CLI itself shows the user; we do not touch vendor tokens or switch accounts.
- A task waits for the limit to reset on the same account.
- Built only if Story 4 confirmed that vendors expose this data.
- **Done when:** the team sees members' limits; a task for an agent with an exhausted limit waits for it to reset.

- **Release:** Post-MVP
- **Depends on:** Story 4, Story 33
- **Docs:** `features.md` §14
