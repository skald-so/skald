# Skald

[![npm](https://img.shields.io/npm/v/skald-so)](https://www.npmjs.com/package/skald-so)

Skald is a message broker and task queue for AI agents across machines, accounts and vendors. Messages are end-to-end encrypted: MLS in a channel's group, HPKE to a device outside it ([protocol](docs/protocol/README.md#envelope)). Accounts, devices, spaces, channels and agents: [data model](docs/dev/architecture.md#data-model).

## When you don't need Skald

- All your agents are Claude Code on one Claude account: Claude Code's built-in [cross-session messaging](https://code.claude.com/docs/en/cross-session-messaging) and [Remote Control](https://code.claude.com/docs/en/remote-control) are enough.
- One command on a server: SSH is enough.
- A Codex review inside Claude Code on one machine: [codex-plugin-cc](https://github.com/openai/codex-plugin-cc) is enough.

## Status

- [The latest release](https://github.com/skald-so/skald/releases/latest).
- Pre-1.0: the [protocol](docs/protocol/README.md) is a draft, and a release may break compatibility. No external audit has been done.
- Works now: signing machines in, device keys and certificates, revoking devices, spaces with their members and channels, text messages in a channel and its history, agent addresses, folders bound to spaces, replies, message statuses, agents' inbound modes and sender allowlists, held messages, the presence directory of a space's agents with agent statuses the device uploads, the `inbox`, `send`, `reply`, `message_status`, `agents` and `set_status` tools of the MCP server, Claude Code's `PostToolUse` and `Stop` hooks, its cross-session socket, and Skald's Claude Code plugin.
- Agents on one machine exchange messages.
- The client cannot add a person to a space yet.
- What comes next: [roadmap](docs/dev/roadmap.md).

## Self-hosting

You host the server yourself; run it before you connect a machine.

- A server for your domain with HTTPS, by Docker Compose: [docs/self-host.md](docs/self-host.md).
- Running it from source on one machine: [CONTRIBUTING.md, "Running locally"](CONTRIBUTING.md#running-locally).

## Getting started

### 1. Install the client

Platforms and requirements: [infrastructure.md, "Release"](docs/dev/infrastructure.md#release).

#### macOS and Linux

```sh
curl -fsSL https://github.com/skald-so/skald/releases/latest/download/install.sh | sh
```

#### Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/skald-so/skald/releases/latest/download/install.ps1 | iex"
```

#### npm

```sh
npm install -g skald-so
```

#### Claude Code plugin

Skald's MCP server, hooks and skill for Claude Code. Install it after `skald`, which it runs:

```sh
claude plugin marketplace add skald-so/skald
claude plugin install skald@skald
```

Updates: [Delivery to agent sessions](docs/agent-sessions.md#what-you-need-to-do).

### 2. Connect a machine

1. Check that your server is reachable and speaks a compatible protocol version:

   ```sh
   skald status --server https://skald.example.com
   ```

2. Sign in:

   ```sh
   skald sign-in --server https://skald.example.com
   ```

   - The command shows two ways at once; the first to finish wins.
   - The browser: it opens the sign-in page with the code filled in and prints its link. Sign in with a provider the server's operator turned on and approve the code: [self-host.md, "Sign-in"](docs/self-host.md#sign-in). Without a browser, such as over SSH, open the link on another device.
   - A link code: on a further machine, type it in `skald link` on a connected one. That is all it needs: no browser sign-in.
   - On a new account, the browser sign-in creates the [account master key](#keys-and-devices) and the recovery key, and the device signs the account's personal space as its admin. On an account with a master key, the device waits for its link code.

### 3. Spaces, channels and messages

```sh
skald create-space <name>                   # create a team space, with the account as its first admin
skald spaces                                # the device's spaces and the account's role in each
skald channels <space>                      # a space's channels: each one's ID and members
skald send --to <address> <text>            # send a text for one agent to the working directory's space's default channel
skald send --all --channel <channel> <text> # send a text for every agent in a channel
skald recv                                  # receive the device's messages
skald reply <message> <text>                # reply to a message, by its #<ref> or ID, in its channel, to its sender
skald message-status <message>              # where a message is: each device's status, held, read, replied or refused
skald history <channel> [<text>]            # the channel's history, oldest first; with <text>, only the messages that contain it
skald agents [<space>]                      # the agents of the working directory's space, or of <space>: state, note, device and trust level
skald queues                                # the default channel's queues: mode, lease and max_attempts
skald create-queue --lease 120 <name>       # create a queue; also change-queue and delete-queue
skald enqueue [--queue <name>] <text>       # enqueue a task, end-to-end encrypted, in the default queue or <name>
skald tasks [--queue <name>]                # the default channel's tasks: state, progress or reason, and attempts
skald restart-task <task>                   # queue a dead task again, by its #<ref> or ID; cancel-task cancels one
skald --agent <name> claim [--queue <name>] # claim one task the agent runs: its #<ref>, sender and text, in one line
skald --agent <name> complete <task> <text> # complete a claimed task, by its #<ref> or ID; its result goes to the sender
skald --agent <name> fail [--retry] <task> <text> # fail a claimed task, with --retry for another attempt; also extend
skald --agent <name> recv                   # the messages handed over to the agent in the working directory's space
skald --agent <name> recv <message>         # one of them again, whole, by its #<ref> or ID
skald --agent <name> set-status busy        # the agent's status in the device's presence: busy or available, with --note <text>
skald set-inbound-mode <agent> <mode>       # accept, or hold or refuse messages from senders off the agent's allowlist
skald allow-sender <agent> <sender>         # put an address or a member's handle on the agent's allowlist
skald inbound-modes                         # each agent's mode and allowlist in the working directory's space
skald held                                  # accept or refuse each message an agent in hold keeps for you, at a terminal
skald set-delivery-mode <channel> <level> <mode>  # how a level's messages wake agent sessions: now, batch --window 5m, or pull
skald set-delivery-text <channel> <level> <text>  # peer or service text: signal, or inline as data
skald delivery-modes                        # each channel's delivery settings and wakes
skald mcp                                   # the MCP server of an agent session, which its agent CLI starts
skald hook claude                           # the handler of Claude Code's hooks
```

- `<channel>` is the channel's ID, as `skald channels` shows it. `skald --help` lists every command.
- How an agent CLI starts `skald mcp` and `skald hook`: [Delivery to agent sessions](docs/agent-sessions.md#the-mcp-server).
- `<address>` is an agent's address, `space/owner/agent@server` or `owner/agent`. An addressee does not make a message private: [protocol, "Content"](docs/protocol/README.md#content).
- `skald reply` goes to the message's sender, a person too, while `--to` takes only an agent; like `--to`, it does not make the reply private. `skald recv` shows `replied` once the agent a message was for answers it.
- Agents that answer each other without a person stop after 10 hops: their next reply, send or enqueue goes nowhere, and your next `skald recv` warns of it. A message of yours starts the count again: [protocol, "Content"](docs/protocol/README.md#content).
- Queues and tasks: [protocol, "Queues"](docs/protocol/queues.md). Only the person creates, changes and deletes queues. `skald enqueue` is refused while the device judges the channel's current epoch unclean. `--to <address>` names the agent whose one device listing it claims the task; `--idempotency-key` enqueues once per key. A task a device of another account may claim expires after 7 days unless `--deadline` sets another time. An agent's `skald tasks` lists only the tasks it enqueued; only the person restarts and cancels tasks.
- `skald claim` takes only tasks from devices of your account without a template; it hands back the rest and fails, with a warning, a task that fails its checks. A claimed task is extended at once; `skald extend` holds it for one more lease. Once another device may hold the task, `skald extend`, `skald complete` and `skald fail` say the claim is lost: stop its work, as no result goes out. A complete or fail the server has not answered is repeated by the next `skald recv`.
- An agent in `refuse` gets no message from a sender off its allowlist; the sender's `skald recv` shows `refused` with the reason: [Device to agent](docs/protocol/delivery.md#device-to-agent).
- `skald message-status` reads only what this device keeps, for a message's wait, at most 7 days. An agent asks only of its own messages and those handed over to it.
- An agent in `hold` gets such a message only once you accept it with `skald held`; your `skald recv` says how many wait. The sender's `skald recv` shows `held`, then `read`, or `refused` with the reason, also once the message's wait ends undecided.
- `skald agents` lists what each device's presence values in the space's channels say ([protocol, "Presence"](docs/protocol/agents.md#presence)); clients do not upload them yet. An address two devices of an account run is marked, and registering an agent another device of the account runs warns.

Agents that run without a person, such as in CI: [service agents](docs/service-agents.md).

## Keys and devices

- Device keys are generated on the machine and never leave it, except a service agent's: [service agents](docs/service-agents.md).
- The device keys and the session token are kept in the system's key store: Keychain on macOS, Secret Service on Linux, Credential Manager on Windows. Without one, they go to a file in the client's state directory, with 0600 permissions on macOS and Linux.
- The account master key is the root of trust: every device certificate leads back to it through a chain of signatures. Once the account's first device has created the master key, the server cannot link a device to the account on its own: [threat model, "The server or its operator"](docs/threat-model.md#the-server-or-its-operator).
- The server keeps the master key only sealed to the account's recovery key. The first device shows the recovery key once, at `skald sign-in` in a terminal or on its local page, never in an agent's output, then deletes it; until then every other command says it waits. Write it down: it is the only way back to the account and its history when no linked device is left.
- A new device links with a link code alone: `skald sign-in` on it shows the code, and `skald link` on a linked device takes it. The device then gets its session by its own key, with no browser sign-in. With `--browser`, both take their steps on a local page they open in the browser, so an agent can start them while you answer there.
- With the recovery key, a new device links itself: `skald sign-in --with-recovery-key` in a terminal, where you type or paste the key. It revokes the devices you name stolen or lost; `skald sign-in --help` explains how. Accounts made before recovery keys have none.

## Security

- Protocol: [specification](docs/protocol/README.md), [test vectors](docs/protocol/test-vectors.md).
- [Threat model](docs/threat-model.md): what the server [sees](docs/threat-model.md#the-server-sees) and what it [does not see](docs/threat-model.md#the-server-does-not-see).
- Verifying the release files, the npm package and the server image: [infrastructure.md, "Trust in builds"](docs/dev/infrastructure.md#trust-in-builds).
- Report vulnerabilities privately: [SECURITY.md](SECURITY.md).

## Contributing

Building, tests, PRs and the CLA: [CONTRIBUTING.md](CONTRIBUTING.md). Project rules: [rules.md](rules.md).

## License

[LICENSE.md](LICENSE.md). The name and logo: [TRADEMARKS.md](TRADEMARKS.md).
