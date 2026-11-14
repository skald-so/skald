# This commit's skald for Linux, for the clean machine of script/self-host-test: built in the image
# that builds the server, so it runs on the machine's glibc, also when the tests run on a Mac.
# Build from the repository root, its .dockerignore the server image's:
#   docker build --file script/lib/self-host/client.Dockerfile --output <dir> .
FROM rust:1.99.1-slim-trixie@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY api/openapi.json api/
COPY crates crates
# The caches keep downloaded crates and the build between runs.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=skald-self-host-client-target,target=/src/target,sharing=locked \
    cargo build --release --locked --package skald-client && \
    cp target/release/skald /usr/local/bin/skald

FROM scratch
COPY --from=build /usr/local/bin/skald /skald
