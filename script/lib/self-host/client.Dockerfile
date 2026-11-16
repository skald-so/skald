# This commit's skald for Linux, for the clean machine of script/self-host-test, and the stand-ins
# for GitHub and Google that run there: built in the image that builds the server, so they run on the
# machine's glibc, also when the tests run on a Mac.
# Build from the repository root, its .dockerignore the server image's:
#   docker build --file script/lib/self-host/client.Dockerfile --output <dir> .
FROM rust:1.100.0-slim-trixie@sha256:e9d95b74296f7126a83ae3d041aeaba1ff4c478f13abc5cc034fcf7c131c3d4a AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY api/openapi.json api/
COPY crates crates
# The caches keep downloaded crates and the build between runs. The touch makes cargo rebuild the
# workspace crates from these sources when COPY kept mtimes older than a build in the cache.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=skald-self-host-client-target,target=/src/target,sharing=locked \
    find crates api -type f -exec touch {} + && \
    cargo build --release --locked --package skald-client --package skald-github-stand-in \
      --package skald-oidc-stand-in && \
    cp target/release/skald target/release/skald-github-stand-in target/release/skald-oidc-stand-in \
      /usr/local/bin/

FROM scratch
COPY --from=build /usr/local/bin/skald /usr/local/bin/skald-github-stand-in \
  /usr/local/bin/skald-oidc-stand-in /
