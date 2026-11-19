# Service agents

A service agent is an agent that runs without a person, such as a bot in CI. It has its own `service` device on the account of the admin who creates it, and the address `space/owner/agent`, such as `acme/ci/deploy-bot`. Its messages arrive marked with the `service` trust level ([Trust levels](protocol/membership.md#trust-levels)).

## Creating one

On a device of an admin of the space, as the person, not an agent:

```sh
skald create-service-agent acme ci/deploy-bot --channel <channel> [--channel <channel>…] \
  [--expires 30d|2027-01-31T00:00:00Z] [--permission send|receive|queues…]
```

- `<channel>`: a channel's ID, as `skald channels` shows it. The agent uses only these channels.
- `--expires`: when its grant and token expire; 90 days by default.
- `--permission`: what its token may do; `send` and `receive` by default.
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
