# MVP-1 — Speck Backend (Rust WebSocket server → Railway)

Goal: stand up ONLY the authoritative game server, prove it over `wss://`, stop.
Frontend is a separate MVP (see MVP-2 files). Minimal resource usage throughout.

## Tasks

- [x] 1. Scaffold `server-rs/` (Cargo.toml + physics.rs, protocol.rs, room.rs, main.rs)
  - PORT-first env logic (Railway injects `PORT`), fallback `SPECK_PORT`, then `3000`.
- [x] 2. Add `Dockerfile`, `railway.toml`, `.dockerignore`
  - Multi-stage build (rust:1.83-slim → debian:bookworm-slim, ~30MB runtime).
  - `railway.toml`: empty `healthcheckPath` (TCP check; server rejects plain HTTP).
  - `.dockerignore`: exclude `target/`, `.git/`, `*.md` (smaller/faster build context).
- [x] 3. Build locally (`cargo build --release`) — compiles clean, `Cargo.lock` generated.
- [x] 4. **Verify locally with a throwaway WS client** — DONE
  - `smoke-test.mjs` (Node built-in WebSocket, no deps).
  - Result: welcome + pong (1ms) + 60.8Hz state stream + self present → PASS.
- [x] 5. Deploy to Railway and verify over `wss://` — DONE
  - Live at `wss://le-walls-and-balls-production.up.railway.app`.
  - Verified: welcome + pong (71ms RTT) + 62.6Hz state stream + self present → PASS.
  - Root dir = `server-rs/`, Dockerfile build, TCP health check, public domain (HTTPS→wss).

## Notes / decisions
- Bumps are broadcast to the whole room for MVP (spec implies targeted; harmless,
  frontend filters by self id). Revisit only if it shows up as bandwidth cost.
- Server is fully stateless (rooms in memory, ephemeral). No DB, no volumes.
