# Other coding agents

Agents beyond the six of [architecture.md](architecture.md#delivery-to-agents), "Delivery to agents". Checked 2026-11-11 against the vendors' docs.

## Config that other agents run
| Config | Agents that also run it |
|---|---|
| Claude Code hooks: `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json` | [Cursor](https://cursor.com/docs/reference/third-party-hooks), on by default; [Copilot](https://code.visualstudio.com/docs/copilot/customization/hooks) in VS Code, its app and CLI, the repository's files only; [Devin](https://docs.devin.ai/cli/extensibility/hooks/overview.md), on by default; [Grok Build](https://github.com/xai-org/grok-build), the user's file always, the project's once the project is trusted; [Trae](https://docs.trae.ai/ide/automate-actions-with-hooks), behind a setting |
| Claude Code MCP servers: `~/.claude.json`, `.mcp.json` | [Devin](https://docs.devin.ai/cli/reference/configuration/read-config-from.md), Grok Build, [Oh My Pi](https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md); [Warp](https://docs.warp.dev/agents/capabilities/mcp/) app, behind a setting; Qoder, CodeBuddy and Deep Agents Code, the project's `.mcp.json` only |
| Codex MCP servers: `~/.codex/config.toml`, `.codex/config.toml` | Oh My Pi; Warp app, behind a setting |
| Gemini CLI MCP servers: `~/.gemini/settings.json`, `.gemini/settings.json` | Oh My Pi |
| VS Code MCP servers: `.vscode/mcp.json` | Oh My Pi |
| Cursor hooks: `~/.cursor/hooks.json`, `.cursor/hooks.json` | Grok Build, the user's file always, the project's once the project is trusted |
| Cursor MCP servers: `~/.cursor/mcp.json`, `.cursor/mcp.json` | Grok Build, Oh My Pi; Devin, the project's file only |
| OpenCode MCP servers: `opencode.json`, `~/.config/opencode/opencode.json` | Devin, Oh My Pi |
| Zed MCP servers: `.zed/settings.json`, `~/.config/zed/settings.json` | Devin |

## Shared standards
| Standard | Agents | What Skald gets |
|---|---|---|
| Claude plugin format | Devin, Factory Droid, Augment, Grok Build, CodeBuddy, Qwen Code, OpenHands, Oh My Pi, Junie | One package with hooks, MCP and a skill |
| [Agent Plugins](https://vercel.com/blog/introducing-agent-plugins) 1.0: skills and MCP, no hooks | Devin, Codex, Cursor, Copilot, Kiro, Mistral Vibe, Qwen Code, OpenHands, OpenClaw | One package with MCP and a skill |
| [Claude Code channels](https://code.claude.com/docs/en/channels-reference) | CodeBuddy, from its own config | The Channels server pushes into its sessions too |
| [ACP](https://agentclientprotocol.com/overview/agents) | About 50 agents | Runs an agent as a worker without a person; does not reach a session that another client runs |
| [Agent Skills](https://agentskills.io/) (`SKILL.md`) | Most agents | A `skald` skill for sending, `inbox` and `skald recv --wait` |
