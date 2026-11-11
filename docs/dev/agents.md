# Other coding agents

Agents beyond the six of [architecture.md](architecture.md#delivery-to-agents), "Delivery to agents". Checked 2026-11-11 against the vendors' docs.

## Setup by `skald connect`
| Agent | How |
|---|---|
| [Devin](https://docs.devin.ai/cli/extensibility/mcp/configuration.md), Desktop and CLI | `devin mcp add -s user` |
| [Cline](https://docs.cline.bot/mcp/adding-and-configuring-servers) | `cline mcp add --yes` |
| [Junie](https://junie.jetbrains.com/docs/junie-cli-mcp-configuration.html) | writes `~/.junie/mcp/mcp.json` |
| [Kilo Code](https://kilo.ai/docs/automate/mcp/using-in-kilo-code) | adds `mcp.skald` to `~/.config/kilo/kilo.jsonc` |
| [Factory Droid](https://docs.factory.com/harness/mcp.md) | `droid mcp add` |
| [Warp](https://docs.warp.dev/agents/capabilities/mcp/) | writes `~/.warp/.mcp.json` |
| [Kiro](https://kiro.dev/docs/mcp/configuration.md), IDE and CLI | `kiro-cli mcp add --scope global` |
| [Zed](https://zed.dev/docs/ai/mcp.md) | adds `context_servers.skald` to `~/.config/zed/settings.json` |
| [Augment](https://docs.augmentcode.com/setup-augment/mcp), Auggie CLI | `auggie mcp add` |
| [Amp](https://ampcode.com/docs/customize/mcp) | `amp mcp add` |
| [Goose](https://goose-docs.ai/docs/guides/config-files) | adds `extensions.skald` to `~/.config/goose/config.yaml` |
| [Qwen Code](https://github.com/QwenLM/qwen-code/blob/main/docs/users/features/mcp.md) | `qwen mcp add -s user` |

- A file edit keeps the other entries and the comments.
- An agent that already has a `skald` server, its own or read from another agent's config, stays as it is.
- Other agents: [roadmap.md](roadmap.md), "Post-MVP". Trae and OpenHands set MCP servers only in their UI; Aider has no MCP.

## Config that other agents run
| Config | Agents that also run it |
|---|---|
| Claude Code hooks: `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json` | [Cursor](https://cursor.com/docs/reference/third-party-hooks), on by default; [Copilot in VS Code](https://code.visualstudio.com/docs/copilot/customization/hooks), the repository's files only; [Devin](https://docs.devin.ai/cli/extensibility/hooks/overview.md), on by default; [Grok Build](https://github.com/xai-org/grok-build), the project's files once the project is trusted; [Trae](https://docs.trae.ai/ide/automate-actions-with-hooks), behind a setting |
| Claude Code MCP servers: `~/.claude.json`, `.mcp.json` | [Devin](https://docs.devin.ai/cli/reference/configuration/read-config-from.md), Grok Build and [Oh My Pi](https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md); Warp, behind a setting |
| Codex MCP servers | Warp, behind a setting |
| Cursor hooks: `~/.cursor/hooks.json`, `.cursor/hooks.json` | Grok Build |
| Cursor MCP servers: `.cursor/mcp.json` | Devin, Grok Build |
| OpenCode MCP servers: `opencode.json`, `~/.config/opencode/opencode.json` | Devin |
| Zed MCP servers: `.zed/settings.json`, `~/.config/zed/settings.json` | Devin |

## Shared standards
| Standard | Agents | What Skald gets |
|---|---|---|
| Claude plugin format | Factory Droid, Augment, Grok Build, CodeBuddy, Qwen Code, OpenHands, Oh My Pi, Junie | One package with hooks, MCP and a skill |
| [Agent Plugins](https://vercel.com/blog/introducing-agent-plugins) 1.0: skills and MCP, no hooks | Codex, Cursor, Copilot, Kiro, Mistral Vibe, Qwen Code, OpenHands, OpenClaw | One package with MCP and a skill |
| [Claude Code channels](https://code.claude.com/docs/en/channels-reference) | CodeBuddy, from its own config | The Channels server pushes into its sessions too |
| [ACP](https://agentclientprotocol.com/overview/agents) | About 50 agents | Runs an agent as a worker without a person; does not reach a session that another client runs |
| [Agent Skills](https://agentskills.io/) (`SKILL.md`) | Most agents | A `skald` skill for sending, `inbox` and `skald recv --wait` |
