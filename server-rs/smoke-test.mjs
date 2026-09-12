// Throwaway smoke test for the Speck backend.
// Uses Node's built-in WebSocket (Node 21+). No dependencies.
// Verifies: join -> welcome, ~60Hz state stream, pong echo.

const URL = process.argv[2] || process.env.SMOKE_URL || "ws://localhost:3000";
const DURATION_MS = 1200;

const ws = new WebSocket(URL);

let welcomeId = null;
let stateCount = 0;
let firstStateAt = null;
let lastStateAt = null;
let gotPong = false;
let selfSeen = false;

const fail = (msg) => {
  console.error("FAIL:", msg);
  process.exit(1);
};

ws.addEventListener("open", () => {
  console.log("connected to", URL);
  ws.send(JSON.stringify({ t: "join", room: "smoke", name: "Tester", color: "#E07A5F" }));
  ws.send(JSON.stringify({ t: "ping", ts: Date.now() }));
  let seq = 0;
  const inputTimer = setInterval(() => {
    ws.send(JSON.stringify({ t: "input", x: 1, y: 0, seq: seq++ }));
  }, 33);
  setTimeout(() => clearInterval(inputTimer), DURATION_MS);
});

ws.addEventListener("message", (ev) => {
  let msg;
  try {
    msg = JSON.parse(ev.data);
  } catch {
    return fail("non-JSON frame: " + ev.data);
  }
  switch (msg.t) {
    case "welcome":
      welcomeId = msg.id;
      console.log("welcome id =", msg.id, "tick =", msg.tick);
      break;
    case "state": {
      stateCount++;
      const now = Date.now();
      if (firstStateAt === null) firstStateAt = now;
      lastStateAt = now;
      if (welcomeId && msg.players.some((p) => p.id === welcomeId)) selfSeen = true;
      break;
    }
    case "pong":
      gotPong = true;
      console.log("pong rtt =", Date.now() - msg.ts, "ms");
      break;
    case "player_join":
    case "player_leave":
    case "bump":
      break;
    default:
      console.log("other:", msg.t);
  }
});

ws.addEventListener("error", (e) => fail("socket error: " + (e.message || e)));

setTimeout(() => {
  ws.close();
  const span = (lastStateAt - firstStateAt) / 1000 || 1;
  const rate = stateCount / span;
  console.log("---");
  console.log("state frames:", stateCount, "over", span.toFixed(2), "s =", rate.toFixed(1), "Hz");
  if (!welcomeId) fail("no welcome received");
  if (!gotPong) fail("no pong received");
  if (!selfSeen) fail("self player never appeared in a state snapshot");
  if (stateCount < 40) fail("state rate too low (expected ~60Hz)");
  console.log("PASS: welcome + pong + ~60Hz state stream, self present");
  process.exit(0);
}, DURATION_MS + 300);
