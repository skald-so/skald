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
| 62 | Web UI kit: theme, shells and components | MVP | 1 |
| 64 | Self-host: Docker Compose | MVP | 6, 7, 9 |
| 8 | GitHub sign-in and device certificate | MVP | 1, 2 |
| 9 | Google sign-in and linked sign-in methods | MVP | 8 |
| 10 | Crypto core | MVP | 2, 3 |
| 11 | Spaces and agent addresses | MVP | 8 |
| 12 | Encrypted mailbox | MVP | 11 |
| 13 | MLS channels, roles and device revocation | MVP | 10, 12 |
| 14 | Client: one-shot mode and helper | MVP | 12 |
| 15 | MCP server and agent commands | MVP | 12 |
| 16 | Encrypted channel history | MVP | 13 |
| 17 | Recovery key and new device linking | MVP | 16 |
| 18 | Key verification without trusting the server | MVP | 17 |
| 19 | Claude Code adapter and plugin | MVP | 4, 14, 15 |
| 20 | Codex adapter | MVP | 4, 14, 15 |
| 21 | Presence, statuses and loop protection | MVP | 13, 15 |
| 22 | Web UI: non-secret sections | MVP | 8, 9, 11, 12, 62 |
| 23 | Space invite by link | MVP | 13, 14, 22 |
| 24 | One-command machine connect | MVP | 7, 14, 17, 19, 20, 22 |
| 25 | Production cloud, website and monitoring | MVP | 6, 7 |
| 26 | Task queue: claim, lease and DLQ | MVP | 13, 15 |
| 27 | Browser linking and history on the web | MVP | 10, 17, 22 |
| 28 | Web: queues, delivery trace, loops | MVP | 21, 26, 27 |
| 35 | Service agents: HPKE and scoped tokens | MVP | 13, 22 |
| 41 | ask_user and attachments | MVP | 21 |
| 65 | Queue templates: review, test, ask | MVP | 19, 20, 26 |
| 63 | Notifications to people | MVP | 21, 22, 26, 41 |
| 66 | Restricted mode for tasks from other people | MVP | 19, 20, 21 |
| 36 | GitHub Action | MVP | 14, 35, 65 |
| 29 | End-to-end MVP scenario | MVP | 9, 18, 21, 23, 24, 26, 27, 28, 36, 41, 63, 65, 66 |
| 30 | Admin rights transfer and account deletion | MVP | 13, 16, 22 |
| 31 | Load testing | MVP | 21, 25 |
| 32 | Documentation: quick start and CLI reference | MVP | 24, 25 |
| 33 | Website: main page | MVP | 25, 62 |
| 34 | Operator CLI | MVP | 12, 30 |
| 37 | Task queue: scheduling and agents | Post-MVP | 19, 20, 26, 36 |
| 38 | Codex background agents via app-server | Post-MVP | 20 |
| 67 | A2A gateway | Post-MVP | 2, 26, 35 |
| 39 | Shared channels between organizations | Post-MVP | 13, 22 |
| 40 | Corporate IdP sign-in | Post-MVP | 9 |
| 42 | Shared team instructions | Post-MVP | 19, 20 |
| 43 | Gemini, Cursor, OpenCode and Copilot adapters | Post-MVP | 15, 24 |
| 44 | Edit coordination: file level | Post-MVP | 5, 19, 20, 26 |
| 45 | Edit coordination: hunk and symbol | Post-MVP | 44 |
| 46 | Web: security center, human inbox, edit map | Post-MVP | 18, 27, 41, 44 |
| 47 | Public cloud: anti-abuse | Post-MVP | 22, 25 |
| 48 | Operator console | Post-MVP | 30, 47 |
| 49 | Analytics and token usage | Post-MVP | 26, 27, 35 |
| 50 | Enterprise: SSO, SAML, SCIM and license | Post-MVP | 40 |
| 51 | Enterprise: audit and retention policies | Post-MVP | 16, 50 |
| 52 | Self-host: Helm and isolated networks | Post-MVP | 40, 50, 64 |
| 53 | History export and migration to a self-hosted server | Post-MVP | 16, 17, 52 |
| 54 | Python and TypeScript SDKs | Post-MVP | 14, 26 |
| 55 | Homebrew, winget and cargo-binstall | Post-MVP | 7 |
| 56 | Managed keys: custodian device | Post-MVP | 16, 27 |
| 57 | Hosted MCP for ChatGPT and Claude.ai | Post-MVP | 15, 56 |
| 58 | skald ui | Post-MVP | 7, 27 |
| 59 | Sealed sender and padding | Post-MVP | 12, 13 |
| 60 | Server federation | Post-MVP | 18, 39 |
| 61 | Capacity view | Post-MVP | 4, 37 |

**Order and parallelism**

A wave starts when the dependencies of its stories are done; stories within a wave can be done in parallel. Priority is the order in this file.

- Wave 1: 1.
- Wave 2: 2, 3, 4, 5, 6, 7, 62.
- Wave 3: 8, 10, 25 (25 can be postponed until launch).
- Wave 4: 9, 11, 33.
- Wave 5: 12, 64.
- Wave 6: 13, 14, 15, 22.
- Wave 7: 16, 19, 20, 21, 23, 26, 35.
- Wave 8: 17, 30, 31, 41, 65, 66.
- Wave 9: 18, 24, 27, 34, 36, 63.
- Wave 10: 28, 32.
- Wave 11: 29 — end of MVP.
- Wave 12: 38, 39, 40, 42, 43, 44, 47, 54, 55, 56, 58, 59, 67.
- Wave 13: 45, 46, 48, 49, 50, 57, 60.
- Wave 14: 37, 51, 52.
- Wave 15: 53, 61.

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

#### Iteration 1.9: Mutation check of changed Rust code
- **Goal:** A review shows which changes in Rust code no test notices.
- **What we do:**
  - `script/mutants <base-ref>`: cargo-mutants mutates only the Rust lines changed since `<base-ref>` and runs the tests of the mutated crate, in parallel, with PostgreSQL as in `script/test`. Exit code 0 only if every mutant is caught; the output names the survivors; the working tree stays clean.
  - `CONTRIBUTING.md` lists cargo-mutants among the tools.
  - Tests: on a small fixture project, the script passes for a change covered by a test and fails, naming the survivor, for an uncovered one.
- **Definition of done:** `script/mutants origin/dev` on a PR branch reports survivors only in the changed Rust code; `script/test` runs the script's tests.

### Story 2: Protocol spec and threat model

**Goal:** Pin down the protocol and the threat model that all other stories rely on.

**Scope:**
- Spec: envelope (plaintext header + ciphertext), agent address with the server name, delivery semantics and receipts (who issues each), a per-sender counter for gap detection, mailbox and cursor, queues and fencing, presence, edit coordination via HMAC fingerprints.
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

