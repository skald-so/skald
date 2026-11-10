# Iterations

## Story 1: Monorepo skeleton and CI
- [x] **1.1:** Cargo workspace, `skald --version` and `script/test`
- [x] **1.2:** CI checks on pull requests
- [x] **1.3:** Licenses, policies and `cargo-deny`
- [x] **1.4:** Server health check and OpenAPI spec
- [x] **1.5:** PostgreSQL and migrations
- [x] **1.6:** SPA skeleton with generated API client
- [x] **1.7:** CI path filters and Renovate
- [x] **1.8:** Branch protection and `CONTRIBUTING.md`
- [x] **1.9:** Mutation check of changed Rust code

## Story 2: Protocol spec and threat model
- [x] **2.1:** Protocol spec: addressing, envelope and versions
- [x] **2.2:** Protocol spec: mailbox, delivery and receipts
- [x] **2.3:** Protocol spec: devices, roster and channels
- [x] **2.4:** Protocol spec: queues and tasks
- [x] **2.5:** Protocol spec: presence and `ask_user`
- [x] **2.6:** Threat model
- [x] **2.7:** Identifiers and agent addresses in `proto`
- [ ] **2.8:** Envelope and receipt types in `proto`
- [ ] **2.9:** Protocol version check on the server
- [ ] **2.10:** Protocol version check in the client

## Story 3: MLS prototypes and library choice
- [x] **3.1:** Prototype harness and a channel group on both libraries
- [x] **3.2:** Removing a device with a new epoch
- [x] **3.3:** KeyPackages and admission checks
- [ ] **3.4:** Commit ordering through a minimal Delivery Service
- [ ] **3.5:** Joining by itself and rejoining
- [ ] **3.6:** WASM build and size
- [ ] **3.7:** Interoperability between the libraries
- [ ] **3.8:** Benchmarks
- [ ] **3.9:** Comparison report and the library choice

## Story 4: Verifying delivery to Claude Code and Codex

## Story 5: Edit comparison prototype with HMAC fingerprints

## Story 6: Staging environment

## Story 7: Release pipeline

## Story 62: Web UI kit: theme, cabinet shell and components
- [x] **62.1:** Theme, fonts and the kit page
- [x] **62.2:** Buttons, status labels and the warning banner
- [x] **62.3:** Sortable table, empty and loading states
- [x] **62.4:** Filter buttons and search
- [x] **62.5:** Dropdown filter
- [x] **62.6:** Reference screenshots of the kit page
- [x] **62.7:** Cabinet shell

## Story 64: Self-host: Docker Compose

## Story 8: GitHub sign-in and device certificate

## Story 9: Google sign-in and linked sign-in methods

## Story 10: Crypto core

## Story 11: Spaces and agent addresses

## Story 12: Encrypted mailbox

## Story 13: MLS channels, roles and device revocation

## Story 14: Client: one-shot mode and helper

## Story 15: MCP server and agent commands

## Story 16: Encrypted channel history

## Story 17: Recovery key and new device linking

## Story 18: Key verification without trusting the server

## Story 19: Claude Code adapter and plugin

## Story 20: Codex adapter

## Story 21: Presence, statuses and loop protection

## Story 22: Web UI: non-secret sections

## Story 23: Space invite by link

## Story 24: One-command machine connect

## Story 25: Production cloud, website and monitoring

## Story 26: Task queue: claim, lease and DLQ

## Story 27: Browser linking and history on the web

## Story 28: Web: queues, delivery trace, loops, ask_user

## Story 35: Service agents: HPKE and scoped tokens

## Story 41: ask_user and attachments

## Story 65: Queue for agents: MCP tools and templates

## Story 63: Notifications to people

## Story 66: Restricted mode for tasks from other people

## Story 36: GitHub Action

## Story 29: End-to-end MVP scenario

## Story 30: Admin rights transfer and account deletion

## Story 31: Load testing

## Story 32: Documentation: quick start and CLI reference

## Story 33: Website: main page

## Story 34: Operator CLI

## Story 37: Task queue: scheduling

## Story 67: A2A gateway

## Story 38: Codex background agents via app-server

## Story 68: Team plan and billing

## Story 39: Shared channels between organizations

## Story 40: Corporate IdP sign-in

## Story 42: Shared team instructions

## Story 43: Gemini, Cursor, OpenCode and Copilot adapters

## Story 44: Edit coordination: file level

## Story 45: Edit coordination: hunk and symbol

## Story 46: Web: security center, human inbox, edit map

## Story 47: Public cloud: anti-abuse

## Story 48: Operator console

## Story 49: Analytics and token usage

## Story 50: Enterprise: SSO, SAML, SCIM and license

## Story 51: Enterprise: audit and retention policies

## Story 52: Self-host: Helm and isolated networks

## Story 53: History export and migration to a self-hosted server

## Story 54: Python and TypeScript SDKs

## Story 55: Homebrew, winget and cargo-binstall

## Story 56: Managed keys: custodian device

## Story 57: Hosted MCP for ChatGPT and Claude.ai

## Story 58: skald ui

## Story 59: Sealed sender and padding

## Story 60: Server federation

## Story 61: Capacity view
