//! Speck authoritative WebSocket game server (spec §B).

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

/// A room plus the outbound channels for every connected client in it.
struct RoomHandle {
    room: Room,
    clients: HashMap<String, UnboundedSender<ServerMsg>>,
}

impl RoomHandle {
    fn new(id: String) -> Self {
        RoomHandle {
            room: Room::new(id),
            clients: HashMap::new(),
        }
    }

    fn broadcast(&self, msg: &ServerMsg) {
        for tx in self.clients.values() {
            let _ = tx.send(msg.clone());
        }
    }

    fn broadcast_except(&self, except: &str, msg: &ServerMsg) {
        for (id, tx) in self.clients.iter() {
            if id != except {
                let _ = tx.send(msg.clone());
            }
        }
    }

    fn send_to(&self, id: &str, msg: &ServerMsg) {
        if let Some(tx) = self.clients.get(id) {
            let _ = tx.send(msg.clone());
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
                let rooms = rooms.clone();
                tokio::spawn(handle_connection(stream, rooms));
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

/// Fixed 60Hz loop: tick every room, collect outbound messages, then send
/// AFTER releasing the write lock (spec §B3 game_loop, pitfall §10).
async fn game_loop(rooms: Rooms) {
    let mut next = Instant::now();
    loop {
        next += STEP_DURATION;

        // Outbound (room_id -> messages) collected under the lock, sent after.
        let mut outbound: Vec<(String, Vec<ServerMsg>)> = Vec::new();
        let mut empty_rooms: Vec<String> = Vec::new();

        {
            let mut guard = rooms.write().await;
            for (room_id, handle) in guard.iter_mut() {
                if handle.room.is_empty() {
                    empty_rooms.push(room_id.clone());
                    continue;
                }
                let mut msgs = handle.room.tick();
                msgs.push(handle.room.state_snapshot());
                outbound.push((room_id.clone(), msgs));
            }
            for id in &empty_rooms {
                guard.remove(id);
            }
        } // lock released here

        // Send after releasing the lock.
        {
            let guard = rooms.read().await;
            for (room_id, msgs) in outbound {
                if let Some(handle) = guard.get(&room_id) {
                    for msg in msgs {
                        match &msg {
                            // Bumps target one player; state broadcasts to all.
                            ServerMsg::Bump { .. } => handle.broadcast(&msg),
                            _ => handle.broadcast(&msg),
                        }
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

    // Outbound channel drained by a forward task.
    let (tx, mut rx) = unbounded_channel::<ServerMsg>();
    let forward = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = match serde_json::to_string(&msg) {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ws_tx.send(Message::Text(text)).await.is_err() {
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
                    handle.send_to_direct(&tx, &ServerMsg::PlayerLeave {
                        id: "full".to_string(),
                    });
                    continue;
                }

                handle.room.add_player(player_id.clone(), name.clone(), color.clone());
                handle.clients.insert(player_id.clone(), tx.clone());

                // Notify others.
                handle.broadcast_except(
                    &player_id,
                    &ServerMsg::PlayerJoin {
                        id: player_id.clone(),
                        name: name.clone(),
                        color: color.clone(),
                    },
                );

                // Welcome the newcomer.
                handle.send_to(
                    &player_id,
                    &ServerMsg::Welcome {
                        id: player_id.clone(),
                        tick: handle.room.tick,
                    },
                );

                // Replay existing players to the newcomer.
                for p in handle.room.players.values() {
                    if p.id != player_id {
                        handle.send_to(
                            &player_id,
                            &ServerMsg::PlayerJoin {
                                id: p.id.clone(),
                                name: p.name.clone(),
                                color: p.color.clone(),
                            },
                        );
                    }
                }

                current_room = Some(room);
            }
            ClientMsg::Input { x, y, seq } => {
                if let Some(room_id) = &current_room {
                    let mut guard = rooms.write().await;
                    if let Some(handle) = guard.get_mut(room_id) {
                        handle.room.set_input(&player_id, x, y, seq);
                    }
                }
            }
            ClientMsg::Leave => {
                if let Some(old) = current_room.take() {
                    leave_room(&rooms, &old, &player_id).await;
                }
            }
            ClientMsg::Ping { ts } => {
                let _ = tx.send(ServerMsg::Pong { ts });
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
        handle.broadcast(&ServerMsg::PlayerLeave {
            id: player_id.to_string(),
        });
    }
}

impl RoomHandle {
    /// Send directly to a channel not yet registered in `clients`
    /// (used to reject a join when the room is full).
    fn send_to_direct(&self, tx: &UnboundedSender<ServerMsg>, msg: &ServerMsg) {
        let _ = tx.send(msg.clone());
    }
}
