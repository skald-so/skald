## Stories

Paths in the "Docs" fields are relative to `docs/dev/` unless they start with `docs/`.

| # | Story | Release | Depends on |
|---|---------|-------|------------|
| 1 | Monorepo skeleton and CI | MVP | — |
| 2 | Protocol spec and threat model | MVP | 1 |
| 3 | MLS prototypes and library choice | MVP | 1 |
| 4 | Verifying delivery to Claude Code and Codex | MVP | 1 |
| 5 | Edit comparison prototype with HMAC fingerprints | MVP | 1 |
| 6 | Staging environment | MVP | 1 |
| 7 | Release pipeline | MVP | 1 |
| 62 | Web UI kit: theme, cabinet shell and components | MVP | 1 |
| 64 | Self-host: Docker Compose | MVP | 6, 7, 9 |
| 8 | GitHub sign-in and device certificate | MVP | 1, 2 |
| 9 | Google sign-in and linked sign-in methods | MVP | 8 |
| 10 | Crypto core | MVP | 2, 3 |
| 11 | Spaces and agent addresses | MVP | 8 |
| 12 | Encrypted mailbox | MVP | 11 |
| 13 | MLS channels, roles and device revocation | MVP | 10, 12 |
| 14 | Client: one-shot mode and helper | MVP | 12, 13 |
| 15 | MCP server and agent commands | MVP | 12, 13, 14 |
| 16 | Encrypted channel history | MVP | 13 |
| 17 | Recovery key and new device linking | MVP | 16 |
| 18 | Key verification without trusting the server | MVP | 17 |
| 19 | Claude Code adapter and plugin | MVP | 4, 14, 15 |
| 20 | Codex adapter | MVP | 4, 14, 15, 17 |
| 21 | Presence, statuses and loop protection | MVP | 13, 15 |
| 22 | Web UI: non-secret sections | MVP | 8, 9, 11, 12, 13, 62 |
| 23 | Space invite by link | MVP | 13, 14, 17, 22 |
| 24 | One-command machine connect | MVP | 6, 7, 14, 17, 18, 19, 20, 22, 23 |
| 25 | Production cloud, website and monitoring | MVP | 6, 7, 12 |
| 26 | Task queue: claim, lease and DLQ | MVP | 13, 14, 15 |
| 27 | Browser linking and history on the web | MVP | 10, 17, 22, 23 |
| 28 | Web: queues, delivery trace, loops, ask_user | MVP | 21, 26, 27, 41 |
| 35 | Service agents: HPKE and scoped tokens | MVP | 13, 14, 16, 22 |
| 41 | ask_user and attachments | MVP | 21 |
| 65 | Queue for agents: MCP tools and templates | MVP | 19, 20, 26 |
| 63 | Notifications to people | MVP | 18, 21, 22, 26, 35, 41 |
| 66 | Restricted mode for tasks from other people | MVP | 19, 20, 21, 26 |
| 36 | GitHub Action | MVP | 14, 35, 65, 66 |
| 43 | Gemini, Cursor, OpenCode and Copilot adapters | MVP | 15, 24 |
| 29 | End-to-end MVP scenario | MVP | 9, 18, 21, 23, 24, 26, 27, 28, 36, 41, 63, 65, 66 |
| 30 | Admin rights transfer and account deletion | MVP | 13, 16, 17, 22 |
| 31 | Load testing | MVP | 21, 25 |
| 32 | Documentation: quick start and CLI reference | MVP | 24, 25 |
| 33 | Website: main page | MVP | 25, 62 |
| 34 | Operator CLI | MVP | 12, 30 |
| 37 | Task queue: scheduling | Post-MVP | 19, 20, 26, 36, 65 |
| 67 | A2A gateway | Post-MVP | 2, 26, 35 |
| 38 | Codex background agents via app-server | Post-MVP | 20 |
| 68 | Team plan and billing | Post-MVP | 22, 25 |
| 39 | Shared channels between organizations | Post-MVP | 13, 22, 68 |
| 40 | Corporate IdP sign-in | Post-MVP | 9 |
| 42 | Shared team instructions | Post-MVP | 19, 20 |
| 44 | Edit coordination: file level | Post-MVP | 5, 19, 20, 26, 65 |
| 45 | Edit coordination: hunk and symbol | Post-MVP | 44 |
| 46 | Web: security center, human inbox, edit map | Post-MVP | 18, 27, 41, 44 |
| 47 | Public cloud: anti-abuse | Post-MVP | 22, 25 |
| 48 | Operator console | Post-MVP | 30, 47, 62 |
| 49 | Analytics and token usage | Post-MVP | 26, 27, 35 |
| 50 | Enterprise: SSO, SAML, SCIM and license | Post-MVP | 40 |
| 51 | Enterprise: audit and retention policies | Post-MVP | 16, 50 |
| 52 | Self-host: Helm and isolated networks | Post-MVP | 40, 50, 64 |
| 53 | History export and migration to a self-hosted server | Post-MVP | 16, 17, 64 |
| 54 | Python and TypeScript SDKs | Post-MVP | 14, 26 |
| 55 | Homebrew, winget and cargo-binstall | Post-MVP | 7 |
| 69 | OpenClaw and Crush setup by `skald connect` | Post-MVP | 24 |
| 56 | Managed keys: custodian device | Post-MVP | 16, 27 |
| 57 | Hosted MCP for ChatGPT and Claude.ai | Post-MVP | 15, 56 |
| 58 | skald ui | Post-MVP | 7, 27 |
| 59 | Sealed sender and padding | Post-MVP | 12, 13 |
| 60 | Server federation | Post-MVP | 18, 39 |
| 61 | Capacity view | Post-MVP | 4, 37 |

**Order and parallelism**

Each story starts by [rules.md](rules.md), rule 6; stories within a wave can be done in parallel. Priority is the order in this file.

- Wave 1: 1.
- Wave 2: 2, 3, 4, 5, 6, 7, 62.
- Wave 3: 8, 10.
- Wave 4: 9, 11.
- Wave 5: 12, 64.
- Wave 6: 13, 25 (25 can be postponed until launch).
- Wave 7: 14, 16, 22, 33.
- Wave 8: 15, 17, 35.
- Wave 9: 18, 19, 20, 21, 23, 26, 30.
- Wave 10: 24, 27, 31, 34, 41, 65, 66.
- Wave 11: 28, 32, 36, 43, 63.
- Wave 12: 29 — end of MVP.
- Wave 13: 37, 38, 40, 42, 44, 47, 49, 53, 54, 55, 56, 58, 59, 67, 68, 69.
- Wave 14: 39, 45, 46, 48, 50, 57, 61.
- Wave 15: 51, 52, 60.

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
- Spec in `docs/protocol/`: identifiers and the agent address with the server name; the envelope in deterministic CBOR with the `mls` and `hpke` schemes; Skald as an A2A extension; protocol versions.
- Mailbox, delivery and receipts, with the per-sender counter for gap detection.
- Devices, roster and channels: the account master key and the device certificate chain, the signed roster, invites and space policy, KeyPackages and Commit ordering, history keys.
- Queues and tasks, presence, `ask_user`.
- The edit-coordination wire format comes with Story 44, after Story 5's findings.
- Room for the custodian device (feature 13), the auditor device (`ee/`) and federation (feature 15) without a new protocol version.
- Threat model: attackers, what the server sees and does not see, trade-offs, prompt injection, loops.
- The spec's types in `proto`; the server and the client check the protocol version.
- The spec and threat model are published openly, marked "no external audit".
- **Done when:** the spec and threat model are in `docs/`; client and server negotiate the protocol version; an outdated client gets "please update".

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `docs/protocol/README.md`, `docs/threat-model.md`, `roadmap.md` — "Preparation", `architecture.md` — "Membership", "Data model", "Cryptography", "Queue", "Security", `infrastructure.md` — "Release", `features.md` §1, §4, §5, §6, §10, §13, §15

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

#### Iteration 2.3: Protocol spec: devices, roster and channels
- **Goal:** The spec fixes how devices, members and channel keys are certified, so the identity, space, channel and history stories build on one contract.
- **What we do:**
  - `docs/protocol/membership.md`, listed in the index in `README.md`.
  - Account master key and device certificate (`architecture.md`, "Device linking"): the certificate's fields, the device kind among them; the master key the first device creates and its copy encrypted under the recovery key; the chain from a certificate to the master key through already linked devices or the recovery key, and the optional SSH key signature; the checks a client makes, so that a server signature alone never admits a device.
  - Sender trust levels: an own device, a space member, a service agent; how a recipient derives them from the certificate chain and the roster.
  - Space roster and policy (`architecture.md`, "Membership"): their fields, the admin's signature on every change, how a client checks a change and what it does with one that fails.
  - Invite: the signed invite, the secret in the link fragment, how the newcomer proves it holds the secret, how the admin's device checks the newcomer's devices.
  - Admin actions from a linked browser, which is not an MLS member: how a roster change it signs reaches the MLS groups.
  - MLS channels: creating a channel's group; KeyPackages: who publishes them, how many and for how long; Commit ordering by the server; who commits the Add of a member's new device; how a device rejoins a channel after missing Commits, after a wipe or offline past the mailbox retention.
  - History key: distribution inside MLS; wrapping to devices, linked browsers and the recovery key; rotation on revocation and on the policy's schedule; the encrypted key chain; what a new member without history access gets.
  - `architecture.md` keeps the decisions and links here for the formats; stories' "Docs:" lines that cite a fact this iteration moves link here.
- **Definition of done:** each item above has its section; every signed structure has a CDDL definition, a signature label and the checks a recipient MUST make; no wire format is repeated in `architecture.md`.

#### Iteration 2.4: Protocol spec: queues and tasks
- **Goal:** The spec fixes the task queue, so Stories 26 and 65 need no breaking change later.
- **What we do:**
  - `docs/protocol/queues.md`, listed in the index; the wire part of `architecture.md`, "Queue" (claim, lease, fencing, modes), moves there and is replaced by a link.
  - Queues: plaintext scheduling fields and the encrypted payload; claim, lease with heartbeat, fencing with `claim_id` and generation, `claim-lost`; at-least-once and at-most-once modes, retries, DLQ; the result returned as an encrypted message.
  - A queue belongs to a channel; a task payload is encrypted with the channel's MLS group, so any device of the channel can claim the task (`architecture.md`, "Queue").
  - The heartbeat in one-shot mode without the helper: the process running the task sends it.
  - Task templates: how a task names its template and carries the template's input and result.
  - Stories' "Docs:" lines that cite "Queue" link to the new place.
- **Definition of done:** each item above has its section; the task states and transitions, the fencing checks, the claim check against the channel's members and the CDDL of the payload and the result are defined; `architecture.md`, "Queue", keeps only server-side decisions and a link.

#### Iteration 2.5: Protocol spec: presence and `ask_user`
- **Goal:** The spec fixes the agent status and the questions agents ask people.
- **What we do:**
  - `docs/protocol/agents.md`, listed in the index.
  - Presence: the encrypted agent status and who can read it; the server keeps only the latest value.
  - `ask_user`: the question and the answer as content bodies; the answer comes from the person's device, in the CLI or a linked browser, signed by it.
  - Stories' "Docs:" lines that cite presence or `ask_user` link to the new sections.
- **Definition of done:** presence and both `ask_user` bodies have CDDL definitions and the checks a recipient MUST make; `architecture.md`, "Data model", links to them.

#### Iteration 2.6: Threat model
- **Goal:** Users and contributors know what Skald protects against and what it does not.
- **What we do:**
  - `docs/threat-model.md`, marked "no external audit" and linked from `AGENTS.md`.
  - Attackers and what each can do: the server or its operator, a network observer, the IdP or GitHub, a stolen device, a removed member, a member or an admin acting against others, a malicious sender, a leaked CI secret.
  - "The server sees", "The server does not see" and "Trade-offs" move from `architecture.md`, "Cryptography", to the threat model; links to them in `architecture.md`, `features.md` and the spec point there.
  - Prompt injection and loops: the defenses in `architecture.md`, "Security", and what they do not stop.
  - Stories' "Docs:" lines that cite the moved lists link to the threat model.
- **Definition of done:** each attacker has a section with what it can do and the defenses, each defense linking to the spec section it relies on; `architecture.md` keeps only links to the moved lists.

#### Iteration 2.7: Identifiers and agent addresses in `proto`
- **Goal:** The client and server share one implementation of the spec's identifiers and addresses.
- **What we do:**
  - Types in `proto` for the identifiers and the agent address defined by the spec.
  - Parsing and printing an address with the server name; an invalid address is rejected with an error naming the problem.
  - Tests: valid identifiers and addresses round-trip; each invalid form from the spec is rejected.
- **Definition of done:** `proto` follows the spec's identifier and address rules; unit tests pass in `script/test`.

#### Iteration 2.8: Envelope and receipt types in `proto`
- **Goal:** Envelopes and receipts have one wire implementation that matches the spec.
- **What we do:**
  - Types in `proto` for the envelope header, the content inside the ciphertext with its A2A message, and receipts.
  - CBOR encoding and decoding of envelopes and their content as in the spec; malformed or truncated input returns an error, never a panic. The CBOR crate is recorded in `tech-stack.md`.
  - Test vectors in `docs/protocol/`: the hex of each envelope and receipt kind, for other implementations.
  - Tests: every type round-trips; `proto` output matches the test vectors byte for byte; malformed and truncated frames are rejected.
