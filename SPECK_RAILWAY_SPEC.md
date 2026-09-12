# Speck — Multiplayer Sliding-Dots Game: Rebuild & Railway Deploy Spec

This document specifies everything needed to recreate the **Speck** game logic from
scratch in a fresh project and deploy both halves to **Railway** as two services.

Speck is a top-down multiplayer physics game: each player controls a glowing "dot"
that slides around a walled rink, bumping into other players, static pillars, and
walls. Movement is momentum-based (accelerate, cap speed, exponential friction,
restitution on bounce). The **server owns the authoritative simulation** at 60Hz;
**clients predict locally** for zero-latency feel and reconcile against server state.

The system splits cleanly into two deployables:

- **Backend** — a Rust WebSocket server running authoritative physics (deploy as a Railway service from a Dockerfile).
- **Frontend** — a React + TanStack Start SPA that renders the game on a canvas and connects to the backend over WebSocket (deploy as a Railway service, or any static/SSR host).

---

## 0. Shared Constants (SINGLE SOURCE OF TRUTH)

These values MUST be identical in the frontend (`constants.ts`) and backend
(`physics.rs`). If they drift, client prediction diverges from the server and
players "rubber-band". Treat this table as the contract between the two services.

| Constant       | Value      | Meaning                                            |
|----------------|------------|----------------------------------------------------|
| `ARENA_W`      | `1800`     | Rink width in world units                          |
| `ARENA_H`      | `1200`     | Rink height in world units                         |
| `PLAYER_R`     | `22`       | Player dot radius                                  |
| `PILLAR_R`     | `48`       | Default pillar radius                              |
| `MAX_SPEED`    | `460`      | Max velocity magnitude (units/sec)                 |
| `ACCEL`        | `1650`     | Input acceleration (units/sec²)                    |
| `FRICTION`     | `2.35`     | Exponential velocity damping coefficient           |
| `RESTITUTION`  | `0.82`     | Bounce energy retention on collision               |
| `STEP`         | `1/60`     | Fixed physics timestep (seconds)                   |
| `WALL_PAD`     | `28`       | Inset of playable area from arena edge             |
| `MAX_PLAYERS`  | `10`       | Server room capacity                               |
| `MAX_PEERS`    | `8`        | Client-side "rink full" display threshold          |

### Static pillars (obstacles) — identical on both sides

```
(520,  360, r=48)
(1280, 360, r=48)
(520,  840, r=48)
(1280, 840, r=48)
(900,  600, r=36)   // center pillar, smaller
```

### Player spawn positions

New players spawn spread around the arena center on a ring of radius `200`:

```
angle = playerIndex * (2π / 10)
x = ARENA_W/2 + cos(angle) * 200
y = ARENA_H/2 + sin(angle) * 200
```

### Player colors (client palette)

```
#E07A5F  #81B29A  #7EB2DD  #F2CC8F  #D4A5A5  #A7C4A0
```

---

## 1. Physics Model (shared behavior, implemented in both languages)

Both sides run the **same** fixed-timestep integrator. The server is authoritative;
the client runs it for the local player only, then reconciles.

### 1.1 Integrate (per body, per step)

```
vel.x += ax * dt          // ax = input.x * ACCEL
vel.y += ay * dt          // ay = input.y * ACCEL

speed = hypot(vel.x, vel.y)
if speed > MAX_SPEED:      // hard speed cap
    s = MAX_SPEED / speed
    vel *= s

damp = exp(-FRICTION * dt) // exponential friction (frame-rate independent)
vel *= damp

pos += vel * dt
```

### 1.2 Wall collision (`resolve_walls`)

Playable bounds are inset by `WALL_PAD + body.r` on each side.
On crossing a bound: clamp position back to the edge, and if moving *into* the wall,
reflect that velocity component scaled by `RESTITUTION`. Returns the impact
magnitude (used to trigger a "bump" event when `> 40`).

```
minX = WALL_PAD + r ;  maxX = ARENA_W - WALL_PAD - r
minY = WALL_PAD + r ;  maxY = ARENA_H - WALL_PAD - r
// clamp + reflect vx/vy * RESTITUTION when moving outward
```

