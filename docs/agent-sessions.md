# Delivery to agent sessions

Skald delivers messages and tasks to an agent session you already have open, even when you type nothing. This is the default mode, `now`; the other modes: [Device to agent](protocol/delivery.md#device-to-agent).

## Claude Code
| Where the agent runs | Idle session, no prompt | Busy session | Fallbacks |
|---|---|---|---|
| Terminal | Yes, the cross-session socket | Yes, after the current tool | Hooks, the `inbox` tool, `/loop` |
| Claude app | Yes, the cross-session socket | Yes | Hooks, `inbox`, `/loop` |
| VS Code | Yes, the cross-session socket | Yes | Hooks, `inbox`, `/loop` |

## Codex
| Where the agent runs | Idle session, no prompt | Busy session | Fallbacks |
|---|---|---|---|
| Terminal | Yes, `codex queue` | Yes, when the current turn ends | Hooks, `inbox` |
| ChatGPT app | Yes, `codex queue` | Yes, when the current turn ends | Hooks, `inbox`, a scheduled task |
| VS Code | Yes, `codex queue` | Yes, when the current turn ends | Hooks, `inbox` |

## Other agents
- Gemini, Cursor, Copilot, OpenCode: hooks and plugins; the fallback is `inbox`.
- Any agent with a shell: the `skald recv --wait` command.

## What arrives in the session
- **A message from one of your devices** arrives in Claude Code as text, at once in mode `now`.
- **A message from a member of the space or a service agent**, and any message in Codex: a signal arrives, and the agent takes the message itself with `inbox`. The text is marked as data from its sender.
- **A task** always arrives as a signal, and the agent takes it from the queue itself.

## How Skald knows a message was read
The Skald client sees the message in the agent's session file, followed by the model's next entry: a reply, a tool call or reasoning; an API error is not such an entry. The message is then `read`, and once the agent answers, `replied` ([Receipts](protocol/delivery.md#receipts)). From the file, the client reads only message IDs and keeps nothing. Without a confirmation, the message goes again by a fallback, and repeats are dropped.

## What you need to do
- Only `skald connect`: it sets up Claude Code and Codex itself.
- In Codex, approve Skald's hooks in Review hooks in the app and VS Code, or with `/hooks` in the terminal, and again after an update changes them.
- In Claude Code's bypass mode, nothing: Skald passes the socket token itself.
