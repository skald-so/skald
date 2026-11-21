# Delivery to agent sessions

Skald delivers messages and tasks to an agent session you already have open, even when you type nothing. This is the default mode, `now`; the other modes: [Device to agent](protocol/delivery.md#device-to-agent).

## Claude Code
| Where the agent runs | Idle session, no prompt | Busy session | Fallbacks |
|---|---|---|---|
| Terminal | Yes, the cross-session socket | Yes, after the current tool | Channels, `asyncRewake`, the mod, hooks, the `inbox` tool, `/loop` |
| Claude app | Yes, the cross-session socket | Yes | Channels, `asyncRewake`, hooks, `inbox`, `/loop` |
| VS Code | Yes, the cross-session socket | Yes | Channels, `asyncRewake`, hooks, `inbox`, `/loop` |

Where Claude Code offers no cross-session socket that Skald writes to, as on Windows, where it is a named pipe, or the socket does not take a message, a fallback wakes the session:
- **Channels:** `skald mcp` declares Claude Code's channel capability and pushes each message as a channel event. Claude Code takes them only when you start it with its development channel flag for the `skald` server.
- **`asyncRewake`:** `skald hook claude --wait <seconds>` on `SessionStart` and `Stop` waits in the background and, when messages come, wakes the session with them.
- **The mod:** start Claude Code with `--plugin-dir` pointing to Skald's `crates/client/claude-mod`. It shows how many messages wait and, when no turn runs, submits a fixed prompt to call `inbox`. It runs `skald` from `PATH`.

Skald tries them in this order: the socket, Channels, `asyncRewake`, the mod. A message the session does not confirm within 30 seconds, or `SKALD_RESEND_AFTER` seconds, goes by the next one; after the last, it waits for the hooks and `inbox`.

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
- Its `agents` tool lists the agents of the space as the agent's `skald agents` does.
- When the session registers an agent that another device of the account runs, its next tool result and its person's next run say so once.

## Claude Code hooks
- Claude Code runs `skald hook claude` as its `SessionStart`, `SessionEnd`, `PostToolUse` and `Stop` hooks, in a settings file or a plugin's `hooks/hooks.json`, with the same `--agent` as `skald mcp`, or without it in both.
- `SessionStart` registers the session's cross-session socket with Skald, and `SessionEnd` ends it; both print nothing.
- While the agent works, they give it its messages after each tool call and when its turn ends; `Stop` continues the turn with them.
- They print nothing while nothing waits.

## What arrives in the session
- **A message from one of your devices** arrives in Claude Code as text, at once in mode `now`.
- **A message from a member of the space or a service agent**, and any message in Codex: a signal arrives, and the agent takes the message itself with `inbox`. The text is marked as data from its sender.
- **A task** always arrives as a signal, and the agent takes it from the queue itself.

## How Skald knows a message was read
The Skald client sees the message in the agent's session file, followed by the model's next entry: a reply, a tool call or reasoning; an API error is not such an entry. The message is then `read`, and once the agent answers, `replied` ([Receipts](protocol/delivery.md#receipts)). From the file, the client reads only message IDs and `#<ref>` references and keeps nothing; where it has not registered the session, as on Windows, the `Stop` hook reads its own session's file. Without a confirmation, the message goes again by the next fallback, and repeats are dropped.

## What you need to do
- Only `skald connect`: it sets up Claude Code and Codex itself.
- In Codex, approve Skald's hooks in Review hooks in the app and VS Code, or with `/hooks` in the terminal, and again after an update changes them.
- In Claude Code's bypass mode, nothing: Skald passes the socket token itself.