### 1.3 Circle–circle collision (`resolve_circle_circle`)

Given bodies A and B with a `is_static` flag on B (pillars are static):

1. Compute delta, distance, overlap. Skip if not overlapping or coincident.
2. **Positional correction**: if B static, push A fully out along the normal.
   Otherwise push A and B apart equally (each by `overlap * 0.5`).
3. **Velocity response**: compute relative velocity along the normal `rvn`.
   If `rvn >= 0` (separating), only apply positional correction.
   Otherwise apply impulse `-(1 + RESTITUTION) * rvn * massFactor`
   (`massFactor` = `1.0` vs static, `0.5` vs dynamic).
4. Returns `max(overlap, impulse)` as the collision magnitude.

### 1.4 Tick order (server room, per 60Hz tick)

1. `tick += 1`
2. For each player: `integrate(body, input.x*ACCEL, input.y*ACCEL, STEP)`; record `last_seq = input.seq`.
3. Player↔pillar collisions (static, push player out + reflect). Emit `bump` if `impulse > 30`.
4. Player↔player collisions — O(n²), n ≤ 10. Separate equally, exchange impulse. Emit `bump` if `magnitude > 20`.
5. Wall collisions per player. Emit `bump` if `hit > 40`.
6. Build state snapshot and broadcast.

---

## 2. Wire Protocol (WebSocket, JSON, tagged by field `t`)

The transport is a single WebSocket carrying JSON text frames. Every message has a
discriminant field `t`. This contract is what binds the two services and MUST match
exactly on both sides.

### 2.1 Client → Server

| `t`      | Fields                          | Notes                                                |
|----------|---------------------------------|------------------------------------------------------|
| `join`   | `room: string`, `name: string`, `color: string` | Join/switch room. Leaving current room first. |
| `input`  | `x: f64`, `y: f64`, `seq: u32`  | Normalized direction (magnitude ≤ 1). `seq` monotonic. |
| `leave`  | —                               | Leave current room.                                  |
| `ping`   | `ts: u64`                       | Latency probe (client timestamp, ms).                |

### 2.2 Server → Client

| `t`             | Fields                                   | Notes                                       |
|-----------------|------------------------------------------|---------------------------------------------|
| `welcome`       | `id: string`, `tick: u32`                | Assigns this client its player id.          |
| `state`         | `tick: u32`, `players: PlayerState[]`    | Full snapshot, broadcast **every tick (60Hz)**. |
| `player_join`   | `id`, `name`, `color`                    | Someone joined (also replayed to newcomer). |
| `player_leave`  | `id`                                     | Someone left. (`id="full"` = room full.)    |
| `pong`          | `ts: u64`                                | Echo of `ping.ts` for RTT.                  |
| `bump`          | `id`, `other`, `impulse: f64`            | Collision event for audio/VFX only.         |

### 2.3 `PlayerState`

```json
{ "id": "uuid", "name": "Player", "color": "#E07A5F",
  "x": 900.0, "y": 600.0, "vx": 120.5, "vy": -45.2, "seq": 142 }
```

`seq` = last input sequence the server processed for that player. The client uses it
to discard acknowledged predicted inputs and replay the rest.

### 2.4 Serde tagging note

Rust uses `#[serde(tag = "t")]` with `#[serde(rename = "...")]` per variant, so the
JSON field is `t` and the value is the lowercase message name. Keep exactly these
string values.

---

# =========================================================
# BACKEND SPEC — Rust WebSocket Game Server
# =========================================================

## B1. Purpose & shape

An authoritative, room-based multiplayer physics server. Single binary, no database,
no filesystem writes, no external services. Holds all rooms in memory, runs one
global 60Hz game loop, and streams state to every connected client over WebSocket.

- **Language:** Rust (edition 2021), async on Tokio.
- **Transport:** WebSocket (`tokio-tungstenite`), JSON via `serde`/`serde_json`.
- **Concurrency:** one task per connection + one global game-loop task; shared state
  behind `Arc<RwLock<HashMap<String, RoomHandle>>>`.
- **Capacity target:** 10 players/room at 60Hz with minimal per-tick allocation.

