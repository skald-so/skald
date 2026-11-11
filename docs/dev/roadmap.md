# Roadmap

Numbers in parentheses are features from [features.md](features.md).

## Preparation (before MVP)
- Protocol spec (envelope, addressing, receipts, devices and roster, queues) and threat model. Published openly, marked "no external audit".
- Prototypes on OpenMLS and on mls-rs; the library is chosen based on the results.
- Verify on current versions of Claude Code (Channels, hooks) and Codex (hooks, `codex queue`, app-server), and what subscription limit data they expose.
- Edit comparison prototype with HMAC fingerprints.

## MVP
**Demo:** Claude on account A asks Codex on machine B under account C for a review through the task queue. The sender closes the laptop; the reply waits in their mailbox. The user follows the thread and the task in a linked browser.

- Encrypted mailbox: cursor, receipts, deduplication, WebSocket push, quotas (1).
- Client: one-shot mode and helper (2).
- Multiple spaces per device, a personal space for each account, agent address with the server name (3, 15).
- MLS for devices, roles, device revocation (4).
- GitHub and Google sign-in, linked sign-in methods, device flow, device certificate (5).
- Key verification without trusting the server: the device key signed with the SSH key from GitHub, cross-signing, key change warnings, fingerprints, DNS root of trust (5).
- Encrypted history, recovery key, new device linking (6).
- Claude Code and Codex adapters: MCP, hooks, Channels (7).
- Gemini, Cursor, OpenCode and Copilot adapters (7).
- Presence, receipts, loop protection, `accept / hold / refuse` (8).
- Web UI `app.skald.so`, non-secret sections: quotas, devices, sessions, members and invites, connecting machines (12).
- Task queue: claim, lease, retries, DLQ; queue MCP tools; Claude Code and Codex claim and complete tasks; templates review, test, ask (10).
- Service and CI agents: HPKE, scoped tokens, rotation, GitHub Action, tasks from issue and PR comments (9).
- `ask_user` and attachments (8).
- Notifications to Telegram, Slack or a webhook (12).
- Restricted mode for tasks from other people (10).
- Browser linking and content sections in the web UI (WASM): history and search, queue panel, delivery trace, loop analysis (12, 6).
- One-command connect with a code (`install.sh`, `install.ps1`, `npx`), auto-setup of Claude Code and Codex, MCP setup of the agents of [agents.md](agents.md#mcp-setup-by-skald-connect), new device confirmation, invite by link, Claude Code plugin (16).
- Admin rights transfer, "only one admin" warning, account deletion (4, 5).
- Load testing before launch.
- Documentation: quick start, CLI reference.
- Website main page.
- Operator CLI `skald-server admin` (17).
- Public cloud launch, protected by quotas, rate limits and manual bans in the operator CLI.
- Everything in the cloud is free: every space gets the Team limits, and an account owns up to the Free number of spaces ([features.md](features.md), "Product").
- Self-host with Docker Compose.

## Post-MVP
List order is priority.
- Task queue: priorities, capability tags, concurrency, cost limits (10).
- A2A gateway.
- Codex background agents via app-server (7).
- Team plan and billing.
- Shared channels between organizations (3).
- Corporate IdPs (5).
- Shared team instructions (8).
- Edit coordination: file level first, then hunk and symbol (11).
- Security center, human inbox with approval from a phone, edit map (12).
- Public cloud: abuse reports and automatic abuse detection.
- Operator console (17).
- Analytics, LLM token usage per task (12).
- Enterprise (`ee/`): license and billing, audit log, SSO and SAML, SCIM, auditor device, retention policies (12).
- Self-host: Helm, isolated networks.
- History export and migration from the cloud to a self-hosted server (6).
- Python and TypeScript SDKs.
- Homebrew, winget, cargo-binstall (16).
- Managed-keys mode, on demand (13).
- `skald ui`: a local copy of the web UI from a signed binary, on demand (12).
- Sealed sender and padding.
- Server federation (15).
- Capacity view, only if vendors expose limit data (14).
