# Features

Accepted features. Which release each one belongs to: [roadmap.md](roadmap.md).

## Product
- **Audience:** agents of different people, accounts and vendors running on laptops, servers, cloud sandboxes and CI. The "one account, Claude only" scenario is not ours: Claude Code has covered it.
- **Positioning:** end-to-end encryption of coordination for a team of agents from different vendors. Everything is encrypted: messages, tasks, presence, edit coordination.
- **Focus:** the task queue and managed delivery, not free-form chat.
- **Model:** open source + cloud + open core (GitLab, Sentry, Bitwarden, ntfy).
  - Licenses: the server under AGPL-3.0; the protocol, crypto core, client, MCP, adapters, SDKs and web UI under `MIT OR Apache-2.0`; organization features in `ee/` under a commercial license. Paths: [LICENSE.md](../../LICENSE.md).
  - CLA with contributors, via CLA Assistant: [CONTRIBUTING.md](../../CONTRIBUTING.md).
  - The Skald name and logo are covered by a trademark policy: [TRADEMARKS.md](../../TRADEMARKS.md).
  - The license is chosen up front and never changed.
- **Everything is free except Enterprise.** Only `ee/` features are paid: by a license key on a self-hosted server or by the Enterprise plan in the cloud.
- **Cloud limits** protect against abuse. They are per-account settings, so paid tiers above them can be added later:
  - up to 3 spaces and 10 devices;
  - about 1M messages per month;
  - undelivered messages are kept for 7 days, history for 30 days;
  - up to 3 service agents.
- **Enterprise (ee):** SSO and SAML, SCIM, audit log, retention policies, admin access revocation, auditor device, SLA, deployment in an isolated network.

## 1. Encrypted mailbox
- The server stores ciphertext for each recipient device. If the recipient is online, the message arrives immediately; otherwise it waits in the mailbox. After reconnecting, the client catches up on missed messages from its cursor.
- No plaintext. If the recipient has no keys, sending fails with an error.
- Receipts `stored → delivered → read → replied`, plus `expired` and `refused`. `idempotency_key` protects against duplicates.
- **Done when:** A sends a message and closes the laptop; B comes online 10 hours later and receives it. They were never online at the same time; the server holds only ciphertext.

## 2. Daemonless operation
- **One-shot mode** for CI, cron and sandboxes. The **helper** on a developer machine starts automatically; no root needed.
- Outbound HTTPS only; works behind a corporate proxy.
- **Done when:** a GitHub Action with a single secret sends and receives an encrypted message, including behind a proxy.

## 3. Spaces and shared channels
- A device belongs to N spaces. A session is bound to a space by its folder: `.skald.toml` or a rule `~/work/clientA/** → clientA`.
- An agent of one client cannot see another client.
- A **shared channel** between two spaces is created only with the consent of admins on both sides; either side can disable it.
- **Done when:** a freelancer is in three spaces; an agent in `~/work/clientA` sees only clientA; there is a clientA↔clientB shared channel.

## 4. MLS group encryption
- Forward secrecy, post-compromise security.
- Revocation of a specific device, e.g. a stolen laptop.
- Roles: admin, member, service agent. Only an admin removes members.
- An admin transfers their rights or appoints a second admin.
- If there is only one admin, the web UI and CLI suggest appointing a second one. Only the recovery key restores admin rights; there is no server-side reset. If an admin loses both their devices and the key, members create a new space and keep the old space's history.
- **Done when:** a removed device with a full copy of its disk cannot read any message sent after the removal (automated test).

## 5. Identity
- Sign-in via OIDC: GitHub, Google, corporate IdP. A self-hosted server uses its own IdP. Device flow for machines without a browser.
- Device certificate: "device key → account".
- **Verification without trusting our server:** the device key is signed with the user's SSH key from GitHub; the client checks the signature directly against GitHub.
- Device cross-signing, key change warnings, fingerprints for manual verification.
- Optional DNS root of trust (TXT record) for organizations with their own domain.
- Account deletion: the server erases the account, devices, mailboxes and wrapped keys. Messages in channels with other members stay with them.
- **Done when:** the client detects a key swap on the server; a self-hosted server works with its own IdP without our cloud.

## 6. Encrypted history
- We store the full channel history, not just undelivered messages.
- Each message is encrypted with the channel's history key. History keys are stored on the server, wrapped to the account's devices and to the recovery key.
- A **new device** gets the history if it was linked from an existing device or the recovery key was entered.
- A **new channel member** sees the history only if an admin allowed it.
- When a member is removed, the history key is rotated.
- Search runs on the client.
- **Export:** `skald export` decrypts the history on the client into JSON or Markdown; a space can be moved from the cloud to a self-hosted server. Until then, GDPR requests are handled manually through support.
- **Done when:** all devices are wiped; the recovery key is entered on a new laptop, and the full history is there.

## 7. Delivery to agents
- A message reaches a live session:
  - Claude Code — via Channels and hooks;
  - Codex — via hooks and app-server, without tmux;
  - Gemini, Cursor, OpenCode, Copilot — via hooks and MCP;
  - any agent — via the CLI.

## 8. Core features (parity with competitors)
- Presence directory (encrypted).
- Receipts and `message_status`.
- Loop protection.
- Inbound modes `accept / hold / refuse`.
- `ask_user`: confirmation from a human.
- Small text attachments.
- Shared team instructions: versions, distribution to machines, injection into `AGENTS.md` or `CLAUDE.md`, all encrypted.

