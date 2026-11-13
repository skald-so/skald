# Contributing

Follow [rules.md](rules.md). Branches and PR checks: [infrastructure.md](docs/dev/infrastructure.md), "Branches" and "PR checks".

## Picking work
- Maintainers plan the work as stories split into iterations; one PR per iteration.
- For a change of your own, open an issue first and agree on its scope with a maintainer.

## Requirements
- Rust stable with `rustfmt` and `clippy`, via [rustup](https://rustup.rs).
- `cargo-deny` and `cargo-mutants`: `cargo install cargo-deny cargo-mutants --locked`.
- release-plz of the version in `script/lib/release.sh`, for the Release PR tests: `cargo install release-plz --version <version> --locked`.
- dist of the version in `dist-workspace.toml`, only to build the release archives by hand, `dist build`; `script/test` does not run it: `cargo install cargo-dist --version <version> --locked`.
- Docker: PostgreSQL for the integration tests, the `skald-server` image, Chrome for the kit page screenshots.
- Node.js 26 with npm.
- git 2.40 or newer: the edit comparison prototype reads the conflicts of `git merge-tree` and the attributes of a commit with `git check-attr --source`.
- `curl`, `jq`, `openssl` and `tar`: the tests run them.
- The OpenSSH client and server (`ssh`, `ssh-add`, `ssh-agent`, `ssh-keygen`, `sshd`): the staging deploy's test runs a local `sshd`.
- Python 3 with its `venv` module: the edit comparison prototype's tests install a package with `pip` in a virtual environment.
- The MLS prototypes' WASM tests: the `wasm32-unknown-unknown` target, `rustup target add wasm32-unknown-unknown`, and `wasm-bindgen-cli` of the `wasm-bindgen` version in `prototypes/mls/Cargo.lock`: `cargo install wasm-bindgen-cli --version <version> --locked`. They run in Node.js.
- Google Chrome: the web tests ([web/README.md](web/README.md)) and the server's and the client's sign-in tests run it headless; `SKALD_CHROME` points to another Chrome or Chromium binary.

## Running locally
The root `Cargo.toml` is a package too, for release-plz: in the root, `cargo build`, `cargo test` and `cargo clippy` cover the crates only with `--workspace` or `-p <crate>`.

PostgreSQL 19 in a container:
```sh
docker run --detach --name skald-postgres --env POSTGRES_PASSWORD=skald \
  --publish 127.0.0.1:5432:5432 postgres:19rc1
```

`skald-server` applies migrations at start and listens on `127.0.0.1:8080`; `SKALD_LISTEN` changes the address:
```sh
SKALD_DATABASE_URL=postgres://postgres:skald@127.0.0.1:5432/postgres cargo run -p skald-server
curl http://127.0.0.1:8080/health
```

GitHub sign-in needs a GitHub OAuth App of your own (GitHub → Settings → Developer settings → OAuth Apps) with the callback URL `http://127.0.0.1:8080/auth/github/callback`. Its client secret goes only in `SKALD_GITHUB_CLIENT_SECRET`:
```sh
read -rs SKALD_GITHUB_CLIENT_SECRET && export SKALD_GITHUB_CLIENT_SECRET  # paste the secret
SKALD_DATABASE_URL=postgres://postgres:skald@127.0.0.1:5432/postgres cargo run -p skald-server -- \
  --public-url http://127.0.0.1:8080 --github-client-id <client ID>
```
Then open `http://127.0.0.1:8080/auth/github`. Without the client ID and secret, sign-in is off. `skald-server --help` lists the other settings.

`skald`:
```sh
cargo run -p skald-client -- --version
cargo run -p skald-client -- sign-in --server http://127.0.0.1:8080
```
`SKALD_STATE_DIR` sets the directory of its local state and secrets file; `SKALD_KEY_STORAGE=file` keeps its secrets in that file instead of the system's key storage.

Web UI: [web/README.md](web/README.md).

## Tests
`./script/test` builds everything and runs all checks; exit code 0 only if all of them pass. It starts its own PostgreSQL container and removes it afterwards. CI runs the same script.

`./script/mutants origin/dev` mutates the Rust code changed on your branch and runs the tests of each mutated crate; it lists every mutant no test catches (rule 2 in [rules.md](rules.md)).

## Sending a PR
- Fork the repository and branch from `dev`; an iteration's branch is `feature/iteration-<N>.<M>`.
- Open the PR against `dev`, titled per rule 3 in [rules.md](rules.md).
- A maintainer squash-merges it once all checks are green. Direct pushes to `dev` and `main` are rejected.

## CLA
Before your first PR is merged, sign the [Skald Contributor License Agreement](https://gist.github.com/DanilZabolotskiy/fdc5ad4dbb10cf670e8af59e1775b54e). CLA Assistant asks you to sign in the PR; one signature covers all your contributions.

Contributions are licensed under the license of the files they change: [LICENSE.md](LICENSE.md).
