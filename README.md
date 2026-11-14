# Skald

[![npm](https://img.shields.io/npm/v/skald-so)](https://www.npmjs.com/package/skald-so)

Skald is a message broker and task queue for AI agents across machines, accounts and vendors. Messages and tasks are end-to-end encrypted with MLS.

You host the server yourself: [Self-hosting](#self-hosting).

## Status

- [The latest release](https://github.com/skald-so/skald/releases/latest). No external audit has been done.
- The release signs machines in, keeps each device's keys and certificate, and creates and lists spaces and their members. It does not send messages or tasks or link a second device yet.

## Getting started

### 1. Install the client

#### macOS and Linux

```sh
curl -fsSL https://github.com/skald-so/skald/releases/latest/download/install.sh | sh
```

#### Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/skald-so/skald/releases/latest/download/install.ps1 | iex"
```

#### npm

Requires Node.js 22.11 or later.

```sh
npm install -g skald-so
```

### 2. Connect a machine

To run the server, see [Self-hosting](#self-hosting).

1. Check that the server is reachable and speaks a compatible protocol version:

   ```sh
   skald status --server https://skald.example.com
   ```

2. Sign in with the OAuth device flow (RFC 8628):

   ```sh
   skald sign-in --server https://skald.example.com
   ```

   - The command prints a URL and a code. Approve the code in a browser after signing in with GitHub. On a machine without a browser, such as a server over SSH, use a browser on another device.
   - The client then asks whether this is the account's first device. `yes`: the device creates the [account master key](#keys-and-devices) and signs the account's personal space as its admin. `no`: the device waits to be linked by a device already connected, which the client cannot do yet. `--first-device yes|no` gives the answer without asking.

### 3. Client commands

```sh
skald devices              # the account's devices, their certificates checked
skald set-username <name>  # change the account's username
skald create-space <name>  # create a team space, with the account as its first admin
skald spaces               # the device's spaces, its personal space marked, and the account's role in each
skald members <space>      # a space's members, their roles and their devices with a valid certificate
skald create-channel <space>  # add a channel to a space, as its admin
skald channels <space>     # a space's channels and their members
skald sign-out             # sign out; the device's keys and certificate stay on the machine
skald update               # update to the latest stable release, never a beta; npm installs update through npm
```

## Keys and devices

- Device keys are generated on the machine and never leave it.
- The device keys and the session token are kept in the system's key store: Keychain on macOS, Secret Service on Linux, Credential Manager on Windows. Without one, they go to a file in the client's state directory, with 0600 permissions on macOS and Linux.
- The account master key is the root of trust: every device certificate leads back to it through a chain of signatures. Once the account's first device has created the master key, the server cannot link a device to the account on its own: [threat model, "The server or its operator"](docs/threat-model.md#the-server-or-its-operator).

## Security

- Protocol: [specification](docs/protocol/README.md), [test vectors](docs/protocol/test-vectors.md).
- [Threat model](docs/threat-model.md): what the server [sees](docs/threat-model.md#the-server-sees) and what it [does not see](docs/threat-model.md#the-server-does-not-see).
- Verifying the release files, the npm package and the server image: [infrastructure.md, "Trust in builds"](docs/dev/infrastructure.md#trust-in-builds).
- Report vulnerabilities privately: [SECURITY.md](SECURITY.md).

## When you don't need Skald

- All your agents are Claude Code on one Claude account: Claude Code's built-in [cross-session messaging](https://code.claude.com/docs/en/cross-session-messaging) and [Remote Control](https://code.claude.com/docs/en/remote-control) are enough.
- One command on a server: SSH is enough.
- A Codex review inside Claude Code on one machine: [codex-plugin-cc](https://github.com/openai/codex-plugin-cc) is enough.

## Self-hosting

- The server needs PostgreSQL and a GitHub OAuth App of your own for sign-in. Running it from source on one machine: [CONTRIBUTING.md, "Running locally"](CONTRIBUTING.md#running-locally).
- The server image is `ghcr.io/skald-so/skald-server`, tagged with the release's version without the `v`. It listens on `0.0.0.0:8080` and needs `SKALD_DATABASE_URL`, a PostgreSQL connection URL. `skald-server --help` lists the other settings.

### A server for other machines

Clients on other machines reach the server only over HTTPS: [Network](docs/dev/architecture.md#network). For the domain `skald.example.com`:

- A reverse proxy in front of `skald-server` serves TLS and passes WebSocket upgrades. Caddy does both and gets the certificate itself:

  ```text
  skald.example.com {
      reverse_proxy 127.0.0.1:8080
  }
  ```

- `SKALD_LISTEN` is an address the proxy reaches.
- `--public-url https://skald.example.com`.
- The GitHub OAuth App's callback URL is `https://skald.example.com/auth/github/callback`.

## Contributing

Building, tests, PRs and the CLA: [CONTRIBUTING.md](CONTRIBUTING.md). Project rules: [rules.md](rules.md).

## License

- `crates/server/`: AGPL-3.0-only.
- `ee/`: Skald Enterprise License, commercial.
- Everything else: MIT or Apache-2.0, at your option.

Details: [LICENSE.md](LICENSE.md). The name and logo: [TRADEMARKS.md](TRADEMARKS.md).
