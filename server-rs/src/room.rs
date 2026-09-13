//! Room state and per-tick physics (spec §1.4, §B3 room.rs).

use crate::physics::{
    integrate, pillars, resolve_circle_circle, resolve_walls, Body, ACCEL, ARENA_H, ARENA_W, STEP,
};
use crate::protocol::{PlayerState, ServerMsg};
use std::collections::HashMap;

pub const MAX_PLAYERS: usize = 10;

#[derive(Clone, Copy, Default)]
pub struct PlayerInput {
    pub x: f64,
    pub y: f64,
    pub seq: u32,
}

pub struct Player {
    pub id: String,
    pub name: String,
    pub color: String,
    pub body: Body,
    pub input: PlayerInput,
    pub last_seq: u32,
}

pub struct Room {
    #[allow(dead_code)]
    pub id: String,
    pub players: HashMap<String, Player>,
    pub pillars: Vec<Body>,
    pub tick: u32,
    /// Pre-allocated player-id ordering buffer to avoid per-tick allocation.
    id_buf: Vec<String>,
    /// Pre-allocated snapshot buffer.
    state_buf: Vec<PlayerState>,
}

impl Room {
    pub fn new(id: String) -> Self {
        Room {
            id,
            players: HashMap::new(),
            pillars: pillars(),
            tick: 0,
            id_buf: Vec::with_capacity(MAX_PLAYERS),
            state_buf: Vec::with_capacity(MAX_PLAYERS),
        }
    }

    pub fn is_full(&self) -> bool {
        self.players.len() >= MAX_PLAYERS
    }

    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// True if any player is still moving or has pending input. Used by the
    /// game loop to back off broadcasting when the whole room is at rest.
    pub fn any_motion(&self) -> bool {
        const REST_SPEED_SQ: f64 = 1.0; // < 1 unit/sec is effectively stopped
        self.players.values().any(|p| {
            let has_input = p.input.x != 0.0 || p.input.y != 0.0;
            let v = &p.body.vel;
            let speed_sq = v.x * v.x + v.y * v.y;
            has_input || speed_sq > REST_SPEED_SQ
        })
    }

    /// Add a player at a spawn position on the ring of radius 200 (spec §0).
    pub fn add_player(&mut self, id: String, name: String, color: String) {
        let index = self.players.len();
        let angle = index as f64 * (2.0 * std::f64::consts::PI / MAX_PLAYERS as f64);
        let x = ARENA_W / 2.0 + angle.cos() * 200.0;
        let y = ARENA_H / 2.0 + angle.sin() * 200.0;
        let player = Player {
            id: id.clone(),
            name,
            color,
            body: Body::new_player(x, y),
            input: PlayerInput::default(),
            last_seq: 0,
        };
        self.players.insert(id, player);
    }

    pub fn remove_player(&mut self, id: &str) {
        self.players.remove(id);
    }

    /// Set normalized input; scale down only when magnitude > 1 (spec §B3).
    pub fn set_input(&mut self, id: &str, x: f64, y: f64, seq: u32) {
        if let Some(p) = self.players.get_mut(id) {
            let mag = (x * x + y * y).sqrt();
            let (nx, ny) = if mag > 1.0 { (x / mag, y / mag) } else { (x, y) };
            p.input = PlayerInput { x: nx, y: ny, seq };
        }
    }

    /// Advance one 60Hz tick. Returns bump events to broadcast (spec §1.4).
    pub fn tick(&mut self) -> Vec<ServerMsg> {
        self.tick = self.tick.wrapping_add(1);
        let mut bumps: Vec<ServerMsg> = Vec::new();

        // 2. Integrate each player from its input.
        self.id_buf.clear();
        for (id, p) in self.players.iter_mut() {
            integrate(&mut p.body, p.input.x * ACCEL, p.input.y * ACCEL, STEP);
            p.last_seq = p.input.seq;
            self.id_buf.push(id.clone());
        }

        // 3. Player ↔ pillar collisions (static). Emit bump if impulse > 30.
        for id in self.id_buf.iter() {
            if let Some(p) = self.players.get_mut(id) {
                for pillar in self.pillars.iter_mut() {
                    let mag = resolve_circle_circle(&mut p.body, pillar);
                    if mag > 30.0 {
                        bumps.push(ServerMsg::Bump {
                            id: id.clone(),
                            other: "pillar".to_string(),
                            impulse: mag,
                        });
                    }
                }
            }
        }

        // 4. Player ↔ player collisions — O(n²), n ≤ 10. Emit bump if magnitude > 20.
        let ids = self.id_buf.clone();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                // Split borrow: take two bodies out, resolve, write back.
                let (a_id, b_id) = (&ids[i], &ids[j]);
                let mut a_body = self.players[a_id].body;
                let mut b_body = self.players[b_id].body;
                let mag = resolve_circle_circle(&mut a_body, &mut b_body);
                self.players.get_mut(a_id).unwrap().body = a_body;
                self.players.get_mut(b_id).unwrap().body = b_body;
                if mag > 20.0 {
                    bumps.push(ServerMsg::Bump {
                        id: a_id.clone(),
                        other: b_id.clone(),
                        impulse: mag,
                    });
                }
            }
        }

        // 5. Wall collisions per player. Emit bump if hit > 40.
        for id in self.id_buf.iter() {
            if let Some(p) = self.players.get_mut(id) {
                let hit = resolve_walls(&mut p.body);
                if hit > 40.0 {
                    bumps.push(ServerMsg::Bump {
                        id: id.clone(),
                        other: "wall".to_string(),
                        impulse: hit,
                    });
                }
            }
        }

        bumps
    }

    /// Build the full state snapshot (spec §2.2 `state`).
    pub fn state_snapshot(&mut self) -> ServerMsg {
        self.state_buf.clear();
        for p in self.players.values() {
            self.state_buf.push(PlayerState {
                id: p.id.clone(),
                name: p.name.clone(),
                color: p.color.clone(),
                x: p.body.pos.x,
                y: p.body.pos.y,
                vx: p.body.vel.x,
                vy: p.body.vel.y,
                seq: p.last_seq,
            });
        }
        ServerMsg::State {
            tick: self.tick,
            players: self.state_buf.clone(),
        }
    }
}
