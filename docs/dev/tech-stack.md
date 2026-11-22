# Tech stack

**All Rust.** One cargo workspace: `proto`, `crypto`, `server`, `client`, `mcp`, `adapters`.

Versions as of 2026-11-09.

| Layer | Choice |
|---|---|
| HTTP, WebSocket | axum 0.8 + tokio; SSE and long-poll as fallbacks |
| Client HTTP, WebSocket | ureq 3 and tungstenite 0.30, blocking. HTTPS and WSS, to the server and in `skald update`, with rustls 0.23 (ring) and the platform's root certificates (rustls-native-certs 0.8; `SSL_CERT_FILE` replaces them); plain HTTP and WS only to a server on this machine ([Network](architecture.md#network)). `HTTPS_PROXY` in `skald update`; `HTTPS_PROXY` and `NO_PROXY` apply to the server too |
| Client updates | `skald update` compares versions with semver 1 and unpacks the release archives with flate2 1 and tar 0.4 (`.tar.gz`) and zip 9 (`.zip`) |
| Database | PostgreSQL 19 (in production, from 19.1), sqlx 0.9: `SKIP LOCKED`, `LISTEN/NOTIFY` |
| Files | A disk volume by default; optionally S3-compatible storage: rusty-s3 0.11 signs the requests, path-style, and the server's HTTP client sends them. Its tests use versitygw in a container, pinned by digest |
| MLS | mls-rs 0.57.1 with mls-rs-core 0.27 and mls-rs-crypto-rustcrypto 0.23: [choice](mls-prototypes.md#choice). chacha20poly1305 0.11 seals the MLS state with XChaCha20-Poly1305; aes-gcm 0.11 seals history links and entries, and the link codes' request and grant, with AES-128-GCM, and the master backup with AES-256-GCM |
| HPKE, signatures | hpke 0.15, ed25519-dalek 3, x25519-dalek 3 |
| SSH signatures | ssh-key 0.7.0-rc.12 (MIT OR Apache-2.0), RustCrypto's, checks a certificate's SSH signature in `crypto`: [Device certificate](../protocol/membership.md#device-certificate). Its 0.7 is the line on ed25519-dalek 3 and sha2 0.11 |
| PAKE | CPace in `crypto` on curve25519-dalek 5 and sha2 0.11, checked against its draft's test vectors and, in tests only, pakery-cpace 0.6; hkdf 0.13 derives the request and grant keys: [link code](../protocol/membership.md#link-code) |
| Recovery key | argon2 0.6 derives the master backup's key and the recovery HPKE key pair with Argon2id: [Account](../protocol/membership.md#account) |
| Hashes, randomness | sha2 0.11; getrandom 0.4 for the random bits of UUIDv7 IDs, for signing-key seeds, X25519 private keys, the MLS state's key and its nonces, the nonces of history links and entries, for session tokens, the sign-in `state`, `nonce` and PKCE verifier, the device flow's device and user codes, and link codes and CPace scalars; zeroize 1 clears the seeds, the ISK, link keys and CPace scalars |
| Keys | keyring-core 1 with the keyring project's stores: apple-native-keyring-store 1 (Keychain), dbus-secret-service-keyring-store 1 (the Secret Service; the Linux client builds in libdbus, under AFL-2.1), windows-native-keyring-store 1. Fallback: a file ([architecture.md](architecture.md#cryptography)) |
| Local state | SQLite, bundled with rusqlite 0.39, the newest that sqlx 0.9 allows: [deny.toml](../../deny.toml) |
| MCP | rmcp 3.11, stdio |
| Agent CLI config | toml_edit 0.25 edits Codex's `config.toml` in `skald setup codex`, keeping its comments and layout |
| Authentication | GitHub OAuth over the server's HTTP client; OIDC over it too, with no OIDC or JOSE crate: the server's own code reads the metadata, keys and ID tokens with serde_json and base64, and ring 0.17 verifies RS256 and ES256; device flow (RFC 8628) |
| Session list | The server's own code, no parser crate, reads a web session's browser and OS by name from the `User-Agent` of its sign-in; no version, IP address or location is kept |
| Server HTTP client | ureq 3 with rustls 0.23 (ring) and the platform's root certificates (rustls-native-certs 0.8), blocking, in tokio's blocking threads |
| API | HTTPS + JSON, OpenAPI spec |
| Envelopes | Deterministic CBOR (RFC 8949, §4.2) with a CDDL schema, over WebSocket, HTTPS and SSE: [protocol spec](../protocol/README.md#encoding). ciborium-ll 0.2 reads and writes CBOR heads; `proto` checks the spec's encoding rules on decoding |
| Frontend | TypeScript 7, Vite 8, React 19, static SPA in `web/`; React Router 8 in declarative mode routes its pages; API client and MSW mocks generated from OpenAPI by Orval 8; Vitest, Testing Library, oxlint, Prettier |
| Web styles | CSS Modules; theme tokens are CSS custom properties with `light-dark()`, following the system setting |
| Web fonts | Fontsource 5 variable packages: Inter, Space Grotesk, JetBrains Mono; bundled with the build |
| Browser tests | Chrome Headless Shell of playwright-core 1; in the crates' tests, one per test binary, a browser context per page |
| Terminal tests | portable-pty 0.9 (MIT), a dev-dependency of `client`: the CLI's tests run it in a pseudo-terminal on Unix and a pseudo-console (ConPTY) on Windows |
| Web accessibility check | axe-core 4 over the kit page and the website's main page and docs page in Chrome Headless Shell |
| Web screenshot comparison | The kit page and the website's main page in the Chromium of the Playwright Docker image (linux/amd64, pinned by digest) on every machine; pixelmatch 8 compares with the references, anti-aliasing aside |
| Website and docs | VitePress 2 (alpha) builds `docs/` into a static site, with its default theme in the main page's look and local search; github-slugger 2 gives headings GitHub's IDs. The main page is static HTML built with Vite. `site/build.mjs` checks every link of the built pages: [infrastructure.md](infrastructure.md#website-and-docs) |
| In-browser decryption | WASM build of the Rust core: `proto` and `crypto` for `wasm32-unknown-unknown`. There getrandom 0.4 with `wasm_js` takes the random generator of the browser or Node.js, and js-sys 0.3 JavaScript's clock for UUIDv7 IDs. wasm-bindgen-test 0.3 runs the tests there |
| SDK | Rust |
| Distribution | GitHub Releases; `install.sh` and `install.ps1` with a one-time code; npm package `skald-so` that runs the release's binary (`npx skald-so`); Claude Code plugin; server Docker image |
| Server image | Built in the `rust` Debian 13 image; runs on distroless `cc-debian13` as a non-root user; base images pinned by digest |
| Infrastructure | Docker Compose, Caddy, WAL-G, Prometheus, Alertmanager, Grafana, Loki, Alloy; details in [infrastructure.md](infrastructure.md) |
| Rate limits | Caddy has none built in: production's Caddy is a build with the module `github.com/mholt/caddy-ratelimit`, made by xcaddy in the `caddy` builder image and copied onto the `caddy` image, both pinned by digest, the module to a commit |