#### Iteration 2.1: Protocol spec: addressing, envelope and versions
- **Goal:** A public spec fixes the identifiers, the envelope and the protocol version that every other story builds on.
- **What we do:**
  - The spec in `docs/protocol/`: `README.md` holds the index and this iteration's sections; marked "no external audit" and linked from `AGENTS.md`.
  - Wire-level facts live only in the spec: the envelope part of `architecture.md`, "Data model", moves here and is replaced by a link.
  - Identifiers of accounts, devices, agents, spaces, channels and messages; the agent address with the server name, e.g. `space/alice/claude-api@skald.example`.
  - Envelope: the plaintext header and the content inside the ciphertext (`architecture.md`, "Data model").
  - A2A compatibility: the content inside the ciphertext carries an A2A message in its JSON form, with the A2A version pinned to the protocol version; Skald's own fields stay in the plaintext header. The spec defines Skald as an A2A extension: an end-to-end encrypted transport with offline delivery and receipts.
  - Comparison with draft-chapman-a2a-mls and AGNTCY SLIM: where Skald is compatible, where it differs and why.
  - Written so it can later be submitted as an A2A extension or an IETF draft.
  - Encoding: JSON over HTTPS for the API, described by the OpenAPI spec; envelopes in binary WebSocket frames.
  - Envelope encoding: deterministic CBOR (RFC 8949, §4.2) with a CDDL schema. Signatures need one byte form, unknown fields let two protocol versions coexist, and CDDL lets others implement the protocol. Recorded in `tech-stack.md`.
  - Protocol version: a number separate from the release version; how the client states it; the server accepts the current and previous versions; the answers to older and newer clients, including "please update".
  - How the custodian device (`features.md` §13), the auditor device (`ee/`) and federation (`features.md` §15) fit in without a breaking change.
- **Definition of done:** the spec covers every item above; other docs link to it instead of repeating its facts (rule 1 in `rules.md`).

#### Iteration 2.2: Protocol spec: mailbox, delivery and receipts
- **Goal:** The spec fixes how a message reaches the recipient's agent and how the sender learns its status.
- **What we do:**
  - `docs/protocol/delivery.md`; `architecture.md`, "Delivery semantics", moves here and is replaced by a link.
  - Mailbox and cursor: a mailbox per recipient device, catch-up from the cursor, ack by `seq`, deletion on ack, expiry.
  - Delivery on each hop: send with `idempotency_key`, server → device, device → agent; deduplication at each step.
  - Receipts `stored`, `delivered`, `read`, `replied`, `held`, `refused`, `expired`: who issues and signs each, and how it reaches the sender.
  - The per-sender counter: how the recipient detects a missing message and reports it.
  - Message order within a channel.
  - Attachments: encrypted blobs uploaded separately and referenced from the content; size limit and expiry.
- **Definition of done:** the spec covers every item above; `architecture.md` keeps only a link to delivery semantics.

#### Iteration 2.3: Protocol spec: queues, presence and edit coordination
- **Goal:** The spec covers the rest of the protocol, so later stories need no breaking changes.
- **What we do:**
  - `docs/protocol/queues.md`, `presence.md` and `edits.md`; the wire-level part of `architecture.md`, "Queue" (claim, lease, fencing, modes), moves here and is replaced by a link.
  - Queues: plaintext scheduling fields and the encrypted payload; claim, lease with heartbeat, fencing with `claim_id` and generation, `claim_lost`; at-least-once and at-most-once modes, retries, DLQ; the result returned as an encrypted message.
  - Task templates: how a task names its template and carries the template's input and result.
  - Sender trust: how the recipient learns from the signed envelope whether a task comes from an own device, a space member or a service agent.
  - `ask_user`: a question from an agent to a person and the answer.
  - Presence: the encrypted agent status; the server keeps only the latest value.
  - Edit coordination: HMAC fingerprints of paths, hunks and symbols with the space key; what the client sends and the server compares; open questions link to `features.md` §11, "To decide".
- **Definition of done:** the spec covers every item above; no fact is repeated between the spec and `architecture.md` or `features.md`.

#### Iteration 2.4: Threat model
- **Goal:** Users and contributors know what Skald protects against and what it does not.
- **What we do:**
  - `docs/threat-model.md`, marked "no external audit" and linked from `AGENTS.md`.
  - Attackers and what each can do: the server or its operator, a network observer, a stolen device, a removed member, a malicious sender.
  - What the server sees and does not see; trade-offs (`architecture.md`, "Cryptography").
  - Prompt injection and loops: the defenses in `architecture.md`, "Security", and what they do not stop.
- **Definition of done:** the threat model covers every item above and links to the spec sections each defense relies on; no fact is repeated between it and `architecture.md`.

#### Iteration 2.5: Identifiers and agent addresses in `proto`
- **Goal:** The client and server share one implementation of the spec's identifiers and addresses.
- **What we do:**
  - Types in `proto` for the identifiers and the agent address defined by the spec.
  - Parsing and printing an address with the server name; an invalid address is rejected with an error naming the problem.
  - Tests: valid identifiers and addresses round-trip; each invalid form from the spec is rejected.
- **Definition of done:** `proto` follows the spec's identifier and address rules; unit tests pass in `script/test`.

#### Iteration 2.6: Envelope and receipt types in `proto`
- **Goal:** Envelopes and receipts have one wire implementation that matches the spec.
- **What we do:**
  - Types in `proto` for the envelope header, the content inside the ciphertext with its A2A message, and receipts.
  - CBOR encoding and decoding of envelopes and their content as in the spec; malformed or truncated input returns an error, never a panic. The CBOR crate is recorded in `tech-stack.md`.
  - Test vectors in `docs/protocol/`: the hex of each envelope and receipt kind, for other implementations.
  - Tests: every type round-trips; `proto` output matches the test vectors byte for byte; malformed and truncated frames are rejected.
- **Definition of done:** `proto` encodes and decodes envelopes per the spec; unit tests pass in `script/test`.

#### Iteration 2.7: Protocol version check on the server
- **Goal:** The server serves clients of the current and previous protocol versions and tells older ones to update.
- **What we do:**
  - The OpenAPI spec first: how a request states the protocol version and the answers to unsupported versions (rule 4 in `rules.md`).
  - The current protocol version in `proto`.
  - `skald-server` accepts the current and previous versions and refuses others as the protocol spec says, with "please update" for a too-old client.
  - Tests: every version case in the protocol spec; responses conform to the OpenAPI spec.
- **Definition of done:** the server enforces the supported version range; server integration tests pass in `script/test`.

#### Iteration 2.8: Protocol version check in the client
- **Goal:** An outdated `skald` tells its user to update instead of failing obscurely.
- **What we do:**
  - The client's server connection, which later commands reuse, states the protocol version on every request.
  - A "please update" answer becomes an error telling the user to update `skald`; other version refusals get their own clear error.
  - Tests: the client against a running `skald-server`: a compatible version connects, a too-old one gets "please update".
- **Definition of done:** client and server negotiate the protocol version; an outdated client gets "please update"; client integration tests with `skald-server` pass in `script/test`.

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

### Story 62: Web UI kit: theme, shells and components

**Goal:** Web UI screens are assembled from ready parts that match the approved design, so screen stories only connect the API.

**Scope:**
- Look per the approved design in `../skald-cloud/design/`: tokens, fonts and rules in its `README.md`.
- Theme: light and dark tokens that follow the system setting; fonts.
- Shells: the cabinet with the side menu, space switcher and account footer; the operator console with top tabs.
- Components: a table sorted by a click on a column header, with an arrow on the active column; filter buttons; a dropdown filter; search; a warning banner; buttons; status labels; empty and loading states.
- A kit page shows every component in every state with fixture data, without API calls.
- Desktop and phone width; usable from the keyboard and with a screen reader.
- **Done when:** the kit page matches the approved design in both themes at desktop and phone width; component tests run in `script/test`.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `features.md` §12, `architecture.md` — "Components", `tech-stack.md`