- **Definition of done:** `proto` encodes and decodes envelopes per the spec; unit tests pass in `script/test`.

#### Iteration 2.9: Protocol version check on the server
- **Goal:** The server serves clients of the current and previous protocol versions and tells older ones to update.
- **What we do:**
  - The OpenAPI spec first: how a request states the protocol version and the answers to unsupported versions (rule 4 in `rules.md`).
  - The current protocol version in `proto`.
  - `skald-server` accepts the current and previous versions and refuses others as the protocol spec says, with "please update" for a too-old client, on HTTPS requests and WebSocket opening requests.
  - The supported range is a server setting, by default the current and the previous version, so tests reproduce every case while only version 1 exists.
  - Tests: every row of the spec's "Server answers" table with the range set to versions 2–3, over HTTPS and WebSocket; requests that state no version by the spec; responses conform to the OpenAPI spec.
- **Definition of done:** the server enforces its supported range, by default the current and previous versions; each row of the "Server answers" table has a server integration test in `script/test`.

#### Iteration 2.10: Protocol version check in the client
- **Goal:** An outdated `skald` tells its user to update instead of failing obscurely.
- **What we do:**
  - `skald status --server <url>`: connects to the server and prints whether it is reachable and whether its protocol version is compatible; later stories add the account and devices.
  - The client's server connection, which `skald status` and later commands reuse, states the protocol version on every request and WebSocket opening request.
  - The client acts on each answer as the spec's "Server answers" table says: an update notice for N−1, an error telling the user to update `skald` for "please update", a retry with a supported version for "too new" if it speaks one, otherwise an error saying the server is older.
  - The versions the client speaks can be set for tests.
  - Tests: `skald status` against a running `skald-server` with its supported range set for each row of the table.
- **Definition of done:** `skald status` handles each row of the spec's "Server answers" table as the spec says; client integration tests with `skald-server` pass in `script/test`.

### Story 3: MLS prototypes and library choice

**Goal:** Compare OpenMLS and mls-rs on Skald scenarios and choose the library for the crypto core.

**Scope:**
- A prototype on each library: device group, adding, removing with a new epoch, KeyPackages, Commit ordering, WASM build.
- Comparison: RFC 9420 compliance, audit, API ergonomics, performance, WASM size, maintenance.
- The comparison goes into a report in the docs; the choice into `tech-stack.md`.
- **Done when:** one library is chosen; prototypes and benchmarks are in the repository.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `roadmap.md` — "Preparation", `architecture.md` — "Cryptography", `tech-stack.md`

#### Iteration 3.1: Prototype harness and a channel group on both libraries
- **Goal:** OpenMLS and mls-rs run the same Skald channel scenarios, so every later result compares them directly.
- **What we do:**
  - A prototype on each library, at the versions in `tech-stack.md`, each on its RustCrypto-based crypto provider, which also builds for WASM. The prototypes live apart from the product crates, which never depend on them; `script/test` builds and tests them, and the CI path filters cover them.
  - One library-neutral scenario suite drives both prototypes; later scenarios, the WASM build and the benchmarks reuse it.
  - The profile of `docs/protocol/membership.md`, "MLS profile": the ciphersuite, the `basic` credential with the device ID, and the device's Ed25519 signing key, generated outside the library, as the leaf's signature key.
  - Scenario: a device creates a channel's group and adds devices from their KeyPackages, as "Creating a channel" says; the added devices join from the Welcome; members exchange application messages as PrivateMessages; proposals and Commits are PublicMessages.
  - Each device saves its state after every step and loads it before the next, as one-shot mode does (`architecture.md`, "Components").
  - The report in `docs/dev/` starts with the scenario list. Each iteration of this story adds, per library, what its scenarios needed: missing features, workarounds, awkward API.
  - Tests: the scenario on both prototypes; every member decrypts each message and gets the sender's device ID from its credential; a replayed application message is rejected.
- **Definition of done:** the scenario passes on both prototypes with state reloaded between steps; scenario tests run in `script/test`; the report lists the findings so far.

#### Iteration 3.2: Removing a device with a new epoch
- **Goal:** Both libraries show that a removed device reads nothing sent after its removal, and give the history key of each epoch.
- **What we do:**
  - Scenario: a member commits the Remove of a device, and a new epoch begins; the removed device, from a full copy of its saved state, decrypts no later message.
  - The Remove Commit carries revocations in its `authenticated_data`, as `docs/protocol/membership.md`, "Group changes", says; every member reads them before it merges the Commit.
  - An empty Commit, with an UpdatePath and no proposals, starts a new epoch ("History key").
  - Each member derives the epoch's history key from the MLS exporter, as "History key" defines it.
  - Findings in the report.
  - Tests: on both prototypes, the removed device's copy fails on every message after the Remove; members read the `authenticated_data` before merging; every member of an epoch derives the same history key; a Remove and an empty Commit each change it, and the removed device cannot derive the new one.
- **Definition of done:** the scenarios pass on both prototypes; scenario tests run in `script/test`.

#### Iteration 3.3: KeyPackages and admission checks
- **Goal:** Both libraries let a device keep KeyPackages as the spec says and keep devices that are not admitted out of a channel.
- **What we do:**
  - A device makes KeyPackages and its last-resort KeyPackage as `docs/protocol/membership.md`, "KeyPackages", says; the last-resort one adds the device to several groups.
  - Before an Add, the adding device checks the KeyPackage as "KeyPackages" says and refuses one that fails. A test fixture stands in for certificates and the roster and gives the admitted devices.
  - A member processing another device's Commit sees each added device's credential before it merges the Commit, and removes a device that is not admitted ("Admitted devices").
  - Findings in the report.
  - Tests: on both prototypes, a valid KeyPackage is added; each failing check of "KeyPackages" refuses its Add; the last-resort KeyPackage adds its device to two groups; a device that another member added without being admitted is removed.
- **Definition of done:** the scenarios pass on both prototypes; scenario tests run in `script/test`.

#### Iteration 3.4: Commit ordering through a minimal Delivery Service
- **Goal:** Both libraries let the server order Commits without group secrets, and Commits sent at once converge on one epoch.
- **What we do:**
  - A minimal in-process Delivery Service stands in for `skald-server`: it follows each channel's epoch and member devices from PublicMessages and GroupInfo only, with each library's support for a party outside the group.
  - Each Commit's sender makes the new epoch's GroupInfo with the extensions `docs/protocol/membership.md`, "MLS profile", names; the Delivery Service keeps the latest.
  - The Delivery Service accepts or refuses each Commit as "Commit ordering" says, and keeps the first Commit of epoch 0 as "Creating a channel" says.
  - The senders of refused Commits drop their pending Commit, process the accepted one and commit again if their change is still needed.
  - Members process handshake messages in the order the Delivery Service stores them.
  - Findings in the report.
  - Tests: on both prototypes, two and three devices commit in one epoch and all converge on the accepted Commit, with the refused changes committed after it; a Commit for an old epoch gets `epoch-mismatch` and one from a non-member `not-a-member`; two devices creating one channel end in the kept group.
- **Definition of done:** the scenarios pass on both prototypes; scenario tests run in `script/test`.

#### Iteration 3.5: Joining by itself and rejoining
- **Goal:** Both libraries support the spec's paths for a device to join or rejoin a channel without a member's help.
- **What we do:**
  - Joining by itself, as `docs/protocol/membership.md`, "Group changes", says: a device outside the group checks the latest GroupInfo's signer and members, then joins with an external Commit; members check the joiner as an Add.
  - A device joining from a Welcome checks its signer and members the same way.
  - Rejoining after missed Commits, as "Rejoining" says: an external Commit that also removes the device's old leaf.
  - A device that was offline processes every Commit it missed in order, then decrypts new messages.
  - Findings in the report.
  - Tests: on both prototypes, an external join that members check; a GroupInfo or Welcome with a non-member signer or a member that is not admitted is refused; a device that missed a Commit fails on the next one, rejoins and holds one leaf; a device that missed 100 Commits catches up.
- **Definition of done:** the scenarios pass on both prototypes; scenario tests run in `script/test`.

#### Iteration 3.6: WASM build and size
- **Goal:** Both libraries work in the browser's WASM build, and their size there is known.
- **What we do:**
  - Both prototypes build for `wasm32-unknown-unknown`; the scenario suite runs in WASM as natively.
  - The check a linked browser makes, as `docs/protocol/README.md`, "Envelope", and `docs/protocol/membership.md`, "Wraps", say: a GroupInfo with its ratchet tree, signed by a member leaf, gives the channel's member devices.
  - The size of each library's release WASM module, raw and compressed, with only the browser check and with the whole suite, in the report.
  - `CONTRIBUTING.md` lists the tools the WASM tests need.
  - Tests: the scenario suite and the browser check run in a WASM runtime; a GroupInfo with a non-member signer or an altered tree is rejected.
- **Definition of done:** both prototypes pass in WASM; WASM tests run in `script/test`; the report holds the sizes.

#### Iteration 3.7: Interoperability between the libraries
- **Goal:** The prototypes show whether OpenMLS and mls-rs interoperate, as a check of their RFC 9420 compliance.
- **What we do:**
  - One channel with devices on both prototypes: in turn, a device on each library creates the group, adds, removes, makes an empty Commit and joins by external Commit; devices on the other library process each step.
  - Messages both ways; members on both libraries derive the same history key.
  - A mismatch goes into the report with its RFC 9420 section and an issue to the library; its test asserts the observed failure, so a library fix shows up.
  - Tests: every step above in both directions.
- **Definition of done:** interop tests run in `script/test`; the report lists each mismatch or states there is none.

#### Iteration 3.8: Benchmarks
- **Goal:** The cost of each channel operation on both libraries is known at the group sizes Skald expects.
- **What we do:**
  - Benchmarks on the scenario suite, the same steps on both libraries: create a group; commit an Add, a Remove and an empty Commit, and process each; join from a Welcome and by external Commit; make KeyPackages; encrypt and decrypt a message; catch up on missed Commits; the Delivery Service's Commit check; the browser's GroupInfo check in WASM.
  - Groups of 2, 10, 100 and 1000 devices.
  - The sizes of a Commit, a Welcome, a GroupInfo, a KeyPackage and a device's saved state at each group size.
  - One command runs them; the report holds the results with the machine and the versions.
  - Tests: each benchmark runs once on the smallest group.
- **Definition of done:** the command gives results for both libraries at every size; the report holds them; the benchmark smoke run passes in `script/test`.

#### Iteration 3.9: Comparison report and the library choice
- **Goal:** One MLS library is chosen on the prototypes' evidence, and Story 10 knows what to take from them.
- **What we do:**
  - The report compares both libraries by:
    - RFC 9420 compliance: the test vectors each passes, the interop results;
    - audit: published audits, their scope and date;
    - API ergonomics: the findings of the earlier iterations;
    - performance: the benchmark results;
    - WASM size;
    - maintenance: release cadence, maintainers, security advisories, minimum Rust version, licenses, dependencies;
    - spec fit: how each library meets each requirement of `docs/protocol/membership.md` the prototypes exercised.
  - The choice with its reasons; `tech-stack.md`, "MLS", names the chosen library and its version and links to the report.
  - For Story 10: the chosen library's workarounds from the findings, and the spec requirements it does not meet with their sections, each left for its own spec change.
- **Definition of done:** the report covers every criterion for both libraries; `tech-stack.md` names one MLS library; no other doc repeats the comparison.

### Story 4: Verifying delivery to Claude Code and Codex

**Goal:** On current versions of Claude Code and Codex, confirm the paths for delivering messages into a live session in their CLIs, desktop apps and IDE extensions, and find out what subscription limit data they expose.

**Scope:**
- Every Claude Code and Codex path in `architecture.md`, "Delivery to agents".
- What subscription limit data the CLIs themselves show the user, within vendor rules.
- A minimal prototype for each path.
- The same paths in the Claude Code and Codex desktop apps and IDE extensions, checked by the developer or reviewer driving the app.
- **Done when:** a report is in the docs; the primary and fallback paths are confirmed for each agent and surface; the "Delivery to agents" table in `architecture.md` is up to date.

- **Release:** MVP
- **Manual check:** the report's primary and fallback delivery paths for each agent.
- **Depends on:** Story 1
- **Docs:** `roadmap.md` — "Preparation", `architecture.md` — "Delivery to agents", `features.md` §7, §14

