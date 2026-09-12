# MVP-2b — Full Frontend (React + TanStack Start → Railway)

Goal: the real game client per spec §F, with client-side prediction and
reconciliation, deployed as a SECOND Railway service.

Do this only AFTER the backend (MVP-1) is confirmed on Railway and the thin
tester (MVP-2a) proves reachability over `wss://`.

## Tasks

- [ ] 1. Scaffold Vite + React 19 + TanStack Start + Tailwind v4 project (`web/`).
- [ ] 2. Game core (`src/game/`):
  - [ ] `constants.ts` — mirror spec §0 EXACTLY (single source of truth vs physics.rs).
  - [ ] `types.ts`, `physics.ts` (mirror of physics.rs), `input.ts`, `audio.ts`.
  - [ ] `rink.ts` — sim loop, canvas render, camera, particles, trails.
  - [ ] `app.tsx`, `lobby.tsx`, `hud.tsx`, `joystick.tsx`, `session-server.tsx`.
- [ ] 3. Networking (`src/lib/multiplayer/`): `ws-client.ts`, `use-server-room.ts`.
- [ ] 4. Prediction + reconciliation (`session-server.tsx`) per spec §F4.
- [ ] 5. Wire server URL: `VITE_GAME_SERVER_URL` (build-time), fallback ws://localhost:3000.
- [ ] 6. Deploy to Railway (second service):
  - Build var `VITE_GAME_SERVER_URL = wss://<backend-domain>` (build-time, inlined).
  - Serve build output on Railway `PORT` (Node/SSR or static + SPA fallback).
  - Enable public domain.

## Acceptance
- Lobby loads, ambient preview animates, no console errors.
- Enter room → connects (wss), self + peers visible, movement feels instant (prediction).
- Two devices in same room see + collide with each other.
- Mobile 390x844: joystick works, no horizontal overflow.

## Critical pitfalls (spec §10)
- Physics constant drift between physics.ts and physics.rs → rubber-banding.
- Must use `wss://` in production (ws:// blocked from HTTPS pages).
- `VITE_GAME_SERVER_URL` is build-time → rebuild when backend URL changes.