#### Iteration 62.1: Theme, fonts and the kit page
- **Goal:** The web UI has the approved colors and fonts in both themes, and one page shows the whole kit.
- **What we do:**
  - Light and dark tokens from "Tokens" in the design `README.md`; the theme follows the system setting.
  - Fonts: Inter for text, Space Grotesk for headings and numbers, JetBrains Mono for code, keys and IDs; bundled with the build, no requests to third-party hosts.
  - A visible keyboard focus outline on every interactive element.
  - The kit page: opens from the dev server separately from the app and stays out of the production build; uses only fixture data and never calls the API; shows the color tokens and font samples. Later iterations add each component to it.
  - A URL parameter forces the light or dark theme on the kit page.
  - A command saves kit page screenshots with headless Chrome, in both themes at both check widths; reviews compare them with the design `png/`.
  - Check widths: 1440 px as in the design screenshots, 360 px for a phone; the page never scrolls sideways.
  - `web/README.md`: how to open the kit page. The styling approach, the font packages and the accessibility checker are recorded in `tech-stack.md`.
  - Tests: the kit page renders without network requests and passes an automated accessibility check that covers every component added later.
- **Definition of done:** the kit page shows the tokens and fonts as in the design `README.md`, light or dark per the system setting or the URL parameter; the screenshot command saves all four views; component tests pass in `script/test`.

#### Iteration 62.2: Buttons, status labels and the warning banner
- **Goal:** Screens show actions and statuses that look and read as in the design.
- **What we do:**
  - Buttons: primary, secondary and danger kinds as on the design screens; disabled and busy states.
  - Status labels in the OK, warning, danger, accent and muted tones: a filled label ("Active", "Flagged"), a dot with text ("Online"), a count badge. The text carries the meaning without color.
  - Warning banner: a message with optional actions, as the loop banner in `04-messages.png`; a screen reader announces it when it appears.
  - Every kind and state on the kit page.
  - Tests: a button acts on a click and from the keyboard, not while disabled or busy; the busy state is announced; the banner's text and actions reach a screen reader.
- **Definition of done:** the kit page shows these components as in the approved design in both themes at desktop and phone width, judged by the kit page screenshots; component tests pass in `script/test`.

#### Iteration 62.3: Sortable table, empty and loading states
- **Goal:** Every list on a screen is a table sorted by a click on a column header, with clear empty and loading states.
- **What we do:**
  - Table as in `08-machines.png` and `13-operator-accounts.png`: two-line cells, monospace cells for keys and IDs, an actions column, a highlighted row as in `07-queues.png`.
  - A click on a sortable column header reports the column and direction; a second click reverses it. The screen sorts the rows itself or asks the API, since long lists are sorted on the server; a helper sorts short lists in the browser. The active column shows an arrow with the direction, other sortable columns a neutral mark; the screen sets the initial order.
  - Headers sort from the keyboard; a screen reader hears the active column and its direction.
  - Empty state: a short message with an optional action. Loading state: a placeholder that a screen reader hears as busy. The table shows them in place of rows; both also work outside a table.
  - At phone width the table scrolls inside its frame.
  - Every state on the kit page with fixture rows.
  - Tests: sorting by click and keyboard in both directions, with the arrow and the announced direction following; a non-sortable column does not sort; the empty and loading states replace the rows.
- **Definition of done:** the kit page shows the table and both states as in the approved design in both themes at desktop and phone width, judged by the kit page screenshots; component tests pass in `script/test`.

#### Iteration 62.4: Filter buttons and search
- **Goal:** Long lists can be narrowed with filter buttons and search.
- **What we do:**
  - Filter buttons: a group with one active choice, as "All · Flagged · Banned · Enterprise" in `13-operator-accounts.png` and "Thread · Graph" in `04-messages.png`; a screen reader hears which choice is active.
  - Search: a labelled field that reports the query as the user types and can be cleared.
  - Both on the kit page, narrowing a fixture table.
  - Tests: a choice by click or keyboard becomes the only active one and is reported; typing reports the query; clearing reports an empty one.
- **Definition of done:** the kit page shows filter buttons and search as in the approved design in both themes at desktop and phone width, judged by the kit page screenshots; component tests pass in `script/test`.

#### Iteration 62.5: Dropdown filter
- **Goal:** A screen offers several filters and a sort order in one compact bar.
- **What we do:**
  - Dropdown filter as in `04-messages.png`: a button with the filter name and the current value ("From anyone", "Sort Oldest first") that opens a list of options; a value other than the default is highlighted.
  - "Clear" next to the filters resets them to their defaults; shown only while a filter is set.
  - Keyboard: Enter, Space or an arrow key opens the list, arrows move, Enter chooses, Escape closes; focus returns to the button. A screen reader hears the name, the value and whether the list is open.
  - At phone width the bar wraps and the list stays on screen.
  - On the kit page: a filter bar over a fixture table.
  - Tests: choosing an option by mouse and keyboard reports it and updates the button; Escape and a click outside close the list without a change; "Clear" restores the defaults.
- **Definition of done:** the kit page shows the filter bar as in the approved design in both themes at desktop and phone width, judged by the kit page screenshots; component tests pass in `script/test`.

#### Iteration 62.6: Cabinet shell
- **Goal:** Every cabinet screen sits in the same frame with the side menu, space switcher and account footer.
- **What we do:**
  - Side menu as in `08-machines.png`: the logo, the space switcher, sections with optional sub-items and count badges, the current section highlighted; the account footer with initials, name, sign-in method and a menu of account actions.
  - The space switcher works like the dropdown filter and reports the chosen space.
  - The page area: a heading with a subtitle and actions on the right.
  - Sections, spaces and the account come from the screen; the kit page passes fixtures.
  - At phone width the side menu hides behind a menu button and opens over the page; choosing a section or Escape closes it.
  - Landmarks for the menu and the page; a screen reader hears the current section as current.
  - On the kit page: the shell around a page built from the earlier components, as `08-machines.png`.
  - Tests: the current section is marked; choosing a space reports it; the menu button opens and closes the side menu, and Escape returns focus to it; the account menu works from the keyboard.
- **Definition of done:** the kit page shows the cabinet as in the approved design in both themes at desktop and phone width, judged by the kit page screenshots; component tests pass in `script/test`.

#### Iteration 62.7: Operator console shell
- **Goal:** Operator screens sit in their own frame with top tabs.
- **What we do:**
  - Top bar as in `13-operator-accounts.png`: the logo with the "operator" label, tabs with optional count badges, the instance name on the right; the current tab highlighted.
  - The page area as in the cabinet shell.
  - Tabs and the instance name come from the screen; the kit page passes the tabs of the design, Inbox first.
  - At phone width the tabs wrap or scroll inside the bar.
  - A screen reader hears the current tab as current and each tab's count.
  - On the kit page: the shell around filter buttons, search and a table, as `13-operator-accounts.png`.
  - Tests: the current tab is marked; a tab's count is part of its accessible name.
- **Definition of done:** the kit page shows the operator console as in the approved design in both themes at desktop and phone width, judged by the kit page screenshots; component tests pass in `script/test`.

### Story 64: Self-host: Docker Compose

**Goal:** A self-hosted server comes up with one command, following the docs.

**Scope:**
- `docker compose up` brings up `skald-server`, PostgreSQL and Caddy; the domain is set with a single variable, and HTTPS works right away.
- Sign-in on a self-hosted server: GitHub or Google with the admin's own OAuth apps; corporate IdPs come with Story 40.
- Backups and upgrades with documented commands.
- Self-host documentation.
- **Done when:** following the docs, a working Skald with its own domain comes up on a clean server; an upgrade keeps the data.

