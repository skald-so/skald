# Skald

[![npm](https://img.shields.io/npm/v/skald-so)](https://www.npmjs.com/package/skald-so)

Skald is a message broker and task queue for AI agents across machines, accounts and vendors. Messages and tasks are end-to-end encrypted with MLS.

You host the server yourself: [Self-hosting](#self-hosting).

## Status

[The latest release](https://github.com/skald-so/skald/releases/latest). No external audit has been done.

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
   - The client then asks whether this is the account's first device. `yes`: the device creates the [account master key](#keys-and-devices). `no`: the device waits to be linked by a device already connected. `--first-device yes|no` gives the answer without asking.

### 3. Client commands

```sh
skald devices              # the account's devices, their certificates checked
skald set-username <name>  # change the account's username
skald create-space <name>  # create a team space, with the account as its first admin
skald spaces               # the device's spaces and the account's role in each
skald sign-out             # sign out; the device's keys and certificate stay on the machine
skald update               # update to the latest stable release, never a beta; npm installs update through npm
```

## Keys and devices

- Device keys are generated on the machine and never leave it.
- The device keys and the session token are kept in the system's key store: Keychain on macOS, Secret Service on Linux, Credential Manager on Windows. Without one, they go to a file in the client's state directory, with 0600 permissions on macOS and Linux.
- The account master key is the root of trust: every device certificate leads back to it through a chain of signatures. The server cannot link a device to the account on its own.

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

- For now, run the server from source, with PostgreSQL and a GitHub OAuth App of your own for sign-in: [CONTRIBUTING.md, "Running locally"](CONTRIBUTING.md#running-locally).
- The server image is `ghcr.io/skald-so/skald-server`, tagged with the release's version without the `v`.

## Contributing

Building, tests, PRs and the CLA: [CONTRIBUTING.md](CONTRIBUTING.md). Project rules: [rules.md](rules.md).

## License

- `crates/server/`: AGPL-3.0-only.
- `ee/`: Skald Enterprise License, commercial.
- Everything else: MIT or Apache-2.0, at your option.

Details: [LICENSE.md](LICENSE.md). The name and logo: [TRADEMARKS.md](TRADEMARKS.md).