#### Iteration 4.1: Prototype harness and the MCP `inbox` path
- **Goal:** Every delivery path is checked the same way against the real CLIs, starting with MCP `inbox`, the fallback both agents share.
- **What we do:**
  - The prototypes, in Rust on the stack of `tech-stack.md`, live apart from the product crates, which never depend on them; `script/test` builds and tests them, and the CI path filters cover them.
  - A local message source stands in for the helper: a test or a person adds a message with a `msg_id`, a sender label and text; a path takes the waiting messages and marks each handed over, as `docs/protocol/delivery.md`, "Device to agent", says.
  - A live check per path, one command against the installed CLI:
    - it runs one session and sends it one message with a random token, and prints the CLI version and pass or fail;
    - with `--all` it also runs the other cases of the list of checks below: a busy session and reaching one of two sessions;
    - by default a local stub stands in for the model, where the CLI accepts a custom model endpoint and the path and its data work under a placeholder key:
      - the stub answers with scripted turns, such as calling the path's tool or running a shell command;
      - the check passes when the token reaches a request the CLI sends to the stub; it spends no tokens;
      - the CLI has no real credential to send: it runs in an environment the check builds, with only the variables the CLI needs and the placeholder key, and with a temporary config home that holds no sign-in, such as `CODEX_HOME` or `CLAUDE_CONFIG_DIR`;
      - the stub fails the check, without printing the value, on a request whose credential headers hold anything but the placeholder; a test of the stub covers it;
    - with `--real`, or where the stub cannot stand in, the real model answers:
      - the check passes when the agent repeats the token;
      - it prints the tokens it used, where the CLI's output gives them;
      - where the stub cannot stand in, the report says why;
    - it runs without a person where the CLI allows it;
    - the owner's preparation, done beforehand, is the only step a person does in an iteration: signing in to the CLIs, and for GUI live checks what 4.7 lists;
    - a step the CLI needs from a person beyond it goes in the report and is left to the story's Manual check, not to an iteration's Definition of done;
    - it attaches hooks, MCP servers and channels only through project config in a temporary directory and changes none of the user's own CLI settings, plugins or sign-in;
    - off the stub, the CLI runs in the owner's own config home on the owner's sign-in (for Claude Code, the subscription, with no API key in its place); the entries the CLI adds for the temporary project, such as trust, session stats and memory, are allowed;
    - with the real model it spends the fewest tokens:
      - the CLI loads as little of the user's own config (MCP servers, plugins, skills, instruction files) as it allows, and the report records what each check still loads;
      - the model runs at the lowest reasoning effort the CLI allows;
      - its prompts are short;
    - the developer runs it once with `--all`, and once with `--real` only where it runs on the stub by default, for the report; the reviewer reruns it with neither flag; `script/test` does not run it.
  - The report in `docs/dev/` starts with the list of checks it answers for each path: an idle and a busy session; the delay; how the message appears in the agent's context, with its sender label; whether the prototype learns that the handover happened, for `read`; reaching one of two sessions on a device; what the path needs: CLI version, sign-in kinds, settings, flags and prompts to the user.
  - MCP `inbox`: a stdio MCP server whose `inbox` tool returns the waiting messages; its live check on Claude Code and Codex.
  - Findings in the report.
  - Tests: the message source hands each message over once; an MCP client calls `inbox` over stdio and gets each waiting message once, with its sender label and `msg_id`.
- **Definition of done:** the `inbox` live check, rerun on each agent, gives the result and CLI version the report records; the report answers every check of the list for `inbox` on each agent; unit and MCP integration tests run in `script/test`.

#### Iteration 4.2: Claude Code Channels
- **Goal:** The Claude Code primary path is confirmed or refuted on the current version, with what it needs.
- **What we do:**
  - A prototype channel: an MCP server that pushes each waiting message into a running Claude Code session through Channels.
  - Its live check, with several messages in a row under `--all`, and the answers to the list of checks.
  - Whether a plugin from the monorepo marketplace can ship the channel (Story 19).
  - Findings in the report.
  - Tests: an MCP client gets one channel event per waiting message, with its sender label and `msg_id`; a pushed message is marked handed over and not pushed again.
- **Definition of done:** the Channels live check, rerun, gives the result and Claude Code version the report records; the report answers every check of the list for Channels; MCP integration tests run in `script/test`.

#### Iteration 4.3: Claude Code `Stop` and `PostToolUse` hooks
- **Goal:** The Claude Code fallback through hooks is confirmed or refuted on the current version, with its limits.
- **What we do:**
  - A hook handler: after a tool call (`PostToolUse`) it adds the waiting messages to the context with `additionalContext`; at the end of a turn (`Stop`) it keeps the turn going and gives the agent the waiting messages, and lets the turn end when none wait.
  - A live check of each hook, and the answers to the list of checks, with the time the handler adds to a tool call.
  - Whether the plugin can ship the hooks (Story 19).
  - Findings in the report.
  - Tests: on hook inputs recorded from the real Claude Code, the handler's output carries each waiting message once, with its sender label and `msg_id`; with messages waiting, `Stop` keeps the turn going and gives them to the agent; with none waiting it adds nothing, and `Stop` lets the turn end.
- **Definition of done:** the live check of each hook, rerun, gives the result and Claude Code version the report records; the report answers every check of the list for each hook; unit tests on the recorded hook inputs run in `script/test`.

#### Iteration 4.4: Codex hooks and `codex queue`
- **Goal:** The Codex primary path through hooks and the `codex queue` fallback are confirmed or refuted on the current version.
- **What we do:**
  - The hook handler for Codex: `PostToolUse` after a tool call, `Stop` at the end of a turn and `UserPromptSubmit` with the user's next prompt add the waiting messages to the context; `Stop` keeps the turn going while messages wait and lets it end when none wait.
  - A live check of each hook, and the answers to the list of checks, with the time the handler adds to a tool call.
  - Whether `codex queue` exists and adds a message to a running session from outside. If it does, a prototype hands the waiting messages to it, with its live check and the answers to the list of checks; if not, the report says so, with the Codex version.
  - Findings in the report.
  - Tests:
    - on hook inputs recorded from the real Codex, the handler's output carries each waiting message once, with its sender label and `msg_id`; with messages waiting, `Stop` keeps the turn going and gives them to the agent; with none waiting it adds nothing, and `Stop` lets the turn end;
    - if `codex queue` exists, against a stand-in for the command: the prototype passes each waiting message once, with its sender label and `msg_id`; it marks a message handed over only after the command succeeds and leaves it waiting when the command fails.
- **Definition of done:** the live check of each hook, rerun, gives the result and Codex version the report records, and the report answers every check of the list for each hook; if `codex queue` exists, the same holds for it, otherwise the report says it does not; unit tests on the recorded hook inputs, and on the `codex queue` stand-in if the command exists, run in `script/test`.

#### Iteration 4.5: Codex app-server: `turn/start` and `turn/steer`
- **Goal:** The Codex path for background agents (Story 38) is confirmed or refuted on the current version.
- **What we do:**
  - A prototype client of the Codex app-server: it starts a thread, gives an idle thread a message with `turn/start` and adds one to a running turn with `turn/steer`; the agent's reply comes back to the client.
  - Its live check and the answers to the list of checks; the approval and sandbox settings under which a turn runs without a person.
  - Whether the app-server reaches a session a person works in.
  - Findings in the report.
  - Tests: against a stand-in that replays an exchange recorded from the real app-server, the client starts and steers turns and gets the reply; each waiting message goes into a `turn/start` or `turn/steer` once, with its sender label and `msg_id`; it is marked handed over only after the app-server accepts it and stays waiting when the app-server refuses it.
- **Definition of done:** the app-server live check, rerun, gives the result and Codex version the report records; the report answers every check of the list for the app-server; integration tests on the recorded exchange run in `script/test`.

#### Iteration 4.6: Subscription limit data
- **Goal:** It is known what subscription limit data Claude Code and Codex show the user and whether Skald may read it, for Story 61.
- **What we do:**
  - Per agent: where the CLI shows the user its limits, such as a command, the status line or the app-server, and what it shows: each limit, its usage and reset time.
  - Per source: whether a local program reads it without vendor tokens, credential files or switching accounts, as `features.md` §14 says; the vendor terms that allow or forbid it, with links.
  - For each source that exists and may be read:
    - a prototype reads the limits from it;
    - its live check follows the rules of 4.1 on the owner's signed-in CLI, without the stub and with neither flag;
    - the developer runs the check once for the report;
    - the check prints the CLI version and passes when the prototype reads each limit the CLI shows the user, with its usage and reset time.
  - Without such a source for an agent, the report says so and there is no prototype.
  - Findings in the report.
  - Tests, for each prototype: on outputs recorded from the real CLI, it gives each limit with its usage and reset time; an output without limits gives none.
- **Definition of done:** the report answers, per agent, whether the data exists and may be read, with its source and the vendor terms; for each source that may be read, its live check, rerun, gives the result and CLI version the report records, and unit tests on its recorded outputs run in `script/test`.

#### Iteration 4.7: Desktop apps and IDE extensions
- **Goal:** It is known which paths confirmed on the CLIs reach a live session in the Claude Code and Codex desktop apps and IDE extensions.
- **What we do:**
  - Surfaces: Claude Code in the Claude desktop app and in its VS Code extension; Codex in its desktop app and in its VS Code extension.
  - The report records how each surface reads project config. Codex in an app takes no `-c` arguments.
  - The owner prepares for GUI live checks:
    - installs the apps and signs in to them;
    - grants the terminal that runs Claude Code macOS Accessibility, Screen Recording, and Automation of System Events and of each app under test;
    - allows GUI runs in chat before the orchestrator starts a developer or reviewer who runs GUI live checks, here or in 4.8. The orchestrator states the allowance in the developer's or reviewer's prompt; without it, they do not drive the app.
  - A GUI live check follows the rules of 4.1, except that it runs on the real model under the owner's sign-in and the developer or reviewer drives the app:
    - code prepares the temporary project with the surface's config and the message source with a random token, and reads the surface version from the installed app or extension;
    - code does not show the token to the developer or reviewer until they report the token they read;
    - the developer or reviewer opens the directory in the app, types the prompt and reads the token from a screenshot of the reply of the Claude Code or Codex session under test, not from tool results or hook output; they use only `screencapture`, `cliclick` and `osascript`;
    - code then takes the token they read and prints the surface version and pass or fail: pass when it matches and the message was handed over;
    - the report holds each surface's steps, so a rerun repeats them;
    - the developer or reviewer captures only the app's window; before each click or keystroke they check that the app is frontmost, and before each click that the point is inside its window;
    - screenshots stay in the temporary directory; they are never committed or attached to a PR.
  - A GUI live check of each path that passed its CLI live check in 4.1–4.5, on each surface, and the answers to the list of checks of 4.1 for it there.
  - Findings in the report: per surface and path, the result with the reason, and what the surface needs beyond the CLI.
  - Tests: the temporary project holds, for each surface, the config the report records for it; the version reader gives each surface's version from recorded app and extension metadata; the check shows no token before the reported one and passes only when the reported token matches and the message was handed over.
- **Definition of done:** on each surface, the GUI live check of every path that passed its CLI live check, rerun by the reviewer, gives the result and surface version the report records; the report answers every check of the list of 4.1 for each such path on each surface; unit tests of the GUI live check's code run in `script/test`.

#### Iteration 4.8: Summary and the "Delivery to agents" table
- **Goal:** Stories 19, 20, 38 and 61 build on confirmed paths and data, and `architecture.md` shows the confirmed paths.
- **What we do:**
  - Every live check reruns with neither flag on the current CLI and surface versions; the report gives those versions and results, and the version each `--all` and `--real` result was taken on.
  - The report sums up, per agent and surface: the primary and fallback paths with their results and what each needs; refuted paths with the reason.
  - For Stories 19, 20 and 38: the paths and setup each takes, and what the daily e2e with real CLIs needs (`infrastructure.md`, "PR checks"); for Story 61: whether the limit data exists.
  - `architecture.md`, "Delivery to agents": the table matches the report and names the surfaces each path works on; refuted paths and "(to verify)" go; it links to the report.
  - `roadmap.md`: the lines on the Claude Code and Codex adapters ("MVP") and on Codex background agents ("Post-MVP") name the agents, not paths, and link to `architecture.md`, "Delivery to agents".
  - `features.md` §16: the `skald connect` step that sets up MCP and hooks sets up the paths of `architecture.md`, "Delivery to agents", and links there instead of naming them.
- **Definition of done:** each primary and fallback path the report names for an agent passes its live check, rerun, on every surface the report names it for, at the version the report gives; the table in `architecture.md` matches the report; the two `roadmap.md` lines and `features.md` §16 name no path; no other doc repeats the report.

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

#### Iteration 5.1: Prototype harness and file fingerprints
- **Goal:** Machines editing the same files of one repository learn it from a server that sees only HMAC fingerprints.
- **What we do:**
  - The prototype, in Rust on the stack of `tech-stack.md`, lives in `prototypes/` apart from the product crates, which never depend on it; `script/test` builds and tests it and `script/mutants` mutates it, as they do `prototypes/mls`.
  - A scenario harness: each machine is a local clone of one test repository with a shared upstream; it commits and edits its working tree through the `git` CLI.
  - A machine's edit set: the files changed since its merge base with the upstream's default branch, committed or not, untracked ones included; a rename counts both paths.
  - Each machine publishes HMAC fingerprints of its edit set's paths with the space key, a test fixture. Fingerprints are scoped to a repository; how clones of one repository get one scope is a finding.
  - A minimal in-process server stand-in, without the key, keeps each machine's latest fingerprints and tells each machine which other machines share fingerprints with it, and which.
  - The report in `docs/dev/` starts with the scenario list and the questions of `features.md` §11, "To decide", that the prototype answers; each iteration adds its findings.
  - Tests: two machines editing one file overlap; different files do not; a rename overlaps an edit of either of its paths; a deletion overlaps an edit of its path; a machine whose changes are merged upstream leaves the overlap once it pulls; the same path in another repository or under another key gives no overlap; nothing the server stand-in receives or keeps contains a path of the test repository.
- **Definition of done:** the scenarios pass; scenario tests run in `script/test`; the report lists the scenarios and the questions.