- **Release:** MVP
- **Depends on:** Story 6, Story 7, Story 9
- **Docs:** `infrastructure.md` — "Self-host", "Repositories", `features.md` — "Product"

### Story 8: GitHub sign-in and device certificate

**Goal:** The user signs in with GitHub, and the machine gets its own keys and a device certificate.

**Scope:**
- Sign-in via GitHub OAuth; device flow (RFC 8628) for machines without a browser. The account is created on first sign-in.
- Device keys (signing and HPKE) are generated on the machine and never leave it; they are stored in Keychain, libsecret or Windows Credential Manager, otherwise in a file with 0600 permissions.
- Device certificate "device key → account", signed by the account and the server.
- Client local state in SQLite.
- CLI: sign in, sign out, list your devices.
- **Done when:** on a machine without a browser, `skald` signs in via device flow and gets a certificate; the server stores only public keys.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 1, Story 2
- **Docs:** `features.md` §5, `architecture.md` — "Device linking", "Data model", "Cryptography"

### Story 9: Google sign-in and linked sign-in methods

**Goal:** A user signs in with Google as well as GitHub, and one account can have several sign-in methods.

**Scope:**
- OIDC sign-in via Google: discovery, ID token verification; the same OIDC code later serves corporate IdPs.
- An account has a list of sign-in methods (provider and subject). A second method is linked explicitly after signing in with the first; accounts are never merged by email.
- The account has its own username, independent of the provider; the GitHub login is only the default.
- Device flow works with any sign-in method.
- Google Cloud project, OAuth consent screen and verified domain `skald.so`.
- **Done when:** a user signs in with Google, links GitHub to the same account and signs in with either; `skald` signs in via device flow with a Google account.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 8
- **Docs:** `features.md` §5

### Story 10: Crypto core

**Goal:** A `crypto` crate with MLS on the chosen library and primitives for history, working the same natively and in WASM.

**Scope:**
- MLS (RFC 9420, `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`): groups, KeyPackages, adding and removing a member, Commit, epochs.
- Channel history key: encrypting message copies, wrapping the key to device keys and the recovery key.
- Stateless HPKE (RFC 9180), Ed25519 signatures.
- **Done when:** tests confirm forward secrecy and that a removed member cannot access new messages; the crate builds for WASM.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 2, Story 3
- **Docs:** `architecture.md` — "Cryptography", `features.md` §4, §6, `tech-stack.md`

### Story 11: Spaces and agent addresses

**Goal:** A device belongs to several spaces, and an agent session automatically works in the space of its folder.

**Scope:**
- Spaces and channels on the server, a signed space roster; a device in N spaces.
- An agent is a named endpoint on a device; an address like `space/alice/claude-api@skald.example` with the server name.
- Binding a session to a space by folder: `.skald.toml` or a rule like `~/work/clientA/** → clientA`.
- Isolation: an agent of one space cannot see other spaces.
- Each account gets a personal space at sign-up; its machines join it by default, so one person can start alone.
- **Done when:** a new account has a personal space with its machines; a device is in three spaces; an agent in `~/work/clientA` sees only clientA.

- **Release:** MVP
- **Depends on:** Story 8
- **Docs:** `features.md` §3, §15, `architecture.md` — "Network", "Data model"

### Story 12: Encrypted mailbox

**Goal:** A message reaches the recipient device even if the sender and recipient were never online at the same time; the server stores only ciphertext.

**Scope:**
- Envelope: a plaintext header and opaque ciphertext; a separate mailbox for each recipient device.
- Recipient online: an immediate WebSocket push; offline: the message waits; after reconnecting, the client catches up from the cursor; the device acks after writing the message locally, and the ack removes it from the mailbox; a redelivered message is dropped by `envelope_id`.
- Server instances wake each other via PostgreSQL `LISTEN/NOTIFY`.
- Receipts `stored → delivered → read → replied`, plus `held`, `refused` and `expired`, issued as in `architecture.md`, "Delivery semantics"; `idempotency_key` rejects duplicates.
- Cloud limits as per-account settings: spaces, devices, messages per month, text up to 64 KB, undelivered messages kept for 7 days.
- CLI: send and receive a message.
- **Done when:** A sends and goes offline, B connects 10 hours later and receives the message; a retry with the same `idempotency_key` does not create a duplicate; a connection dropped before the ack neither loses nor duplicates the message.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 11
- **Docs:** `features.md` §1, "Product", `architecture.md` — "Network", "Data model", "Delivery semantics", "Security"

### Story 13: MLS channels, roles and device revocation

**Goal:** All channel messages are MLS-encrypted between devices, and an admin can revoke a specific device.

**Scope:**
- The server as the MLS Delivery Service: stores KeyPackages, orders Commits, delivers MLS control messages through mailboxes.
- Channel = MLS group, member = device; addressing to the right session happens inside the ciphertext.
- Sending to a recipient without keys fails with an error; there is no plaintext on the server.
- Roles: admin, member, service agent; only an admin removes members.
- Device revocation is Remove + Commit, a new epoch.
- **Done when:** an automated test shows that a removed device with a full copy of its disk cannot read any message sent after the removal.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 10, Story 12
- **Docs:** `features.md` §4, `architecture.md` — "Cryptography", "Data model"

### Story 14: Client: one-shot mode and helper

**Goal:** The client works without a daemon or root: one-shot in CI and sandboxes, and through an automatic helper on a developer machine.

**Scope:**
- One-shot mode: load state → sync → run the operation → save.
- `skald helper` is started by the first client; the helper holds the WebSocket, dispatches messages to sessions over a unix socket, holds the local state lock and exits after being idle.
- Outbound HTTPS on 443 only, `HTTPS_PROXY` support; push fallbacks: SSE and long-poll.
- `skald recv --wait` for any agent with a shell.
- **Done when:** two sessions on one machine receive their own messages through a single helper; one-shot mode sends and receives behind a proxy.

- **Release:** MVP
- **Depends on:** Story 12
- **Docs:** `features.md` §2, `architecture.md` — "Components", "Network", "Delivery to agents"

### Story 15: MCP server and agent commands

**Goal:** Any agent with MCP or a shell sends and reads messages on behalf of its session.

**Scope:**
- `skald` as an MCP server (stdio): sending and `inbox`; the same operations in the CLI.
- A session registers as an agent in the space of its folder.
- Inbound content is data, not an instruction: a sender label and a prompt injection warning.
- **Done when:** two agents in different folders of one space exchange messages via MCP tools.

- **Release:** MVP
- **Depends on:** Story 12
- **Docs:** `features.md` §7, `architecture.md` — "Components", "Security", `tech-stack.md`

### Story 16: Encrypted channel history

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
- **Acceptance tests:** first
- **Depends on:** Story 13
- **Docs:** `features.md` §6, "Product", `architecture.md` — "Cryptography", "Data model"

### Story 17: Recovery key and new device linking

**Goal:** The user links new devices and does not lose the history, even if all devices are wiped.

**Scope:**
- The recovery key is generated on the machine on first connect and shown to the user; history keys are wrapped to it as well.
- A new device is confirmed on an already linked device or with the recovery key, and then receives the history keys.
- **Done when:** all devices are wiped; the recovery key is entered on a new laptop, and the full history is there.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 16
- **Docs:** `features.md` §6, §16, `architecture.md` — "Device linking", "Cryptography"