## B2. Dependencies (`Cargo.toml`)

```toml
[package]
name = "speck-server"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = "0.24"
futures-util = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
rand = "0.8"

[profile.release]
opt-level = 3
lto = true
```

## B3. Module layout

```
server-rs/
  Cargo.toml
  Dockerfile
  railway.toml
  src/
    main.rs       # networking, connection handling, game loop
    physics.rs    # constants, Vec2/Body, integrate + collision resolvers, pillars()
    protocol.rs   # ClientMsg / ServerMsg / PlayerState (serde)
    room.rs       # Room: players, per-tick physics, state snapshot
```

### `physics.rs`
- Constants from §0. `Vec2 { x, y }`, `Body { pos, vel, r, is_static }`.
- `Body::new_player(x,y)`, `Body::new_pillar(x,y,r)`.
- `resolve_circle_circle(a, b) -> f64`, `resolve_walls(body) -> f64`,
  `integrate(body, ax, ay, dt)`, `pillars() -> Vec<Body>` (the 5 pillars from §0).

### `protocol.rs`
- `ClientMsg` (`#[derive(Deserialize)] #[serde(tag="t")]`): `Join{room,name,color}`,
  `Input{x,y,seq}`, `Leave`, `Ping{ts}`.
- `ServerMsg` (`#[derive(Serialize, Clone)] #[serde(tag="t")]`): `Welcome{id,tick}`,
  `State{tick,players}`, `PlayerJoin{id,name,color}`, `PlayerLeave{id}`, `Pong{ts}`,
  `Bump{id,other,impulse}`.
- `PlayerState { id, name, color, x, y, vx, vy, seq }`.

### `room.rs`
- `Room { id, players: HashMap<String,Player>, pillars: Vec<Body>, tick: u32, ... }`
  with pre-allocated `id_buf` and `state_buf` to avoid per-tick allocation.
- `Player { id, name, color, body, input: PlayerInput{x,y,seq}, last_seq }`.
- `add_player` (assigns spawn per §0), `remove_player`, `set_input` (normalizes input,
  scaling down only when magnitude > 1), `tick() -> Vec<ServerMsg>` (bump events),
  `state_snapshot() -> ServerMsg::State`, `is_full`, `is_empty`.
- `MAX_PLAYERS = 10`.

### `main.rs`
- `RoomHandle { room: Room, clients: HashMap<String, UnboundedSender<ServerMsg>> }`
  with `broadcast`, `broadcast_except`, `send_to`.
- `main`: read port from env, bind `TcpListener` on `0.0.0.0:PORT`, spawn game loop,
  accept loop → spawn `handle_connection` per socket.
- **`game_loop`** — fixed 60Hz. Each tick: lock rooms, run `room.tick()` collecting
  bumps, take `state_snapshot()`, **collect outbound messages, release the lock, then
  send** (never send while holding the write lock). Remove empty rooms. Sleep until
  next tick; if behind, skip ahead rather than spiral.
- **`handle_connection`** — perform WS handshake (a failed handshake = plain HTTP
  health probe → just drop it). Assign `player_id = Uuid::v4()`. Spawn a forward task
  draining an `mpsc` channel to the socket. Read loop dispatches `ClientMsg`:
  - `Join` → leave old room, get/create room, reject if full (`PlayerLeave{id:"full"}`),
    add player + client sender, notify others (`PlayerJoin`), send `Welcome`, then
    replay existing players to the newcomer.
  - `Input` → `room.set_input(player_id, x, y, seq)`.
  - `Leave` → leave room.
  - `Ping{ts}` → reply `Pong{ts}` immediately.
  - On disconnect/close → leave room, abort forward task.

## B4. Environment / runtime

- **Port:** read from env var **`SPECK_PORT`** (default `3000` in code; the Dockerfile
  sets `SPECK_PORT=8080`). **On Railway, bind the port Railway provides** — see B6.
- Binds `0.0.0.0` (all interfaces) — required for container networking.
- No secrets, no DB, no volumes. Fully stateless across restarts (rooms are ephemeral).

## B5. Dockerfile (multi-stage, ~30MB runtime image)

