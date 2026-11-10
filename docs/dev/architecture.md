# Architecture

## Components
- **`skald-server`** — the server. It does not see message content. Responsible for:
  - sign-in: GitHub OAuth, OIDC, device flow;
  - directory: spaces, rosters, public keys, KeyPackages;
  - MLS Delivery Service;
  - mailboxes, history, queues;
  - push, quotas, anti-abuse.
- **`skald`** — the client, a single binary: CLI, MCP server (stdio), hook handler, local helper. Keys and all encryption live only here and in a linked browser.
  - **One-shot mode** (CI, cron, sandboxes): load state → sync → run the operation → save.
  - **Helper `skald helper`:** started by the first client. Holds the WebSocket, dispatches messages to sessions over a unix socket, exits after being idle. No root or system service needed.
- **Adapters:**
  - Claude Code — MCP, Channels, hooks, skill;
  - Codex — MCP, hooks, app-server;
  - Gemini CLI, Cursor, OpenCode, Copilot — MCP and hooks;
  - GitHub Action;
  - everything else — via the CLI.
- **Frontend** — a single static SPA at `app.skald.so`:
  - gets the non-secret sections from the server API;
  - content sections are decrypted in the browser by a WASM build of the Rust core;
  - `skald ui` (on demand) — the same build served from localhost by a signed binary.

## Device linking
- The one-time code from the website lives for a few minutes and works once. It binds the device to the account.
- Keys are generated on the machine.
- A new device is signed by an already linked device (cross-signing) or by the recovery key.
- **A browser is also a linked device**, confirmed with a code or QR. Its key is stored in IndexedDB and can be revoked. The browser does not become a full MLS member:
  - it reads through history keys wrapped to its key;
  - it sends actions (approve, restart a task) as signed messages via HPKE.

