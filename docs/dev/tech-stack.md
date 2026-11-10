# Tech stack

**All Rust.** One cargo workspace: `proto`, `crypto`, `server`, `client`, `mcp`, `adapters`.

**Why Rust:**
- mature MLS implementations exist only in Rust; the Go ones are immature;
- the core compiles to WASM for the web;
- there is an official MCP SDK, `rmcp`.

Downside: slower to write than Go.

Versions as of 2026-11-09.

| Layer | Choice |
|---|---|
| HTTP, WebSocket | axum 0.8 + tokio; SSE and long-poll as fallbacks |
| Database | PostgreSQL 19 (in production, from 19.1), sqlx 0.9: `SKIP LOCKED`, `LISTEN/NOTIFY` |
| Files | S3-compatible storage (Cloudflare R2 or Hetzner Object Storage) |
| MLS | OpenMLS 0.9 or mls-rs 0.57, chosen after prototyping |
| HPKE, signatures | hpke 0.15, ed25519-dalek 3, x25519-dalek 3 |
| Keys | keyring 4, fallback: a 0600 file |
| Local state | SQLite |
| MCP | rmcp 3.9, stdio |
| Authentication | openidconnect 4, GitHub OAuth, device flow (RFC 8628) |
| API | HTTPS + JSON, OpenAPI spec |
| Envelopes | Deterministic CBOR (RFC 8949, §4.2) with a CDDL schema, over WebSocket, HTTPS and SSE: [protocol spec](../protocol/README.md#encoding) |
| Frontend | TypeScript 7, Vite 8, React 19, static SPA in `web/`; API client and MSW mocks generated from OpenAPI by Orval 8; Vitest, Testing Library, oxlint, Prettier |
| Web styles | CSS Modules; theme tokens are CSS custom properties with `light-dark()`, following the system setting |
| Web fonts | Fontsource 5 variable packages: Inter, Space Grotesk, JetBrains Mono; bundled with the build |
| Web accessibility check | axe-core 4 over the kit page in headless Chrome, driven by playwright-core 1 |
| In-browser decryption | WASM build of the Rust core |
| SDK | Rust; later Python (PyO3) and TypeScript (WASM or napi) |
| Distribution | GitHub Releases; `install.sh` and `install.ps1` with a one-time code; npm package with prebuilt binaries (`npx skald-so`); Claude Code plugin; server Docker image. Later: Homebrew, winget, cargo-binstall, Helm |
| Infrastructure | Hetzner, Docker Compose, Caddy, WAL-G, Prometheus, Grafana, Sentry; details in [infrastructure.md](infrastructure.md) |

**Why deterministic CBOR for envelopes:**
- signatures need one byte form;
- unknown fields let two protocol versions coexist;
- CDDL lets others implement the protocol.