```dockerfile
# Build stage
FROM rust:1.82-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY src ./src
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/speck-server /usr/local/bin/speck-server
ENV SPECK_PORT=8080
EXPOSE 8080
CMD ["speck-server"]
```

## B6. Railway deployment (BACKEND service)

Railway routes public HTTPS/WSS traffic to your container and injects a `PORT` env var
that your process must listen on. The code reads `SPECK_PORT`, so bridge the two.

**Recommended: make the server read Railway's `PORT`.** Update `main.rs` to prefer
`PORT`, falling back to `SPECK_PORT`, then `3000`:

```rust
let port = std::env::var("PORT")
    .or_else(|_| std::env::var("SPECK_PORT"))
    .ok()
    .and_then(|s| s.parse::<u16>().ok())
    .unwrap_or(3000);
```

`railway.toml` (already in this repo, keep as-is):

```toml
[build]
dockerfilePath = "Dockerfile"

[deploy]
healthcheckPath = ""          # TCP-only check; the server rejects plain HTTP handshakes
healthcheckTimeout = 5
restartPolicyType = "ALWAYS"
```

### Steps

1. Create a Railway service with **root directory = `server-rs/`** (or a repo that
   contains only the server). Railway detects the `Dockerfile`.
2. **Do not set a fixed `PORT`** — Railway provides it. If you keep the `SPECK_PORT`
   approach instead, set a service variable `SPECK_PORT` to match the injected port,
   or (cleaner) adopt the `PORT`-first code above.
3. Enable a **public domain** on the service. Railway serves it over HTTPS, which
   upgrades WebSocket to **`wss://`** automatically.
4. **Health check:** leave `healthcheckPath` empty (TCP check). An HTTP health path
   fails because the server only speaks the WebSocket handshake and drops plain HTTP.
5. The resulting URL becomes the frontend's `VITE_GAME_SERVER_URL`, e.g.
   `wss://speck-server-production.up.railway.app`.

### Backend acceptance checklist

- Container listens on the injected port on `0.0.0.0`.
- WS handshake succeeds; a `join` yields a `welcome` then a stream of `state` at ~60/s.
- Two clients in the same room see each other and collide.
- Empty rooms are reclaimed; server survives client churn without leaking tasks.

---

# =========================================================
# FRONTEND SPEC — React + TanStack Start Canvas Client
# =========================================================

## F1. Purpose & shape

A single-page app that renders the rink on a 2D canvas, samples local input, runs
client-side prediction for the local dot, interpolates remote dots, and reconciles
against authoritative `state` snapshots from the backend.

- **Stack:** React 19, TanStack Start/Router, Vite, Tailwind v4. (The game core is
  framework-agnostic TypeScript; only mounting uses React.)
- **Rendering:** Canvas 2D with camera, offscreen static-layer caching, particles,
  trails, screen shake, vignette.
- **Audio:** WebAudio (procedural bump/whoosh SFX), unlocked on first user gesture.
- **Input:** keyboard (WASD/arrows), on-screen joystick (mobile), gamepad.

## F2. File layout (game core)

```
src/
  game/
    constants.ts        # §0 shared constants, colors, pillars, storage keys
    types.ts            # Body, Snapshot, Particle, NetMsg union, ControlsProbe
    physics.ts          # resolveCircleCircle, resolveWalls, integrate (mirror of physics.rs)
    input.ts            # Input: keyboard/stick/gamepad → normalized {x,y}
    audio.ts            # GameAudio: unlock, bump(strength), whoosh(speed)
    rink.ts             # Rink class: sim loop, render, camera, prediction target
    app.tsx             # SpeckApp: lobby ↔ session switch, profile persistence
    lobby.tsx           # name/color/room entry UI
    hud.tsx             # in-game overlay (peer list, invite, mute, leave)
    joystick.tsx        # mobile virtual stick
    session-server.tsx  # server-authoritative session: prediction + reconciliation
  lib/multiplayer/
    ws-client.ts        # WsClient: WebSocket wrapper, reconnect, sendInput/ping
    use-server-room.ts  # useServerRoom hook: connection state, peers, event fan-out
  routes/
    index.tsx           # mounts <SpeckApp initialRoom={search.room} />
    __root.tsx          # document shell
```

