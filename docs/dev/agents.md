# Other coding agents

Agents beyond the six of [architecture.md](architecture.md#delivery-to-agents), "Delivery to agents". Checked 2026-11-11 against the vendors' docs.

## MCP setup by `skald connect`
`skald connect` adds the Skald MCP server to each of these agents it finds, after confirmation. The agent then sends and reads messages through MCP.

| Agent | How |
|---|---|
| [Devin](https://docs.devin.ai/cli/extensibility/mcp/configuration.md), Desktop and CLI | `devin mcp add -s user` |
| [Cline](https://docs.cline.bot/mcp/adding-and-configuring-servers) | `cline mcp add --yes` |
| [Junie](https://junie.jetbrains.com/docs/junie-cli-mcp-configuration.html) | writes `~/.junie/mcp/mcp.json` |
| [Kilo Code](https://kilo.ai/docs/automate/mcp/using-in-kilo-code) | `kilo mcp add` |
| [Factory Droid](https://docs.factory.com/harness/mcp.md) | `droid mcp add` |
| [Warp](https://docs.warp.dev/agents/capabilities/mcp/) | writes `~/.warp/.mcp.json` |
| [Kiro](https://kiro.dev/docs/mcp/configuration.md), IDE and CLI | `kiro-cli mcp add --scope global` |
| [Zed](https://zed.dev/docs/ai/mcp.md) | adds `context_servers.skald` to `~/.config/zed/settings.json` |
| [Augment](https://docs.augmentcode.com/setup-augment/mcp), Auggie CLI | `auggie mcp add` |
| [Amp](https://ampcode.com/docs/customize/mcp) | `amp mcp add` |
| [Goose](https://goose-docs.ai/docs/guides/config-files) | adds `extensions.skald` to `~/.config/goose/config.yaml` |
| [Qwen Code](https://github.com/QwenLM/qwen-code/blob/main/docs/users/features/mcp.md) | `qwen mcp add -s user` |

- A file edit keeps the other entries and the comments.
- An agent that already has a `skald` server, its own or read from Claude Code's config, stays as it is.
- Without MCP setup: Trae and OpenHands set MCP only in their UI; Letta Code keeps it per agent ID; Aider has no MCP.

## Claude Code config that other agents run
| Agent | What it reads |
|---|---|
| Cursor | Hooks of `~/.claude/settings.json`, on by default ([docs](https://cursor.com/docs/reference/third-party-hooks)) |
| Copilot in VS Code | Hooks of the repository's `.claude/settings.json` ([docs](https://code.visualstudio.com/docs/copilot/customization/hooks)) |
| Devin | Hooks and plugins of `~/.claude/settings.json` and `.claude/settings.json`, on by default; MCP servers of `~/.claude.json` and `.mcp.json` ([docs](https://docs.devin.ai/cli/reference/configuration/read-config-from.md)) |
| Grok Build | Hooks of `~/.claude/settings.json` and `.claude/settings.json`, on by default; MCP servers of `~/.claude.json` and `.mcp.json` ([repo](https://github.com/xai-org/grok-build)) |
| Oh My Pi | MCP servers of `~/.claude.json` and `.mcp.json` ([docs](https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md)) |
| Warp | MCP servers of `~/.claude.json` and `.mcp.json`, behind a setting |
| Trae | Claude Code hooks, behind a setting ([docs](https://docs.trae.ai/ide/automate-actions-with-hooks)) |
| CodeBuddy, Crush | Claude Code channels |

## Shared standards
| Standard | Agents | What Skald gets |
|---|---|---|
| Claude plugin format | Factory Droid, Devin, Augment, Grok Build, CodeBuddy, Qwen Code, OpenHands, Oh My Pi, Junie | One package with hooks, MCP and a skill |
| [Agent Plugins](https://vercel.com/blog/introducing-agent-plugins) 1.0: skills and MCP, no hooks | Codex, Cursor, Copilot, Kiro, Mistral Vibe, Qwen Code, OpenHands, OpenClaw | One package with MCP and a skill |
| [ACP](https://agentclientprotocol.com/overview/agents) | About 50 agents | Runs an agent as a worker without a person; does not reach a session that another client runs |
| [Agent Skills](https://agentskills.io/) (`SKILL.md`) | Most agents | A `skald` skill for sending, `inbox` and `skald recv --wait` |