### Story 18: Key verification without trusting the server

**Goal:** The client detects a key swap even if the server is compromised.

**Scope:**
- The device key is signed with the user's SSH key from GitHub; the client checks the signature directly against GitHub.
- Accounts without a GitHub SSH key rely on cross-signing, key change warnings and fingerprints.
- Device cross-signing, key change warnings, fingerprints for manual verification.
- Optional DNS root of trust (TXT record) for organizations with their own domain.
- **Done when:** the client detects a key swap on the server (automated test).

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 17
- **Docs:** `features.md` §5, `architecture.md` — "Device linking"

### Story 19: Claude Code adapter and plugin

**Goal:** A message reaches a live Claude Code session; the adapter installs as a plugin.

**Scope:**
- Primary path: Channels; fallbacks: `Stop` and `PostToolUse` hooks with `additionalContext`, MCP `inbox` (based on the results of Story 4).
- A skill with rules for working with Skald.
- A Claude Code plugin with MCP, hooks and the skill in the monorepo marketplace (`/plugin marketplace add skald-so/skald`), auto-update from the marketplace.
- e2e tests against real Claude Code, nightly run in CI.
- **Done when:** a message sent to an agent appears in the context of a running Claude Code session.

- **Release:** MVP
- **Depends on:** Story 4, Story 14, Story 15
- **Docs:** `features.md` §7, §16, `architecture.md` — "Components", "Delivery to agents", `infrastructure.md` — "Channels", "Release", "PR checks"

### Story 20: Codex adapter

**Goal:** A message reaches a live Codex session without tmux.

**Scope:**
- `PostToolUse`, `Stop`, `UserPromptSubmit` hooks; fallbacks: MCP `inbox` and `codex queue`, if Story 4 confirmed it.
- e2e tests against real Codex, nightly run in CI.
- **Done when:** a message sent to an agent appears in the context of a running Codex session.

- **Release:** MVP
- **Depends on:** Story 4, Story 14, Story 15
- **Docs:** `features.md` §7, `architecture.md` — "Delivery to agents", `infrastructure.md` — "PR checks"

### Story 21: Presence, statuses and loop protection

**Goal:** Core features for parity with competitors: you can see who is online and where a message is; agents do not loop; a human controls inbound messages.

**Scope:**
- Presence directory: the agent status is encrypted; the server stores only the latest value.
- `message_status` in MCP and the CLI.
- Loop protection: a hop counter in the envelope, a client-side limit on replies without a human, a server-side rate limit.
- Inbound modes `accept / hold / refuse`, a sender allowlist; `hold` waits for human confirmation in the CLI, `refuse` produces a `refused` receipt.
- **Done when:** two agents replying to each other stop at the limit; a message in `hold` mode does not reach the agent's context until confirmed.

- **Release:** MVP
- **Depends on:** Story 13, Story 15
- **Docs:** `features.md` §8, `architecture.md` — "Data model", "Security"

### Story 22: Web UI: non-secret sections

**Goal:** On `app.skald.so`, the user signs up with GitHub and manages the account without opening any content.

**Scope:**
- Static SPA on `app.skald.so`; "Sign in with GitHub" or "Sign in with Google" gives one-click sign-up.
- Layout per the approved design in `../skald-cloud/design/`.
- Sections: quotas, devices, sessions, space members and roles.
- Sessions are web UI and CLI sign-ins, with revocation; the server shows only metadata: where, when, last use.
- Data comes only from the server API; the page receives neither keys nor content.
- **Done when:** a new user signs up in one click and sees their quotas, devices and space members.

- **Release:** MVP
- **Depends on:** Story 8, Story 9, Story 11, Story 12, Story 62
- **Docs:** `features.md` §12, §16, `architecture.md` — "Components", `tech-stack.md`

### Story 23: Space invite by link

**Goal:** An admin invites a person by link, and the newcomer's devices join the channels automatically.

**Scope:**
- An invite link is created in the web UI and CLI and accepted after sign-in.
- The admin's device adds the newcomer's devices to the space's channels by itself.
- Active invite links are visible in the web UI and can be revoked.
- Invites never reach the agent's context.
- **Done when:** a user from another account joins the space via the link, and their agents exchange messages with the admin's agents.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 14, Story 22
- **Docs:** `features.md` §16, §12, `architecture.md` — "Security"

### Story 24: One-command machine connect

**Goal:** One command per machine and one confirmation for each additional one; after that, the agents on the machine are ready to work.

**Scope:**
- The "Connect machines" section: the website detects the OS and shows a single command with a one-time code: `curl … | sh`, `irm … | iex` or `npx skald-so connect <code>`. The code lives for a few minutes, works once and does not transfer keys.
- `install.sh` and `install.ps1` in GitHub Releases: download the binary, verify the checksum, run `skald connect`.
- `skald connect`: generates keys, registers the device with the code without a second sign-in, shows the recovery key, finds Claude Code and Codex and, after confirmation, sets up MCP and hooks, starts the helper.
- The next machine: the same command, plus confirmation on a linked device or with the recovery key.
- The status on the website updates by itself: "✓ MacBook connected: Claude Code, Codex".
- **Done when:** one-click sign-up, one command per machine; after that, Claude Code and Codex on two machines exchange messages.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 7, Story 14, Story 17, Story 19, Story 20, Story 22
- **Docs:** `features.md` §16, `architecture.md` — "Device linking", `infrastructure.md` — "Channels"

### Story 25: Production cloud, website and monitoring

**Goal:** The cloud is ready for the public MVP launch.

**Scope:**
- Production on dedicated Hetzner servers: a primary and a replica, PostgreSQL 19 with synchronous replication that switches to asynchronous with an alert when the replica is down, WAL-G backups to object storage.
- Docker Compose + Caddy with automatic TLS; the same image that is published for self-hosting.
- One-click release rollout to production with manual approval in GitHub Environments.
- Monitoring: Prometheus, Grafana, Sentry, uptime checks, status page.
- Addresses: `skald.so` — website, `app.` — web UI, `api.` — API, `docs.` — documentation; the website and docs are built from `docs/` on GitHub Pages.
- **Done when:** a release is deployed to production with one click; restore from backup has been tested; losing the primary loses no `stored` message; monitoring alerts arrive.

- **Release:** MVP
- **Depends on:** Story 6, Story 7
- **Docs:** `infrastructure.md` — "Cloud", "Channels"

### Story 26: Task queue: claim, lease and DLQ

**Goal:** A queue in which a task is never lost and never runs on two agents at once.

**Scope:**
- Named queues; a task is an encrypted payload plus plaintext scheduling fields; `idempotency_key`.
- Atomic claim; a heartbeat extends the lease.
- Mode per queue: at-least-once (default) returns a task with an expired lease or a failure to the queue; at-most-once sends it to the DLQ without a retry.
- Fencing: `claim_id` with an increasing generation, extension and completion via compare-and-swap; whoever lost the lease gets `claim_lost`.
- Retries with exponential backoff, then DLQ.
- The result returns to the sender as an encrypted message.
- CLI to enqueue, claim, extend and complete; the helper sends the heartbeat.
- Queue load test.
- **Done when:** a stopped agent gets `claim_lost`; a live but slow agent does not lose its task; failed tasks go to the DLQ; an at-most-once task never runs twice.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 15
- **Docs:** `features.md` §10, `architecture.md` — "Queue", "Data model"