## F3. Client networking

### `WsClient` (`ws-client.ts`)
- Constructed with `{ url, room, name, color, onEvent, onConnected, onDisconnected }`.
- `connect()` opens the socket; on open sends `{ t:"join", room, name, color }`.
- `sendInput(x, y)` → increments an internal `inputSeq`, sends `{t:"input",x,y,seq}`,
  returns the seq (client stores it for reconciliation).
- `ping()` → `{t:"ping", ts: Date.now()}`.
- `close()` → sends `{t:"leave"}`, closes socket, cancels reconnect.
- **Auto-reconnect** every 1s on close (unless explicitly closed).
- Parses each incoming JSON frame into a `ServerEvent` and calls `onEvent`.

### `useServerRoom` hook (`use-server-room.ts`)
- Owns a `WsClient`, exposes `{ selfId, peers, joined, connected, latencyMs,
  sendInput, onStateUpdate, onBump }`.
- Maps events: `welcome` → set `selfId`, `joined=true`; `state` → fan out to
  state listeners; `player_join`/`player_leave` → maintain peer map; `pong` →
  `latencyMs = Date.now() - ts`; `bump` → fan out to bump listeners.
- Pings every 2s while connected.

## F4. Client-side prediction & reconciliation (`session-server.tsx`)

This is the heart of the "zero-latency feel". The `Rink` already runs full local
physics for the self dot every frame. The session layer corrects it against server
truth:

- Keep an `inputBuffer` of `{seq, x, y}` frames sent to the server.
- Every animation frame: sample input; if nonzero (or was nonzero last frame),
  `sendInput` and push `{seq,x,y}` to the buffer. Trim to `MAX_INPUT_BUFFER = 64`.
- On each `state` update, find self in `players`:
  1. Drop buffered inputs with `seq <= server.seq`.
  2. Starting from the server's authoritative `{x,y,vx,vy}`, **replay** the remaining
     unacknowledged inputs using the *same* integrate/wall math (`ACCEL`, `MAX_SPEED`,
     `FRICTION`, `RESTITUTION=0.82`, `WALL_PAD+PLAYER_R` bounds).
  3. Compare replayed position to the locally predicted `rink.self`:
     - error `> TELEPORT_THRESHOLD (150)` → **snap** to reconciled state.
     - error `> 0.5` → nudge position by `errX * CORRECTION_RATE (0.15)`; take server
       velocity directly.
- Remote players: feed each into `rink.applyRemoteState(id, {t:"pos",x,y,vx,vy,color,name})`;
  the Rink interpolates them with a Hermite spline at `INTERP_DELAY_MS = 90` behind
  real time. Drop remotes not present in the latest snapshot.
- `bump` events for the self id trigger `rink.audio.bump(impulse)`.

## F5. Rink simulation & rendering (`rink.ts`)

- Fixed-timestep accumulator loop: at each rAF, add `dt` (capped at `MAX_ACC=0.1`),
  run `step(STEP)` up to 8 times, keep `interpAlpha` for sub-frame interpolation of
  the self dot.
- `step`: sample input → `integrate(self, moveX*ACCEL, moveY*ACCEL, dt, MAX_SPEED, FRICTION)`
  → collide vs pillars/remotes (as static ghosts) → `resolveWalls` → update trail,
  particles, camera (critically-damped spring), audio whoosh over speed 280.
- Rendering: cached offscreen static layer (floor gradient, grid, rink border,
  pillars) redrawn only when the camera moves; dynamic trail, particles, remote dots,
  self dot with glow/squash/flash/specular; cached vignette; screen shake from `trauma`.
- **Server mode flag:** in `session-server.tsx` the Rink is created with
  `showOrbs = false` (server owns all entities; practice orbs are lobby-only).
- Exposes `window.__controlsTest` in dev/QA for automated control checks.

## F6. Lobby, profile, rooms (`app.tsx`)

