//! Room state and per-tick physics (spec §1.4, §B3 room.rs).

use crate::level::{Level, CAMPAIGN_LEVELS};
use crate::physics::{
    integrate, pillars, resolve_circle_circle, resolve_walls, Body, ACCEL, ARENA_H, ARENA_W, STEP,
};
use crate::protocol::{
    CampaignState, EnemyState, ItemState, PlayerState, ServerMsg, WallState, ZoneState,
};
use std::collections::HashMap;

pub const MAX_PLAYERS: usize = 10;

/// Campaign lifecycle. A room starts in `Lobby`; once every player is ready it
/// auto-advances into `Playing`, runs `CAMPAIGN_LEVELS` levels, then lands in
/// `Victory` and resets ready flags so the group can run it again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Lobby,
    Playing,
    Victory,
}

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
    /// Campaign ready flag (Lobby phase).
    pub ready: bool,
}

pub struct Room {
    #[allow(dead_code)]
    pub id: String,
    pub players: HashMap<String, Player>,
    pub pillars: Vec<Body>,
    pub tick: u32,
    /// Campaign phase for this room.
    pub phase: Phase,
    /// The active level while `phase == Playing`; `None` in Lobby/Victory.
    pub level: Option<Level>,
    /// Seed for procedural generation; re-rolled each time a campaign starts so
    /// repeat runs differ, while a single run stays internally reproducible.
    campaign_seed: u64,
    /// Pre-allocated player-id ordering buffer to avoid per-tick allocation.
    id_buf: Vec<String>,
    /// Pre-allocated snapshot buffer.
    state_buf: Vec<PlayerState>,
    /// Pre-allocated player-body buffer reused for enemy AI + item checks so the
    /// level step never allocates in the hot loop.
    body_buf: Vec<Body>,
}