#### Iteration 5.2: Exact check with `git merge-tree`
- **Goal:** A client confirms each overlap the server reports with `git merge-tree`, and the server sees none of the changes.
- **What we do:**
  - On an overlap, the client gets the other machine's changes, committed and not, over an in-process stand-in for an end-to-end encrypted message that bypasses the server stand-in.
  - It runs `git merge-tree` on both sides and their merge base and gives the conflicting files and lines, or a clean merge.
  - The check works when the machines' bases differ, such as when one has pulled newer upstream commits.
  - `script/test` checks for the git version the check needs; `CONTRIBUTING.md` names it.
  - Findings in the report: what the check needs from the other machine and its size; whether a shared git remote could carry the changes instead.
  - Tests: edits to the same lines give a conflict with its files and lines; edits to distant lines of one file merge cleanly; modify/delete and rename conflicts are found; machines on different bases get the right result; nothing the server stand-in receives or keeps during the check contains a path or line of the exchanged changes.
- **Definition of done:** the scenarios pass; scenario tests run in `script/test`.

#### Iteration 5.3: Hunk fingerprints
- **Goal:** Edits to the same lines of a file overlap at hunk level; edits to different parts of it do not.
- **What we do:**
  - Each machine also publishes HMAC fingerprints of its changed hunks with the space key; the server stand-in compares them as it does paths.
  - Two machines' hunk fingerprints match where `git merge-tree` reports a conflict between them, also when unrelated commits shift the lines between their bases.
  - Findings in the report: the scheme, and its misses and false matches on the scenarios, such as on common lines.
  - Tests: edits to the same lines overlap at hunk level and conflict in `git merge-tree`; edits to different hunks of one file overlap at file level only; bases that differ by lines added above the hunk still match; nothing the server stand-in receives contains a line of the test repository.
- **Definition of done:** the scenarios pass; scenario tests run in `script/test`.

#### Iteration 5.4: Symbol fingerprints
- **Goal:** Edits to the same function overlap at symbol level, even when they merge cleanly; edits to different functions of one file do not.
- **What we do:**
  - Each machine also publishes, for each changed line, the HMAC fingerprint of the innermost function, method or type enclosing it, named with its enclosing symbols; the server stand-in compares them as it does paths.
  - At least three languages, Rust and TypeScript among them; a file in another language gets file and hunk fingerprints only.
  - Findings in the report: how symbols are found, what each language needs, and the misses and false matches on the scenarios.
  - Tests, in each supported language: edits to different functions of one file overlap at file level only; edits to distant lines of one function overlap at symbol level while `git merge-tree` merges them cleanly; edits to two methods of one type do not overlap at symbol level; a symbol moved within its file keeps its fingerprint; a file in another language gets no symbol fingerprints; nothing the server stand-in receives contains a symbol name.
- **Definition of done:** the scenarios pass in each supported language; scenario tests run in `script/test`.

#### Iteration 5.5: Measurement on real repositories
- **Goal:** It is known how often overlaps at each level match real conflicts on real repositories.
- **What we do:**
  - Repositories: public ones whose default branch keeps merge commits, in the languages the symbol fingerprints support; together they hold lockfiles, database migrations and checked-in generated code. The report names each with its URL and the range of merges measured.
  - One command clones them at fixed commits into a temporary directory and replays each merge as two machines: each parent's changes since their merge base. A rerun gives the same counts, lifetimes and sizes.
  - Per repository and level: warnings, `git merge-tree` conflicts, warnings without a conflict, conflicts without a warning.
  - For the reservation TTL: how long each side lived, from its first commit after the merge base to the merge.
  - For intents: whether both sides of a warning only add lines, or also change or delete them.
  - Cost: the time to compute a machine's fingerprints and their size at each level, on the largest sides. Times are recorded with the machine and not compared on a rerun.
  - The developer runs it for the report; the reviewer reruns it; `script/test` does not run it.
  - Findings in the report, with the machine and the git version.
  - Tests: on a fixture repository with known merges, the replay gives the expected counts at each level.
- **Definition of done:** the measurement, rerun by the reviewer, gives the counts, lifetimes and sizes the report records; tests on the fixture repository run in `script/test`.

#### Iteration 5.6: Noise from lockfiles, migrations and generated code
- **Goal:** It is known how much noise lockfiles, migrations and generated code add, and which handling of each removes it without hiding conflicts.
- **What we do:**
  - Each warning without a conflict and each conflict of 5.5 is classed by the files behind it: lockfile, migration, generated code or other. How each class is recognized, such as by name patterns, `.gitattributes` or generator markers, is a finding.
  - Ways to handle each class, such as leaving it out, comparing it at file level only, or warning about it apart; each way's noise and missed conflicts on the same merges.
  - It also counts the merges whose sides add migrations that clash without sharing a file, such as two with one sequence number.
  - The command of 5.5 prints the new numbers; findings in the report.
  - Tests: fixture files of each class are classed right; each way of handling gives the expected counts on the fixture repository.
- **Definition of done:** the measurement, rerun by the reviewer, gives the numbers the report records for each class and way; tests on the fixture repository run in `script/test`.

#### Iteration 5.7: Path normalization on macOS and Windows
- **Goal:** One file gets one fingerprint on every machine, whatever its OS and the spelling of its path.
- **What we do:**
  - Fingerprints take the path relative to the repository, in one Unicode normalization form and with `/` separators; hunks and symbols take content with one line ending.
  - Paths an agent's tools give, absolute, through a symlink or from another worktree, map to that form.
  - Whether to fold case: a case-insensitive file system holds one file where a case-sensitive one can hold two. The report gives the choice, with the paths of the measured repositories that collide once case is folded or Unicode normalized; the command of 5.5 prints them.
  - The command of 5.5 prints its numbers again with the new form; the report updates them.
  - macOS: a command checks git on this Mac's case-insensitive volume: a case-only rename, file names in NFC and NFD, `core.ignorecase` and `core.precomposeunicode`. The reviewer reruns it; `script/test` does not run it.
  - Windows: the same check, with backslashes, drive letters and CRLF under each `core.autocrlf` setting, runs in a CI job on `windows-latest` whenever the prototype changes, and on demand. Test inputs cover only what the runner cannot produce; the report says which.
  - `infrastructure.md`, "PR checks", names the Windows job as a check outside `script/test` and what starts it; "Branches" says whether it is required.
  - Findings in the report.
  - Tests: every spelling of one path gives one fingerprint; LF and CRLF versions of one edit give the same hunk and symbol fingerprints; machines with macOS, Windows and Linux spellings of one file overlap.
- **Definition of done:** the macOS check, rerun by the reviewer, and the Windows job, read in the PR's checks, give what the report records; the command of 5.5, rerun by the reviewer, gives the colliding paths and numbers the report records; unit and scenario tests run in `script/test`.

#### Iteration 5.8: Edits via Bash and generators
- **Goal:** It is known which data source catches the edits an agent makes via Bash and generators, and when.
- **What we do:**
  - Sources: the working-tree diff the earlier iterations use; hooks that see an agent's edit tool calls; declarations of the files an agent will edit, made before it edits.
  - Scripted sessions stand in for agents on clones of the measured repositories; no agent CLI runs. A session mixes edit tool writes, which hooks see, with shell commands: in-place edits, moves and deletions, formatters, package managers that rewrite a lockfile and code generators, all needing no account. The command pins each tool's version, refuses another and records it; package managers run offline from a cache the command fills at pinned versions in its own directory, never from the machine's shared caches.
  - Per source: the share of changed files it misses by kind of command, and when it learns of an edit: at the edit, at the next scan or never.
  - One command runs the sessions; the reviewer reruns it; `script/test` does not run it.
  - Findings in the report.
  - Tests: on a scripted session on a fixture repository, each source reports exactly the files it can see; the diff reports every changed file, the generator's included; a file changed and restored is not reported.
- **Definition of done:** the measurement, rerun by the reviewer, gives the numbers the report records; tests on the fixture repository run in `script/test`.

#### Iteration 5.9: What the server learns and the space key
- **Goal:** Story 44 knows what fingerprints show the server, and what a holder of the space key learns from them.
- **What we do:**
  - What the server stand-in learns without the key, on the measured merges: which machines edit the same thing, how much and when each edits, and how often each fingerprint recurs.
  - What a holder of the key learns: a fixed dictionary of common paths and symbol names, kept in the repository, run against the measured fingerprints, and the share it recovers.
  - One command prints both; the reviewer reruns it; `script/test` does not run it.
  - A removed member or a revoked device keeps the key: the report says when the key must change and what a change costs, such as fingerprints that stop matching across it.
  - Findings in the report.
  - Tests: with the key, the dictionary recovers a fixture path and symbol name; without it or with another key, it recovers none.
- **Definition of done:** the command, rerun by the reviewer, gives what the report records: what the server learns without the key and the shares a key holder recovers; unit tests run in `script/test`.

#### Iteration 5.10: Preliminary answers to "To decide"
- **Goal:** Stories 44 and 45 start from preliminary answers to `features.md` §11, "To decide", backed by the prototype's evidence.
- **What we do:**
  - The report answers each question of "To decide" with the evidence of the earlier iterations, or says what is still open and what would answer it.
  - For Story 44: what a client sends and what the server compares at file level; for Story 45: the hunk and symbol schemes with their measured noise and misses.
  - The commands of 5.5, 5.8 and 5.9 and the macOS check of 5.7 rerun on the final prototype, and the Windows job is started on this iteration's PR; the report's results are updated to them.
  - `features.md` §11, "To decide", links to the answers.
- **Definition of done:** the commands of 5.5, 5.8 and 5.9 and the macOS check of 5.7, rerun by the reviewer, and the Windows job's run on this PR give the results the report records, timings aside; the report answers or marks open each question of "To decide"; `features.md` §11 links to it; no other doc repeats the report.

### Story 6: Staging environment

**Goal:** Every merge into `dev` is deployed to staging automatically, so stories can be checked on a live server.

**Scope:**
- `skald-server` Docker image; on a merge into `dev`, CI builds the `:edge` image and publishes it to GHCR.
- Private `skald-cloud` repository with configs and deployment.
- Staging as in `infrastructure.md`, "Cloud"; attachments in a disk volume, the self-host default (`infrastructure.md`, "Self-host").
- **Done when:** a merge into `dev` shows up on staging within a few minutes; the health check responds over HTTPS.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `infrastructure.md` — "Flow", "Repositories", "Cloud", "Self-host", `tech-stack.md`

#### Iteration 6.1: `skald-server` Docker image
- **Goal:** `skald-server` runs from a Docker image, checked on every change that affects it.
- **What we do:**
  - A Docker image of `skald-server` built from the workspace: it holds the binary and only what it needs at run time, runs it as a non-root user, listens on all interfaces inside the container and takes its settings from the environment, as the binary does.
  - Its labels name the source repository and the commit it was built from.
  - `script/test` builds the image and tests it with PostgreSQL in a container; the CI path filters run this when the server, its dependencies or the image definition change; `infrastructure.md`, "PR checks", lists it.
  - Tests: a container from the image applies the migrations and answers the health check against PostgreSQL; it exits with an error when PostgreSQL is unreachable; its process does not run as root; the path filter tests cover the image's paths.
- **Definition of done:** `script/test` builds the image and its integration tests pass, locally and in CI; a PR that changes only docs or the web UI does not build the image.

#### Iteration 6.2: Publishing `:edge` to GHCR
- **Goal:** Every merge into `dev` publishes the image that staging runs.
- **What we do:**
  - On every push to `dev`, a workflow builds the image from that commit and pushes it to GHCR as `skald-server`, tagged `:edge` and with the commit.
  - When merges overlap, `:edge` never moves back to an older commit.
  - The build reuses the dependency build of earlier runs, so a merge reaches GHCR within a few minutes.
  - The workflow never runs for PRs; its token can write packages and nothing else.
  - The first publish runs on this iteration's merge; 6.5 checks it.
- **Definition of done:** the workflow runs only on pushes to `dev`, builds from the image definition that `script/test` tests, and its token has no write access besides packages.