- Profile `{name, color}` persisted in `localStorage` under `STORAGE_KEY="speck-profile"`.
- Public room id `PUBLIC_ROOM="speck-public"`; private rooms `speck-<CODE>` where CODE
  is sanitized to `[A-Z0-9]{1,8}`. Room code reflected in the URL `?room=` param.
- Lobby shows an ambient (non-playing) Rink preview behind the entry UI.
- Entering a room unlocks audio and mounts `<SessionServer>`.

## F7. Server URL configuration (THE key deploy wiring)

The frontend picks the backend WebSocket URL from a build-time env var:

```ts
serverUrl={import.meta.env.VITE_GAME_SERVER_URL || "ws://localhost:3000"}
```

- **Local dev:** run the Rust server (`cargo run`, default `:3000`); leave the var
  unset → client uses `ws://localhost:3000`.
- **Production:** set `VITE_GAME_SERVER_URL` to the backend's public **`wss://`** URL
  from Railway (§B6), e.g. `wss://speck-server-production.up.railway.app`.
- `VITE_`-prefixed vars are inlined into the client bundle at **build time**, so set
  it as a build variable and rebuild when the backend URL changes.

## F8. Railway deployment (FRONTEND service)

The frontend is a TanStack Start app. Build it and serve the output. On Railway,
create a **second service** for the frontend.

### Option A — Node service (SSR/preview server)
1. Root directory = frontend project root.
2. Build command: `npm ci && npm run build`.
3. Start command: the framework's production server (`npm run start` / the built
   Nitro/Node output) listening on Railway's injected `PORT`.
4. Set build variable **`VITE_GAME_SERVER_URL = wss://<backend-domain>`**.
5. Enable a public domain for the frontend.

### Option B — Static build behind a static host
1. `npm run build` produces the client assets.
2. Serve them with any static file server bound to `PORT`, with SPA fallback to
   `index.html` (so `?room=CODE` deep links work).
3. Same `VITE_GAME_SERVER_URL` build variable.

> Note: this repo's `package.json` is a Vite/TanStack Start setup where `npm run dev`
> serves on `0.0.0.0:8080`. For a clean Railway rebuild you can keep TanStack Start,
> or extract just `src/game/**` + `src/lib/multiplayer/**` into a minimal Vite React
> app — the game core has no dependency on TanStack, auth, or the database.

### Frontend acceptance checklist

- Loads to the lobby with the ambient preview animating, no console errors.
- Entering a room connects (`wss://`), shows self + any peers, movement feels instant.
- Two browsers/devices in the same room see and collide with each other.
- Mobile (≈390×844): joystick works, no horizontal overflow, touch-friendly.
- Latency readout populates; leaving returns to the lobby cleanly.

---

## 9. Two-service wiring summary (Railway)

```
┌────────────────────────────┐         wss://          ┌───────────────────────────┐
│  FRONTEND service          │  ───────────────────▶   │  BACKEND service          │
│  React + TanStack (Vite)   │   join / input / ping   │  Rust WS server (Docker)  │
│  serves canvas game UI     │  ◀───────────────────   │  60Hz authoritative sim   │
│  VITE_GAME_SERVER_URL ─────┼─ points at backend URL  │  reads Railway PORT        │
│  listens on Railway PORT   │   welcome/state/bump    │  listens on Railway PORT   │
└────────────────────────────┘                         └───────────────────────────┘
```

Deploy order: **backend first** → copy its public `wss://` domain → set
`VITE_GAME_SERVER_URL` on the frontend → build/deploy frontend.

## 10. Common pitfalls

- **Physics constant drift** between `physics.ts` and `physics.rs` → rubber-banding.
  Keep §0 as the single source of truth.
- **Wrong scheme:** production must use `wss://` (secure). `ws://` from an HTTPS page
  is blocked by browsers.
- **HTTP health check on the backend** fails — the server rejects non-WebSocket
  handshakes. Use a TCP check (empty `healthcheckPath`).
- **Not reading Railway's `PORT`** — the container must bind the injected port.
- **`VITE_GAME_SERVER_URL` is build-time** — changing it requires a rebuild, not just
  a restart.
- **Sending state inside the room lock** — always collect outbound messages, release
  the lock, then send, or the game loop stalls under load.
```
