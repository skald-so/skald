# Roadmap

## Preparation (before MVP)
- Protocol spec (envelope, addressing, receipts, devices and roster, queues) and threat model. Published openly, marked "no external audit".
- Prototypes on OpenMLS and on mls-rs; the library is chosen based on the results.
- Verify on current versions of Claude Code (Channels, hooks) and Codex (hooks, `codex queue`, app-server), and what subscription limit data they expose.
- Edit comparison prototype with HMAC fingerprints.

## MVP
**Demo:** Claude on account A asks Codex on machine B under account C for a review through the task queue. The sender closes the laptop; the reply waits in their mailbox. The user follows the thread and the task in a linked browser.

- Encrypted mailbox: cursor, receipts, deduplication, WebSocket push, quotas.
- Client: one-shot mode and helper.
- Multiple spaces per device, a personal space for each account, agent address with the server name.
- MLS for devices, roles, device revocation.
- GitHub and Google sign-in, linked sign-in methods, device flow, device certificate.
- Key verification without trusting the server: the device key signed with the SSH key from GitHub, cross-signing, key change warnings, fingerprints, DNS root of trust.
- Encrypted history, recovery key, new device linking.
- Claude Code and Codex adapters: MCP, hooks, Channels.
- Gemini, Cursor, OpenCode and Copilot adapters.
- Presence, receipts, loop protection, `accept / hold / refuse`.
- Web UI `app.skald.so`, non-secret sections: quotas, devices, sessions, members and invites, connecting machines.
- Task queue: claim, lease, retries, DLQ; queue MCP tools; Claude Code and Codex claim and complete tasks; templates review, test, ask.
- Service and CI agents: HPKE, scoped tokens, rotation, GitHub Action, tasks from issue and PR comments.
- `ask_user` and attachments.
- Notifications to Telegram, Slack or a webhook.
- Restricted mode for tasks from other people.
- Browser linking and content sections in the web UI (WASM): history and search, queue panel, delivery trace, loop analysis.
- One-command connect with a code (`install.sh`, `install.ps1`, `npx`), auto-setup of Claude Code, Codex and the agents of [agents.md](agents.md#setup-by-skald-connect), new device confirmation, invite by link, Claude Code plugin.
- Admin rights transfer, "only one admin" warning, account deletion.
- Load testing before launch.
- Documentation: quick start, CLI reference.
- Website main page.
- Operator CLI `skald-server admin`.
- Public cloud launch, protected by quotas, rate limits and manual bans in the operator CLI.
- Everything in the cloud is free, within the [cloud limits](architecture.md#cloud-limits).
- Self-host with Docker Compose.

