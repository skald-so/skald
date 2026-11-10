# Contributing

Follow [rules.md](rules.md). Branches and PR checks: [infrastructure.md](docs/dev/infrastructure.md), "Branches" and "PR checks".

## Picking a story
- Stories and their iterations are in [task.md](task.md); finished iterations are ticked in [iteration.md](iteration.md).
- Take a story whose "Depends on" stories are done; "Order and parallelism" in `task.md` lists the waves.
- One PR per iteration, in `task.md` order: it does all of "What we do" and meets "Definition of done".
- A story without iterations is planned first, in its own PR: iterations go to `task.md`, their lines to `iteration.md`.

## Requirements
- Rust stable with `rustfmt` and `clippy`, via [rustup](https://rustup.rs).
- `cargo-deny` and `cargo-mutants`: `cargo install cargo-deny cargo-mutants --locked`.
- Docker.
- Node.js 26 with npm.
- Google Chrome: the web tests run it headless ([web/README.md](web/README.md)).

## Running locally
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

`skald`:
```sh
cargo run -p skald-client -- --version
```

Web UI: [web/README.md](web/README.md).

## Tests
`./script/test` builds everything and runs all checks; exit code 0 only if all of them pass. It starts its own PostgreSQL container and removes it afterwards. CI runs the same script.

`./script/mutants origin/dev` mutates the Rust code changed on your branch and runs the tests of each mutated crate; it lists every mutant no test catches (rule 2 in [rules.md](rules.md)).

## Sending a PR
- Fork the repository and branch from `dev`: `feature/iteration-<N>.<M>`.
- Tick the iteration `[x]` in `iteration.md` in the same PR.
- Open the PR against `dev`, titled per rule 3 in [rules.md](rules.md).
- A maintainer squash-merges it once all checks are green. Direct pushes to `dev` and `main` are rejected.

## CLA
Before your first PR is merged, sign the [Skald Contributor License Agreement](https://gist.github.com/DanilZabolotskiy/fdc5ad4dbb10cf670e8af59e1775b54e). CLA Assistant asks you to sign in the PR; one signature covers all your contributions.

Contributions are licensed under the license of the files they change: [LICENSE.md](LICENSE.md).
