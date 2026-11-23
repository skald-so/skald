# Architecture

## Components
- **`skald-server`** — the server. It does not see message content. Responsible for:
  - sign-in: GitHub OAuth, OIDC, device flow;
  - directory: spaces, rosters, public keys, KeyPackages;
  - MLS Delivery Service;
  - mailboxes, history, queues;
  - push, notification relay, quotas, anti-abuse.
- **`skald`** — the client, a single binary: CLI, MCP server (stdio), hook handler, local helper. Keys and all encryption live only here and in a linked browser.
  - **One-shot mode** (CI, cron, sandboxes; `--one-shot` or `SKALD_ONE_SHOT=1`): load state → sync → run the operation → save. Such a command starts no helper, but goes through one that runs.
  - **Helper `skald helper`:** one per local state, started in the background by the first command that connects as the device, such as `skald send` or `skald recv`. Holds the device's connection over any transport and the local state's lock, dispatches messages to sessions. No root or system service needed.
    - Each command that changes the local state hands its operation, working directory and `--agent` to the helper over a unix socket in the state directory, and shows its output and exit code. A command whose helper cannot start runs one-shot.
    - `skald link` takes its person's answers in its own process and hands each step after them to the helper, which keeps the link code's run between steps; without a helper, it holds the lock for the whole run.
    - The helper reaches the server with the environment, such as `HTTPS_PROXY`, of the command that started it.
    - While idle, it keeps the device's connection open. Once the server closes it because the account is deleted, the helper says so and exits. A command whose session the server refuses asks it, by a session by the device's key, whether the account is deleted ([Device key session](../protocol/membership.md#device-key-session)). Once either learned it, each command that reaches the server says so, naming the state directory that `skald delete-account` erases, and sends nothing.
    - A helper older than a command exits for it; a command older than the helper fails, naming both versions, and the helper keeps running.
    - A `skald recv --wait` waits through it as a session. The helper takes in each arrival once for all its sessions and hands each its own, marking a message handed over once the session has written it out; KeyPackages, GroupInfo and the revocation check stay off that path. A one-shot `skald recv --wait` holds its own connection, takes the lock only while it receives, not while it waits, and goes on through a helper that starts meanwhile.
    - A Claude Code session registers with it through its `SessionStart` hook: the agent, the folder, the session's cross-session socket, its token and its session file. The helper writes each message for the agent into the socket of its last registered session that runs, and marks it handed over once the session file confirms it ([agent-delivery.md, "Read confirmations"](agent-delivery.md#read-confirmations)). `SessionEnd`, or a socket that is gone or takes no connection, ends the registration.
    - On Unix it writes through the session's `skald mcp`, which relays only lines with the session's token: on Linux a session that bypasses permission prompts takes a line as its own only from a process Claude Code started, whatever the token. Without it the helper writes into the socket itself.
    - A Codex session registers with the device through its hooks, `skald hook codex`, as the agent its `skald mcp` registers: its thread, the session file and config home its hooks give, the folder's space and the Codex process that runs its hooks, past the npm package's launcher and, on Windows, shells. `SessionStart` starts the helper when none runs, without waiting for it; in a folder in no space nothing registers. The session lives while that process runs, and keeps the helper running; the helper takes in what comes for its space. Its hooks never wait on the network: they read and write the local state without its lock, and one that has not finished within 3 seconds prints nothing.
    - A Codex session is busy from a hook of its turn until the `Stop` that ends the turn. Each message for the agent goes to its live session whose hook ran last before the agent's first hook, or the helper's look for idle sessions, after the device took the message in. Every message, own included, goes in only as the signal ([protocol](../protocol/delivery.md#device-to-agent)): a fixed line saying that messages wait and `inbox` takes them, with no sender, text, count or reference. `PostToolUse` and `UserPromptSubmit` add it as context and `Stop` continues the turn with it, once for the messages whose signal did not go; otherwise every hook prints nothing.
    - An idle Codex session with messages whose signal did not go gets the signal from the helper by `codex queue` on its thread, which starts a turn. The helper runs the `codex` of the session's Codex process, so a desktop app's or an IDE extension's bundled Codex serves its own sessions, on the session's config home. A session has one queued signal until its next hook. A `codex queue` that fails leaves the messages to the hooks and `inbox`. It queues only the signal: [threat model](../threat-model.md#another-account-on-the-machine).
    - It keeps the device's presence in each channel whose group it is in ([protocol](../protocol/agents.md#presence)): each agent registered in the space, `available` or `busy` while a session of it runs through the helper, an ended `skald recv --wait` for 30 seconds more, otherwise `offline`; while it holds a claim, `busy` with `started` and `waiting` with `awaiting-approval`. It uploads in every channel together when a state or note changes and before `expires` (900 seconds ahead) passes, in one channel on entering a new epoch, and its agents `offline` before it exits. A value is sealed only under the current epoch judged clean; `revision` is recorded before the upload. A failed upload is retried within 30 seconds, silently.
    - Without the helper, each run as an agent checks the presence once, after the command, its agents `offline`; it uploads only on a change or near `expires`. A revoked device uploads nothing.
    - It sends the lease heartbeat ([protocol](../protocol/queues.md#claims-and-leases)) of each claim of an agent while a session of it runs through the helper, as for presence: an extend each third of the lease, with the progress of the claim's last extend. Without the helper, a one-shot `skald mcp` or `skald recv --wait` extends its agent's claims itself, looking each second. An extend that gets `claim-lost` or a lasting refusal, or a lease that passed on the device's clock, marks the claim lost; the agent's next tool result or `skald recv` says so.
    - It exits after 10 minutes idle, with no Claude Code session registered and no Codex session live, on sign-out and before a sign-in.
    - Only the OS account that runs it reaches its socket: [threat model](../threat-model.md#another-account-on-the-machine).
  - **MCP server `skald mcp`:** an agent CLI starts it from its config, on stdio; its session's folder is its working directory ([agent-sessions.md](../agent-sessions.md#the-mcp-server)).
    - Before it answers `initialize`, or `server/discover` from a client that opens without `initialize` (MCP 2026-07-28), the session registers as its agent in the folder's space: the messages the device takes in from then on are kept for it.
    - Without `--agent`, the agent's name joins with `-` the name the client gives itself at `initialize` or in a request's `_meta`, the device's certificate `name` and the device's reference: the first 6 characters of the base32 of SHA-256 of the device ID, as a message's reference is made. Each name is made as a username is ([Data model](#data-model)) and left out if empty; over 63 characters, the device's name is cut from its end, then the client's. So each device of the account gives an agent CLI its own address.
    - `inbox` returns the agent's lines of `skald recv`, written in one write, and marks them handed over once the result is written.
    - `reply` replies as the agent's `skald reply` does and returns what it says.
    - `set_status` sets the agent's state and note as its `skald set-status` does. A new session starts `available`, with no note.
    - `message_status` returns what the agent's `skald message-status` says.
    - Out of one-shot mode, the server holds a session of the helper for its whole run, starting it again if it exits: the helper knows the agent runs, takes in arrivals for it and does not exit idle under it.
    - Each tool call goes through the helper or runs one-shot, as a command does, and through a newer helper too. Once the `skald` installed where the server's binary was is newer, as after an update while the server runs, the server starts it from there for each call, which it hands over as to a helper.
- **Adapters**, one per agent: [Delivery to agents](#delivery-to-agents). The Claude Code adapter also ships a skill.
- **GitHub Action:** the client in one-shot mode and the GitHub bridge, which runs inside the Action, not on the server. A `/skald <template> …` comment on an issue or PR from a user with write access becomes a task; the result returns as a PR comment. The trust level of such tasks: [threat model](../threat-model.md#the-idp-or-github).
- **Frontend** — a single static SPA at `app.skald.so`:
  - its pages' paths start with none of the first segments of the OpenAPI spec's paths, such as `/device`, so one origin serves the SPA and the API;
  - a self-hosted server serves its release's SPA on its own domain: [self-host.md](../self-host.md#the-web-app);
  - without a web session, every page shows the sign-in page;
  - gets the non-secret sections from the server API;
  - content sections are decrypted in the browser by a WASM build of the Rust core.

## Device linking
- The one-time sign-in code from the website binds the device to the account. It lives for a few minutes, works once and transfers no keys.
- Keys are generated on the machine.
- **Account master key:** a new device creates it when the server reports none for the account; the device asks its user nothing. It is the account's root of trust and is stored on the server encrypted under the recovery key.
- Every device certificate chains to the master key: a new device is signed by an already linked device or by the recovery key. A server signature alone never admits a device.
- **Link code:** a new device shows a short code; its user enters it on an already linked `client` device. A PAKE keyed by the code carries the keys and the certificate, so the server never sees the code and cannot link a device itself.
- **A further machine needs only a link code.** `skald sign-in` on a machine without a device shows the device flow and a 12-character link code at once, asking nothing; the first to finish wins:
  - the code: the device stores its certificate and gets a session by its key ([protocol spec](../protocol/membership.md#device-key-session)); the device flow is dropped;
  - the browser: on an account without a master key, the device is its first, and its code's run ends; with `--with-recovery-key`, it links with the recovery key; otherwise it keeps waiting for the code, and linked devices say it waits.
- Formats and checks: [protocol spec](../protocol/membership.md#account).
- The device key can also be signed with the user's SSH key from GitHub: [protocol spec](../protocol/membership.md#device-certificate).
- The recovery key is 12 characters, shown as 3 groups of 4. It is typed or pasted only in the CLI, never in a web page.
- **Local page:** `skald sign-in --browser` and `skald link --browser` take the link code's steps on a page `skald` opens in the browser, as their person or an agent runs them: the code and a fresh one, the reminder and the new device's confirmation; the sign-in page also shows the browser sign-in's link and a waiting recovery key. The commands' output holds no code, confirmation or recovery key; the page takes no recovery key.
  - It listens only on 127.0.0.1, on a port the system picks, while its command runs.
  - It answers only requests whose query holds its one-time secret, with its own Host and its own Origin or none; every answer carries `Cache-Control: no-store`.
  - `skald` never prints the secret: the browser opens a file in the state directory, readable only by its user, that takes it to the page, as Jupyter does; the file is deleted when the command ends.
  - Without a browser to open, the command says so and to run it in a terminal, and sends nothing.
  - An agent's run shows its warnings only on the page and keeps them for the person's `skald recv`.
- **A browser is also a linked device**, linked only with the link code it shows, entered on an already linked `client` device. Its key is stored in IndexedDB and can be revoked. The browser does not become a full MLS member:
  - it reads through history keys wrapped to its key;
  - it sends the person's messages to their own agents and actions (approve, answer `ask_user`) as signed messages via HPKE;
  - it enqueues, restarts and cancels tasks through the API: [protocol spec](../protocol/queues.md).

## Membership
- **Roster:** a space's members with their roles and master keys, its service agents and its channels; members' devices come from their certificate chains. An admin's device, a client or a linked browser, signs every change; clients reject a change without a valid signature.
- **Roles:**
  - only an admin's device changes a role: `skald make-admin` makes a member an admin; `skald hand-over-admin` also makes this account a member, in the same revision;
  - a device linked with the recovery key is an admin's device wherever its account is an admin;
  - the server cannot change a role or sign a revision: when a space's only admin loses every device and the recovery key, its members make a new space and still read the old one's history.
- **Space policy:** the most rights a task from another account or a service agent gets without the approval of the machine's user, the history key rotation period, and the most waking delivery mode per sender trust level. An admin's device signs it; clients reject a policy without a valid signature.
- **Handle:** a space creator's handle is its account's username, as the server gives it when the space is created.
- **Personal space:**
  - each account has one, which it owns; the server creates it at the account's first request for it;
  - its name is the account's username, numbered as a username is when another space has that name: [Data model](#data-model);
  - the account's first device signs its revision 1 once the server has taken the device's certificate, and pins it as its personal space in its local state;
  - a device never takes a personal space from the server's word: it pins the one whose revision 1 it signed, or one as the protocol spec's [Linking a device](../protocol/membership.md#linking-a-device) and [Anchors](../protocol/membership.md#anchors) say; otherwise it has none.
- **Default channel:**
  - every space has one: the first channel the roster lists, with every member of the space, each with `history` true; later revisions keep it first, and every invite names it;
  - an admin's device that accepts a revision of a space without a channel, or whose first channel lacks a member, signs the next revision with a new default channel first and creates its MLS group.
- **Folder binding:**
  - a session is in the space of its folder, its working directory;
  - a rule in the device's local state binds a folder and everything below it to a space, such as `~/work/clientA → clientA`;
  - so does a `.skald.toml` in the folder, `space = "<name>"`, once the device's user approved it in the CLI as its bytes are; a changed one binds nothing until approved again, so a cloned repository cannot move its sessions to another space;
  - the nearest folder with a rule or an approved `.skald.toml` decides; at the same folder, the rule;
  - a folder nothing binds is in the device's pinned personal space; without one, in no space;
  - a binding to a space the device is not in leaves the folder in no space, never in another;
  - folders compare as canonical paths, so a symlink binds as its target.
- **Invite:**
  - an admin's device signs it: [protocol spec](../protocol/membership.md#invite);
  - it admits one person;
  - its secret travels only in the link's fragment and never reaches the server;
  - the admin's device admits the newcomer by itself, after checking the newcomer's devices.
- Formats, checks, and how changes reach the MLS groups: [protocol spec](../protocol/membership.md#roster).
- Without a linked browser, the web UI only shows the roster, the policy and invites, and revokes invites.

## Network
- Outbound HTTPS on 443 only.
- The client reaches its server through the proxy `HTTPS_PROXY` (or `https_proxy`) names, by an HTTP `CONNECT` tunnel; TLS stays end to end:
  - a server whose host `NO_PROXY` (or `no_proxy`) lists, as curl reads it, and plain HTTP to this machine take no proxy;
  - credentials in the proxy's URL go as `Proxy-Authorization: Basic`; no other scheme;
  - a proxy that fails fails the command, named by its host and port only; the client never connects directly instead.
- The client speaks plain HTTP only to its own machine: `localhost`, `127.0.0.0/8`, `[::1]`; to any other server, HTTPS only.
- The client takes the login and public SSH keys of each GitHub user a certificate names from GitHub's API, unauthenticated, as it reaches its server, to check the certificate's SSH signature ([Device certificate](../protocol/membership.md#device-certificate)). It checks each certificate once and keeps the result; one it cannot check, as with GitHub unreachable or at its rate limit, waits for a later run, and fails no command. A device that signs its own certificate takes its GitHub user's ID and keys there the same way.
- The client looks up the records of a space's [domain](../protocol/membership.md#domain) with DNS over HTTPS at the resolver the setting `SKALD_DNS_RESOLVER` names, Cloudflare's `https://cloudflare-dns.com/dns-query` by default, as it reaches its server. It keeps them by resolver, so a run with another one, as an agent could make, changes nothing later runs show; a resolver that fails fails no command.
- `localhost` is always 127.0.0.1 and ::1: the client asks no resolver for it.
- Push over WebSocket; SSE and long-poll are fallbacks. A long-poll waits at most the server setting `--long-poll-max-wait` (`SKALD_LONG_POLL_MAX_WAIT`), 30 seconds by default. An idle SSE stream sends a keep-alive every `--sse-keep-alive` (`SKALD_SSE_KEEP_ALIVE`), 15 seconds by default.
- The client falls back to SSE, then long-poll, when the transport before does not open, such as when a proxy refuses it, or breaks or sends nothing for the request timeout before its catch-up ends. Over both it sends and acks in HTTPS requests. A run keeps the transport that worked and its connection, and the helper keeps it between commands; a catch-up on a WebSocket after its first takes the entries it pushed since, and on an SSE stream goes over long-poll. A dropped connection tries WebSocket first again; so do the helper and a one-shot `skald recv --wait` on SSE or long-poll after a minute. History items wait for a run over WebSocket ([delivery](../protocol/delivery.md#frames)).
- Multiple server instances wake each other via PostgreSQL `LISTEN/NOTIFY`:
  - a notification names a mailbox with new entries, a stored roster revision, an ended session, a revoked device, a deleted account's device, a space with a member's account deleted or a space with a new acceptance, never an envelope;
  - an instance takes no device connection before it first listens;
  - an instance whose listening connection drops or goes silent listens again, then sends its connections what they missed;
  - each instance takes up to 11 PostgreSQL connections: 10 in its pool and 1 that listens; a replaced listening connection can stay open for a while after its replacement.
- A server's addresses: the public address, the setting `--public-url` (`SKALD_PUBLIC_URL`), and the web address, `--web-url` (`SKALD_WEB_URL`), by default the public one. Browser sign-in is on the web address: `info` in the [OpenAPI spec](../../api/openapi.json). Devices prove their keys over the public address ([delivery](../protocol/delivery.md)), and the server's name defaults to its host. A self-hosted server leaves the web address unset.
- Agent address: [protocol spec](../protocol/README.md#agent-address). Its `server` is the server setting `--server-name` (`SKALD_SERVER_NAME`): `getServer` in the [OpenAPI spec](../../api/openapi.json).

## Data model
| Entity | Description |
|---|---|
| Account | A person or organization. Reached through its sign-in methods; optionally bound to an SSH key on GitHub. Its master key is the root of trust for its devices: [Device linking](#device-linking). Has its own username, which any of its sessions changes: `Username` in the [OpenAPI spec](../../api/openapi.json). Sign-up makes it from the GitHub login, or from the email's local part for Google: lowercased, each run of characters a [`name`](../protocol/README.md#agent-address) does not allow made `-`, cut to 63 characters, `-` trimmed at both ends, `user` if nothing is left; when that is taken or reserved, the first free of `-2`, `-3` and on is appended, within 63 characters. Deletes itself through a session whose sign-in is recent, never while it is the only admin of a space with other members; at once it loses its sessions, tokens, sign-in methods and username, its devices are served as revoked ones, the spaces only it is in are deleted, and those it shares keep it, marked deleted: `deleteAccount` in the OpenAPI spec |
| Sign-in method | A provider and the subject it gives the person: GitHub and the GitHub user's ID, or an OIDC provider by its issuer and the ID token's `sub`. Belongs to one account at a time and never moves to another; once removed, it may sign up anew or be linked to another account. The first sign-in with it creates the account, the same way for every provider, unless the server's sign-up is closed and the identity is on none of its lists: then it creates nothing ([self-host.md, "Sign-up"](../self-host.md#sign-up)). A web session links another one to its account and removes one, never the last: `startGitHubLink`, `startGoogleLink` and `removeSignInMethod` in the [OpenAPI spec](../../api/openapi.json). A GitHub method keeps the GitHub login for display, updated at each sign-in. A web session records the method it signed in with and ends with it: `Session` in the OpenAPI spec |
| Device | A client installation. Ed25519, MLS credential, HPKE X25519. The certificate binds it to the account: [Device linking](#device-linking) |
| Agent | A named endpoint on a device bound to a session, or a service agent (`ci/deploy-bot`) with its own key and a scoped token: `createServiceToken` in the [OpenAPI spec](../../api/openapi.json) |
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
- **Key storage:** Keychain, the Secret Service or Windows Credential Manager; otherwise a file in the state directory, with 0600 permissions on Unix; on Windows it has the access rights it inherits from the directory. A service agent's keys come from its key secret, never from key storage.
- **Local state:** SQLite with an OS file lock, freed when its holder dies. The helper holds it while it runs; without one, a run that may change the state holds it for the run, and another waits for it up to the request timeout. A Codex hook takes no lock: it changes only its agent's registration, sessions and signals, and a save of the MLS state waits for it to truncate the WAL. So each write transaction takes SQLite's write lock as it begins, waiting for a hook's write.
- **MLS state:** a device's groups, KeyPackages' private keys and history keys are stored only sealed, under a key in key storage, or, for a service agent, a key derived from its key secret; the store overwrites the previous sealed state and deletes it securely, so no older one is kept. A message the device decrypted waits sealed under the same key until a run shows it.
- What the server sees: [threat model](../threat-model.md#the-server-sees); what it does not see: [threat model](../threat-model.md#the-server-does-not-see); trade-offs: [threat model](../threat-model.md#trade-offs).

## Delivery semantics
[Protocol spec](../protocol/delivery.md).

## Queue
- Queues, tasks, claims, leases, fencing and modes: [protocol spec](../protocol/queues.md).
- Claim: `SELECT … FOR UPDATE SKIP LOCKED`.
- `skald send` sends a message and `skald enqueue` a task; the MCP server has both as tools, with `claim` and `complete` for tasks.
- `skald tasks` lists tasks with their [state](../protocol/queues.md#task-states); the MCP tool `task_status` gives one task's.
- `skald connect` asks only whether the machine takes its own account's tasks. What it takes from others, [per queue](../protocol/queues.md#running-a-task), the user turns on and changes at any time with `skald serve`, only in the CLI on the machine:
  - `skald serve`: each queue's settings;
  - `skald serve <queue> --dir <folder> --peer read-only`, and `--service` for service agents;
  - `skald serve <queue> --peer approve --in session`;
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
| Any agent with MCP | — | MCP `inbox` |
| Any agent with a shell | — | `skald recv --wait` |
| CI | — | One-shot mode |

Adapters are separate modules with e2e tests against real CLIs.

`read` is issued once the agent's session file shows the message in an entry that went to the model, followed by the model's own entry, then on weaker signals; an unconfirmed message goes again by the next path: [agent-delivery.md, "Read confirmations"](agent-delivery.md#read-confirmations).

## Notifications
- To Telegram, Slack or a webhook.
- The server sends the events it sees in metadata: an agent is gone, DLQ is growing, a token is expiring.
- The person's device triggers the events that come from content or its agents' sessions: a task waiting for the person's yes in `skald approve`, a `hold` or `ask_user` waiting for the person, a session waiting on its agent CLI's own permission prompt, an invalid task result ([protocol spec](../protocol/queues.md#results)), a key changed. The server only relays them, so they need a running device of the person.
- A Claude Code session waiting on its own permission prompt is detected by its `PermissionRequest` and `Notification` hooks ([agent-delivery.md, "Permission prompts"](agent-delivery.md#permission-prompts)).
- A Codex session waiting on its own permission prompt is detected by its `PermissionRequest` hook and by the `item/commandExecution/requestApproval` of the background app-server in its config home, to which the helper keeps a client that answers nothing ([agent-delivery.md, "Codex permission prompts"](agent-delivery.md#codex-permission-prompts)).
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
  - rate limit per device, enforced by the server: [Cloud limits](#cloud-limits).
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
| Spaces an account owns: the ones it created, its personal space included | 2: the server setting `--owned-spaces-limit` (`SKALD_OWNED_SPACES_LIMIT`); unset, no limit |
| Members per space | No limit |
| Devices per member | 50: the server setting `--devices-limit` (`SKALD_DEVICES_LIMIT`), devices with a certificate per account, `service` and revoked devices aside; unset, no limit |
| Service agents per space | 10: the server setting `--service-agents-limit` (`SKALD_SERVICE_AGENTS_LIMIT`), service agent entries in the space's latest revision; a revision that adds entries beyond it is refused, one that keeps or lowers their number is taken; unset, no limit |
| Messages per space | About 10M per month: the server setting `--monthly-messages-limit` (`SKALD_MONTHLY_MESSAGES_LIMIT`), per calendar month in UTC, each message counted once whatever its envelopes, a handshake message ([Mailbox](../protocol/delivery.md#mailbox)) not at all; unset, no limit |
| Messages per device | 120 per minute: the server setting `--send-rate-limit` (`SKALD_SEND_RATE_LIMIT`), messages one device stores within a sliding window of `--send-rate-window` (`SKALD_SEND_RATE_WINDOW`) seconds, a minute unset, counted as the messages per space, across the server's instances; unset, no limit |
| Message text | 64 KB: the server setting `--message-size-limit` (`SKALD_MESSAGE_SIZE_LIMIT`), an envelope's payload or a task's `ct` in bytes; unset, no limit |
| Attachment | 10 MB |
| Undelivered messages kept | 7 days: the server setting `--mailbox-retention` (`SKALD_MAILBOX_RETENTION`), in seconds; unset, until delivered or the envelope's `ttl` ends |
| History kept | 90 days: the server setting `--history-retention` (`SKALD_HISTORY_RETENTION`), in seconds, each entry from when the server stored it ([Uploads](../protocol/membership.md#uploads)); unset, kept for good |

The web app's Usage section shows the owned spaces, devices, monthly messages and service agents with their limits, and the message text, undelivered messages and history limits, each limit while it is set: `getAccountUsage` and `getSpaceUsage` in the [OpenAPI spec](../../api/openapi.json). The server counts them also without the settings.
