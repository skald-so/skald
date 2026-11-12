# The skald-server image: the release binary on a distroless base, run as a non-root user.
# Build from the repository root, naming the commit:
#   docker build --file crates/server/Dockerfile --build-arg REVISION=$(git rev-parse HEAD) .
# Settings come from the environment, as for the binary: skald-server --help. SKALD_DATABASE_URL is
# required; the server listens on all interfaces, port 8080.
# script/image-test tests the image; .github/workflows/edge.yml publishes it.

FROM rust:1.99.1-slim-trixie@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7 AS chef
# The cache keeps downloaded crates between builds.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install cargo-chef --version 0.1.78 --locked
WORKDIR /src

# The dependencies of the workspace, from its manifests alone. Not `--bin skald-server`: it drops the
# root manifest, a package of its own.
FROM chef AS plan
COPY Cargo.toml Cargo.lock repository.rs ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

# The dependency build: a layer reused while the recipe stays the same, so a change to the
# workspace's own sources rebuilds only them.
FROM chef AS deps
COPY --from=plan /src/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo chef cook --release --locked --package skald-server --recipe-path recipe.json

FROM deps AS build
COPY Cargo.toml Cargo.lock repository.rs ./
COPY api/openapi.json api/
COPY crates crates
# cook deletes what it built for its stand-ins of the workspace crates; the touch also makes cargo
# rebuild them from these sources when COPY kept mtimes older than the dependency build.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    find crates api -type f -exec touch {} + && \
    cargo build --release --locked --package skald-server && \
    cp target/release/skald-server /usr/local/bin/skald-server
# After the build, so that a new commit reuses it.
ARG REVISION
RUN test -n "$REVISION" || { echo "set the commit: --build-arg REVISION=<sha>" >&2; exit 1; }

# glibc and libgcc only: the binary links nothing else.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:cc66d222f0a7ec996b78a99fe76a969bc80b539a5016ad866c14d1949bc6d0ab
ARG REVISION
LABEL org.opencontainers.image.title=skald-server \
      org.opencontainers.image.source=https://github.com/skald-so/skald \
      org.opencontainers.image.revision=$REVISION \
      org.opencontainers.image.licenses=AGPL-3.0-only
COPY --from=build /usr/local/bin/skald-server /usr/local/bin/skald-server
# nonroot in the base image.
USER 65532:65532
ENV SKALD_LISTEN=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/skald-server"]
