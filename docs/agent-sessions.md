# Delivery to agent sessions

Skald delivers messages and tasks to an agent session you already have open, even when you type nothing. This is the default mode, `now`; the others: [Delivery settings](#delivery-settings).

## Claude Code
| Where the agent runs | Idle session, no prompt | Busy session | Fallbacks |
|---|---|---|---|
| Terminal | Yes, the cross-session socket | Yes, after the current tool | Channels, `asyncRewake`, the mod, hooks, the `inbox` tool, `/loop` |
| Claude app | Cross-session socket ([app checks](#app-checks)) | Yes, after the current tool | `asyncRewake`, installed mod, hooks, `inbox`, `/loop` |
| VS Code | Cross-session socket ([app checks](#app-checks)) | Yes, after the current tool | `asyncRewake`, the mod, hooks, `inbox`, `/loop` |

Where Claude Code offers no cross-session socket that Skald writes to, as on Windows, where it is a named pipe, or the socket does not take a message, a fallback wakes the session:
- **Channels:** `skald mcp` declares Claude Code's channel capability and pushes each message as a channel event. Claude Code takes them only when you start it with its development channel flag for the `skald` server.
- **`asyncRewake`:** `skald hook claude --wait <seconds>` on `SessionStart` and `Stop` waits in the background and, when messages come, wakes the session with them.
- **The mod:** start Claude Code with `--plugin-dir` pointing to Skald's `crates/client/claude-mod`. In the Claude app, install the mod from a [local marketplace](dev/agent-delivery.md#waking-an-idle-session). In VS Code, set `CLAUDE_CODE_PLUGIN_DIRS` to the mod directory in `claudeCode.environmentVariables`. It shows how many messages wait and, when no turn runs, submits a fixed prompt to call `inbox`. It runs `skald` from `PATH`.

Skald tries them in this order: the socket, Channels, `asyncRewake`, the mod. A message the session does not confirm within 30 seconds, or `SKALD_RESEND_AFTER` seconds, goes by the next one; after the last, it waits for the hooks and `inbox`.

### Host checks

The checks ran Claude Code 2.1.330 on macOS with Skald's plugin and a loopback model stub. Each session ran the Skald CLI, called its MCP `inbox` and returned a `read` receipt for each tested wake path.

- **Paseo 0.17.1:** The SDK and terminal sessions woke through the socket, `asyncRewake` and the mod. Paseo's terminal hook marked the workspace as needing attention.
- **Herdr 0.9.5:** The session woke through the socket, `asyncRewake` and the mod. Herdr's hook reported the Claude Code session ID.
- Skald's Channels development flag requires an interactive CLI. The Agent SDK ignores it. Channels remained unconfirmed in the hosted terminal checks with the stub. The separate CLI tests verify it. [Claude Code reference](https://code.claude.com/docs/en/channels-reference#test-during-the-research-preview) · [Skald Channels checks](dev/agent-delivery.md#channels)

### App checks

- The [earlier app checks](dev/agent-delivery.md#waking-an-idle-session) confirmed socket delivery in the Claude app and Claude Code for VS Code.
- **VS Code 1.147.0, Claude Code 2.1.330:** With a loopback model stub, an own message woke the idle session with its text and got a `read` receipt. A peer message woke it with a signal. The text and the `read` receipt followed an MCP `inbox` call.
- Model limits blocked the Skald delivery checks with the live model in both apps and with the project stub in the Claude app (6.28899.6).

## Codex
| Where the agent runs | Idle session, no prompt | Busy session | Fallbacks |
|---|---|---|---|
| Terminal | Yes, `codex queue` | Yes, when the current turn ends | Hooks, `inbox` |
| ChatGPT app | Yes, `codex queue` | Yes, when the current turn ends | Hooks, `inbox`, a scheduled task |
| VS Code | Yes, `codex queue` | Yes, when the current turn ends | Hooks, `inbox` |

## Other agents
- Gemini, Cursor, Copilot, OpenCode: hooks and plugins; the fallback is `inbox`.
- Any agent with a shell: the `skald recv --wait` command.

## The MCP server
- An agent CLI starts `skald mcp` as an MCP server on stdio in the session's folder, such as with `claude mcp add skald -- skald mcp` for Claude Code.
- The session is an agent of the folder's space, named after the agent CLI and the device.
- Two sessions of one agent CLI in one space on one device share that agent and its messages; `skald --agent <name> mcp` gives each its own.
- Its `inbox` tool returns the messages that came for the session since it started, each once; with `message`, a `#<ref>` it returned, that message again, whole.
- Its `send` tool sends as `skald send` does: `to` takes an agent's address, or `*` for every agent in the channel.
- Its `reply` tool answers one of them, by its `#<ref>` or ID, as `skald reply` does.
- Its `set_status` tool sets the session's state, `busy` or `available`, and a note of up to 128 bytes, as `skald --agent <name> set-status` does. Members of the space see them in the device's presence while the session runs; a new session starts `available`, with no note.
- Its `message_status` tool says where a message the session sent or got is, as `skald message-status` does.
- A task the session claims with `skald --agent <the session's agent> claim` stays claimed while the session runs: the device extends the claim. Once the session stops, the lease ends; a claim found lost is said in the next tool result.
- Its `agents` tool lists the agents of the space as the agent's `skald agents` does.
- When the session registers an agent that another device of the account runs, its next tool result and its person's next run say so once.

## Claude Code hooks
- Claude Code runs `skald hook claude` as its `SessionStart`, `SessionEnd`, `PostToolUse`, `Stop`, `PermissionRequest` and `Notification` hooks, the last for every type or `permission_prompt`, in a settings file or a plugin's `hooks/hooks.json`, with the same `--agent` as `skald mcp`, or without it in both.
- `SessionStart` registers the session's cross-session socket with Skald, and `SessionEnd` ends it; both print nothing.
- While the agent works, they give it its messages after each tool call and when its turn ends; `Stop` continues the turn with them.
- They print nothing while nothing waits.
- `PermissionRequest` and `Notification` tell Skald that the session waits on its [permission prompt](#permission-prompts). They print nothing.

## Permission prompts
- When a Claude Code or Codex session waits on its own permission prompt, your `skald recv` and `skald recv --wait` show one line: the agent, its space and folder, without the prompt's content. With `--json`: its session, when it began to wait and, once the prompt is answered or dropped, `ended`.
- Agents never see it, and it does not leave the device.
- Codex tells it by its `PermissionRequest` hook and by its background app-server, which a terminal session joins unless started with `--no-daemon`. Skald never answers the prompt.

## What arrives in the session
- **A message from one of your devices** arrives in Claude Code as text, at once in mode `now`.
- **A message from a member of the space or a service agent**, and any message in Codex: a signal arrives, and the agent takes the message itself with `inbox`. The text is marked as data from its sender. With `inline`: [Delivery settings](#delivery-settings).
- **A task** always arrives as a signal, and the agent takes it from the queue itself.

## Delivery settings
You set them on the device, per channel and sender trust level, `own`, `peer` or `service`, in the folder of the channel's space:

- `skald set-delivery-mode <channel> <level> <mode>`: how a message wakes the session.
  - `now`, the default: at once.
  - `batch --window 5m`: the level's messages of the channel wait until the window, opened by the first of them, ends, then arrive together.
  - `pull`: never; they wait for `inbox` or the agent's `skald recv`, and no hook gives them.
- `skald set-delivery-text <channel> <level> <text>`, for `peer` and `service`: `signal`, the default, or `inline`, the text at once, marked as data from its sender. Codex sessions get only the signal under `inline` too. Its risk: [threat model, "Prompt injection and loops"](threat-model.md#prompt-injection-and-loops).
- `skald delivery-modes`: the settings of each channel, and its wakes: each time Skald pushed messages or their signal into a session, by the socket, a fallback, a hook or `codex queue`, counts one in each channel they came in. `--json` gives them too.

Agents cannot change or list the settings.

## How Skald knows a message was read
The Skald client sees the message in the agent's session file, followed by the model's next entry: a reply, a tool call or reasoning; an API error is not such an entry. The message is then `read`, and once the agent answers, `replied` ([Receipts](protocol/delivery.md#receipts)). From the file, the client reads only message IDs and `#<ref>` references and keeps nothing; where it has not registered the session, as on Windows, the `Stop` hook reads its own session's file. Without a confirmation, the message goes again by the next fallback, and repeats are dropped.

## What you need to do
- `skald connect`: it sets up Claude Code and Codex itself.
- To set up Claude Code yourself, install [Skald's plugin](../README.md#claude-code-plugin) after `skald`: it gives each session the MCP server, the hooks and a skill with the rules for Skald's messages. Without `skald`, it does nothing.
- The plugin is that of the latest stable release. Claude Code updates it only with auto-update on: in `/plugin`, Marketplaces, `skald`. Without it: `claude plugin marketplace update skald`, then `claude plugin update skald@skald`.
- In Codex, approve Skald's hooks in Review hooks in the app and VS Code, or with `/hooks` in the terminal, and again after an update changes them.
- In Claude Code's bypass mode, nothing: Skald passes the socket token itself.