### Story 27: Browser linking and history on the web

**Goal:** The browser becomes a linked device and shows content, decrypting it locally.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- The "Link this browser" button: a code or QR from a linked machine, or the recovery key; the browser key is in IndexedDB and can be revoked.
- A WASM build of the Rust core in the SPA; the browser reads through history keys wrapped to its key and sends actions as signed messages via HPKE.
- Sections: message texts and threads, a "which agent messages whom" graph, history search in the browser.
- SRI and reproducible builds for the JS served by the server.
- **Done when:** a linked browser shows the channel history, and the server receives only ciphertext; a revoked browser cannot read new messages.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 10, Story 17, Story 22
- **Docs:** `features.md` §12, §6, `architecture.md` — "Components", "Device linking", "Cryptography"

### Story 28: Web: queues, delivery trace, loops

**Goal:** The browser shows what happens to tasks and messages and lets you control it.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- Queue panel: who holds a task, how much lease is left, `claim_lost`, retries, DLQ with "restart" and "cancel" buttons.
- Delivery trace: the `stored → delivered → read` path and the reason if a message was not delivered.
- Loop analysis: agent↔agent threads without a human, a "pause" button.
- **Done when:** a task from the DLQ is restarted from the browser, and a looping thread is paused.

- **Release:** MVP
- **Depends on:** Story 21, Story 26, Story 27
- **Docs:** `features.md` §12

### Story 35: Service agents: HPKE and scoped tokens

**Goal:** A service agent (`ci/deploy-bot`) without a persistent disk exchanges end-to-end encrypted messages using a scoped token.

**Scope:**
- A service agent with its own key, stored in an environment secret.
- A scoped token: send-only, queue-only, specific channels, expiry; token and key rotation.
- Stateless HPKE encryption for ephemeral agents.
- The "Service agents and tokens" web UI section: permissions, expiry, last use, rotation and revocation; the token value is shown once, at creation. The number of service agents is capped by the account limit.
- **Done when:** messages are exchanged with an agent in one-shot mode without a persistent disk, with only ciphertext on the server; a token used outside its permissions or after expiry is rejected.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 22
- **Docs:** `features.md` §9, "Product", `architecture.md` — "Data model", "Cryptography"

### Story 41: ask_user and attachments

**Goal:** An agent asks a human and sends small files.

**Scope:**
- `ask_user`: the agent requests confirmation from a human and waits for the answer.
- Encrypted attachments up to 10 MB, such as diffs and logs.
- **Done when:** the agent gets a human's answer to `ask_user`; an attachment reaches the recipient, and the server sees only ciphertext.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 21
- **Docs:** `features.md` §8, `architecture.md` — "Security"

### Story 65: Queue templates: review, test, ask

**Goal:** A team uses the queue without inventing its own task format.

**Scope:**
- Templates: review (a PR or diff → findings), test (a command → result and logs), ask (a question about a repository → answer).
- A template sets the task's input and result and the instructions the agent gets.
- `skald queue create --template <name>`; the Claude Code and Codex adapters run template tasks with the template's instructions.
- A team adds its own templates to the space.
- **Done when:** a review task from the template, sent by Claude Code, is done by Codex and returns findings in the template's format.

- **Release:** MVP
- **Depends on:** Story 19, Story 20, Story 26
- **Docs:** `features.md` §10

### Story 63: Notifications to people

**Goal:** A person learns at once that an agent waits for them, without watching the web UI.

**Scope:**
- Channels: Telegram, Slack, a webhook; set per person in the web UI.
- Events: a task or message addressed to the person, a `hold` or `ask_user` waiting, DLQ growing, an agent gone, a key changed, a token expiring.
- The server sends only metadata: who, which channel or queue, the event. A content preview comes only from the person's own device, if they turn it on.
- Rate limits, quiet hours, unsubscribe in one click.
- **Done when:** an `ask_user` and a DLQ entry arrive in Telegram within seconds; a notification sent by the server carries no message text.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 21, Story 22, Story 26, Story 41
- **Docs:** `features.md` §12, `architecture.md` — "Security"

### Story 66: Restricted mode for tasks from other people

**Goal:** An agent works on a task from another person only with the rights the space allows, so injected instructions cannot run commands on the machine.

**Scope:**
- Trust levels: own devices, space members, service agents. The sender comes from the signed envelope, not from the text.
- The space policy sets what each level may do: read only, no shell, no network, or everything.
- The Claude Code and Codex adapters start such a task in the matching mode: Claude Code permission settings, the Codex sandbox. A mode the agent cannot enforce is refused, not weakened.
- The threat model states what this does not stop.
- **Done when:** a task from another member that asks to run a shell command is refused by the agent in read-only mode; the same task from an own device runs.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 19, Story 20, Story 21
- **Docs:** `architecture.md` — "Security", `features.md` §8

### Story 36: GitHub Action

**Goal:** A GitHub Actions workflow with a single secret sends and receives encrypted messages, and a PR comment can hand a task to an agent.

**Scope:**
- A GitHub Action built on the client's one-shot mode and a service agent key.
- The Action is published together with the release.
- GitHub bridge: a comment `/skald <template> …` on an issue or PR from a user with write access becomes a task in the space's queue; the result returns as a comment on the PR.
- **Done when:** a GitHub Action with a single secret sends and receives an encrypted message, including behind a proxy; a `/skald review` comment on a PR returns the review as a PR comment; a comment from a user without write access is ignored.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 14, Story 35, Story 65
- **Docs:** `features.md` §2, §9, §16, `architecture.md` — "Components"

### Story 29: End-to-end MVP scenario

**Goal:** The MVP demo passes in full, automatically, on real agents.

**Scope:**
- Demo: Claude Code on account A asks Codex on machine B under account C for a review; the sender goes offline, and the reply waits in their mailbox.
- The review request goes through the task queue; the user follows the thread and the task in a linked browser.
- The scenario starts with the new-user path: sign-up (account A with GitHub, account C with Google), one-command machine connect, invite by link.
- The clients verify each other's device keys without trusting the server.
- It also covers: a review from a `/skald review` PR comment, an `ask_user` answered after a Telegram notification, and a task from another member run in restricted mode.
- Daily run on staging with real Claude Code and Codex in CI.
- It checks the "Done when" criteria of features 1, 4, 5, 6, 10 and 16.
- **Done when:** the demo passes automatically; the run report is available in CI.

- **Release:** MVP
- **Depends on:** Story 9, Story 18, Story 21, Story 23, Story 24, Story 26, Story 27, Story 28, Story 36, Story 41, Story 63, Story 65, Story 66
- **Docs:** `roadmap.md` — "MVP", `features.md` §1, §4, §5, §6, §10, §16, `infrastructure.md` — "PR checks"

### Story 30: Admin rights transfer and account deletion

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
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 16, Story 22
- **Docs:** `features.md` §4, §5, `architecture.md` — "Data model", "Cryptography"

### Story 31: Load testing

**Goal:** Before the public launch, know what load the server can handle and remove bottlenecks.

**Scope:**
- Scenarios: delivery and WebSocket push, cursor catch-up after a mass reconnect, MLS Commits in large channels, history writes, multiple instances with `LISTEN/NOTIFY`.
- Target numbers with headroom over the cloud limits (about 1M messages per month) are recorded in the docs.
- Runs on a production-like configuration; bottlenecks found are fixed or recorded.
- The run is repeated before releases.
- **Done when:** there is a report with the maximum load and the bottlenecks; production handles the target load with headroom.

