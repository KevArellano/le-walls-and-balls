//! Speck authoritative WebSocket game server (spec §B).

mod level;
mod physics;
mod protocol;
mod room;

use crate::protocol::{ClientMsg, ServerMsg};
use crate::room::Room;

use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const STEP_DURATION: Duration = Duration::from_nanos(1_000_000_000 / 60);

/// After a room goes fully quiet (no input, everyone at rest), keep sending
/// state at this reduced cadence so late-arriving clients still get a snapshot
/// and connections stay warm — without paying 60Hz for a static scene.
const IDLE_KEEPALIVE_EVERY: u32 = 30; // ~2 snapshots/sec while idle

/// Active `state` broadcast rate. The simulation always runs at 60Hz, but the
/// client interpolates remote players ~90ms behind real time, so 60Hz snapshots
/// are wasted bandwidth for remotes — 20Hz looks identical after interpolation
/// and cuts egress ~3×. Bumps and `level`/`campaign` events are still sent
/// immediately (never rate-limited). Set to 1 to broadcast every tick (60Hz)
/// for debugging.
const STATE_EVERY: u32 = 3; // 60Hz / 3 = 20Hz

/// Outbound frames are pre-encoded ONCE and shared (cheap `Arc` clone per
/// client) so a 10-client room does one encode per tick, not ten. Control-plane
/// messages are JSON text; the hot-path `state` frame is binary.
#[derive(Clone)]
enum Outbound {
    Text(Arc<str>),
    Binary(Arc<[u8]>),
}

/// Encode a control-plane message as shared JSON text.
fn encode(msg: &ServerMsg) -> Outbound {
    // Serialization of our own types never fails; fall back defensively.
    let s = serde_json::to_string(msg).unwrap_or_else(|_| String::from("{}"));
    Outbound::Text(s.into())
}

/// Wrap an already-built binary state frame as a shared outbound frame.
fn encode_binary(bytes: Vec<u8>) -> Outbound {
    Outbound::Binary(bytes.into())
}

/// A room plus the outbound channels for every connected client in it.
struct RoomHandle {
    room: Room,
    clients: HashMap<String, UnboundedSender<Outbound>>,
    /// Ticks elapsed since the room last had any motion/input.
    idle_ticks: u32,
}

impl RoomHandle {
    fn new(id: String) -> Self {
        RoomHandle {
            room: Room::new(id),
            clients: HashMap::new(),
            idle_ticks: 0,
        }
    }

    /// Broadcast an already-encoded frame (cheap Arc clone per client).
    fn broadcast(&self, frame: &Outbound) {
        for tx in self.clients.values() {
            let _ = tx.send(frame.clone());
        }
    }

    fn broadcast_except(&self, except: &str, frame: &Outbound) {
        for (id, tx) in self.clients.iter() {
            if id != except {
                let _ = tx.send(frame.clone());
            }
        }
    }

    fn send_to(&self, id: &str, frame: &Outbound) {
        if let Some(tx) = self.clients.get(id) {
            let _ = tx.send(frame.clone());
        }
    }
}

type Rooms = Arc<RwLock<HashMap<String, RoomHandle>>>;