#### Iteration 6.3: Preparing the move to Caddy on the staging VPS
- **Goal:** Everything the web server on the staging VPS does now has a reviewed replacement, a check and a rollback before anything on the VPS changes.
- **What we do:**
  - In 6.3–6.7:
    - the developer works on the VPS through the owner's SSH access;
    - `skald-cloud` changes go through a PR to its `main`, linked from the iteration PR; the iteration's reviewer reviews it too; the orchestrator merges it right after the iteration PR; the next iteration starts only once it is in `skald-cloud`'s `main`;
    - a command or runbook step that removes, stops, restarts, prunes or reconfigures anything on the VPS (containers, images, volumes, networks, Caddy configs, logs, Docker or system settings) touches only Skald's own: the staging stack, its Caddy site and the deploy account of 6.6. Only 6.4 goes beyond it, as far as the inventory and the runbook of 6.3 say, and so does the owner's firewall step of 6.7, as the runbook of 6.6 says. The tests of such a command show that the other projects' things stay.
  - In 6.3–6.8, the iteration PR, its commits and its reviews name nothing taken from the VPS or the inventory: no other project, site, port or tool, no connection detail of 6.7, and none of the owner's decisions. They link to the `skald-cloud` PR; output and findings that would name such things, such as the site check's or a port scan's, go only there. In an iteration without a `skald-cloud` PR, the review states its findings without them.
  - This iteration only reads from the VPS; it changes nothing there.
  - `skald-cloud` gets a `script/test` that runs its tests (`rules.md`, rule 2).
  - An inventory in `skald-cloud` of what the web server that runs on the VPS now (the old web server) does:
    - every port and protocol it listens on, plain HTTP, other ports and TCP or UDP proxies included;
    - each site: its domains, what it serves (proxies, WebSocket, static files, redirects, access rules, limits) and its certificates;
    - what serves each entry after the move: Caddy, or, for what Caddy does not serve, a proposal;
    - every tool that changes or reloads the old web server, wherever it runs, such as certificate renewal with its hooks or another project's deploy: whether it moves to Caddy or is stopped, as a proposal for a tool of another project. The developer finds them by searching the VPS's timers, cron, certificate renewal hooks, sudo rules, SSH forced commands, and the scripts that touch the old web server's config or service.
  - A site check in `skald-cloud`: for each entry of the inventory, it compares the responses on each of its ports, port 80 included, with a baseline recorded through the old web server, and checks that each certificate is valid for the site's domains.
  - Caddy configs for every entry Caddy serves, each site in its own config, so changing one leaves the others alone; Caddy validates them.
  - The runbook in `skald-cloud`:
    - the switch: Caddy holds a valid certificate for every site before the old web server stops; the switch interrupts the sites only for the restart; the old web server is stopped and disabled, its config and certificates kept;
    - the rollback: it renews the kept certificates, then switches back to the old web server.
  - Tests, against local stand-in sites: the site check passes when the responses match the baseline and fails, naming the entry, on a different status or redirect target, a port that does not answer, or a certificate not valid for the site's domains.
- **Definition of done:** comparing the inventory with the old web server's loaded config and listening sockets, the reviewer finds no entry missing; repeating the search for tools on the VPS, the reviewer finds no tool missing; every entry and tool of the inventory says what happens to it after the move; the site check, rerun by the reviewer, passes against the baseline; Caddy validates the configs; the site check's tests pass in `skald-cloud`'s `script/test`; the iteration PR and its commits name nothing taken from the VPS or the inventory.

#### Iteration 6.4: Switching the staging VPS to Caddy
- **Goal:** One Caddy serves every site on the staging VPS, and the other projects' sites keep working through and after the move.
- **What we do:**
  - Before the iteration starts, the owner:
    - decides, from the proposals in the inventory of 6.3, what serves each entry Caddy does not serve and what happens to each tool of another project; for a tool that runs outside the VPS, the decision names who changes it and when, before or after the switch;
    - allows the switch in chat.
  - The orchestrator puts the owner's decisions and the allowance in the developer's and the reviewer's prompts; the developer records the decisions in the inventory.
  - The owner makes each change in another project, at the time its decision names.
  - Caddy runs on the VPS with the configs of 6.3, apart from any project's stack, so no project's deploy stops it; its logs rotate with a size cap, so they cannot fill the VPS's disk.
  - Each entry and tool of the inventory moves or stops as it says.
  - The switch follows the runbook of 6.3, with the site check running through it.
  - The developer rehearses the rollback once: to the old web server and back to Caddy, with the site check after each step.
  - Fixes to the inventory, the configs or the runbook found on the way go to `skald-cloud`.
- **Definition of done:** the inventory records exactly the owner's decisions from the reviewer's prompt; only Caddy serves ports 80 and 443 on the VPS, and every other entry of the inventory is served as it says; the old web server is stopped and disabled, its config and certificates kept; the reviewer finds the size cap on Caddy's logs, within what the VPS's disk has free; each tool of the inventory works with Caddy or is stopped, as it says; the site check, rerun by the reviewer, passes for every entry; its output in the `skald-cloud` PR, through the switch and after each step of the rehearsed rollback, shows no entry down beyond a restart and the kept certificates renewed before the switch back; the iteration PR and its commits name nothing taken from the VPS or the inventory.

#### Iteration 6.5: Staging stack at `dev.skald.so`
- **Goal:** Staging runs `skald-server` from the `:edge` image behind Caddy, deployed by one command.
- **What we do:**
  - Before the iteration starts, the owner makes the `skald-server` package in GHCR public; the merge of 6.2 has published it.
  - The staging stack in `skald-cloud`: `skald-server` from an `:edge` digest; PostgreSQL 19 with its data in a persistent volume; a persistent disk volume for attachments, mounted into the server container for Story 41.
  - A Caddy site for `dev.skald.so`, in its own config, passes HTTPS and WebSocket to the server; adding it interrupts no other site.
  - Only Caddy reaches the server, and only the server reaches PostgreSQL; the server container runs without privileges or host access beyond its volumes.
  - The stack's secrets live only on the VPS; `skald-cloud` names them without values.
  - The stack cannot use up the shared VPS: its containers have memory and CPU limits, their logs rotate with a size cap, and the database and attachment volumes cannot grow past a set size. These limits are set on the stack's own containers and volumes, not VPS-wide, and fit in what the VPS has free next to the other projects.
  - A deploy command on the VPS takes an image digest, pulls it and restarts the server with it. It passes only when the running server container has that digest and the health check over `https://dev.skald.so` answers ok.
  - After every deploy, passed or failed, the deploy command keeps only the server image it runs and the last one that passed. It leaves every image that is not a server image in place, whether a container runs it or not, and every container, volume and network that is not the stack's.
  - The deploy command prints only its steps, the digest and the health check result, never the stack's settings or container logs.
  - The runbook in `skald-cloud`: bringing the stack up, applying a config change, deploying and rolling back a digest, logs, where the secrets are; the stack's limits and the VPS's free memory and disk they were chosen against.
  - `infrastructure.md`, "Repositories": `skald-cloud` also holds the staging configs.
  - Tests, with stand-ins for Docker and the HTTPS check: the deploy command passes when the requested digest runs and the health check answers ok, and fails on a failed pull, another running digest or a failed health check; after a passed and after a failed deploy, only the server image it runs and the last one that passed stay; every image that is not a server image stays, whether a container runs it or not, and so does every container, volume and network that is not the stack's; the output of a failing deploy contains none of the stack's secret values.
- **Definition of done:** `https://dev.skald.so/health` answers ok with a valid certificate; the deploy command, rerun by the reviewer with the current `:edge` digest, passes, and the image's labels name a `dev` commit; the database and the attachment volume keep their data across that redeploy; after it, only the server image it runs and the last one that passed stay, and the other projects' containers, images, volumes, networks and Caddy sites are as before it; on the stack's running containers and volumes, the reviewer finds the memory and CPU limits, the log size cap and the volume size limits, set on them alone and within what the VPS has free next to the other projects; PostgreSQL and the server's port are closed from outside; the site check of 6.3 passes; the deploy command's tests pass in `skald-cloud`'s `script/test`; the iteration PR and its commits name nothing taken from the VPS or the inventory.

#### Iteration 6.6: Deploy access to the staging VPS
- **Goal:** CI can deploy staging and nothing else on the shared VPS.
- **What we do:**
  - A deploy account on the VPS whose SSH key runs only the deploy command of 6.5 with one digest of the `skald-server` image: no shell, port forwarding or other command, and no access to other projects' files.
  - Any other input is refused before anything runs.
  - The runbook in `skald-cloud`: how the owner creates the key pair and installs the public key with this restriction; rotating and revoking the key; what the VPS's firewall must let in from GitHub-hosted runners; the GitHub environment of 6.7 and its secrets by name, with the value each holds: the private key, the host key CI pins, and the SSH destination with the account, the address and the port in one value.
  - No secret of that environment is a value that GitHub's masking could find in the commit, the digest or the health check result of the deploy log: a short value, such as the port or the account, is never a secret on its own.
  - Tests, with a stand-in for the deploy command: one well-formed digest of the `skald-server` image reaches it; other commands, extra arguments, other images and malformed digests are refused without running it.
- **Definition of done:** run as the deploy account the way SSH runs the key's command, it deploys the current `:edge` digest and refuses every other input, checked by the reviewer through the owner's SSH access; the runbook's install steps give the key exactly this restriction, and its secrets follow the masking rule above; the tests pass in `skald-cloud`'s `script/test`; the iteration PR and its commits name nothing taken from the VPS or the inventory.

#### Iteration 6.7: Deploying every merge into `dev`
- **Goal:** Each `:edge` image CI publishes reaches staging without a person.
- **What we do:**
  - Before the iteration starts, the owner:
    - creates the deploy key pair and installs the public key on the VPS as the runbook of 6.6 says;
    - in `skald-so/skald`, creates the GitHub environment and its secrets as the runbook says; only `dev` may deploy from it;
    - opens the VPS's firewall to GitHub-hosted runners, if the runbook says it is needed.
  - After the workflow of 6.2 publishes `:edge`, a deploy job in that environment connects to the VPS with the secrets the runbook names, checks the pinned host key and deploys the published digest; it fails when the deploy fails.
  - Deploys run one at a time; when merges overlap, staging never moves back to an older commit.
  - The job's public log shows the commit and digest deployed and the health check result, and none of the environment's secrets.
  - `infrastructure.md`, "Cloud": the staging deploy runs in this repository's CI, in an environment only `dev` deploys from.
  - The first deploy runs on this iteration's merge; 6.8 checks it and its log.
- **Definition of done:** the installed key carries the restriction of 6.6; the environment admits only `dev`, its secrets are exactly those the runbook names, and no other secret of the repository holds the key; only the deploy job uses the environment; it takes the connection details only from those secrets and pins the host key; the workflow names no connection detail; the iteration PR and its commits name nothing taken from the VPS or the inventory.

#### Iteration 6.8: Staging status check
- **Goal:** Anyone can tell whether staging runs the latest merge into `dev`, and a merge is shown to reach staging within a few minutes.
- **What we do:**
  - A command reports, for the latest commit on `dev`: its merge time; the latest staging deploy in CI, with its commit, result and how long after the merge it finished; the health check over `https://dev.skald.so`.
  - It passes only when the latest staging deploy is of that commit and succeeded, and the health check answers ok. Otherwise it fails, naming the cause: the publish or the deploy failed, has not run or is still running, or the health check fails.
  - It uses only public data: the CI runs of `skald-so/skald` and the health check.
  - `infrastructure.md`, "Cloud": the staging line names the command.
  - Tests, on recorded GitHub API responses and a local stand-in for the health check: it passes for a succeeded deploy of the latest commit with the health check ok, and fails, naming the cause, in each other case above.
- **Definition of done:** the command, run by the reviewer, passes for the latest commit on `dev` and shows that it reached staging within a few minutes of its merge; the log of the deploy run it reports shows the commit, the digest and the health check result in full, none of them masked, and the reviewer finds no secret in it, the connection details of 6.7 included, and states this without naming any; its unit tests run in `script/test`; the iteration PR and its commits name nothing taken from the VPS or the inventory.

### Story 7: Release pipeline

**Goal:** A release ships with one action, merging the Release PR, and the artifacts are signed and verifiable.

**Scope:**
- The release flow of `infrastructure.md`, "Flow" and "Release": release-plz, dist, `vX.Y.Z` tags, betas, `main` moving to the tag.
- The channels of `infrastructure.md`, "Channels", except those marked "Later".
- Signed and verifiable artifacts as in `infrastructure.md`, "Trust in builds".
- The `skald update` command.
- **Done when:** merging the Release PR publishes the release to all channels; `gh attestation verify` passes; `skald update` installs the new version.

- **Release:** MVP
- **Depends on:** Story 1
- **Docs:** `infrastructure.md` — "Flow", "Release", "Channels", "Trust in builds", `tech-stack.md`

### Story 62: Web UI kit: theme, cabinet shell and components

**Goal:** Web UI screens are assembled from ready parts that match the approved design, so screen stories only connect the API.

**Scope:**
- Look per the approved design in `../skald-cloud/design/`: tokens, fonts and rules in its `README.md`.
- Theme: light and dark tokens that follow the system setting; fonts.
- The cabinet shell with the side menu, space switcher and account footer. The operator console shell comes with Story 48.
- Components: a table sorted by a click on a column header, with an arrow on the active column; filter buttons; a dropdown filter; search; a warning banner; buttons; status labels; empty and loading states.
- A kit page shows every component in every state with fixture data, without API calls.
- Reference screenshots of the kit page in the repository; `script/test` compares the kit page with them.
- Desktop and phone width; usable from the keyboard and with a screen reader.
- **Done when:** the kit page matches the approved design in both themes at desktop and phone width and matches its reference screenshots; component tests and the screenshot comparison run in `script/test`.

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

#### Iteration 62.6: Reference screenshots of the kit page
- **Goal:** Every visual change to the kit shows up in its PR as changed reference screenshots, and a change without them fails `script/test`.
- **What we do:**
  - Reference screenshots of every kit page view in both themes at both check widths, made by the screenshot command and committed in `web/`.
  - `script/test` takes the kit page screenshots in headless Chrome and compares them with the references; a difference beyond anti-aliasing fails the run and saves a diff image.
  - A command updates the references.
  - `web/README.md`: how to update the references; a reviewer compares changed references with the design in `skald-cloud`.
  - Tests: an unchanged kit page passes; a changed color or spacing of a component fails with a diff image.
- **Definition of done:** references of every view from 62.1–62.5 are in the repository; `script/test` fails on a visual change without updated references and passes with them, on a developer machine and in CI.

