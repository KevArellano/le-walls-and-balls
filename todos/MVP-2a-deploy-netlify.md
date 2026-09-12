# MVP-2a-deploy — Host the thin tester on Netlify

Goal: publish `web-tester/index.html` to a public Netlify URL so the browser →
`wss://` Railway → browser loop works from anywhere, with zero build step.

## What's already wired
- `netlify.toml` (repo root): `publish = "web-tester"`, no build command,
  SPA fallback (`/* -> /index.html`, 200) so `?room=CODE` deep links work,
  `no-cache` on index.html.
- `web-tester/index.html` now auto-detects host:
  - Served from localhost → defaults to `ws://localhost:3000` (no auto-connect
    unless `?server=` given).
  - Served remotely (Netlify) → defaults to
    `wss://le-walls-and-balls-production.up.railway.app` and auto-connects.

## Deploy steps (user must authorize — opens a browser)
1. `npx netlify-cli login`            # one-time auth, opens browser
2. `npx netlify-cli deploy --dir web-tester`        # draft/preview deploy
3. Verify the preview URL loads, auto-connects, dot moves (WASD).
4. `npx netlify-cli deploy --dir web-tester --prod` # promote to production URL

## Notes
- Netlify free tier is plenty: single static file, negligible bandwidth.
- Backend URL is baked into the HTML as a constant. If the Railway domain
  changes, edit `PROD_WS` in `web-tester/index.html` and redeploy.
- No secrets involved. Nothing server-side. Fully static.

## Acceptance
- Public Netlify URL loads to the tester, auto-connects to Railway over wss://.
- Two devices/tabs on the same room see and collide with each other.
