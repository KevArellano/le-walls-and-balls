// Verify idle-room backoff: join, send NO input, count state frames over 2s.
// Expect a low keepalive rate (~2/s), not 60/s and not 0.
const URL = process.env.SMOKE_URL || "ws://localhost:3000";
const ws = new WebSocket(URL);
let states = 0, firstAt = null, lastAt = null, welcomed = false;
ws.addEventListener("open", () => {
  ws.send(JSON.stringify({ t: "join", room: "idle", name: "Idle", color: "#81B29A" }));
});
ws.addEventListener("message", (ev) => {
  const m = JSON.parse(ev.data);
  if (m.t === "welcome") welcomed = true;
  if (m.t === "state") { states++; const n = Date.now(); if (!firstAt) firstAt = n; lastAt = n; }
});
setTimeout(() => {
  ws.close();
  const span = (lastAt - firstAt) / 1000 || 1;
  const rate = states / span;
  console.log("idle state frames:", states, "over", span.toFixed(2), "s =", rate.toFixed(1), "Hz");
  if (!welcomed) { console.error("FAIL: no welcome"); process.exit(1); }
  if (states === 0) { console.error("FAIL: no keepalive states at all"); process.exit(1); }
  if (rate > 15) { console.error("FAIL: idle room still near 60Hz (backoff not working)"); process.exit(1); }
  console.log("PASS: idle backoff active (low keepalive rate, connection kept warm)");
  process.exit(0);
}, 2200);