#### Iteration 62.7: Cabinet shell
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
- **Definition of done:** the kit page shows the cabinet as in the approved design in both themes at desktop and phone width; the PR adds the reference screenshots of these views; component tests and the screenshot comparison pass in `script/test`.

### Story 64: Self-host: Docker Compose

**Goal:** A self-hosted server comes up with one command, following the docs.

**Scope:**
- Self-host as in `infrastructure.md`, "Self-host": one `docker compose up`, the domain in a single variable, HTTPS right away.
- Sign-in on a self-hosted server: GitHub or Google with the admin's own OAuth apps; corporate IdPs come with Story 40.
- Attachments in a disk volume by default, optionally in an external S3-compatible bucket (`infrastructure.md`, "Self-host").
- Backups and upgrades with documented commands.
- Self-host documentation.
- **Done when:** following the docs, a working Skald with its own domain comes up on a clean server, with attachments on its disk or in an external bucket; an upgrade keeps the data.

- **Release:** MVP
- **Depends on:** Story 6, Story 7, Story 9
- **Docs:** `infrastructure.md` — "Self-host", "Repositories", `features.md` — "Product", `tech-stack.md`

### Story 8: GitHub sign-in and device certificate

**Goal:** The user signs in with GitHub, and the machine gets its own keys and a device certificate.

**Scope:**
- Sign-in via GitHub OAuth; device flow (RFC 8628) for machines without a browser. The account is created on first sign-in.
- Device keys (signing and HPKE) are generated on the machine and never leave it; key storage as in `architecture.md`, "Cryptography".
- The first device creates the account master key, which signs its certificate, as the spec defines (iteration 2.3); a server signature alone never admits a device (`architecture.md`, "Device linking"). Signing by other devices and the master key's copy under the recovery key come with Story 17.
- Client local state in SQLite.
- CLI: sign in, sign out, list your devices.
- **Done when:** on a machine without a browser, `skald` signs in via device flow and gets a certificate signed by the account master key; a certificate signed only by the server is rejected; the server stores only public keys.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 1, Story 2
- **Docs:** `features.md` §5, `architecture.md` — "Device linking", "Data model", "Cryptography", `docs/protocol/README.md`, `docs/protocol/membership.md`

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
- History keys: encrypting message copies, wrapping a key to browser keys and the recovery key, the key chain (`docs/protocol/membership.md`, "History").
- Stateless HPKE (RFC 9180), Ed25519 signatures.
- **Done when:** tests confirm forward secrecy and that a removed member cannot access new messages; the crate builds for WASM.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 2, Story 3
- **Docs:** `architecture.md` — "Cryptography", `features.md` §4, §6, `tech-stack.md`, `docs/protocol/membership.md`

### Story 11: Spaces and agent addresses

**Goal:** A device belongs to several spaces, and an agent session automatically works in the space of its folder.

**Scope:**
- Spaces and channels on the server; a device in N spaces.
- Creating a team space and its channels in the CLI; the creator is the space's first admin.
- The signed roster and space policy (`architecture.md`, "Membership"): an admin's device signs every change.
- An agent is a named endpoint on a device, with an address as in the protocol spec.
- Binding a session to a space by folder (`features.md` §3).
- Isolation: an agent of one space cannot see other spaces.
- A personal space for each account at sign-up; its machines join it by default.
- In the cloud, an account owns up to the Free number of spaces (`features.md`, "Product"); a self-hosted server applies no plan limits.
- **Done when:** a new account has a personal space with its machines; its third own space is refused; a device is in three spaces; an agent in `~/work/clientA` sees only clientA; a roster change without an admin's signature is rejected.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 8
- **Docs:** `features.md` §3, §15, `architecture.md` — "Membership", "Network", "Data model", `docs/protocol/README.md`, `docs/protocol/membership.md`

### Story 12: Encrypted mailbox

**Goal:** A message reaches the recipient device even if the sender and recipient were never online at the same time; the server stores only ciphertext.

**Scope:**
- Envelopes as in the protocol spec; a mailbox per recipient device.
- Delivery from the sending device to the recipient device as in `docs/protocol/delivery.md`: send with retries, WebSocket push, catch-up from the cursor, ack after the local write; duplicates dropped by the sending device and `msg_id`.
- Server instances wake each other (`architecture.md`, "Network").
- Server statuses `stored`, `delivered` and `expired`. `read` and `replied` come with Story 15, `held` and `refused` with Story 21.
- Cloud limits (`features.md`, "Product") as settings per space and per member, at the Team values until billing (Story 68): devices, messages per month, message text, undelivered message retention.
- A self-hosted server applies no plan limits.
- CLI: send and receive a message.
- **Done when:** A sends and goes offline, B connects 10 hours later and receives the message; a retry with the same `msg_id` does not create a duplicate; a connection dropped before the ack neither loses nor duplicates the message; a message not delivered in time ends in `expired`; a message over the monthly message limit gets `quota-exceeded` and one over the message text limit `too-large`; a self-hosted server applies no plan limits.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 11
- **Docs:** `features.md` §1, "Product", `architecture.md` — "Network", "Data model", "Security", `docs/protocol/delivery.md`

### Story 13: MLS channels, roles and device revocation

**Goal:** All channel messages are MLS-encrypted between devices, and an admin can revoke a specific device.

**Scope:**
- The server as the MLS Delivery Service: stores KeyPackages, orders Commits, delivers handshake messages through mailboxes.
- Creating a channel makes its MLS group; a member is a device; addressing to the right session happens inside the ciphertext.
- No plaintext path: sending to a recipient without keys fails (`features.md` §1).
- Roles: admin, member, service agent; only an admin removes members; role changes are signed (`architecture.md`, "Membership").
- Device revocation is Remove + Commit, a new epoch.
- A member's new device joins its channels, and a device that missed Commits rejoins them, as the spec defines (iteration 2.3).
- Gap detection with the per-sender counter (`docs/protocol/delivery.md`): the recipient tells its user about a missing message.
- **Done when:** an automated test shows that a removed device with a full copy of its disk cannot read any message sent after the removal; a message the server drops is reported missing once a later message from the same device arrives; a device offline past the mailbox retention rejoins its channels.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 10, Story 12
- **Docs:** `features.md` §4, `architecture.md` — "Cryptography", "Data model", "Membership", `docs/protocol/delivery.md`, `docs/protocol/membership.md`

### Story 14: Client: one-shot mode and helper

**Goal:** The client works without a daemon or root: one-shot in CI and sandboxes, and through an automatic helper on a developer machine.

**Scope:**
- One-shot mode and the helper as in `architecture.md`, "Components"; the helper holds the local state lock.
- Network as in `architecture.md`, "Network": outbound HTTPS on 443 only, `HTTPS_PROXY`, SSE and long-poll fallbacks.
- The device-to-agent hop of `docs/protocol/delivery.md`: a crash during a handover repeats it and never loses it.
- `skald recv --wait` for any agent with a shell.
- **Done when:** two sessions on one machine receive their own messages through a single helper; one-shot mode sends and receives behind a proxy; a helper killed during a handover hands the message over again after a restart.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 12, Story 13
- **Docs:** `features.md` §2, `architecture.md` — "Components", "Network", "Delivery to agents", `docs/protocol/delivery.md`

### Story 15: MCP server and agent commands

**Goal:** Any agent with MCP or a shell sends and reads messages on behalf of its session.

**Scope:**
- `skald` as an MCP server (stdio): sending and `inbox`; the same operations in the CLI.
- A session registers as an agent in the space of its folder.
- Receipts `read` and `replied` (`docs/protocol/delivery.md`).
- Inbound content is data, not an instruction: a sender label and a prompt injection warning (`architecture.md`, "Security").
- **Done when:** two agents in different folders of one space exchange messages via MCP tools; the sender sees `read` once the recipient agent has the message and `replied` when its answer arrives.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 12, Story 13, Story 14
- **Docs:** `features.md` §7, `architecture.md` — "Components", "Security", `docs/protocol/delivery.md`, `tech-stack.md`

### Story 16: Encrypted channel history

**Goal:** The server stores the full channel history, and only devices with the history key can read it.

**Scope:**
- History copies under the channel's history key, keys wrapped to linked browsers and the recovery key, the key chain (`docs/protocol/membership.md`, "History").
- Rotation when any device is revoked and on the space policy's schedule; an admin changes the period.
- A new channel member sees the history only if an admin allowed it.
- Reading and searching the history happens on the client (CLI).
- History is kept for the limit of the space's plan (`features.md`, "Product"); retention policies are Enterprise (Story 51).
- **Done when:** a member's new device reads the channel history, including the history from before a rotation; a revoked device with a full copy of its disk reads no history written after its revocation.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13
- **Docs:** `features.md` §4, §6, "Product", `architecture.md` — "Cryptography", "Data model", "Membership", `docs/protocol/membership.md`

### Story 17: Recovery key and new device linking

**Goal:** The user links new devices and does not lose the history, even if all devices are wiped.

**Scope:**
- The recovery key is generated on the machine on first connect and shown to the user; the account master key and the history keys are encrypted to it (`architecture.md`, "Device linking").
- The recovery key is entered only in the CLI (`architecture.md`, "Device linking").
- A new device is confirmed on an already linked device or with the recovery key, which signs its certificate. It then joins the account's channels and gets the history keys as the spec defines (iteration 2.3), also when the recovery key replaced every device.
- **Done when:** all devices are wiped; the recovery key is entered on a new laptop, and the full history is there; its agents exchange messages in the old channels again.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 16
- **Docs:** `features.md` §6, §16, `architecture.md` — "Device linking", "Cryptography", `docs/protocol/membership.md`

### Story 18: Key verification without trusting the server

**Goal:** The client detects a key swap even if the server is compromised.

**Scope:**
- Verification without trusting the server, as in `features.md` §5: the device key signed with the user's SSH key from GitHub and checked against GitHub; cross-signing, key change warnings, fingerprints; the optional DNS root of trust.
- Accounts without a GitHub SSH key rely on cross-signing, key change warnings and fingerprints.
- **Done when:** the client detects a key swap on the server (automated test).

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 17
- **Docs:** `features.md` §5, `architecture.md` — "Device linking", `docs/protocol/membership.md`

### Story 19: Claude Code adapter and plugin

**Goal:** A message reaches a live Claude Code session; the adapter installs as a plugin.

**Scope:**
- The Claude Code paths of `architecture.md`, "Delivery to agents", as Story 4 confirmed them.
- A skill with rules for working with Skald.
- A Claude Code plugin with MCP, hooks and the skill in the monorepo marketplace (`infrastructure.md`, "Channels"), auto-update from the marketplace.
- e2e tests against real Claude Code, nightly run in CI.
- **Done when:** a message sent to an agent appears in the context of a running Claude Code session.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 4, Story 14, Story 15
- **Docs:** `features.md` §7, §16, `architecture.md` — "Components", "Delivery to agents", `infrastructure.md` — "Channels", "Release", "PR checks"

### Story 20: Codex adapter

**Goal:** A message reaches a live Codex session without tmux.

**Scope:**
- The Codex paths of `architecture.md`, "Delivery to agents", except app-server (Story 38), as Story 4 confirmed them.
- e2e tests against real Codex, nightly run in CI.
- **Done when:** a message sent to an agent appears in the context of a running Codex session.

- **Release:** MVP
- **Acceptance tests:** first
- **Manual check:** Claude Code and Codex on two machines exchange encrypted messages through the CLI and MCP.
- **Depends on:** Story 4, Story 14, Story 15, Story 17
- **Docs:** `features.md` §7, `architecture.md` — "Delivery to agents", `infrastructure.md` — "PR checks"

### Story 21: Presence, statuses and loop protection

**Goal:** Core features for parity with competitors: you can see who is online and where a message is; agents do not loop; a human controls inbound messages.

**Scope:**
- Presence directory: the agent status is encrypted; the server stores only the latest value.
- `message_status` in MCP and the CLI.
- Loop protection as in `architecture.md`, "Security".
- Inbound modes `accept / hold / refuse`, a sender allowlist; `hold` waits for human confirmation in the CLI; receipts `held` and `refused` (`docs/protocol/delivery.md`).
- **Done when:** two agents replying to each other stop at the limit; a message in `hold` mode does not reach the agent's context until confirmed; its sender sees `held`, then `read` or `refused`.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 15
- **Docs:** `features.md` §8, `architecture.md` — "Data model", "Security", `docs/protocol/delivery.md`, `docs/protocol/agents.md`

### Story 22: Web UI: non-secret sections

**Goal:** On `app.skald.so`, the user signs up and manages the account without opening any content.

**Scope:**
- Static SPA on `app.skald.so`; one-click sign-up with GitHub or Google.
- Layout per the approved design in `../skald-cloud/design/`.
- Sections: each space's plan and limits and the spaces the account owns (`features.md`, "Product"), devices, sessions, space members and roles.
- Sessions: web UI and CLI sign-ins with metadata (where, when, last use) and revocation.
- Members and roles are only shown: they change on an admin's device (`features.md` §12).
- Data comes only from the server API; the page receives neither keys nor content.
- **Done when:** a new user signs up in one click and sees each space's plan and limits, their devices and space members; a revoked session loses access at once.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 8, Story 9, Story 11, Story 12, Story 13, Story 62
- **Docs:** `features.md` §12, §16, `architecture.md` — "Components", "Membership", `tech-stack.md`