## 9. End-to-end encrypted service and CI agents
- An agent has its own key (in an environment secret) and a scoped token: send-only, queue-only, specific channels, expiry, rotation.
- Stateless HPKE encryption.
- ChatGPT and Claude.ai cannot work through remote MCP with end-to-end encryption. They need managed-keys mode (feature 13).
- **Done when:** messages are exchanged with a CI agent that has no persistent disk; the server holds only ciphertext.

## 10. Task queue
- Named queues. Task = encrypted payload + plaintext scheduling fields.
- Atomic claim, lease with heartbeat, fencing token and `claim_lost`, retries, DLQ.
- Priorities, capability tags (`needs: codex`), per-queue concurrency, per-task cost limit.
- The result returns to the sender as an encrypted message. At-least-once semantics.
- **Done when:**
  - Claude Code, Codex and CI process 100 tasks, duplicates are rejected, failed tasks go to the DLQ;
  - a stopped agent gets `claim_lost`;
  - a live but slow agent does not lose its task.

## 11. Cross-machine edit coordination
- Warns before merge that agents of different people and machines are editing the same thing.
- File level first, then hunk and symbol.
- The server sees only HMAC fingerprints. Clients compute the exact overlap with `git merge-tree`.
- The action is set by the space policy: notify, warn, ask a human, block pre-push.
- Claiming a task reserves its edit scope.
- **To decide:**
  - data source: diff, hooks or declarations;
  - when to warn;
  - reservation TTL;
  - how to catch edits made via Bash and generators;
  - path normalization;
  - noise from lockfiles and migrations;
  - whether intents like "replacing / extending" are needed.

## 12. Web UI
There is one interface: `app.skald.so`. Decryption happens in the browser, as in WhatsApp Web or Element.
- **Non-secret sections** are available right after sign-in:
  - quotas; Enterprise license and invoices;
  - devices and service agents;
  - sessions and tokens: web and CLI sign-ins, invite links, service agent tokens. Metadata only (where, when, permissions, expiry, last use), one-click revocation. The token value is shown once, at creation;
  - members, roles, invites, shared channel approval;
  - connecting machines;
  - metadata statistics.
- **Content sections** open after the "Link this browser" button. Linking is done once: with a code or QR from an already linked machine, or with the recovery key.
  - message texts, threads, a "which agent messages whom" graph, history search;
  - **queue panel:** who holds a task, how much lease is left, `claim_lost`, retries, DLQ with "restart" and "cancel" buttons;
  - **delivery trace:** the `stored → delivered → read` path and the reason if a message was not delivered;
  - **security center:** devices and fingerprints, one-click revocation, key rotation, "who can read the channel";
  - **human inbox:** all `hold` and `ask_user` items, approval from a phone (the phone's browser is a linked device);
  - **loop analysis:** agent↔agent threads without a human, a "pause" button;
  - **analytics:** computed by the client, stored encrypted;
  - **edit map.**
- **Alerts** to Telegram, Slack or a webhook: DLQ is growing, an agent is gone, a key changed, a token is expiring.
- **Auditor device (ee):** the organization admin sees all conversations; members know about it.
- LLM token usage is shown only from what the CLI itself shows the user. Low priority.
- **`skald ui`** (on demand) — the same page served from localhost by a signed binary. It is for those who do not trust code delivered from the server. It does not create a second interface.

## 13. Managed-keys mode
- The server keeps its own custodian device in channels and therefore can read them. This is needed for the web without linking a device, server-side search and hosted MCP (ChatGPT, Claude.ai).
- An admin chooses the mode for the whole space. The choice is signed and visible to all members. The default is end-to-end encryption.

## 14. Capacity view
- The team sees members' subscription limits. A task waits for the limit to reset on the same account.
- We take only what the CLI itself shows the user. We never touch vendor tokens or switch accounts (ToS).
- Built only if vendors expose this data.

## 15. Server federation
- Lets organizations on different servers communicate. The server name is part of the agent address from the first release.

## 16. One-command connect
- **Sign-up** on the website: "Sign in with GitHub".
- **Connecting a machine.** The website detects the OS and shows a single command with a one-time code: `curl … | sh`, `irm … | iex` or `npx skald-so connect <code>`. The command:
  - downloads the binary;
  - generates keys on the machine;
  - registers the device with the code, without a second sign-in;
  - shows the recovery key;
  - finds Claude Code and Codex and, after confirmation, sets up MCP and hooks;
  - starts the helper.

  The code lives for a few minutes and works once. It does not transfer keys.
- **Status on the website** updates by itself: "✓ MacBook connected: Claude Code, Codex".
- **The next machine** uses the same command. The new device is confirmed on an already linked device or with the recovery key.
- **Team invite** is by link. The admin's device adds the newcomer to channels by itself.
- **Other ways:** Claude Code plugin, GitHub Action (feature 9), later Homebrew, winget and `cargo binstall`.
- **Done when:** one-click sign-up, one command per machine, one confirmation for each additional machine. After that, Claude Code and Codex on two machines exchange messages.

## 17. Operator tools
- For instance operators: the cloud owner and self-hosted server admins.
- **CLI** `skald-server admin`: accounts, quota usage, disabling and deleting an account.
- **Console** in the web UI: accounts and organizations, quota usage and licenses, abuse reports and bans, deletion and export requests, system health.
- Metadata only: content stays encrypted from the operator too.
- **Done when:** an operator bans an account and handles a deletion request from the console, without SQL.

## Out of scope
- **agent-fleet** (distributing agent configs to machines): it has a different buyer. Only shared team instructions remain (feature 8).
