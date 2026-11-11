# Other coding agents

Agents beyond the six of [architecture.md](architecture.md#delivery-to-agents), "Delivery to agents". Checked 2026-11-11 against the vendors' docs.

## Setup by `skald connect`
| Agent | How |
|---|---|
| [Devin](https://docs.devin.ai/cli/extensibility/mcp/configuration.md), Desktop and CLI | `devin mcp add -s user` |
| [Cline](https://docs.cline.bot/mcp/adding-and-configuring-servers), CLI | `cline mcp add --yes` |
| [Junie](https://junie.jetbrains.com/docs/junie-cli-mcp-configuration.html) | writes `~/.junie/mcp/mcp.json` |
| [Kilo Code](https://kilo.ai/docs/automate/mcp/using-in-kilo-code) | adds `mcp.skald` to `~/.config/kilo/kilo.jsonc` |
| [Factory Droid](https://docs.factory.com/harness/mcp.md) | `droid mcp add` |
| [Warp](https://docs.warp.dev/agents/capabilities/mcp/), app | writes `~/.warp/.mcp.json` |
| [Warp](https://docs.warp.dev/agents/cli/configuration/), Agent CLI | writes `~/.warp_cli/.mcp.json` on macOS |
| [Kiro](https://kiro.dev/docs/mcp/configuration.md), IDE and CLI | `kiro-cli mcp add --scope global` |
| [Zed](https://zed.dev/docs/ai/mcp.md) | adds `context_servers.skald` to `~/.config/zed/settings.json` |
| [Augment](https://docs.augmentcode.com/cli/integrations), Auggie CLI | `auggie mcp add` |
| [Amp](https://ampcode.com/docs/customize/mcp) | `amp mcp add` |
| [Goose](https://goose-docs.ai/docs/guides/config-files) | adds `extensions.skald` to `~/.config/goose/config.yaml` |
| [Qwen Code](https://github.com/QwenLM/qwen-code/blob/main/docs/users/features/mcp.md) | `qwen mcp add -s user` |
| [Pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/mcp.md) | `pi mcp add` |
| [Hermes Agent](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp) | adds `mcp_servers.skald` to `~/.hermes/config.yaml` |
| [Qoder](https://docs.qoder.com/cli/mcp-servers), CLI | `qoder mcp add -s user` |
| [CodeBuddy](https://www.codebuddy.ai/docs/cli/mcp), CodeBuddy Code | `codebuddy mcp add --scope user` |
| [Kimi Code](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/mcp.md) | writes `~/.kimi-code/mcp.json` |
| [Mistral Vibe](https://docs.mistral.ai/vibe/code/cli/mcp-servers) | `vibe mcp add --transport stdio` |
| [Deep Agents Code](https://docs.langchain.com/oss/deepagents/code/mcp-tools) | writes `~/.deepagents/.mcp.json` |

- A file edit keeps the other entries and the comments.
- An agent that already has a `skald` server in its own user config, or reads one from another agent's user config, stays as it is.
- Grok Build and Oh My Pi read the server from other agents' config: [Config that other agents run](#config-that-other-agents-run).
- Without setup: Trae, OpenHands, Qoder IDE, CodeBuddy IDE, and Cline's and Augment's IDE extensions set MCP servers only in their UI; Letta Code adds them only from inside a session; the Warp Agent CLI's docs name its MCP file on macOS only; Aider has no MCP.

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