### Story 23: Space invite by link

**Goal:** An admin invites a person by link, and the newcomer's devices join the channels automatically.

**Scope:**
- An admin creates an invite link in the CLI; it is signed, and its secret never reaches the server (`architecture.md`, "Membership").
- The newcomer accepts after sign-in; the admin's device checks the newcomer's devices through their certificate chains, adds the newcomer to the roster and its devices to the space's channels by itself.
- Active invite links are shown in the web UI and can be revoked there.
- Invites never reach the agent's context.
- **Done when:** a user from another account joins the space via the link, and their agents exchange messages with the admin's agents; the server never receives the invite secret; a member or device the server adds without an admin's signature or a valid certificate chain is rejected.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 14, Story 17, Story 22
- **Docs:** `features.md` §16, §12, `architecture.md` — "Membership", "Security", `docs/protocol/membership.md`

### Story 24: One-command machine connect

**Goal:** One command per machine and one confirmation for each additional one; after that, the agents on the machine are ready to work.

**Scope:**
- The "Connect machines" section with the command and the one-time code of `features.md` §16.
- `install.sh` and `install.ps1` in GitHub Releases: download the binary, verify the checksum, run `skald connect`.
- `skald connect` does every step of `features.md` §16, including signing the device key with the user's SSH key from GitHub (Story 18).
- The next machine: the same command, plus confirmation on a linked device or with the recovery key.
- The status on the website updates by itself.
- **Done when:** one-click sign-up, one command per machine; after that, Claude Code and Codex on two machines exchange messages; for an account with an SSH key on GitHub, the device key is signed with it; each agent of `agents.md`, "Setup by `skald connect`", found on the machine has the Skald MCP server in every project, in its own user config or read from another agent's user config, with its other entries kept.

- **Release:** MVP
- **Acceptance tests:** first
- **Manual check:** the new-user path on staging: sign-up, one-command connect on macOS, Linux and Windows, invite by link.
- **Depends on:** Story 6, Story 7, Story 14, Story 17, Story 18, Story 19, Story 20, Story 22, Story 23
- **Docs:** `features.md` §16, `agents.md` — "Setup by `skald connect`", `architecture.md` — "Device linking", `infrastructure.md` — "Channels", `docs/protocol/membership.md`

### Story 25: Production cloud, website and monitoring

**Goal:** The cloud is ready for the public MVP launch.

**Scope:**
- Production as in `infrastructure.md`, "Cloud": a primary and a replica with synchronous replication, WAL-G backups, the image published for self-hosting, rollout with manual approval, monitoring, the addresses.
- An S3-compatible bucket for attachments (`tech-stack.md`, "Files").
- The website and docs built from `docs/` on GitHub Pages (`infrastructure.md`, "Channels").
- Abuse protection at the public launch: quotas, rate limits and manual bans in the operator CLI (Story 34); automatic abuse detection comes with Story 47.
- **Done when:** a release is deployed to production with one click; restore from backup has been tested; losing the primary loses no `stored` message; monitoring alerts arrive.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 6, Story 7, Story 12
- **Docs:** `infrastructure.md` — "Cloud", "Channels", `tech-stack.md`

### Story 26: Task queue: claim, lease and DLQ

**Goal:** A queue in which a task is never lost and never runs on two agents at once.

**Scope:**
- Named queues and tasks as in `features.md` §10 and `docs/protocol/queues.md`: an encrypted payload with plaintext scheduling fields, `idempotency_key`, atomic claim, lease with heartbeat, fencing and `claim-lost`, both modes, retries with exponential backoff, DLQ.
- The result returns to the sender as an encrypted message.
- A queue belongs to a channel; any member device of the channel can claim its tasks (`docs/protocol/queues.md`).
- CLI to enqueue, claim, extend and complete; the helper sends the heartbeat, in one-shot mode the process running the task.
- Queue load test.
- **Done when:** a stopped agent gets `claim-lost`; a live but slow agent does not lose its task; failed tasks go to the DLQ; an at-most-once task never runs twice; a repeated enqueue with the same `idempotency_key` is rejected; a device outside the queue's channel cannot claim or read a task.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 14, Story 15
- **Docs:** `features.md` §10, `architecture.md` — "Queue", `docs/protocol/queues.md`

### Story 27: Browser linking and history on the web

**Goal:** The browser becomes a linked device and shows content, decrypting it locally.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- The "Link this browser" button: a code or QR from a linked device, never the recovery key (`architecture.md`, "Device linking"); the browser key is in IndexedDB and can be revoked.
- A WASM build of the Rust core in the SPA; the browser reads through history keys wrapped to its key and sends actions as signed messages via HPKE.
- Revoking a browser rotates the history key (`architecture.md`, "Cryptography").
- Sections: message texts and threads, a "which agent messages whom" graph, history search in the browser.
- Admin actions signed in the browser: creating invites, making a member an admin, removing a member; the MLS changes follow as the spec defines (iteration 2.3).
- A reproducible web build (`infrastructure.md`, "Trust in builds").
- **Done when:** a linked browser shows the channel history, and the server receives only ciphertext; a revoked browser reads nothing written after its revocation, history included; an admin creates an invite and removes a member from a linked browser; a rebuild from the release tag matches the files `app.skald.so` serves.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 10, Story 17, Story 22, Story 23
- **Docs:** `features.md` §12, §6, `architecture.md` — "Components", "Device linking", "Membership", "Cryptography", `infrastructure.md` — "Trust in builds", `docs/protocol/membership.md`

### Story 28: Web: queues, delivery trace, loops, ask_user

**Goal:** The browser shows what happens to tasks and messages, lets you control it and answer agents.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- The queue panel, the delivery trace and loop analysis of `features.md` §12.
- Answering `ask_user` in a linked browser, as a signed action.
- **Done when:** a task from the DLQ is restarted from the browser; a looping thread is paused; an `ask_user` answered in the browser reaches the agent.

- **Release:** MVP
- **Depends on:** Story 21, Story 26, Story 27, Story 41
- **Docs:** `features.md` §12, `architecture.md` — "Device linking", `docs/protocol/agents.md`

### Story 35: Service agents: HPKE and scoped tokens

**Goal:** A service agent (`ci/deploy-bot`) without a persistent disk exchanges end-to-end encrypted messages using a scoped token.

**Scope:**
- A service agent as in `features.md` §9: its own key in an environment secret, a scoped token, token and key rotation.
- An admin creates a service agent and its token in the CLI and signs its grant (`docs/protocol/README.md`, "Service grant"); the token value is shown once.
- Stateless HPKE for ephemeral agents; a long-lived service agent with persistent state is an MLS member like a device (`architecture.md`, "Cryptography").
- How a service agent without a persistent disk pins its account's master key and the roster revision it starts from between runs: added to `docs/protocol/membership.md`.
- What the server takes from the token alone, without proof of the device key: any request it authenticates as the device, such as mailbox reads and acks, Commits, GroupInfo, KeyPackages, links, wraps, queue key objects and history entries. Each rule goes into the spec next to the server check it changes; a proof-of-key requirement starts in `api/openapi.json`; `docs/threat-model.md`, "A leaked CI secret", lists what the token alone can do.
- The "Service agents and tokens" web UI section: permissions, expiry, last use, revocation.
- The number of service agents is capped by the space's limit (`features.md`, "Product").
- **Done when:** messages are exchanged with an agent in one-shot mode without a persistent disk, with only ciphertext on the server; a token used outside its permissions or after expiry is rejected; content from a service agent without a valid grant is rejected.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 14, Story 16, Story 22
- **Docs:** `features.md` §9, "Product", `architecture.md` — "Data model", "Cryptography", "Membership", `docs/protocol/README.md`, `docs/protocol/membership.md`, `docs/protocol/delivery.md`, `docs/protocol/queues.md`, `docs/threat-model.md`

### Story 41: ask_user and attachments

**Goal:** An agent asks a human and sends small files.

**Scope:**
- `ask_user`: the agent asks a person and waits for the answer; the person answers in the CLI. Answering in a linked browser comes with Story 28.
- Encrypted attachments within the cloud limit (`features.md`, "Product"), kept in the attachment storage (`tech-stack.md`, "Files").
- **Done when:** the agent gets a human's answer to `ask_user`; an attachment reaches the recipient, and the server sees only ciphertext.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 21
- **Docs:** `features.md` §8, "Product", `architecture.md` — "Security", `docs/protocol/delivery.md`, `docs/protocol/agents.md`, `docs/threat-model.md`, `tech-stack.md`

### Story 65: Queue for agents: MCP tools and templates

**Goal:** Claude Code and Codex take tasks from the queue and hand them in, in formats a team does not have to invent.

**Scope:**
- Queue tools in MCP: enqueue, claim, extend and complete, as in the CLI of Story 26.
- The Claude Code and Codex adapters claim a task, run it with its template's instructions and complete it.
- Templates: review (a PR or diff → findings), test (a command → result and logs), ask (a question about a repository → answer). A template sets the task's input and result and the agent's instructions (`features.md` §10).
- Enqueueing a task from a template in the CLI and MCP (`--template <name>`).
- A team adds its own templates to the space: the team template object, its format and storage, is added to `docs/protocol/queues.md`, "Templates".
- **Done when:** a review task from the template, sent by Claude Code, is claimed and completed by Codex and returns findings in the template's format; an agent that lost its claim cannot complete the task.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 19, Story 20, Story 26
- **Docs:** `features.md` §10, `architecture.md` — "Delivery to agents", `docs/protocol/queues.md`

### Story 63: Notifications to people

**Goal:** A person learns at once that an agent waits for them, without watching the web UI.

**Scope:**
- Channels: Telegram, Slack, a webhook; set per person in the web UI.
- The events of `features.md` §12.
- The server sends the events it sees in metadata; the person's device triggers the events that come from content, and the server relays them (`features.md` §12). What the server sees: `docs/threat-model.md`, "The server sees".
- No content in notifications, no content preview.
- Rate limits, quiet hours, unsubscribe in one click.
- **Done when:** an `ask_user` triggered by the person's device and a DLQ entry noticed by the server arrive in Telegram and on a webhook within seconds; for the `ask_user`, the server receives only the event class.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 18, Story 21, Story 22, Story 26, Story 35, Story 41
- **Docs:** `features.md` §12, `architecture.md` — "Security", `docs/protocol/agents.md`, `docs/threat-model.md`

### Story 66: Restricted mode for tasks from other people

**Goal:** An agent works on a task from another person only with the rights the space allows, so injected instructions cannot run commands on the machine.

**Scope:**
- Trust levels: own devices, space members, service agents. The sender comes from the signed envelope and its certificate chain, not from the text.
- The signed space policy (`architecture.md`, "Membership") sets what each level may do: read only, no shell, no network, or everything.
- The Claude Code and Codex adapters start such a task in the matching mode: Claude Code permission settings, the Codex sandbox. A mode the agent cannot enforce is refused, not weakened.
- The threat model states what this does not stop.
- **Done when:** a task from another member that asks to run a shell command is refused by the agent in read-only mode; the same task from an own device runs; a policy the server changed without an admin's signature has no effect.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 19, Story 20, Story 21, Story 26
- **Docs:** `architecture.md` — "Security", "Membership", `features.md` §10, `docs/protocol/membership.md`, `docs/threat-model.md`

### Story 36: GitHub Action

**Goal:** A GitHub Actions workflow with a single secret sends and receives encrypted messages, and a PR comment can hand a task to an agent.

**Scope:**
- A GitHub Action built on the client's one-shot mode and a service agent key, published together with the release.
- The GitHub bridge runs inside the Action, not on the server (`features.md` §9): a `/skald <template> …` comment from a user with write access becomes a task in the space's queue; the result returns as a PR comment.
- Tasks from comments always run in restricted mode at the service-agent trust level (Story 66).
- **Done when:** a GitHub Action with a single secret sends and receives an encrypted message, including behind a proxy; a `/skald review` comment on a PR returns the review as a PR comment, and the server sees neither in plaintext; a comment from a user without write access is ignored; a task from a comment runs in restricted mode.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 14, Story 35, Story 65, Story 66
- **Docs:** `features.md` §2, §9, §16, `architecture.md` — "Components", "Security"

### Story 43: Gemini, Cursor, OpenCode and Copilot adapters

**Goal:** Messages reach live sessions of Gemini, Cursor, OpenCode and Copilot on every surface the scope lists.

**Scope:**
- Their paths in `architecture.md`, "Delivery to agents", first checked on current CLI versions with live checks, as Story 4 does for Claude Code and Codex.
- Surfaces: Cursor in its IDE and CLI, Copilot in VS Code and its CLI, Gemini in Gemini CLI and Antigravity, OpenCode in its CLI and desktop app; the GUI surfaces are checked by the developer or reviewer driving the app, as Story 4 does.
- `skald connect` finds these agents and, after confirmation, connects them.
- Other agents also run the config Skald writes for one agent (`agents.md`, "Config that other agents run"). The hook handler and the MCP server find which agent runs them, from the hook input, MCP `initialize` or the environment, and hand over only that agent's messages:
  - the hook handler hands over nothing in an agent without Skald hooks of its own, such as Devin and Grok Build;
  - the Channels server pushes events only into an agent that takes them, such as Claude Code.