- **Release:** MVP
- **Depends on:** Story 21, Story 25
- **Docs:** `features.md` — "Product", `architecture.md` — "Network", `infrastructure.md` — "Cloud"

### Story 32: Documentation: quick start and CLI reference

**Goal:** A new user connects and starts working from the docs, without help from the author.

**Scope:**
- Quick start, one person first: sign-up, connecting two own machines, the first task between their agents; then inviting the team and the recovery key.
- The CLI and MCP tool reference is generated from the code and never diverges from it.
- In plain language: what the server sees, that there has been no external audit, what to do after losing a device and after losing the admin.
- Published on `docs.skald.so` together with the release.
- **Done when:** a person without context connects Claude Code and Codex on two machines using the quick start; the reference matches `skald --help`.

- **Release:** MVP
- **Depends on:** Story 24, Story 25
- **Docs:** `features.md` §16, `architecture.md` — "Cryptography", `infrastructure.md` — "Channels", "Cloud"

### Story 33: Website: main page

**Goal:** `skald.so` explains what Skald is and leads to sign-up.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- What Skald is and how it works, what the server sees and does not see, pricing (everything is free except Enterprise); links to the docs, GitHub and sign-in at `app.skald.so`.
- Built and published together with the docs from `docs/` on GitHub Pages.
- **Done when:** `skald.so` shows the main page as in the approved design, at desktop and phone width; all links resolve.

- **Release:** MVP
- **Depends on:** Story 25, Story 62
- **Docs:** `features.md` — "Product", `architecture.md` — "Cryptography", `infrastructure.md` — "Channels", "Cloud"

### Story 34: Operator CLI

**Goal:** The operator runs the MVP cloud without SQL.

**Scope:**
- `skald-server admin`: list accounts with quota usage, disable and re-enable an account, delete an account.
- Metadata only, like the server itself.
- **Done when:** an operator finds an account and disables it, and the account loses access; deletion removes the data as in Story 30.

- **Release:** MVP
- **Depends on:** Story 12, Story 30
- **Docs:** `features.md` §17

### Story 37: Task queue: scheduling and agents

**Goal:** Claude Code, Codex and CI work through a shared queue according to priorities and capabilities.

**Scope:**
- Priorities, capability tags (`needs: codex`), per-queue concurrency, per-task cost limit.
- Queue tools in MCP; the Claude Code and Codex adapters take and hand in tasks, CI does so via the GitHub Action.
- Queue limits are per-account settings.
- **Done when:** Claude Code, Codex and CI process 100 tasks, duplicates are rejected, failed tasks go to the DLQ.

- **Release:** Post-MVP
- **Depends on:** Story 19, Story 20, Story 26, Story 36
- **Docs:** `features.md` §10, "Product"

### Story 38: Codex background agents via app-server

**Goal:** A Codex background agent receives messages without an open interactive session and without tmux.

**Scope:**
- The adapter drives Codex via app-server: `turn/start` starts a turn, `turn/steer` delivers a message into a running turn.
- e2e tests against real Codex.
- **Done when:** a background Codex receives a message and replies without human involvement.

- **Release:** Post-MVP
- **Depends on:** Story 20
- **Docs:** `features.md` §7, `architecture.md` — "Delivery to agents"

### Story 67: A2A gateway

**Goal:** An A2A agent exchanges tasks and messages with Skald agents.

**Scope:**
- A gateway translates between A2A over HTTPS and the Skald A2A extension: tasks, messages, results.
- The gateway holds a service agent key; end-to-end encryption ends at the gateway, and the docs say so.
- **Done when:** an A2A client sends a task to a Skald queue through the gateway and gets the result.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 2, Story 26, Story 35
- **Docs:** `architecture.md` — "Components"

### Story 39: Shared channels between organizations

**Goal:** Two spaces of different organizations communicate through a shared channel with the consent of both sides.

**Scope:**
- A shared channel joins two spaces; it is created only with the consent of admins on both sides, and either side can disable it.
- Approval in the web UI and CLI.
- **Done when:** a freelancer is in three spaces, with a clientA↔clientB shared channel; after one side disables it, no messages pass through it.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 22
- **Docs:** `features.md` §3, §12, `architecture.md` — "Data model"

### Story 40: Corporate IdP sign-in

**Goal:** Organizations sign in with their corporate IdP, and a self-hosted server works with its own IdP.

**Scope:**
- OIDC sign-in via a corporate IdP, on the OIDC support from Story 9.
- A self-hosted server uses its own IdP, with no calls to our cloud.
- **Done when:** a self-hosted server works with its own IdP without our cloud.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 9
- **Docs:** `features.md` §5

### Story 42: Shared team instructions

**Goal:** The team maintains shared instructions for agents, and they reach all machines automatically.

**Scope:**
- Versioned instructions, stored and distributed encrypted.
- Distribution to the space's machines and injection into `AGENTS.md` or `CLAUDE.md`.
- No distribution of agent configs (agent-fleet is out of scope).
- **Done when:** a new instruction version reaches all machines in the space and appears in `CLAUDE.md` and `AGENTS.md`.

- **Release:** Post-MVP
- **Depends on:** Story 19, Story 20
- **Docs:** `features.md` §8, "Out of scope"

### Story 43: Gemini, Cursor, OpenCode and Copilot adapters

**Goal:** Messages reach live Gemini CLI, Cursor, OpenCode and Copilot sessions.

**Scope:**
- Delivery via hooks and plugins; the fallback is MCP `inbox`.
- `skald connect` finds these agents and, after confirmation, connects them.
- e2e tests against real CLIs.
- **Done when:** a message reaches a live session of each of the four agents.

- **Release:** Post-MVP
- **Depends on:** Story 15, Story 24
- **Docs:** `features.md` §7, `architecture.md` — "Components", "Delivery to agents"

### Story 44: Edit coordination: file level

**Goal:** Agents of different people and machines learn before merge that they are editing the same files.

**Scope:**
- Clients publish HMAC fingerprints of changed files with the space key; the server sees only fingerprints.
- The action follows the space policy: notify, warn, ask a human, block pre-push.
- Claiming a task reserves its edit scope; reservations have a TTL.
- Decisions on the questions in `features.md` §11 "To decide", based on Story 5.
- **Done when:** two agents on different machines editing the same file get a warning before merge; the server does not know the paths.

- **Release:** Post-MVP
- **Depends on:** Story 5, Story 19, Story 20, Story 26
- **Docs:** `features.md` §11, `architecture.md` — "Cryptography"

### Story 45: Edit coordination: hunk and symbol

**Goal:** Edit overlap warnings that are more precise than file level.

**Scope:**
- Hunk and symbol fingerprints.
- Clients compute the exact overlap via `git merge-tree`.
- **Done when:** edits to different functions in one file trigger no warning; edits to the same function do.

- **Release:** Post-MVP
- **Depends on:** Story 44
- **Docs:** `features.md` §11

### Story 46: Web: security center, human inbox, edit map

