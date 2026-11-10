# Features

Accepted features. Which release each one belongs to: [roadmap.md](roadmap.md).

## Product
- **Audience:** agents of different people, accounts and vendors running on laptops, servers, cloud sandboxes and CI.
- **Positioning:** end-to-end encryption of coordination for a team of agents from different vendors. Everything is encrypted: messages, tasks, presence, edit coordination.
- **Focus:** the task queue and managed delivery, not free-form chat.
- **A2A:** Skald is an extension of A2A; agents keep A2A's message model: [protocol spec](../protocol/README.md#a2a-extension).
- **Model:** open source + cloud + open core.
  - Licenses: the server under AGPL-3.0; the protocol, crypto core, client, MCP, adapters, SDKs and web UI under `MIT OR Apache-2.0`; organization features in `ee/` under a commercial license. Paths: [LICENSE.md](../../LICENSE.md).
  - CLA with contributors, via CLA Assistant: [CONTRIBUTING.md](../../CONTRIBUTING.md).
  - The Skald name and logo are covered by a trademark policy: [TRADEMARKS.md](../../TRADEMARKS.md).
  - The license is chosen up front and never changed.
- **Plans** apply to a space, not to an account. An account is always free and joins any number of spaces. Limits: the table below.
  - **Team** is paid per member of the space per month, at any size, and is required above the Free member limit.
  - **Enterprise (`ee/`):** SSO and SAML, SCIM, audit log, retention policies, admin access revocation, auditor device, SLA, deployment in an isolated network. By a license key on a self-hosted server or by the Enterprise plan in the cloud, with limits by contract.
  - The spaces an account owns are the ones it created, its personal space included; Team and Enterprise spaces do not count.
  - Shared channels: [§3](#3-spaces-and-shared-channels).
  - A self-hosted server applies no plan limits; only `ee/` needs a license.

**Cloud limits** by plan:

| Limit | Free | Team |
|---|---|---|
| Spaces an account owns | 2 | not counted |
| Members per space | 3 | no limit |
| Devices per member | 10 | 50 |
| Service agents per space | 3 | 10 |
| Messages per space | About 1M per month | About 10M per month |
| Message text | 64 KB | 64 KB |
| Attachment | 10 MB | 10 MB |
| Undelivered messages kept | 7 days | 7 days |
| History kept | 30 days | 90 days |

## 1. Encrypted mailbox
- The server stores ciphertext for each recipient device. If the recipient is online, the message arrives immediately; otherwise it waits in the mailbox. After reconnecting, the client catches up on missed messages from its cursor.
- No plaintext. If the recipient has no keys, sending fails with an error.
- Receipts `stored → delivered → read → replied`, plus `held`, `refused` and `expired`: every message ends in a status the sender sees. A retry with the same `msg_id` from the same device creates no duplicate. Semantics: [protocol spec](../protocol/delivery.md).
- **Done when:** A sends a message and closes the laptop; B comes online 10 hours later and receives it. They were never online at the same time; the server holds only ciphertext.

## 2. Daemonless operation
- **One-shot mode** for CI, cron and sandboxes. The **helper** on a developer machine starts automatically; no root needed.
- Outbound HTTPS only; works behind a corporate proxy.
- **Done when:** a GitHub Action with a single secret sends and receives an encrypted message, including behind a proxy.

## 3. Spaces and shared channels
- A device belongs to N spaces. A session is bound to a space by its folder: `.skald.toml` or a rule `~/work/clientA/** → clientA`.
- Each account has a personal space; its machines join it by default, so one person can start alone.
- An agent of one client cannot see another client.
- A **shared channel** connects two spaces on Team or Enterprise. It is created only with the consent of admins on both sides; either side can disable it.
- **Done when:** a freelancer is in three spaces: personal, clientA and clientB; an agent in `~/work/clientA` sees only clientA; there is a clientA↔clientB shared channel.

## 4. MLS group encryption
- Forward secrecy, post-compromise security.
- Not covered: the stored history (feature 6). A revoked device keeps the history keys it held and still reads the history written before its revocation; it reads nothing written after.
- Revocation of a specific device, e.g. a stolen laptop.
- Roles: admin, member, service agent. Only an admin removes members. Signing of member and role changes: [architecture.md](architecture.md), "Membership".
- An admin transfers their rights or appoints a second admin.
- If there is only one admin, the web UI and CLI suggest appointing a second one. Only the recovery key restores admin rights; there is no server-side reset. If an admin loses all their devices and the key, members create a new space and keep the old space's history.
- **Done when:** a removed device with a full copy of its disk cannot read any message sent after the removal (automated test).

## 5. Identity
- Sign-in via GitHub (OAuth 2.0), Google and corporate IdPs (OIDC). A self-hosted server uses its own IdP. Device flow for machines without a browser.
- One account can have several sign-in methods; a method is linked explicitly, accounts are never merged by email.
- Device certificate: "device key → account". Who signs it: [architecture.md](architecture.md), "Device linking".
- **Verification without trusting our server:** the device key is signed with the user's SSH key from GitHub; the client checks the signature directly against GitHub.
- Device cross-signing, key change warnings, fingerprints for manual verification.
- Optional DNS root of trust (TXT record) for organizations with their own domain.
- **Account deletion** is deferred:
  - at once, the server ends the account's sign-ins and tokens, deletes its mailboxes and stops delivery to its devices;
  - the first member device of each of the account's channels to come online removes the account's devices and rotates the history key;
  - 30 days after the deletion, the server erases the account, its devices and wrapped keys, whether or not every channel has removed them.
  - Messages in channels with other members stay with them.
- **Done when:** the client detects a key swap on the server; a self-hosted server works with its own IdP without our cloud.

## 6. Encrypted history
- We store the full channel history, not just undelivered messages.
- History keys, their rotation and the key chain: [architecture.md](architecture.md), "Cryptography".
- A **new device** gets the history if it was linked from an existing device or the recovery key was entered.
- A **new channel member** sees the history only if an admin allowed it.
- Search runs on the client.
- **Export:** `skald export` decrypts the history on the client into JSON or Markdown; a space can be moved from the cloud to a self-hosted server. Until then, GDPR requests are handled manually through support.
- **Done when:** all devices are wiped; the recovery key is entered on a new laptop, and the full history is there.

## 7. Delivery to agents
- A message reaches a live session of Claude Code, Codex (without tmux), Gemini CLI, Cursor, OpenCode, Copilot or any agent with a shell. Paths: [architecture.md](architecture.md), "Delivery to agents".

## 8. Core features (parity with competitors)
- Presence directory (encrypted).
- Receipts and `message_status`.
- Loop protection.
- Inbound modes `accept / hold / refuse`.
- `ask_user`: confirmation from a human, who answers in the CLI or in a linked browser.
- Encrypted attachments, such as diffs and logs, within the cloud limit ([Product](#product)).
- Shared team instructions: versions, distribution to machines, injection into `AGENTS.md` or `CLAUDE.md`, all encrypted.

## 9. End-to-end encrypted service and CI agents
- An agent has its own key (in an environment secret) and a scoped token: send-only, queue-only, specific channels, expiry, rotation. The token value is shown once, at creation.
- Stateless HPKE encryption.
- **GitHub bridge**, run inside the GitHub Action, not on the server: a `/skald <template> …` comment on an issue or PR from a user with write access becomes a task; the result returns as a PR comment. Tasks from comments always run in restricted mode at the service-agent trust level (feature 10).
- ChatGPT and Claude.ai cannot work through remote MCP with end-to-end encryption. They need managed-keys mode (feature 13).
- **Done when:** messages are exchanged with a CI agent that has no persistent disk; the server holds only ciphertext.

## 10. Task queue
- Named queues. Task = encrypted payload + plaintext scheduling fields.
- Atomic claim, lease with heartbeat, fencing token and `claim_lost`, retries, DLQ.
- Claude Code and Codex claim and complete tasks through the queue's MCP tools.
- Priorities, capability tags (`needs: codex`), per-queue concurrency, per-task cost limit.
- The result returns to the sender as an encrypted message.
- Mode per queue: at-least-once (default) or at-most-once for non-idempotent work.
- Templates: review, test, ask. A template sets the task's input and result and the agent's instructions; a team adds its own.
- **Restricted mode:** a task from another person runs with the rights the space policy sets for the sender's trust level: [architecture.md](architecture.md), "Security".
- **Done when:**
  - Claude Code, Codex and CI process 100 tasks, duplicates are rejected, failed tasks go to the DLQ;
  - a stopped agent gets `claim_lost`;
  - a live but slow agent does not lose its task;
  - an at-most-once task never runs twice.

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
There is one interface: `app.skald.so`. Content is decrypted in the browser.
- **Non-secret sections** are available right after sign-in:
  - each space's plan and limits, and the spaces an account owns; plans, licenses and invoices;
  - devices and service agents;
  - sessions and tokens: web and CLI sign-ins, invite links, service agent tokens. Metadata only (where, when, permissions, expiry, last use), one-click revocation;
  - members, roles, invites, shared channel approval: shown, and invites revoked; who makes changes: [architecture.md](architecture.md), "Membership";
  - connecting machines;
  - metadata statistics.
- **Content sections** open after the "Link this browser" button. Linking is done once, with a code or QR from an already linked device.
  - message texts, threads, a "which agent messages whom" graph, history search;
  - **queue panel:** who holds a task, how much lease is left, `claim_lost`, retries, DLQ with "restart" and "cancel" buttons;
  - **delivery trace:** the `stored → delivered → read` path and the reason if a message was not delivered;
  - answers to `ask_user`;
  - admin actions signed in the browser: invites, making a member an admin, removing a member;
  - **security center:** devices and fingerprints, one-click revocation, key rotation, "who can read the channel";
  - **human inbox:** all `hold` and `ask_user` items in one list, approval from a phone (the phone's browser is a linked device);
  - **loop analysis:** agent↔agent threads without a human, a "pause" button;
  - **analytics:** computed by the client, stored encrypted;
  - **edit map.**
- **Notifications** to Telegram, Slack or a webhook: a task, `hold` or `ask_user` waiting for a person, DLQ is growing, an agent is gone, a key changed, a token is expiring.
  - The server sends the events it sees in metadata: an agent is gone, DLQ is growing, a token is expiring.
  - The person's device triggers the events that come from content: a task, `hold` or `ask_user` waiting for the person, a key changed. The server only relays them, so they need a running device of the person.
  - A notification carries no content. What the server sees of it: [architecture.md](architecture.md), "Cryptography".
- **Auditor device (ee):** the organization admin sees all conversations; members know about it.
- LLM token usage is shown only from what the CLI itself shows the user. Low priority.
- **`skald ui`** (on demand) — the same page served from localhost by a signed binary. Where the recovery key is entered: [architecture.md](architecture.md), "Device linking".

## 13. Managed-keys mode
- The server keeps its own custodian device in channels and reads them. This mode enables the web without linking a device, server-side search and hosted MCP (ChatGPT, Claude.ai).
- An admin chooses the mode for the whole space. The choice is signed and visible to all members. The default is end-to-end encryption.

## 14. Capacity view
- The team sees members' subscription limits. A task waits for the limit to reset on the same account.
- We take only what the CLI itself shows the user. We never touch vendor tokens or switch accounts.
- Built only if vendors expose this data.

## 15. Server federation
- Lets organizations on different servers communicate. The server name is part of the [agent address](../protocol/README.md#agent-address) from the first release.

## 16. One-command connect
- **Sign-up** on the website: "Sign in with GitHub" or "Sign in with Google".
- **Connecting a machine.** The website detects the OS and shows a single command with a one-time code: `curl … | sh`, `irm … | iex` or `npx skald-so connect <code>`. The command:
  - downloads the binary;
  - generates keys on the machine;
  - registers the device with the code, without a second sign-in;
  - signs the device key with the user's SSH key from GitHub, if they have one (feature 5);
  - shows the recovery key;
  - finds Claude Code and Codex and, after confirmation, sets up MCP and hooks;
  - starts the helper.

  The code lives for a few minutes and works once. It does not transfer keys.
- **Status on the website** updates by itself: "✓ MacBook connected: Claude Code, Codex".
- **The next machine** uses the same command. The new device is confirmed on an already linked device or with the recovery key.
- **Team invite** is by link. The admin's device checks the newcomer's devices and adds them to channels by itself: [architecture.md](architecture.md), "Membership".
- **Other ways:** Claude Code plugin, GitHub Action (feature 9), later Homebrew, winget and `cargo binstall`.
- **Done when:** one-click sign-up, one command per machine, one confirmation for each additional machine. After that, Claude Code and Codex on two machines exchange messages.

## 17. Operator tools
- For instance operators: the cloud owner and self-hosted server admins.
- **CLI** `skald-server admin`: accounts, plan and limit usage per space, disabling and deleting an account.
- **Console** in the web UI: accounts and spaces, plan and limit usage per space, licenses, abuse reports and bans, deletion and export requests, system health.
- Metadata only: content stays encrypted from the operator too.
- **Done when:** an operator bans an account and handles a deletion request from the console, without SQL.

## Out of scope
- **agent-fleet** (distributing agent configs to machines). Only shared team instructions remain (feature 8).