- e2e tests against real CLIs.
- **Done when:** a message reaches a live session of each of the four agents on each of its surfaces; with Skald's hooks and MCP server set up for Claude Code, in the user's and the repository's config, and for Cursor, each message reaches only its own session, once: Claude Code, Cursor and Copilot sessions get theirs; Devin and Grok Build sessions get none through hooks and only their own through MCP `inbox`.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 15, Story 24
- **Docs:** `features.md` §7, `agents.md` — "Config that other agents run", `architecture.md` — "Components", "Delivery to agents"

### Story 29: End-to-end MVP scenario

**Goal:** The MVP demo passes in full, automatically, on real agents.

**Scope:**
- The demo of `roadmap.md`, "MVP": Claude Code on account A asks Codex on machine B under account C for a review through the task queue.
- The scenario starts with the new-user path: sign-up of A and C, one-command machine connect, invite by link.
- It also covers a review from a `/skald review` PR comment, an `ask_user` answered after a notification, and a task from another member run in restricted mode.
- The automated run signs in through a test OIDC provider instead of GitHub and Google and sends notifications to a test webhook instead of Telegram; the devices of A and C verify each other by fingerprints the test compares.
- Daily run on staging with real Claude Code and Codex in CI.
- **Done when:** the demo passes automatically, the run report is available in CI, and the run checks that:
  - the reply waits in the mailbox of A, who went offline, and arrives when A is back; the server holds only ciphertext (feature 1);
  - a device removed during the run reads no message sent after the removal (feature 4);
  - after all of A's devices are wiped, the recovery key on a new machine restores the full history (feature 6);
  - the review task is claimed once, and its result returns to A (feature 10);
  - sign-up takes one click, each machine one command, each additional machine one confirmation (feature 16);
  - the review from the PR comment arrives as a PR comment, the `ask_user` notification reaches the webhook, and the restricted-mode task is refused a shell command.

- **Release:** MVP
- **Manual check:** a week of real work through Skald, the development of Skald included.
- **Depends on:** Story 9, Story 18, Story 21, Story 23, Story 24, Story 26, Story 27, Story 28, Story 36, Story 41, Story 63, Story 65, Story 66
- **Docs:** `roadmap.md` — "MVP", `features.md` §1, §4, §5, §6, §10, §16, `infrastructure.md` — "PR checks"

### Story 30: Admin rights transfer and account deletion

**Goal:** A space is never left without an admin, and a user can delete their account.

**Scope:**
- Admin rights transfer, a second admin and the "only one admin" warning in the web UI and CLI (`features.md` §4); the change is signed and visible to all members.
- Only the recovery key restores admin rights; there is no server-side reset.
- Account deletion from the web UI and CLI, deferred as in `features.md` §5.
- Spaces where the account was the only member are deleted. The only admin of a space with other members must transfer their rights first; otherwise deletion fails.
- **Done when:** a space with a single admin shows a warning; after the transfer, the new admin removes members; after account deletion, its former devices cannot read new messages, and 30 days later the server has none of its data.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 16, Story 17, Story 22
- **Docs:** `features.md` §4, §5, `architecture.md` — "Data model", "Cryptography", "Membership", `docs/protocol/membership.md`

### Story 31: Load testing

**Goal:** Before the public launch, know what load the server can handle and remove bottlenecks.

**Scope:**
- Scenarios: delivery and WebSocket push, cursor catch-up after a mass reconnect, MLS Commits in large channels, history writes, multiple instances with `LISTEN/NOTIFY`.
- Target numbers with headroom over the cloud limits (`features.md`, "Product") are recorded in the docs.
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
- Connecting any agent: the MCP config that runs the `skald` stdio server, a `skald recv --wait` loop for an agent with only a shell, and the instructions that tell the agent to read its messages.
- In plain language: what the server sees, that there has been no external audit, what to do after losing a device and after losing the admin.
- Published on `docs.skald.so` together with the release.
- **Done when:** a person without context connects Claude Code and Codex on two machines using the quick start; a person without context, following only the "connecting any agent" page, connects one agent that `skald connect` does not set up over MCP and another with only a shell over the `skald recv --wait` loop; each sends a message and, during a task, reads a message sent to it without being told to check; the reference matches `skald --help`.

- **Release:** MVP
- **Depends on:** Story 24, Story 25
- **Docs:** `features.md` §16, `docs/threat-model.md`, `architecture.md` — "Delivery to agents", `infrastructure.md` — "Channels", "Cloud"

### Story 33: Website: main page

**Goal:** `skald.so` explains what Skald is and leads to sign-up.

**Scope:**
- Layout per the approved design in `../skald-cloud/design/`.
- What Skald is and how it works, what the server sees and does not see, plans: Free, Team and Enterprise (`features.md`, "Product"), a block beyond the approved design; links to the docs, GitHub and sign-in at `app.skald.so`.
- Built and published together with the docs from `docs/` on GitHub Pages.
- **Done when:** `skald.so` shows the main page as in the approved design, at desktop and phone width; all links resolve.

- **Release:** MVP
- **Depends on:** Story 25, Story 62
- **Docs:** `features.md` — "Product", `docs/threat-model.md`, `infrastructure.md` — "Channels", "Cloud"

### Story 34: Operator CLI

**Goal:** The operator runs the MVP cloud without SQL.

**Scope:**
- `skald-server admin`: list accounts and spaces with plan and limit usage per space, disable (a manual ban) and re-enable an account, delete an account, deferred as in `features.md` §5.
- Metadata only, like the server itself.
- **Done when:** an operator finds an account and disables it, and the account loses access; deletion removes the data as in Story 30.

- **Release:** MVP
- **Acceptance tests:** first
- **Depends on:** Story 12, Story 30
- **Docs:** `features.md` §5, §17

### Story 37: Task queue: scheduling

**Goal:** Tasks reach agents by priority and capability, within concurrency and cost limits.

**Scope:**
- Priorities, capability tags (`needs: codex`), per-queue concurrency, per-task cost limit (`features.md` §10).
- Queue limits are settings per space, so a plan can change them.
- **Done when:** Claude Code, Codex and CI process 100 tasks by priority, each on an agent with the tags it needs and never above the queue's concurrency; duplicates are rejected, failed tasks go to the DLQ.

- **Release:** Post-MVP
- **Depends on:** Story 19, Story 20, Story 26, Story 36, Story 65
- **Docs:** `features.md` §10, "Product"

### Story 67: A2A gateway

**Goal:** An A2A agent exchanges tasks and messages with Skald agents.

**Scope:**
- A gateway translates between A2A over HTTPS and the Skald A2A extension: tasks, messages, results.
- The gateway holds a service agent key; end-to-end encryption ends at the gateway, and the docs say so.
- **Done when:** an A2A client sends a task to a Skald queue through the gateway and gets the result.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 2, Story 26, Story 35
- **Docs:** `architecture.md` — "Components", `docs/protocol/README.md`

### Story 38: Codex background agents via app-server

**Goal:** A Codex background agent receives messages without an open interactive session and without tmux.

**Scope:**
- The Codex app-server path of `architecture.md`, "Delivery to agents".
- e2e tests against real Codex.
- **Done when:** a background Codex receives a message and replies without human involvement.

- **Release:** Post-MVP
- **Depends on:** Story 20
- **Docs:** `features.md` §7, `architecture.md` — "Delivery to agents"

### Story 68: Team plan and billing

**Goal:** A space moves to Team and pays per member; the Free limits apply to spaces that do not.

**Scope:**
- Moving a space to Team and back; payment per member per month through a payment provider; invoices in the web UI.
- The Free limits of the table in `features.md`, "Product", start to apply in the cloud; the server refuses a roster change that takes a Free space over its member limit.
- Team and Enterprise spaces stop counting toward the spaces an account owns; a space can return to Free only while its owner is under the Free number of spaces.
- A space already over a Free limit when the limits start, or a Team space left unpaid, keeps its members, devices, service agents and history but cannot add more until it is within the limits; history beyond the Free retention is deleted 30 days after a notice to the space's admins. An unpaid Team space whose owner already has the Free number of spaces moves to the Free limits all the same; the owner then creates no new spaces until under the limit.
- What the server sees for billing: `docs/threat-model.md`, "The server sees".
- **Done when:** the 4th member of a Free space is refused until the space moves to Team; after payment the space gets the Team limits; an account with two Free spaces cannot move a third space back to Free; an unpaid Team space keeps its data, cannot grow, and loses history beyond the Free retention 30 days after the notice.

- **Release:** Post-MVP
- **Depends on:** Story 22, Story 25
- **Docs:** `features.md` — "Product", §12, `docs/threat-model.md`

### Story 39: Shared channels between organizations

**Goal:** Two spaces of different organizations communicate through a shared channel with the consent of both sides.

**Scope:**
- A shared channel joins two spaces on Team or Enterprise; it is created only with the consent of admins on both sides, and either side can disable it (`features.md` §3).
- Each side's consent is signed by its admin's device (`architecture.md`, "Membership").
- Approval in the CLI and a linked browser.
- **Done when:** a freelancer is in three spaces, with a clientA↔clientB shared channel; after one side disables it, no messages pass through it; a shared channel with a Free space is refused.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 13, Story 22, Story 68
- **Docs:** `features.md` §3, §12, `architecture.md` — "Data model", "Membership", `docs/protocol/membership.md`

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

### Story 44: Edit coordination: file level

**Goal:** Agents of different people and machines learn before merge that they are editing the same files.

**Scope:**
- The edit-coordination wire format in the protocol spec, from Story 5's findings: what the client sends and what the server compares.
- Clients publish HMAC fingerprints of changed files with the space key; the server sees only fingerprints.
- The action follows the space policy (`features.md` §11).
- Claiming a task reserves its edit scope; reservations have a TTL.
- Decisions on the questions in `features.md` §11 "To decide", based on Story 5.
- **Done when:** two agents on different machines editing the same file get a warning before merge; the server does not know the paths.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 5, Story 19, Story 20, Story 26, Story 65
- **Docs:** `features.md` §11, `architecture.md` — "Cryptography", `docs/protocol/README.md`

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
- The security center, the human inbox with approval from a phone and the edit map of `features.md` §12.
- **Done when:** an `ask_user` is approved from a phone; a device is revoked with one click; the map shows overlapping edits.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 18, Story 27, Story 41, Story 44
- **Docs:** `features.md` §12, `docs/protocol/agents.md`

### Story 47: Public cloud: anti-abuse

**Goal:** The cloud is protected from abuse.

**Scope:**
- Abuse reports, automatic abuse detection and bans based on them; rules in `skald-cloud`. Manual bans come with Story 34.
- **Done when:** a banned account loses access.

- **Release:** Post-MVP
- **Depends on:** Story 22, Story 25
- **Docs:** `features.md` — "Product", §12, `architecture.md` — "Security", `infrastructure.md` — "Repositories"

### Story 48: Operator console

**Goal:** The instance operator — the cloud owner or a self-hosted server admin — runs the service from the web UI.

**Scope:**
- A web UI section visible only to instance admins.
- Layout per the approved design in `../skald-cloud/design/`.
- The operator console shell from the kit components (Story 62): a top bar with the "operator" label, tabs with count badges and the instance name.
- Accounts and spaces, plan and limit usage per space, licenses.
- Abuse reports and bans; account deletion and export requests.
- System health.
- Metadata only: the operator cannot read content.
- **Done when:** an operator bans an account and handles a deletion request from the console, without SQL; a regular user does not see the section.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 30, Story 47, Story 62
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
- **Depends on:** Story 16, Story 17, Story 64
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

### Story 69: OpenClaw and Crush setup by `skald connect`

**Goal:** `skald connect` connects OpenClaw and Crush like the agents of `agents.md`, "Setup by `skald connect`".

**Scope:**
- OpenClaw: adds `mcp.servers.skald` to `~/.openclaw/openclaw.json` (JSON5), keeping the other entries and the comments.
- Crush: adds the Skald MCP server to `~/.config/crush/crushrc`, keeping the rest of the file.
- `agents.md`, "Setup by `skald connect`", gets a row for each, and its "OpenClaw and Crush" line goes.
- **Done when:** with OpenClaw and Crush on a machine, `skald connect` adds the Skald MCP server to each, with its other config kept.

- **Release:** Post-MVP
- **Depends on:** Story 24
- **Docs:** `features.md` §16, `agents.md` — "Setup by `skald connect`"

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
- **Docs:** `features.md` §13, `architecture.md` — "Cryptography", `docs/threat-model.md`

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
- Entering the recovery key, which `app.skald.so` never accepts (`architecture.md`, "Device linking").
- **Done when:** `skald ui` opens the web UI from localhost, and the content sections work without loading JS from the server; the recovery key entered there restores the history.

- **Release:** Post-MVP
- **Depends on:** Story 7, Story 27
- **Docs:** `features.md` §12, `architecture.md` — "Components", "Device linking"

### Story 59: Sealed sender and padding

**Goal:** Reduce the metadata the server sees.

**Scope:**
- Sealed sender: the server does not see which device sent a message.
- Padding: the server does not see the exact message size.
- The threat model is updated.
- **Done when:** the server cannot tell which device sent a message, and ciphertext sizes are evened out.

- **Release:** Post-MVP
- **Acceptance tests:** first
- **Depends on:** Story 12, Story 13
- **Docs:** `architecture.md` — "Cryptography", `docs/threat-model.md`

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