#[tokio::main]
async fn main() {
    // PORT-first (Railway injects PORT), then SPECK_PORT, then 3000 (spec §B6).
    let port = std::env::var("PORT")
        .or_else(|_| std::env::var("SPECK_PORT"))
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(3000);

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    println!("speck-server listening on {addr}");

    let rooms: Rooms = Arc::new(RwLock::new(HashMap::new()));

    // Global 60Hz game loop.
    tokio::spawn(game_loop(rooms.clone()));

    // Accept loop.
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                // Disable Nagle's algorithm: send small frames immediately
                // instead of coalescing them. Critical for real-time input/state
                // latency — Nagle can add tens of ms per frame.
                let _ = stream.set_nodelay(true);
                let rooms = rooms.clone();
                tokio::spawn(handle_connection(stream, rooms));
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

/// Fixed 60Hz loop: tick every room, collect outbound frames, then send
/// AFTER releasing the write lock (spec §B3 game_loop, pitfall §10).
async fn game_loop(rooms: Rooms) {
    let mut next = Instant::now();
    loop {
        next += STEP_DURATION;

        // Outbound (room_id -> pre-encoded frames) collected under the lock.
        let mut outbound: Vec<(String, Vec<Outbound>)> = Vec::new();
        let mut empty_rooms: Vec<String> = Vec::new();

        {
            let mut guard = rooms.write().await;
            for (room_id, handle) in guard.iter_mut() {
                if handle.room.is_empty() {
                    empty_rooms.push(room_id.clone());
                    continue;
                }

                let bumps = handle.room.tick();

                // Idle detection: a room is "active" this tick if anything
                // happened (a bump) or any body is still moving. Otherwise we
                // back off to a low keepalive cadence to save CPU/bandwidth.
                let moving = handle.room.any_motion();
                if !bumps.is_empty() || moving {
                    handle.idle_ticks = 0;
                } else {
                    handle.idle_ticks = handle.idle_ticks.saturating_add(1);
                }
                // State cadence: while active, broadcast at STATE_EVERY (20Hz);
                // while idle, fall back to the slower keepalive. Bumps and
                // level/campaign events (in `bumps`) are always sent immediately.
                let idle = handle.idle_ticks > 1;
                let send_state = if idle {
                    handle.idle_ticks % IDLE_KEEPALIVE_EVERY == 0
                } else {
                    handle.room.tick % STATE_EVERY == 0
                };

                // Encode once per room per tick.
                let mut frames: Vec<Outbound> = Vec::with_capacity(bumps.len() + 1);
                for b in &bumps {
                    frames.push(encode(b));
                }
                if send_state {
                    frames.push(encode_binary(handle.room.state_frame()));
                }

                if !frames.is_empty() {
                    outbound.push((room_id.clone(), frames));
                }
            }
            for id in &empty_rooms {
                guard.remove(id);
            }
        } // write lock released here

        // Send after releasing the write lock (never send while write-locked).
        {
            let guard = rooms.read().await;
            for (room_id, frames) in outbound {
                if let Some(handle) = guard.get(&room_id) {
                    for frame in &frames {
                        handle.broadcast(frame);
                    }
                }
            }
        }

        // Sleep until next tick; if behind, skip ahead rather than spiral.
        let now = Instant::now();
        if next > now {
            tokio::time::sleep(next - now).await;
        } else {
            next = now;
        }
    }
}

async fn handle_connection(stream: TcpStream, rooms: Rooms) {
    // A failed WS handshake is a plain HTTP probe (e.g. TCP health check) → drop.
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(_) => return,
    };

    let player_id = Uuid::new_v4().to_string();
    let (mut ws_tx, mut ws_rx) = ws.split();

    // Outbound channel drained by a forward task. Frames arrive pre-serialized.
    let (tx, mut rx) = unbounded_channel::<Outbound>();
    let forward = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            // Frame is already encoded; map to the matching WS frame kind.
            let ws_msg = match frame {
                Outbound::Text(s) => Message::Text(s.to_string()),
                Outbound::Binary(b) => Message::Binary(b.to_vec()),
            };
            if ws_tx.send(ws_msg).await.is_err() {
                break;
            }
        }
    });

    let mut current_room: Option<String> = None;

    while let Some(frame) = ws_rx.next().await {
        let msg = match frame {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Binary(_)) => continue,
            Ok(Message::Frame(_)) => continue,
            Err(_) => break,
        };

        let parsed: ClientMsg = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(_) => continue,
        };

        match parsed {
            ClientMsg::Join { room, name, color } => {
                // Leave old room first.
                if let Some(old) = current_room.take() {
                    leave_room(&rooms, &old, &player_id).await;
                }

                let mut guard = rooms.write().await;
                let handle = guard
                    .entry(room.clone())
                    .or_insert_with(|| RoomHandle::new(room.clone()));

                if handle.room.is_full() {
                    let _ = tx.send(encode(&ServerMsg::PlayerLeave {
                        id: "full".to_string(),
                    }));
                    continue;
                }

                let my_idx =
                    handle
                        .room
                        .add_player(player_id.clone(), name.clone(), color.clone());
                handle.clients.insert(player_id.clone(), tx.clone());
                // A join is motion-worthy: wake the room from idle immediately.
                handle.idle_ticks = 0;

                // Notify others.
                handle.broadcast_except(
                    &player_id,
                    &encode(&ServerMsg::PlayerJoin {
                        id: player_id.clone(),
                        idx: my_idx,
                        name: name.clone(),
                        color: color.clone(),
                    }),
                );

                // Welcome the newcomer.
                handle.send_to(
                    &player_id,
                    &encode(&ServerMsg::Welcome {
                        id: player_id.clone(),
                        idx: my_idx,
                        tick: handle.room.tick,
                    }),
                );

                // Replay existing players to the newcomer.
                for p in handle.room.players.values() {
                    if p.id != player_id {
                        handle.send_to(
                            &player_id,
                            &encode(&ServerMsg::PlayerJoin {
                                id: p.id.clone(),
                                idx: p.idx,
                                name: p.name.clone(),
                                color: p.color.clone(),
                            }),
                        );
                    }
                }

                // If a campaign is already in progress, send the newcomer the
                // static level geometry so they can render it (they won't get it
                // from the per-tick deltas otherwise).
                if handle.room.has_level() {
                    let lvl = handle.room.level_message();
                    handle.send_to(&player_id, &encode(&lvl));
                }

                current_room = Some(room);
            }
            ClientMsg::Input { x, y, seq } => {
                if let Some(room_id) = &current_room {
                    let mut guard = rooms.write().await;
                    if let Some(handle) = guard.get_mut(room_id) {
                        handle.room.set_input(&player_id, x, y, seq);
                        // Input is motion: wake the room so state resumes 60Hz.
                        handle.idle_ticks = 0;
                    }
                }
            }
            ClientMsg::Leave => {
                if let Some(old) = current_room.take() {
                    leave_room(&rooms, &old, &player_id).await;
                }
            }
            ClientMsg::Ready { ready } => {
                if let Some(room_id) = &current_room {
                    let mut guard = rooms.write().await;
                    if let Some(handle) = guard.get_mut(room_id) {
                        // Toggling ready is a meaningful event: wake the room so
                        // the ready state broadcasts promptly.
                        handle.idle_ticks = 0;
                        // Auto-start the moment every player is ready. Broadcast
                        // the one-shot level geometry so all clients can render
                        // the freshly generated arena.
                        if handle.room.set_ready(&player_id, ready) {
                            if let Some(level_msg) = handle.room.start_campaign() {
                                handle.broadcast(&encode(&level_msg));
                            }
                        }
                    }
                }
            }
            ClientMsg::Ping { ts } => {
                // Reply immediately, bypassing the game loop.
                let _ = tx.send(encode(&ServerMsg::Pong { ts }));
            }
        }
    }

    // Disconnect: leave room, stop forwarding.
    if let Some(old) = current_room.take() {
        leave_room(&rooms, &old, &player_id).await;
    }
    forward.abort();
}

async fn leave_room(rooms: &Rooms, room_id: &str, player_id: &str) {
    let mut guard = rooms.write().await;
    if let Some(handle) = guard.get_mut(room_id) {
        handle.room.remove_player(player_id);
        handle.clients.remove(player_id);
        handle.broadcast(&encode(&ServerMsg::PlayerLeave {
            id: player_id.to_string(),
        }));
    }
}