impl Room {
    pub fn new(id: String) -> Self {
        Room {
            id,
            players: HashMap::new(),
            pillars: pillars(),
            tick: 0,
            phase: Phase::Lobby,
            level: None,
            campaign_seed: 0,
            id_buf: Vec::with_capacity(MAX_PLAYERS),
            state_buf: Vec::with_capacity(MAX_PLAYERS),
            body_buf: Vec::with_capacity(MAX_PLAYERS),
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
    ///
    /// While a campaign is `Playing`, enemies keep chasing, so the room is never
    /// idle — we must stay at 60Hz for the AI to broadcast. Lobby and Victory
    /// still back off to the keepalive cadence when everyone stops.
    pub fn any_motion(&self) -> bool {
        if self.phase == Phase::Playing {
            return true;
        }
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
            ready: false,
        };
        self.players.insert(id, player);
    }

    pub fn remove_player(&mut self, id: &str) {
        self.players.remove(id);
        // If the room empties out mid-campaign, reset to Lobby so the next
        // group starts clean.
        if self.players.is_empty() {
            self.reset_to_lobby();
        }
    }

    /// Set a player's ready flag. Returns true if the campaign should auto-start
    /// as a result (all present players ready, not already playing, ≥1 player).
    /// Ready-up works from both Lobby and Victory (run it again).
    pub fn set_ready(&mut self, id: &str, ready: bool) -> bool {
        if let Some(p) = self.players.get_mut(id) {
            p.ready = ready;
        }
        self.phase != Phase::Playing && self.all_ready()
    }

    fn all_ready(&self) -> bool {
        !self.players.is_empty() && self.players.values().all(|p| p.ready)
    }

    fn reset_to_lobby(&mut self) {
        self.phase = Phase::Lobby;
        self.level = None;
        for p in self.players.values_mut() {
            p.ready = false;
        }
    }

    /// Begin the campaign at level 0. Re-seeds so each run differs. Idempotent
    /// guard: only starts when not already playing (Lobby or Victory).
    pub fn start_campaign(&mut self) {
        if self.phase == Phase::Playing {
            return;
        }
        // Cheap, non-crypto seed from tick + player count. Deterministic within
        // a run; different between runs.
        self.campaign_seed = (self.tick as u64)
            .wrapping_mul(0x2545F4914F6CDD1D)
            ^ (self.players.len() as u64).wrapping_add(1);
        self.phase = Phase::Playing;
        self.load_level(0);
    }

    fn load_level(&mut self, index: u8) {
        self.level = Some(Level::generate(index, self.campaign_seed));
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

        // 6. Campaign step: enemy AI, item pickups, level progression. Only
        //    while playing. `id_buf` gives a stable player ordering to align
        //    the body buffer with player ids for writing results back.
        if self.phase == Phase::Playing {
            self.step_campaign(&mut bumps);
        }

        bumps
    }

    /// Run one campaign tick: step enemies against player bodies, collect items,
    /// and check the level-clear condition (all enemies down + everyone in the
    /// exit zone). Advances the level or ends the campaign as needed.
    fn step_campaign(&mut self, bumps: &mut Vec<ServerMsg>) {
        // Copy current player bodies into the reusable buffer (order = id_buf).
        self.body_buf.clear();
        for id in self.id_buf.iter() {
            if let Some(p) = self.players.get(id) {
                self.body_buf.push(p.body);
            }
        }

        // Take the level out to satisfy the borrow checker (we mutate both the
        // level and self.players). Put it back before returning.
        let mut level = match self.level.take() {
            Some(l) => l,
            None => return,
        };

        // Enemy physics against players; writes pushed-back bodies + returns the
        // per-player impulse for bump VFX.
        let impulses = level.step_enemies(&mut self.body_buf);

        // Item pickups against the (post-collision) player bodies.
        let _collected = level.collect_items(&self.body_buf);

        // Write mutated bodies back to the players and emit enemy bumps.
        for (i, id) in self.id_buf.iter().enumerate() {
            if let Some(p) = self.players.get_mut(id) {
                p.body = self.body_buf[i];
            }
            if impulses[i] > 20.0 {
                bumps.push(ServerMsg::Bump {
                    id: id.clone(),
                    other: "enemy".to_string(),
                    impulse: impulses[i],
                });
            }
        }

        // Progression check: enemies cleared AND every player inside the exit.
        let everyone_in_exit = !self.players.is_empty()
            && self
                .players
                .values()
                .all(|p| level.in_exit(p.body.pos.x, p.body.pos.y));

        if level.enemies_cleared() && everyone_in_exit {
            let next = level.index + 1;
            if next >= CAMPAIGN_LEVELS {
                // Campaign complete. Rest in Victory and clear ready flags so
                // the group re-readies to run again (set_ready allows this).
                self.level = None;
                self.phase = Phase::Victory;
                for p in self.players.values_mut() {
                    p.ready = false;
                }
                bumps.push(ServerMsg::Campaign {
                    phase: "victory".to_string(),
                    level: CAMPAIGN_LEVELS,
                });
                return;
            } else {
                self.level = Some(Level::generate(next, self.campaign_seed));
                bumps.push(ServerMsg::Campaign {
                    phase: "playing".to_string(),
                    level: next,
                });
                return;
            }
        }

        // Not progressing this tick — put the level back.
        self.level = Some(level);
    }

    /// Build the full state snapshot (spec §2.2 `state`), including the campaign
    /// block. The campaign block is always present so clients can render the
    /// lobby ready-up and victory screens; it is `None` only if a room somehow
    /// has no players (never broadcast in that case).
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
                ready: p.ready,
            });
        }
        ServerMsg::State {
            tick: self.tick,
            players: self.state_buf.clone(),
            campaign: Some(self.campaign_snapshot()),
        }
    }

    /// Build the campaign portion of the snapshot from current phase + level.
    fn campaign_snapshot(&self) -> CampaignState {
        let phase = match self.phase {
            Phase::Lobby => "lobby",
            Phase::Playing => "playing",
            Phase::Victory => "victory",
        };
        let total = self.players.len() as u8;

        let (level_idx, walls, enemies, items, exit, in_zone) = match &self.level {
            Some(l) => {
                let walls = l
                    .walls
                    .iter()
                    .map(|w| WallState {
                        x: w.pos.x,
                        y: w.pos.y,
                        r: w.r,
                    })
                    .collect();
                let enemies = l
                    .enemies
                    .iter()
                    .map(|e| EnemyState {
                        x: e.body.pos.x,
                        y: e.body.pos.y,
                        r: e.body.r,
                        hp: e.hp,
                        alive: e.alive,
                    })
                    .collect();
                let items = l
                    .items
                    .iter()
                    .map(|it| ItemState {
                        x: it.x,
                        y: it.y,
                        collected: it.collected,
                    })
                    .collect();
                let exit = ZoneState {
                    x: l.exit.x,
                    y: l.exit.y,
                    r: l.exit.r,
                };
                let in_zone = self
                    .players
                    .values()
                    .filter(|p| l.in_exit(p.body.pos.x, p.body.pos.y))
                    .count() as u8;
                (l.index, walls, enemies, items, exit, in_zone)
            }
            None => (
                0,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                ZoneState {
                    x: 0.0,
                    y: 0.0,
                    r: 0.0,
                },
                0,
            ),
        };

        CampaignState {
            phase: phase.to_string(),
            level: level_idx,
            levels: CAMPAIGN_LEVELS,
            walls,
            enemies,
            items,
            exit,
            in_zone,
            total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_all(room: &mut Room) -> bool {
        let ids: Vec<String> = room.players.keys().cloned().collect();
        let mut trigger = false;
        for id in ids {
            trigger = room.set_ready(&id, true);
        }
        trigger
    }

    /// Force the current level clearable: kill all enemies and teleport every
    /// player into the exit zone. Mirrors what a real group achieves by play.
    fn force_clear(room: &mut Room) {
        if let Some(level) = room.level.as_mut() {
            for e in level.enemies.iter_mut() {
                e.alive = false;
            }
            let (ex, ey) = (level.exit.x, level.exit.y);
            for p in room.players.values_mut() {
                p.body.pos.x = ex;
                p.body.pos.y = ey;
                p.body.vel.x = 0.0;
                p.body.vel.y = 0.0;
            }
        }
    }

    #[test]
    fn auto_starts_when_all_ready() {
        let mut room = Room::new("t".into());
        room.add_player("a".into(), "A".into(), "#fff".into());
        room.add_player("b".into(), "B".into(), "#fff".into());
        assert_eq!(room.phase, Phase::Lobby);

        // One ready is not enough.
        assert!(!room.set_ready("a", true));
        assert_eq!(room.phase, Phase::Lobby);

        // Both ready -> trigger. Caller starts the campaign.
        assert!(room.set_ready("b", true));
        room.start_campaign();
        assert_eq!(room.phase, Phase::Playing);
        let l = room.level.as_ref().expect("level generated");
        assert_eq!(l.index, 0);
        assert!(!l.walls.is_empty());
        assert!(!l.enemies.is_empty());
        assert!(!l.items.is_empty());
    }

    #[test]
    fn progresses_through_all_levels_to_victory() {
        let mut room = Room::new("t".into());
        room.add_player("a".into(), "A".into(), "#fff".into());
        room.add_player("b".into(), "B".into(), "#fff".into());
        assert!(ready_all(&mut room));
        room.start_campaign();

        // Clear each of the 3 levels. After each clear tick the level index
        // advances; after the last it flips to Victory.
        for expected in 0..CAMPAIGN_LEVELS {
            assert_eq!(room.phase, Phase::Playing);
            assert_eq!(room.level.as_ref().unwrap().index, expected);
            force_clear(&mut room);
            room.tick();
        }
        assert_eq!(room.phase, Phase::Victory);
        assert!(room.level.is_none());
        // Ready flags reset so the group can run it again.
        assert!(room.players.values().all(|p| !p.ready));
    }

    #[test]
    fn can_restart_from_victory() {
        let mut room = Room::new("t".into());
        room.add_player("a".into(), "A".into(), "#fff".into());
        assert!(ready_all(&mut room));
        room.start_campaign();
        for _ in 0..CAMPAIGN_LEVELS {
            force_clear(&mut room);
            room.tick();
        }
        assert_eq!(room.phase, Phase::Victory);

        // Re-ready from Victory triggers another run.
        assert!(room.set_ready("a", true));
        room.start_campaign();
        assert_eq!(room.phase, Phase::Playing);
        assert_eq!(room.level.as_ref().unwrap().index, 0);
    }

    #[test]
    fn does_not_progress_until_everyone_in_exit() {
        let mut room = Room::new("t".into());
        room.add_player("a".into(), "A".into(), "#fff".into());
        room.add_player("b".into(), "B".into(), "#fff".into());
        assert!(ready_all(&mut room));
        room.start_campaign();

        // Kill enemies and put only ONE player in the exit.
        if let Some(level) = room.level.as_mut() {
            for e in level.enemies.iter_mut() {
                e.alive = false;
            }
            let (ex, ey) = (level.exit.x, level.exit.y);
            let a = room.players.get_mut("a").unwrap();
            a.body.pos.x = ex;
            a.body.pos.y = ey;
        }
        // Move B far from the exit.
        let b = room.players.get_mut("b").unwrap();
        b.body.pos.x = 100.0;
        b.body.pos.y = 100.0;

        room.tick();
        // Still on level 0 because not everyone is in the zone.
        assert_eq!(room.phase, Phase::Playing);
        assert_eq!(room.level.as_ref().unwrap().index, 0);
    }

    #[test]
    fn empty_room_resets_to_lobby() {
        let mut room = Room::new("t".into());
        room.add_player("a".into(), "A".into(), "#fff".into());
        assert!(ready_all(&mut room));
        room.start_campaign();
        assert_eq!(room.phase, Phase::Playing);

        room.remove_player("a");
        assert_eq!(room.phase, Phase::Lobby);
        assert!(room.level.is_none());
    }
}
