# Roadmap

Numbers in parentheses are features from [features.md](features.md).

## Preparation (before MVP)
- Protocol spec (envelope, addressing, receipts, queues) and threat model. Published openly, marked "no external audit".
- Prototypes on OpenMLS and on mls-rs; the library is chosen based on the results.
- Verify on current versions of Claude Code (Channels, hooks) and Codex (hooks, `codex queue`, app-server), and what subscription limit data they expose.
- Edit comparison prototype with HMAC fingerprints.

## MVP
**Demo:** Claude on account A asks Codex on machine B under account C for a review through the task queue. The sender closes the laptop; the reply waits in their mailbox. The user follows the thread and the task in a linked browser.

- Encrypted mailbox: cursor, receipts, `idempotency_key`, WebSocket push, quotas (1).
- Client: one-shot mode and helper (2).
- Multiple spaces per device, agent address with the server name (3, 15).
- MLS for devices, roles, device revocation (4).
- GitHub OIDC sign-in and device flow, device certificate (5).
- Encrypted history, recovery key, new device linking (6).
- Claude Code and Codex adapters: MCP, hooks, Channels (7).
- Presence, receipts, loop protection, `accept / hold / refuse` (8).
- Web UI `app.skald.so`, non-secret sections: quotas, devices, sessions, members and invites, connecting machines (12).
- Task queue: claim, lease, retries, DLQ (10).
- Browser linking and content sections in the web UI (WASM): history and search, queue panel, delivery trace, loop analysis (12, 6).
- One-command connect with a code (`install.sh`, `install.ps1`, `npx`), auto-setup of Claude Code and Codex, new device confirmation, invite by link, Claude Code plugin (16).
- Admin rights transfer, "only one admin" warning, account deletion (4, 5).
- Load testing before launch.
- Documentation: quick start, CLI reference.
- Website main page.
- Operator CLI `skald-server admin` (17).

## Post-MVP
List order is priority.
- Service and CI agents: HPKE, scoped tokens, rotation, GitHub Action (9).
- Task queue: scheduling and agent integration (10).
- Codex background agents via app-server (7).
- Shared channels between organizations (3).
- Key verification via the SSH key on GitHub, cross-signing, fingerprints, DNS root of trust, Google and corporate IdPs (5).
- `ask_user`, attachments, shared team instructions (8).
- Gemini, Cursor, OpenCode, Copilot adapters (7).
- Edit coordination: file level first, then hunk and symbol (11).
- Security center, human inbox with approval from a phone, edit map (12).
- Public cloud: anti-abuse.
- Operator console (17).
- Alerts, analytics, LLM token usage per task (12).
- Enterprise (`ee/`): license and billing, audit log, SSO and SAML, SCIM, auditor device, retention policies (12).
- Self-host documentation, Docker Compose, Helm.
- History export and migration from the cloud to a self-hosted server (6).
- Python and TypeScript SDKs.
- Homebrew, winget, cargo-binstall (16).
- Managed-keys mode, on demand (13).
- `skald ui`: a local copy of the web UI from a signed binary, on demand (12).
- Sealed sender and padding.
- Server federation (15).
- Capacity view, only if vendors expose limit data (14).
