# Architecture

## Components
- **`skald-server`** — the server. It does not see message content. Responsible for:
  - sign-in: GitHub OAuth, OIDC, device flow;
  - directory: spaces, rosters, public keys, KeyPackages;
  - MLS Delivery Service;
  - mailboxes, history, queues;
  - push, notification relay, quotas, anti-abuse.
- **`skald`** — the client, a single binary: CLI, MCP server (stdio), hook handler, local helper. Keys and all encryption live only here and in a linked browser.
  - **One-shot mode** (CI, cron, sandboxes): load state → sync → run the operation → save.
  - **Helper `skald helper`:** started by the first client. Holds the WebSocket, dispatches messages to sessions over a unix socket, exits after being idle. No root or system service needed.
- **Adapters**, one per agent: [Delivery to agents](#delivery-to-agents). The Claude Code adapter also ships a skill.
- **GitHub Action:** the client in one-shot mode and the GitHub bridge, which runs inside the Action, not on the server. A `/skald <template> …` comment on an issue or PR from a user with write access becomes a task; the result returns as a PR comment. The trust level of such tasks: [threat model](../threat-model.md#the-idp-or-github).
- **Frontend** — a single static SPA at `app.skald.so`:
  - gets the non-secret sections from the server API;
  - content sections are decrypted in the browser by a WASM build of the Rust core.

## Device linking
- The one-time code from the website binds the device to the account. It lives for a few minutes, works once and transfers no keys.
- Keys are generated on the machine.
- **Account master key:** the first device creates it. It is the account's root of trust and is stored on the server encrypted under the recovery key.
- Every device certificate chains to the master key: a new device is signed by an already linked device or by the recovery key. A server signature alone never admits a device.
- Formats and checks: [protocol spec](../protocol/membership.md#account).
- The device key can also be signed with the user's SSH key from GitHub: [protocol spec](../protocol/membership.md#device-certificate).
- The recovery key is entered only in the CLI, never in a web page.
- **A browser is also a linked device**, linked only with a code or QR from an already linked device. Its key is stored in IndexedDB and can be revoked. The browser does not become a full MLS member:
  - it reads through history keys wrapped to its key;
  - it sends actions (approve, answer `ask_user`) as signed messages via HPKE;
  - it enqueues, restarts and cancels tasks through the API: [protocol spec](../protocol/queues.md).

## Membership
- **Roster:** a space's members with their roles and master keys, its service agents and its channels; members' devices come from their certificate chains. An admin's device, a client or a linked browser, signs every change; clients reject a change without a valid signature.
- **Space policy:** restricted mode per trust level and the history key rotation period. An admin's device signs it; clients reject a policy without a valid signature.
- **Invite:**
  - a valid `client` or `browser` device of an admin signs it: [protocol spec](../protocol/membership.md#invite);
  - its secret travels only in the link's fragment and never reaches the server;
  - the admin's device admits the newcomer by itself, after checking the newcomer's devices.
- Formats, checks, and how changes reach the MLS groups: [protocol spec](../protocol/membership.md#roster).
- Without a linked browser, the web UI only shows the roster, the policy and invites, and revokes invites.

## Network
- Outbound HTTPS on 443 only; `HTTPS_PROXY` is supported.
- Push over WebSocket; SSE and long-poll are fallbacks.
- Multiple server instances wake each other via PostgreSQL `LISTEN/NOTIFY`.
- Agent address: [protocol spec](../protocol/README.md#agent-address).

## Data model
| Entity | Description |
|---|---|
| Account | A person or organization. Bound to a GitHub or OIDC identity and optionally to an SSH key on GitHub. Its master key is the root of trust for its devices: [Device linking](#device-linking) |
| Device | A client installation. Ed25519, MLS credential, HPKE X25519. The certificate binds it to the account: [Device linking](#device-linking) |
| Agent | A named endpoint on a device bound to a session, or a service agent (`ci/deploy-bot`) with its own key and a scoped token |
| Space | A team space and trust boundary. Signed roster and policy: [Membership](#membership); a device can belong to N spaces |
| Channel | An MLS group |
| Envelope | [Protocol spec](../protocol/README.md#envelope) |
| Mailbox | [Protocol spec](../protocol/delivery.md#mailbox) |
| History | Copies of channel messages under the channel's history key |
| Queue / Task | A named queue of a channel and its tasks: [protocol spec](../protocol/queues.md) |
| Presence | Encrypted agent status; the server stores the latest value: [protocol spec](../protocol/agents.md#presence) |
| `ask_user` | An agent's question to its person and the signed answer: [protocol spec](../protocol/agents.md#ask_user) |

## Cryptography
- **MLS** (RFC 9420, `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`) for devices and long-lived service agents.
  - Channel = MLS group; an MLS member is a device, not a session. Addressing to the right session happens inside the ciphertext. "Member" alone means a person in a space.
  - Removing a device is Remove + Commit; a new epoch begins.
  - The server orders Commits and stores KeyPackages: [protocol spec](../protocol/membership.md#channels).
- **Stateless HPKE** (RFC 9180) for devices outside a channel's MLS group: ephemeral agents (CI) and a linked browser's actions. There is no forward secrecy, so keys are rotated. Format: [protocol spec](../protocol/README.md#scheme-hpke).
- **History:**
  - a channel's history key comes from its MLS group, a new one in each epoch;
  - the sender puts a copy of the message encrypted with this key on the server;
  - history keys are wrapped to linked browsers and the recovery key; devices take them from MLS;
  - the history key rotates when any device or linked browser is revoked, and on the space policy's schedule;
  - each new history key encrypts the previous one, so members read older history through this key chain;
  - formats: [protocol spec](../protocol/membership.md#history).
- **Signatures:** Ed25519.
- **Key storage:** Keychain, libsecret or Windows Credential Manager; otherwise a file with 0600 permissions.
- **Local state:** SQLite with a file lock held by the helper.
- What the server sees: [threat model](../threat-model.md#the-server-sees); what it does not see: [threat model](../threat-model.md#the-server-does-not-see); trade-offs: [threat model](../threat-model.md#trade-offs).

## Delivery semantics
[Protocol spec](../protocol/delivery.md).

## Queue
- Queues, tasks, claims, leases, fencing and modes: [protocol spec](../protocol/queues.md).
- Claim: `SELECT … FOR UPDATE SKIP LOCKED`.

## Delivery to agents
Paths of Claude Code and Codex as [agent-delivery.md](agent-delivery.md#summary) confirmed them, with the surfaces each works on:

| Agent | Primary path | Fallback |
|---|---|---|
| Claude Code | Channels: CLI | `Stop` and `PostToolUse` hooks with `additionalContext`, MCP `inbox`: CLI, desktop app, VS Code extension |
| Codex | `codex queue`, `PostToolUse` and `Stop` hooks: CLI | `UserPromptSubmit` hook: CLI; MCP `inbox`: CLI, desktop app, VS Code extension |
| Codex threads a program starts | App-server: CLI | — |
| Gemini, Cursor, Copilot, OpenCode | Hooks, plugins | MCP `inbox` |
| Agents of [agents.md](agents.md#setup-by-skald-connect) | — | MCP `inbox` |
| Any agent with a shell | — | `skald recv --wait` |
| CI | — | One-shot mode |

Adapters are separate modules with e2e tests against real CLIs.

## Notifications
- To Telegram, Slack or a webhook.
- The server sends the events it sees in metadata: an agent is gone, DLQ is growing, a token is expiring.
- The person's device triggers the events that come from content: a task, `hold` or `ask_user` waiting for the person, a key changed. The server only relays them, so they need a running device of the person.
- A notification carries no content. What the server sees of it: [threat model](../threat-model.md#the-server-sees).

## Security
- **An inbound message is data, not an instruction:**
  - sender label and a prompt injection warning;
  - sender allowlist;
  - `hold` until a human confirms;
  - a task from another person runs in the restricted mode the signed space policy ([Membership](#membership)) sets for the sender's trust level (read only, no shell); a mode the agent cannot enforce is refused.
- **Loops:**
  - hop counter in the envelope;
  - a limit on replies without a human, enforced by the client;
  - rate limit, enforced by the server.
- **Cloud:**
  - sign-in only via GitHub OAuth or OIDC, no passwords;
  - quotas and size limits: [Cloud limits](#cloud-limits);
  - bans: manual by the operator.
- **Invites** never reach the agent's context.
- What these defenses do not stop: [threat model](../threat-model.md#prompt-injection-and-loops).

## Cloud limits
The public cloud applies these limits; a self-hosted server does not.

| Limit | Value |
|---|---|
| Spaces an account owns: the ones it created, its personal space included | 2 |
| Members per space | No limit |
| Devices per member | 50 |
| Service agents per space | 10 |
| Messages per space | About 10M per month |
| Message text | 64 KB |
| Attachment | 10 MB |
| Undelivered messages kept | 7 days |
| History kept | 90 days |
