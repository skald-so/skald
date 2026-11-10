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
- **GitHub Action:** the client in one-shot mode and the GitHub bridge ([features.md §9](features.md#9-end-to-end-encrypted-service-and-ci-agents)).
- **Frontend** — a single static SPA at `app.skald.so`:
  - gets the non-secret sections from the server API;
  - content sections are decrypted in the browser by a WASM build of the Rust core;
  - `skald ui` (on demand) — the same build served from localhost by a signed binary.

## Device linking
- The one-time code from the website ([features.md §16](features.md#16-one-command-connect)) binds the device to the account.
- Keys are generated on the machine.
- **Account master key:** the first device creates it. It is the account's root of trust and is stored on the server encrypted under the recovery key.
- Every device certificate chains to the master key: a new device is signed by an already linked device or by the recovery key. A server signature alone never admits a device.
- Formats and checks: [protocol spec](../protocol/membership.md#account).
- The device key can also be signed with the user's SSH key from GitHub ([features.md §5](features.md#5-identity)).
- The recovery key is entered only in the CLI and `skald ui`, never in a web page.
- **A browser is also a linked device**, linked only with a code or QR from an already linked device. Its key is stored in IndexedDB and can be revoked. The browser does not become a full MLS member:
  - it reads through history keys wrapped to its key;
  - it sends actions (approve, answer `ask_user`) as signed messages via HPKE;
  - it enqueues, restarts and cancels tasks through the API: [protocol spec](../protocol/queues.md).

## Membership
- **Roster:** a space's members with their roles and master keys, its service agents and its channels; members' devices come from their certificate chains. An admin's device, a client or a linked browser, signs every change; clients reject a change without a valid signature.
- **Space policy:** restricted mode per trust level, the history key rotation period, the space mode ([protocol spec](../protocol/README.md#custodian-auditor-and-federation)), the edit-coordination action ([features.md §11](features.md#11-cross-machine-edit-coordination)). An admin's device signs it; clients reject a policy without a valid signature.
- **Invite:**
  - an admin's device signs it;
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
| Channel | An MLS group. A shared channel joins two spaces |
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
- **Edit coordination:** the server sees only HMAC fingerprints of paths, hunks and symbols, computed with the space key.
- **The custodian device** in a managed-keys space: [protocol spec](../protocol/README.md#custodian-device).
- **The server sees:**
  - accounts: sign-in methods and usernames;
  - the roster: members, roles, devices and service agents of each space, and the devices of each channel;
  - devices: kind, name, public keys and certificates, the agent CLIs found on them;
  - service agent names and token permissions;
  - invites, sign-in sessions and tokens: who, when, from where, expiry, last use;
  - IP addresses and times of connections;
  - queue names and settings, task tags and templates;
  - when presence changes, and its size in steps of 4 KB, not its value;
  - notifications: the address, and the class of each event a device triggers; for `ask_user`, so which envelopes likely carry the question;
  - HMAC fingerprints of edits;
  - space, channel and device identifiers;
  - which device messages whom;
  - message size and time;
  - when each device takes its messages;
  - which envelopes ask for delivery statuses, so which are likely receipts, and which hold MLS handshake messages;
  - attachment sizes, uploads and downloads;
  - task scheduling fields and states, and which device enqueues, claims and completes each task;
  - quota counters;
  - each space's plan, member count and billing contact; card data stays with the payment provider.
- **The server does not see:** message texts, agent and session names other than service agents, threads, files, repositories, presence values, task payloads, edited paths and code.
- **Trade-offs:**
  - HPKE messages (ephemeral agents, browser actions, `ask_user` questions and answers) have no forward secrecy, and the server can drop an ephemeral agent's message unnoticed;
  - in managed-keys mode the server reads the space and writes as the agents delegated to it; a delegation lasts until it expires or its signing device is revoked;
  - a leaked recovery key exposes the history;
  - the history is outside MLS forward secrecy: [features.md §4](features.md#4-mls-group-encryption);
  - metadata is visible; sealed sender and padding come later;
  - on the web the server delivers the JS, so a linked browser trusts the served build; the build is reproducible ([infrastructure.md](infrastructure.md), "Trust in builds"), and `skald ui` serves it from a signed binary;
  - there has been no external crypto audit; the spec and threat model are public;
  - the limits of trust in devices, rosters and history: [protocol spec](../protocol/membership.md#security-considerations).

## Delivery semantics
[Protocol spec](../protocol/delivery.md).

## Queue
- Queues, tasks, claims, leases, fencing and modes: [protocol spec](../protocol/queues.md).
- Claim: `SELECT … FOR UPDATE SKIP LOCKED`.

## Delivery to agents
| Agent | Primary path | Fallback |
|---|---|---|
| Claude Code | Channels | `Stop` and `PostToolUse` hooks with `additionalContext`, MCP `inbox` |
| Codex | `PostToolUse`, `Stop`, `UserPromptSubmit` hooks; for background agents, app-server (`turn/start`, `turn/steer`) | MCP `inbox`, `codex queue` (to verify) |
| Gemini, Cursor, Copilot, OpenCode | Hooks, plugins | MCP `inbox` |
| Any agent with a shell | — | `skald recv --wait` |
| CI | — | One-shot mode |

Adapters are separate modules with e2e tests against real CLIs.

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
  - quotas and size limits: [features.md](features.md), "Product";
  - bans: manual by the operator; abuse reports and automatic detection.
- **Invites** never reach the agent's context.
