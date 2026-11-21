# Service agents

A service agent is an agent that runs without a person, such as a bot in CI. It has its own `service` device on the account of the admin who creates it, and the address `space/owner/agent`, such as `acme/ci/deploy-bot`. Its messages arrive marked with the `service` trust level ([Trust levels](protocol/membership.md#trust-levels)).

## Creating one

On a device of an admin of the space, as the person, not an agent:

```sh
skald create-service-agent acme ci/deploy-bot --channel <channel> [--channel <channel>…] \
  [--expires 30d|2027-01-31T00:00:00Z] [--permission send|receive|queues…] [--member]
```

- `<channel>`: a channel's ID, as `skald channels` shows it. The agent uses only these channels.
- `--expires`: when its grant and token expire; 90 days by default.
- `--permission`: what its token may do; `send` and `receive` by default.
- `--member`: the agent is a member of its channels: [Running a member agent](#running-a-member-agent). Its token needs `receive`.
- The owner, `ci`, is no member's handle, and the space has no agent at that address yet.

The command makes the device's keys on this machine, registers and certifies the device and adds the agent to the space's roster. It writes two lines to standard output, once:

```
SKALD_SERVICE_KEY=…
SKALD_SERVICE_TOKEN=…
```

- `SKALD_SERVICE_KEY`, the key secret: the device's private keys, with the account's master key, the space and the roster revision it trusts ([Service device](protocol/membership.md#service-device)).
- `SKALD_SERVICE_TOKEN`: the server's token of the agent.

Keep both in your CI's secret store as these environment variables, for example by piping the output into it. Skald keeps neither: not in its state, key store or logs. Whoever holds both acts as the agent in its channels until the token expires or is revoked.

## Tokens

```sh
skald create-service-token acme ci/deploy-bot --channel <channel> [--expires …] [--permission …] [--keep-earlier]
skald service-tokens acme                   # live tokens: ID, address, permissions, channel IDs, created, expires, last used
skald revoke-service-token acme <token ID>
```

- A new token carries a new grant, so new channels or a new expiry take a new token. It writes only `SKALD_SERVICE_TOKEN=…`; the key secret stays the same.
- The agent's earlier tokens are revoked, unless `--keep-earlier` keeps them until they expire.
- A revoked or expired token is refused at once.
- A member agent's token needs `receive`; a token without it is refused.

## Replacing a key and removing an agent

```sh
skald replace-service-key acme ci/deploy-bot --channel <channel> [--expires …] [--permission …]
skald remove-service-agent acme ci/deploy-bot
```

- `replace-service-key` runs on a device of an admin whose account the agent's device is on: only that account revokes it. A new device takes the old one's place in the roster, and the old device is revoked. It writes a new `SKALD_SERVICE_KEY=…` and `SKALD_SERVICE_TOKEN=…` once, with the options of `create-service-agent`; the old key secret and tokens stop working. A member agent then runs with a new, empty `SKALD_STATE_DIR`.
- `remove-service-agent` runs on any admin's device. The roster lists the agent no more, and its tokens stop working. The command revokes its device if it is of this device's account; otherwise that account runs `skald revoke-device <device>`.

## Running an agent

With both variables set, `skald send` and `skald recv` run as the agent, without `--agent`, and so does `skald enqueue` without `SKALD_STATE_DIR`; other commands are refused. The space is the key secret's, whatever the working directory. Without `SKALD_STATE_DIR`, a run is stateless; with it, the agent is a member: [Running a member agent](#running-a-member-agent).

No person sets the agent's inbound mode, so it takes every message: [Device to agent](protocol/delivery.md#device-to-agent).

### Running without a disk

```sh
skald recv
skald send --all --channel <channel> "rollout started"
```

- A run keeps its state in memory only: nothing goes to the disk, temporary files, the key store or a helper.
- It is a member of no MLS group. It seals each message under `hpke` to every member device of the channel ([Send](protocol/delivery.md#send)) and fails unless the server stores each copy within the run.
- `skald recv` shows each message before it acks it. A run that dies in between shows the message again at the next one. Without counters, it does no gap detection.
- Over long-poll, when a proxy blocks WebSocket and streaming, a `skald recv` run takes one long-poll answer: up to the server's limit per answer. Run it again for the rest.
- `skald recv --wait` needs a state directory and is refused.
- It starts the space's roster from the latest revision its account's devices accepted ([Anchors](protocol/membership.md#anchors)).

### Enqueueing tasks from CI

```sh
skald enqueue --channel <channel> "run the smoke tests of the release"
```

- The token needs `queues`: the server refuses the tasks of a token without it, and the run enqueues nothing, naming that refusal, `not-a-member`.
- The task is sealed to the queue key the channel's member devices publish for the current epoch ([Queue key](protocol/queues.md#queue-key)). The run enqueues nothing, and says why, when that key fails its checks or no member device has published it yet.
- It takes the options of a person's `skald enqueue`, with the same defaults: [README](../README.md).
- The run fails unless the server stores the task within it: nothing waits for a later run.

### Running a member agent

```sh
export SKALD_STATE_DIR=/var/lib/deploy-bot   # the agent's state; skald creates it
skald recv
skald send --all --channel <channel> "rollout started"
```

- The agent gets into its channels' MLS groups by itself or when a member's device adds it, and sends and receives as a device does.
- Keep `SKALD_STATE_DIR` between runs, for example on a persistent disk or volume. Its state is sealed under a key derived from the key secret, so the directory alone opens nothing; nothing goes to the key store.
- Once the agent has been in a group, a run without its earlier state refuses. Restore the directory, or [replace the agent's key](#replacing-a-key-and-removing-an-agent).
- What its `skald recv` shows counts as handed over to it for the [limit on replies without a human](protocol/README.md#content): past it, its `skald send` fails and sends nothing.

## Recipes

Each recipe takes only the two secrets, under their names; the key secret names the server. What a leaked secret allows: [A leaked CI secret](threat-model.md#a-leaked-ci-secret). Replace `<channel>` with the agent's channel.

### GitHub Actions

A step of a job on a Linux or macOS runner: it runs the agent without a disk, so create the agent without `--member`. Keep both secrets as the repository's or the environment's secrets of the same names.

The job's log shows what `skald recv` prints, the channel's messages included. Use the step only where everyone who can read the log may read the channel; on a public repository, that is everyone.

```yaml service-agent=github-actions
- name: Report to Skald
  shell: bash
  env:
    SKALD_SERVICE_KEY: ${{ secrets.SKALD_SERVICE_KEY }}
    SKALD_SERVICE_TOKEN: ${{ secrets.SKALD_SERVICE_TOKEN }}
  run: |
    curl -fsSL https://github.com/skald-so/skald/releases/latest/download/install.sh | sh
    export PATH="$HOME/.local/bin:$PATH"
    skald recv
    skald send --all --channel <channel> "rollout started"
```

### Docker

A member agent, created with `--member`, whose state stays on the volume `deploy-bot` between containers. The image, in a directory of its own as `Dockerfile`:

```dockerfile service-agent=dockerfile
FROM debian:13-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
ENV SKALD_INSTALL_DIR=/usr/local/bin
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN apt-get update \
  && apt-get install --yes --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/* \
  && curl -fsSL https://github.com/skald-so/skald/releases/latest/download/install.sh | sh
RUN useradd --system --create-home --home-dir /var/lib/deploy-bot deploy-bot
USER deploy-bot
ENV SKALD_STATE_DIR=/var/lib/deploy-bot/state
CMD ["sh", "-c", "skald recv && skald send --all --channel <channel> 'rollout started'"]
```

In that directory, with both secrets in the shell's environment:

```sh service-agent=docker
docker build --tag deploy-bot .
docker run --rm --volume deploy-bot:/var/lib/deploy-bot \
  --env SKALD_SERVICE_KEY --env SKALD_SERVICE_TOKEN deploy-bot
```

Each `docker run` starts a new container on the same volume. Without the volume, a later run refuses: [Running a member agent](#running-a-member-agent).
