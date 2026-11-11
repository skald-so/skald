# Tech stack

**All Rust.** One cargo workspace: `proto`, `crypto`, `server`, `client`, `mcp`, `adapters`.

Versions as of 2026-11-09.

| Layer | Choice |
|---|---|
| HTTP, WebSocket | axum 0.8 + tokio; SSE and long-poll as fallbacks |
| Database | PostgreSQL 19 (in production, from 19.1), sqlx 0.9: `SKIP LOCKED`, `LISTEN/NOTIFY` |
| Files | A disk volume by default; optionally S3-compatible storage (Cloudflare R2 or Hetzner Object Storage) |
| MLS | mls-rs 0.57 with mls-rs-crypto-rustcrypto 0.23: [choice](mls-prototypes.md#choice) |
| HPKE, signatures | hpke 0.15, ed25519-dalek 3, x25519-dalek 3 |
| Hashes, random IDs | sha2 0.11; getrandom 0.4 for the random bits of UUIDv7 IDs |
| Keys | keyring 4, fallback: a 0600 file |
| Local state | SQLite |
| MCP | rmcp 3.9, stdio |
| Authentication | openidconnect 4, GitHub OAuth, device flow (RFC 8628) |
| API | HTTPS + JSON, OpenAPI spec |
| Envelopes | Deterministic CBOR (RFC 8949, §4.2) with a CDDL schema, over WebSocket, HTTPS and SSE: [protocol spec](../protocol/README.md#encoding). ciborium-ll 0.2 reads and writes CBOR heads; `proto` checks the spec's encoding rules on decoding |
| Frontend | TypeScript 7, Vite 8, React 19, static SPA in `web/`; API client and MSW mocks generated from OpenAPI by Orval 8; Vitest, Testing Library, oxlint, Prettier |
| Web styles | CSS Modules; theme tokens are CSS custom properties with `light-dark()`, following the system setting |
| Web fonts | Fontsource 5 variable packages: Inter, Space Grotesk, JetBrains Mono; bundled with the build |
| Web accessibility check | axe-core 4 over the kit page in headless Chrome, driven by playwright-core 1 |
| Web screenshot comparison | The kit page in the Chromium of the Playwright Docker image (linux/amd64, pinned by digest) on every machine; pixelmatch 8 compares with the references, anti-aliasing aside |
| In-browser decryption | WASM build of the Rust core |
| SDK | Rust; later Python (PyO3) and TypeScript (WASM or napi) |
| Distribution | GitHub Releases; `install.sh` and `install.ps1` with a one-time code; npm package with prebuilt binaries (`npx skald-so`); Claude Code plugin; server Docker image. Later: Homebrew, winget, cargo-binstall, Helm |
| Infrastructure | Hetzner, Docker Compose, Caddy, WAL-G, Prometheus, Grafana, Sentry; details in [infrastructure.md](infrastructure.md) |
