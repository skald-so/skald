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
- The one-time sign-in code from the website binds the device to the account. It lives for a few minutes, works once and transfers no keys.
- Keys are generated on the machine.
- **Account master key:** the first device creates it, once its user confirms it is the first. It is the account's root of trust and is stored on the server encrypted under the recovery key.
- Every device certificate chains to the master key: a new device is signed by an already linked device or by the recovery key. A server signature alone never admits a device.
- **Link code:** a new device shows a short code; its user enters it on an already linked `client` device. A PAKE keyed by the code carries the keys and the certificate, so the server never sees the code and cannot link a device itself.
- Formats and checks: [protocol spec](../protocol/membership.md#account).
- The device key can also be signed with the user's SSH key from GitHub: [protocol spec](../protocol/membership.md#device-certificate).
- The recovery key is entered only in the CLI, never in a web page.
- **A browser is also a linked device**, linked only with the link code it shows, entered on an already linked `client` device. Its key is stored in IndexedDB and can be revoked. The browser does not become a full MLS member:
  - it reads through history keys wrapped to its key;
  - it sends actions (approve, answer `ask_user`) as signed messages via HPKE;
  - it enqueues, restarts and cancels tasks through the API: [protocol spec](../protocol/queues.md).

## Membership
- **Roster:** a space's members with their roles and master keys, its service agents and its channels; members' devices come from their certificate chains. An admin's device, a client or a linked browser, signs every change; clients reject a change without a valid signature.
- **Space policy:** the most rights a task from another account or a service agent gets without the approval of the machine's user, and the history key rotation period. An admin's device signs it; clients reject a policy without a valid signature.
- **Handle:** a space creator's handle is its account's username, as the server gives it when the space is created.
- **Invite:**
  - an admin's device signs it: [protocol spec](../protocol/membership.md#invite);
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
| Account | A person or organization. Reached through its sign-in methods; optionally bound to an SSH key on GitHub. Its master key is the root of trust for its devices: [Device linking](#device-linking). Has its own username, which any of its sessions changes: `Username` in the [OpenAPI spec](../../api/openapi.json). Sign-up makes it from the GitHub login, or from the email's local part for Google: lowercased, each run of characters a [`name`](../protocol/README.md#agent-address) does not allow made `-`, cut to 63 characters, `-` trimmed at both ends, `user` if nothing is left; when that is taken or reserved, the first free of `-2`, `-3` and on is appended, within 63 characters |
| Sign-in method | A provider and the subject it gives the person: GitHub and the GitHub user's ID, or an OIDC provider by its issuer and the ID token's `sub`. Belongs to one account; the first sign-in with it creates the account, the same way for every provider. A GitHub method keeps the GitHub login for display, updated at each sign-in |
| Device | A client installation. Ed25519, MLS credential, HPKE X25519. The certificate binds it to the account: [Device linking](#device-linking) |
| Agent | A named endpoint on a device bound to a session, or a service agent (`ci/deploy-bot`) with its own key and a scoped token |
| Space | A team space and trust boundary. Signed roster and policy: [Membership](#membership); a device can belong to N spaces |
| Channel | An MLS group |
| Envelope | [Protocol spec](../protocol/README.md#envelope) |
| Mailbox | [Protocol spec](../protocol/delivery.md#mailbox) |
| History | Copies of channel messages under the channel's history key |
| Queue / Task | A named queue of a channel and its tasks: [protocol spec](../protocol/queues.md) |
| Presence | Encrypted agent status and the queues a device serves; the server stores the latest value: [protocol spec](../protocol/agents.md#presence) |
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
- **Key storage:** Keychain, the Secret Service or Windows Credential Manager; otherwise a file in the state directory, with 0600 permissions on Unix; on Windows it has the access rights it inherits from the directory.
- **Local state:** SQLite with a file lock held by the helper.
- **MLS state:** a device's groups, KeyPackages' private keys and history keys are stored only sealed, under a key in key storage; the store overwrites the previous sealed state and deletes it securely, so no older one is kept.
- What the server sees: [threat model](../threat-model.md#the-server-sees); what it does not see: [threat model](../threat-model.md#the-server-does-not-see); trade-offs: [threat model](../threat-model.md#trade-offs).

## Delivery semantics
[Protocol spec](../protocol/delivery.md).

## Queue
- Queues, tasks, claims, leases, fencing and modes: [protocol spec](../protocol/queues.md).
- Claim: `SELECT … FOR UPDATE SKIP LOCKED`.
- `skald send` sends a message and `skald enqueue` a task; the MCP server has both as tools, with `claim` and `complete` for tasks.
- `skald connect` asks only whether the machine takes its own account's tasks. What it takes from others, [per queue](../protocol/queues.md#running-a-task), the user turns on and changes at any time with `skald serve`, only in the CLI on the machine:
  - `skald serve`: each queue's settings;
  - `skald serve <queue> --dir <folder> --member read-only`, and `--service` for service agents;
  - `skald serve <queue> --member approve --in session`;
  - `skald serve <queue> --off`.
- `skald approve` lists the tasks waiting for the user's yes and answers them.
- The web UI shows each device's queues and tiers from its presence.

## Delivery to agents
For users, per surface, with what reaches a session from whom: [Delivery to agent sessions](../agent-sessions.md); the rule by the sender's trust level: [delivery.md, "Device to agent"](../protocol/delivery.md#device-to-agent). Paths of Claude Code and Codex as [agent-delivery.md](agent-delivery.md#summary) confirmed them, with the surfaces each works on:

| Agent | Primary path | Fallback |
|---|---|---|
| Claude Code | Cross-session socket: CLI, desktop app, VS Code extension. Channels: CLI | `asyncRewake` hook, mod, `/loop`, `Stop` and `PostToolUse` hooks with `additionalContext`, MCP `inbox`: CLI, desktop app, VS Code extension |
| Codex | `codex queue`, `PostToolUse` and `Stop` hooks: CLI, desktop app, VS Code extension | `UserPromptSubmit` hook, MCP `inbox`: CLI, desktop app, VS Code extension. Scheduled task: desktop app |
| Codex threads a program starts | App-server: CLI | — |
| Gemini, Cursor, Copilot, OpenCode | Hooks, plugins | MCP `inbox` |
| Agents of [agents.md](agents.md#setup-by-skald-connect) | — | MCP `inbox` |
| Any agent with a shell | — | `skald recv --wait` |
| CI | — | One-shot mode |

Adapters are separate modules with e2e tests against real CLIs.

`read` is issued once the agent's session file shows the message in an entry that went to the model, followed by the model's own entry, then on weaker signals; an unconfirmed message goes again by the next path: [agent-delivery.md, "Read confirmations"](agent-delivery.md#read-confirmations).

## Notifications
- To Telegram, Slack or a webhook.
- The server sends the events it sees in metadata: an agent is gone, DLQ is growing, a token is expiring.
- The person's device triggers the events that come from content: a task waiting for the person's yes in `skald approve`, a `hold` or `ask_user` waiting for the person, a key changed. The server only relays them, so they need a running device of the person.
- A notification carries no content. What the server sees of it: [threat model](../threat-model.md#the-server-sees).

## Security
- **The sender, not the kind of text, sets its weight** ([delivery.md, "Device to agent"](../protocol/delivery.md#device-to-agent)):
  - sender label with its trust level, and a prompt injection warning;
  - text from another account or a service agent only through `inbox`, labelled as data;
  - sender allowlist;
  - `hold` until a human confirms.
- **Tasks from others** ([queues.md, "Running a task"](../protocol/queues.md#running-a-task)):
  - run nowhere until the machine's user turns them on per queue;
  - run in an executor by default, a new session without the user's context, environment, MCP servers, hooks and plugins;
  - without the user's yes, get at most the signed space policy's ceiling ([Membership](#membership));
  - the agent CLI enforces the mode; Skald adds no sandbox, and a mode the agent cannot enforce is refused;
  - a task that needs more rights stops at once and waits for the user's yes.
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
