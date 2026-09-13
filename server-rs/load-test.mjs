// Load test: spawn N concurrent clients in ONE room, all sending input,
// and verify the server sustains ~60Hz state with all N players present.
// Also verifies an 11th client is rejected (room full) when N=10.
// Node built-in WebSocket, no deps.
//
// Usage: SMOKE_URL=ws://localhost:3000 node load-test.mjs [N] [room]

const URL = process.env.SMOKE_URL || "ws://localhost:3000";
const N = parseInt(process.argv[2] || "10", 10);
const ROOM = process.argv[3] || "load-" + Math.random().toString(36).slice(2, 7);
const DURATION_MS = 3000;
const PALETTE = ["#E07A5F", "#81B29A", "#7EB2DD", "#F2CC8F", "#D4A5A5", "#A7C4A0"];

const clients = [];
let ready = 0;

function makeClient(i) {
  const ws = new WebSocket(URL);
  const c = {
    ws,
    id: null,
    stateCount: 0,
    first: null,
    last: null,
    maxPlayersSeen: 0,
    seq: 0,
  };
  ws.addEventListener("open", () => {
    ws.send(JSON.stringify({ t: "join", room: ROOM, name: "L" + i, color: PALETTE[i % PALETTE.length] }));
    ready++;
    // Each client drives a different direction so bodies actually collide.
    const angle = (i / N) * Math.PI * 2;
    const ix = Math.cos(angle), iy = Math.sin(angle);
    c.inputTimer = setInterval(() => {
      if (ws.readyState === 1) ws.send(JSON.stringify({ t: "input", x: ix, y: iy, seq: c.seq++ }));
    }, 33);
  });
  ws.addEventListener("message", (ev) => {
    let m; try { m = JSON.parse(ev.data); } catch { return; }
    if (m.t === "welcome") c.id = m.id;
    else if (m.t === "state") {
      const now = Date.now();
      if (c.first === null) c.first = now;
      c.last = now;
      c.stateCount++;
      if (m.players.length > c.maxPlayersSeen) c.maxPlayersSeen = m.players.length;
    }
  });
  ws.addEventListener("error", () => {});
  return c;
}

for (let i = 0; i < N; i++) clients.push(makeClient(i));

// After the main clients settle, test that an extra client is rejected when full.
let fullRejected = null;
setTimeout(() => {
  if (N >= 10) {
    const extra = new WebSocket(URL);
    extra.addEventListener("open", () =>
      extra.send(JSON.stringify({ t: "join", room: ROOM, name: "overflow", color: "#fff" }))
    );
    extra.addEventListener("message", (ev) => {
      let m; try { m = JSON.parse(ev.data); } catch { return; }
      if (m.t === "player_leave" && m.id === "full") fullRejected = true;
      if (m.t === "welcome") fullRejected = false; // got in => NOT rejected
    });
    setTimeout(() => { try { extra.close(); } catch {} }, 800);
  }
}, 1500);

setTimeout(() => {
  let totalHz = 0, minHz = Infinity, maxPlayers = 0, connected = 0;
  for (const c of clients) {
    if (c.id) connected++;
    const span = (c.last - c.first) / 1000 || 1;
    const hz = c.stateCount / span;
    if (c.stateCount > 5) { totalHz += hz; minHz = Math.min(minHz, hz); }
    maxPlayers = Math.max(maxPlayers, c.maxPlayersSeen);
    clearInterval(c.inputTimer);
    try { c.ws.close(); } catch {}
  }
  const avgHz = totalHz / connected;
  console.log("URL:", URL, "room:", ROOM);
  console.log("clients connected:", connected, "/", N);
  console.log("max players in one snapshot:", maxPlayers);
  console.log("avg state Hz:", avgHz.toFixed(1), " min client Hz:", minHz.toFixed(1));
  if (N >= 10) console.log("11th client rejected (room full):", fullRejected);
  console.log("---");
  const ok =
    connected === N &&
    maxPlayers === N &&
    avgHz > 45 &&
    (N < 10 || fullRejected === true);
  console.log(ok ? "PASS" : "FAIL");
  process.exit(ok ? 0 : 1);
}, DURATION_MS);