**Goal:** A human manages security and approvals from the browser, including from a phone.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- Security center: devices and fingerprints, one-click revocation, key rotation, "who can read the channel".
- Human inbox: all `hold` and `ask_user` items, approval from a phone (the phone's browser is a linked device).
- Edit map.
- **Done when:** an `ask_user` is approved from a phone; a device is revoked with one click; the map shows overlapping edits.

- **Release:** Post-MVP
- **Depends on:** Story 18, Story 27, Story 41, Story 44
- **Docs:** `features.md` §12

### Story 47: Public cloud: anti-abuse

**Goal:** The cloud is protected from abuse.

**Scope:**
- Reports and bans based on reports; rules in `skald-cloud`.
- **Done when:** a banned account loses access.

- **Release:** Post-MVP
- **Depends on:** Story 22, Story 25
- **Docs:** `features.md` — "Product", §12, `architecture.md` — "Security", `infrastructure.md` — "Repositories"

### Story 48: Operator console

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
- **Depends on:** Story 30, Story 47
- **Docs:** `features.md` §17, §12, `architecture.md` — "Security"

### Story 49: Analytics and token usage

**Goal:** The team sees statistics on agent work.

**Scope:**
- Metadata statistics in the non-secret sections.
- Analytics: computed by the client, stored encrypted, shown in a linked browser.
- LLM token usage per task, only from what the CLI itself shows the user; low priority.
- **Done when:** metadata statistics are in the non-secret sections; analytics is visible only in a linked browser.

- **Release:** Post-MVP
- **Depends on:** Story 26, Story 27, Story 35
- **Docs:** `features.md` §12

### Story 50: Enterprise: SSO, SAML, SCIM and license

**Goal:** Organizations manage access through their own systems; `ee/` features are enabled by a license key.

**Scope:**
- `ee/` features under a commercial license are enabled by a license key in the same image.
- The Enterprise plan in the cloud: billing and the "Enterprise license, invoices" web UI section.
- Layout per the approved design in `../skald-cloud/design/`.
- SSO and SAML, SCIM.
- Admin access revocation.
- **Done when:** an employee removed in the organization's IdP automatically loses access; without a license key, `ee/` features are off; an organization that buys the cloud Enterprise plan gets `ee/` features right away.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 40
- **Docs:** `features.md` — "Product", `infrastructure.md` — "Cloud"

### Story 51: Enterprise: audit and retention policies

**Goal:** An organization controls and retains conversations by its own rules.

**Scope:**
- Audit log.
- Auditor device: the organization admin sees all conversations; members know about it.
- History retention policies.
- **Done when:** the auditor device reads the organization's channels, and members see an audit notice; history is deleted per policy.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 16, Story 50
- **Docs:** `features.md` — "Product", §12

### Story 52: Self-host: Helm and isolated networks

**Goal:** A self-hosted Skald runs in Kubernetes and in an isolated network.

**Scope:**
- Helm chart.
- Deployment in an isolated network (ee).
- Documentation for both.
- **Done when:** following the docs, Skald comes up from the Helm chart in Kubernetes and in a network without internet access.

- **Release:** Post-MVP
- **Depends on:** Story 40, Story 50, Story 64
- **Docs:** `infrastructure.md` — "Self-host", "Repositories", `features.md` — "Product"

### Story 53: History export and migration to a self-hosted server

**Goal:** A user takes their data and moves from the cloud to a self-hosted server without losing history.

**Scope:**
- `skald export` decrypts on the client the history available to the device and saves it as JSON or Markdown.
- Moving a space from the cloud to a self-hosted server: members, channels, history; devices reconnect to the new server.
- Account metadata export from the web UI instead of manual GDPR requests through support.
- **Done when:** a space has moved from the cloud to a self-hosted server, and members read all the previous history; `skald export` outputs the history in a readable form.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 16, Story 17, Story 52
- **Docs:** `features.md` §6, `infrastructure.md` — "Self-host"

### Story 54: Python and TypeScript SDKs

**Goal:** Developers embed Skald into their Python and TypeScript services.

**Scope:**
- The Rust SDK is the foundation for the others.
- Python SDK (PyO3) and TypeScript SDK (WASM or napi): messages and the task queue.
- Packages are published together with the release.
- **Done when:** a Python example and a TypeScript example send a message and process a task from the queue.

- **Release:** Post-MVP
- **Depends on:** Story 14, Story 26
- **Docs:** `tech-stack.md`

### Story 55: Homebrew, winget and cargo-binstall

**Goal:** `skald` installs through familiar package managers.

**Scope:**
- The `skald-so/homebrew-tap` repository; dist updates the formula.
- winget package.
- Publishing to crates.io as `skald-so` via trusted publishing, and installation via `cargo binstall skald-so`.
- **Done when:** after a release, the new version installs via Homebrew, winget and `cargo binstall`.

- **Release:** Post-MVP
- **Depends on:** Story 7
- **Docs:** `features.md` §16, `infrastructure.md` — "Repositories", "Channels", "Trust in builds"

### Story 56: Managed keys: custodian device

**Goal:** A space can choose a mode in which the server reads content for the sake of convenience.

**Scope:**
- The server keeps its own custodian device in channels.
- An admin chooses the mode for the whole space; the choice is signed and visible to all members; the default is end-to-end encryption.
- In this mode, the web works without linking a device, and search runs on the server.
- **Done when:** in a managed-keys space, the web shows the history without linking the browser; members see that the mode is on.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 16, Story 27
- **Docs:** `features.md` §13, `architecture.md` — "Cryptography"

### Story 57: Hosted MCP for ChatGPT and Claude.ai

**Goal:** ChatGPT and Claude.ai work with a space via remote MCP.

**Scope:**
- Remote MCP on the server, only for managed-keys spaces.
- Connecting ChatGPT and Claude.ai.
- **Done when:** ChatGPT and Claude.ai send and read the space's messages via hosted MCP.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 15, Story 56
- **Docs:** `features.md` §9, §13

### Story 58: skald ui

**Goal:** A web UI without code delivered from the server, for those who do not trust that code.

**Scope:**
- `skald ui` serves the same SPA build from localhost out of a signed binary; there is no second interface.
- **Done when:** `skald ui` opens the web UI from localhost, and the content sections work without loading JS from the server.

- **Release:** Post-MVP
- **Depends on:** Story 7, Story 27
- **Docs:** `features.md` §12, `architecture.md` — "Components"

### Story 59: Sealed sender and padding

**Goal:** Reduce the metadata the server sees.

**Scope:**
- Sealed sender: the server does not see which device sent a message.
- Padding: the server does not see the exact message size.
- The threat model is updated.
- **Done when:** the envelope's plaintext header has no sender, and ciphertext sizes are evened out.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 12, Story 13
- **Docs:** `architecture.md` — "Cryptography"

### Story 60: Server federation

**Goal:** Organizations on different Skald servers communicate with each other.

**Scope:**
- Delivery between servers by the server name in the agent address.
- Shared channels between spaces on different servers.
- **Done when:** agents on two different servers exchange messages through a shared channel.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 18, Story 39
- **Docs:** `features.md` §15, `architecture.md` — "Network"

### Story 61: Capacity view

**Goal:** The team sees members' subscription limits, and a task waits for a limit to reset.

**Scope:**
- Only data that the CLI itself shows the user; we do not touch vendor tokens or switch accounts.
- A task waits for the limit to reset on the same account.
- Built only if Story 4 confirmed that vendors expose this data.
- **Done when:** the team sees members' limits; a task for an agent with an exhausted limit waits for it to reset.

- **Release:** Post-MVP
- **Depends on:** Story 4, Story 37
- **Docs:** `features.md` §14