## Network
- Outbound HTTPS on 443 only; `HTTPS_PROXY` is supported.
- Push over WebSocket; SSE and long-poll are fallbacks.
- Multiple server instances wake each other via PostgreSQL `LISTEN/NOTIFY`.
- Agent address: [protocol spec](../protocol/README.md#agent-address).

## Data model
| Entity | Description |
|---|---|
| Account | A person or organization. Bound to a GitHub or OIDC identity and optionally to an SSH key on GitHub |
| Device | A client installation. Ed25519, MLS credential, HPKE X25519. The certificate is signed by the account and the server |
| Agent | A named endpoint on a device bound to a session, or a service agent (`ci/deploy-bot`) with its own key and a scoped token |
| Space | A team space and trust boundary. Signed roster; a device can belong to N spaces |
| Channel | An MLS group. A shared channel joins two spaces |
| Envelope | [Protocol spec](../protocol/README.md#envelope) |
| Mailbox | `mailbox(device_id, seq, envelope_id, ciphertext_ref, expires_at)`: undelivered messages only, deleted on ack |
| History | Copies of channel messages under the channel's history key |
| Queue / Job | Encrypted payload + plaintext scheduling fields: state, priority, attempts, `lease_until`, tags, `idempotency_key`, deadline |
| Presence | Encrypted agent status; the server stores the latest value |

## Cryptography
- **MLS** (RFC 9420, `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`) for devices and long-lived service agents.
  - Channel = MLS group; a group member is a device, not a session. Addressing to the right session happens inside the ciphertext.
  - Removing a member is Remove + Commit; a new epoch begins.
  - The server orders Commits and stores KeyPackages.
- **Stateless HPKE** (RFC 9180) for devices outside a channel's MLS group: ephemeral agents (CI) and a linked browser's actions. There is no forward secrecy, so keys are rotated. Format: [protocol spec](../protocol/README.md#scheme-hpke).
- **History:**
  - a channel has a symmetric history key, distributed inside MLS;
  - the sender puts a copy of the message encrypted with this key on the server;
  - history keys are wrapped to the account's devices and to the recovery key.
- **Signatures:** Ed25519.
- **Key storage:** Keychain, libsecret or Windows Credential Manager; otherwise a file with 0600 permissions.
- **Local state:** SQLite with a file lock held by the helper.
- **Edit coordination:** the server sees only HMAC fingerprints of paths, hunks and symbols, computed with the space key.
- **The custodian device** in a managed-keys space: [protocol spec](../protocol/README.md#custodian-device).
- **The server sees:**
  - space, channel and device identifiers;
  - which device messages whom;
  - message size and time;
  - task scheduling fields;
  - quota counters.
- **The server does not see:** message texts, agent and session names, threads, files, repositories, presence, task payloads.
- **Trade-offs:**
  - HPKE messages (ephemeral agents, browser actions) have no forward secrecy, and the server can drop an ephemeral agent's message unnoticed;
  - in managed-keys mode the server reads the space and writes as the agents delegated to it; a delegation lasts until it expires or its signing device is revoked;
  - a leaked recovery key exposes the history;
  - metadata is visible; sealed sender and padding come later;
  - on the web the server delivers the JS, so reproducible builds and SRI are needed;
  - there has been no external crypto audit; the spec and threat model are public.

## Delivery semantics
- No end-to-end exactly-once: every hop is at-least-once with deduplication.
- **Send:** the client keeps the message in a local outbox and retries until the server answers; the server drops duplicates by `idempotency_key` (kept 7 days) and answers `stored` after the commit.
- **Server → device:** WebSocket push or catch-up from the cursor; the device acks by `seq` after writing the message to its local store; the server then deletes it from the mailbox; a redelivered message is dropped by `envelope_id`.
- **Device → agent:** the helper hands the message to the agent (Channels, hooks, MCP `inbox`) and marks it handed over; a crash in between may repeat the handover, never lose it.
- **Receipts:**

| Status | Meaning | Issued by |
|---|---|---|
| `stored` | Committed on the server | Server |
| `delivered` | Written on the recipient device | Server, from the device ack |
| `read` | Handed to the agent | Recipient device, signed, inside the channel's encryption |
| `replied` | A reply with `reply_to` arrived | The reply itself |
| `held`, `refused` | The recipient's inbound mode | Recipient device, signed |
| `expired` | Not delivered within the retention period (cloud: 7 days) | Server |

- Every message ends in a status the sender sees; nothing is lost silently.
- **Order:** total within a channel, set by the server; none across channels.
- **Gap detection:** the per-sender counter inside the ciphertext lets the recipient report a missing message, so the server cannot drop one unnoticed.
- `read` means handed to the agent, not acted on. Work that must be done goes through the queue.
- **Durability:** `stored` is sent after the replica has the write ([infrastructure.md](infrastructure.md), "Cloud").

## Queue
- Claim: `SELECT … FOR UPDATE SKIP LOCKED`.
- A heartbeat extends the lease.
- Fencing:
  - `claim_id` + an increasing generation;
  - extension and completion via compare-and-swap;
  - whoever lost the lease gets `claim_lost`.
- Mode per queue:
  - **at-least-once** (default): an expired lease or a failure returns the task to the queue; retries with exponential backoff, then DLQ; the handler must be idempotent;
  - **at-most-once**, for non-idempotent work (deploy, sending mail): an expired lease or a failure sends the task to the DLQ without a retry; a human decides.
- `idempotency_key` on enqueue; the agent can use `task_id` to make its own side effects idempotent.

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
  - a task from another person runs in the restricted mode the space policy sets for the sender's trust level (read only, no shell); a mode the agent cannot enforce is refused.
- **Loops:**
  - hop counter in the envelope;
  - a limit on replies without a human, enforced by the client;
  - rate limit, enforced by the server.
- **Cloud:**
  - sign-in only via GitHub OAuth or OIDC, no passwords;
  - quotas;
  - text up to 64 KB, attachments up to 10 MB;
  - device limit;
  - bans based on reports.
- **Invites** never reach the agent's context.
