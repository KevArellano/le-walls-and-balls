# Future — Netcode Optimizations (transport & wire size)

Backlog of "faster / smaller" ideas beyond the current v3 binary state protocol.
NONE of these are needed for small-room MVP; capture only. Current state is a good
stopping point (~10 bytes/player flat, 20Hz cadence, binary hot path).

Context: the hot-path `state` message is already a compact little-endian binary WS
frame (spec §2.7). Control-plane messages stay JSON. Rooms are small (≤10). See the
egress history: JSON v1 → compact JSON v2 (~25-30× cut) → binary v3.

## Faster than WebSocket (latency, esp. under packet loss)

- [ ] **WebTransport (HTTP/3 / QUIC).** The real successor for browser games.
  Unreliable, unordered datagrams avoid TCP head-of-line blocking — a dropped state
  frame is skipped, not stalled behind a retransmit. Correct behavior for a game
  that sends a fresh snapshot every tick.
  - Cost: genuine architecture change (QUIC server, connection lifecycle, replace
    `tokio-tungstenite`); Safari support incomplete → keep a WS fallback.
  - Win shows up under loss (mobile/congested), not on a clean link.
- [ ] **WebRTC data channels (unreliable mode).** Same UDP/unordered benefit, longer
    browser history, but heavyweight signaling/ICE/DTLS built for P2P — awkward for
    client-server. Only if WebTransport isn't viable.
- [ ] Raw UDP — NOT possible from a browser client. Native clients only.

## Smaller than the current binary frame

- [ ] **Bit-packing (cheapest structural win).** Fields are byte-aligned today. An x
    in an 1800-wide arena needs ~11 bits (not 16); velocity ±460 ~10 bits. Bit-level
    packing trims a player record from ~10 bytes toward ~6-7 losslessly.
    Contained change to the encoder/decoder; no transport churn. **Do this first if
    we pursue "smaller".**
- [ ] **Coarser quantization (trade precision for bits).** Quantize position to a
    2-unit grid (invisible after interpolation) and velocity to buckets. This is
    where "smaller than binary" really lives — sending less information on purpose.
- [ ] **Per-field changed-mask + delta values.** Most players barely move between two
    20Hz frames; send a bitmask of which fields changed and only small signed deltas.
- [ ] **Entropy/range coding.** Run the frame through an arithmetic coder keyed on the
    value distribution — approaches the information-theoretic floor (pro netcode).
    Materially more complex + harder to debug. Defer until we hit a real ceiling.
- [ ] **Baseline / acknowledged-delta.** Server sends diffs from each client's
    last-acked state. Pairs naturally with WebTransport (must handle lost baselines);
    tricky over unreliable transport.

## Guidance / order of operations
- If pursuing **smaller**: bit-packing → coarser quantization → changed-mask deltas.
  Stop before arithmetic coding unless bandwidth is actually the bottleneck.
- If pursuing **faster**: WebTransport with a WebSocket fallback. Treat as its own
  project, not a tweak.
- Small rooms make all of this optional. Revisit if player counts grow or if
  tail-latency under loss becomes a complaint.
