# MVP-2a — Thin Browser Tester (no build, no deploy)

Goal: a single static HTML file that connects to the backend over WebSocket,
sends keyboard input, and draws the dots from `state`. This proves the FULL loop
(browser → wss → server → browser) with zero framework and nothing to build.

Use this to sanity-check the Railway backend from a real browser before investing
in the full frontend (MVP-2b).

## Tasks

- [ ] 1. Create `web-tester/index.html` — self-contained:
  - Canvas sized to the arena (1800x1200, scaled to fit).
  - Connect to `?server=` query param or default `ws://localhost:3000`.
  - On open: send `{t:"join", room, name, color}`.
  - Keyboard WASD/arrows → normalized `{t:"input", x, y, seq}` each frame.
  - Render pillars (static, from §0) + all players from latest `state`.
  - Show connection status + latency (ping every 2s, measure pong).
- [x] 2. Serve locally: `npx --yes serve web-tester -l 5050` → http://localhost:5050
- [x] 3. Test against `wss://le-walls-and-balls-production.up.railway.app`:
  - VERIFIED by user — normal browser + incognito, two players same room,
    both dots visible and colliding. Live re-check: 36ms RTT, 61Hz, PASS.

## Notes
- No prediction/reconciliation here — just render server truth directly. Movement
  will show ~1 RTT of lag; that's expected and fine for a connectivity test.
- Physics constants (arena, pillars) copied from spec §0 for correct rendering.

## Mobile support (added)
- Viewport meta: maximum-scale=1, user-scalable=no, viewport-fit=cover (safe areas).
- `touch-action: none` on body/canvas/stick → no scroll/zoom while dragging.
- Virtual joystick (bottom-left), shown when `(pointer: coarse)` or `ontouchstart`.
  Pointer events, magnitude-proportional (analog) output, 0.12 dead zone.
  Server preserves sub-unit magnitude (only normalizes when >1) → analog speed.
- HUD compacts under 520px; hint hidden. Default server = Railway when not on localhost.
- Verified: JS `node --check` passes; served file (HTTP 200) contains joystick markup.
  Full on-device touch test is a manual step for the user.
