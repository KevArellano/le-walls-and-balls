//! Campaign levels: procedurally generated arenas with walls, enemies, items,
//! and a shared exit zone (spec: campaign extension).
//!
//! Design goals mirror the rest of the server: allocate on transition, never in
//! the hot tick loop. A `Level` owns reusable `Vec`s that are cleared + refilled
//! when a new level is generated, and read cheaply every tick.

use crate::physics::{
    integrate, resolve_circle_circle, resolve_walls, Body, ARENA_H, ARENA_W, PLAYER_R, STEP,
    WALL_PAD,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Number of levels in a campaign run before Victory.
pub const CAMPAIGN_LEVELS: u8 = 3;

/// Enemy behaviour tuning. Enemies are slow "chasers" — a soft threat the group
/// clears by ramming. Kept well under player speed so bumping is always viable.
const ENEMY_R: f64 = 26.0;
const ENEMY_SPEED: f64 = 95.0;
const ENEMY_ACCEL: f64 = 520.0;
/// Impulse magnitude a player bump must exceed to damage an enemy.
const ENEMY_HIT_IMPULSE: f64 = 55.0;

const ITEM_R: f64 = 16.0;

/// Exit zone radius; players must gather inside it to progress.
const EXIT_R: f64 = 130.0;

/// A roaming enemy the players clear by bumping. `hp` drops on hard hits.
#[derive(Clone, Copy)]
pub struct Enemy {
    pub body: Body,
    pub hp: i32,
    pub alive: bool,
}

/// A collectible pickup. Removed from the active set once grabbed.
#[derive(Clone, Copy)]
pub struct Item {
    pub x: f64,
    pub y: f64,
    pub collected: bool,
}

/// The shared region every player must occupy (with enemies cleared) to advance.
#[derive(Clone, Copy)]
pub struct ExitZone {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// A fully generated level. Walls reuse the static `Body` collision path, so no
/// new physics is introduced — only new placement.
pub struct Level {
    pub index: u8,
    pub walls: Vec<Body>,
    pub enemies: Vec<Enemy>,
    pub items: Vec<Item>,
    pub exit: ExitZone,
}

impl Level {
    /// Procedurally generate level `index` (0-based) from a seed. Same seed +
    /// index always yields the same arena, which keeps runs reproducible for
    /// debugging while still feeling fresh between campaigns.
    ///
    /// Difficulty scales with `index`: more walls, more and tougher enemies,
    /// slightly fewer items.
    pub fn generate(index: u8, seed: u64) -> Self {
        // Distinct stream per level so adjacent levels don't correlate.
        let mut rng = StdRng::seed_from_u64(seed ^ ((index as u64).wrapping_mul(0x9E3779B9)));

        let lvl = index as i32;
        let wall_count = 4 + lvl * 2; // 4, 6, 8
        let enemy_count = 3 + lvl * 2; // 3, 5, 7
        let item_count = (5 - lvl).max(2); // 5, 4, 3
        let enemy_hp = 2 + lvl; // 2, 3, 4

        // Exit zone: place it away from center so the group has to traverse.
        // Alternate corners by level for variety.
        let (ex, ey) = exit_anchor(index, &mut rng);
        let exit = ExitZone {
            x: ex,
            y: ey,
            r: EXIT_R,
        };

        // Playable inset bounds shared by all placements.
        let bounds = Bounds {
            min_x: WALL_PAD + PLAYER_R + 40.0,
            max_x: ARENA_W - WALL_PAD - PLAYER_R - 40.0,
            min_y: WALL_PAD + PLAYER_R + 40.0,
            max_y: ARENA_H - WALL_PAD - PLAYER_R - 40.0,
        };

        // Reserve zones we must keep clear so the level is always solvable:
        // the spawn ring around center, and the exit zone.
        let center = (ARENA_W / 2.0, ARENA_H / 2.0);
        let keepouts = [
            (center.0, center.1, 260.0), // spawn ring (radius 200 + margin)
            (exit.x, exit.y, exit.r + 80.0),
        ];

        // --- Walls (static pillars of varied radius) ---
        let mut walls: Vec<Body> = Vec::with_capacity(wall_count as usize);
        let mut placed: Vec<(f64, f64, f64)> = Vec::new();
        for _ in 0..wall_count {
            let r = rng.gen_range(34.0..64.0);
            if let Some((x, y)) = sample_point(&mut rng, bounds, r + 30.0, &keepouts, &placed) {
                walls.push(Body::new_pillar(x, y, r));
                placed.push((x, y, r));
            }
        }

        // --- Enemies (dynamic chasers) ---
        let mut enemies: Vec<Enemy> = Vec::with_capacity(enemy_count as usize);
        for _ in 0..enemy_count {
            if let Some((x, y)) = sample_point(&mut rng, bounds, ENEMY_R + 24.0, &keepouts, &placed)
            {
                let mut body = Body::new_player(x, y);
                body.r = ENEMY_R;
                enemies.push(Enemy {
                    body,
                    hp: enemy_hp,
                    alive: true,
                });
                placed.push((x, y, ENEMY_R));
            }
        }

        // --- Items (static pickups) ---
        let mut items: Vec<Item> = Vec::with_capacity(item_count as usize);
        for _ in 0..item_count {
            if let Some((x, y)) = sample_point(&mut rng, bounds, ITEM_R + 20.0, &keepouts, &placed) {
                items.push(Item {
                    x,
                    y,
                    collected: false,
                });
                placed.push((x, y, ITEM_R));
            }
        }

        Level {
            index,
            walls,
            enemies,
            items,
            exit,
        }
    }

    /// Advance enemy AI + physics one step. Enemies seek the nearest player,
    /// collide with walls, each other, and players. Returns collision impulses
    /// against players as `(player_index_in_slice, impulse)` for bump VFX —
    /// but damage is applied here so the room stays simple.
    ///
    /// `players` are the dynamic player bodies for this tick (already integrated
    /// by the room). We read their positions to steer and write back the impulse
    /// exchange so ramming an enemy pushes the player, matching player↔player.
    pub fn step_enemies(&mut self, players: &mut [Body]) -> Vec<f64> {
        // Per-player accumulated impulse for optional bump events (index-aligned).
        let mut player_impulse = vec![0.0_f64; players.len()];

        // 1. Steer + integrate each living enemy toward the nearest player.
        for e in self.enemies.iter_mut() {
            if !e.alive {
                continue;
            }
            let mut best_d2 = f64::INFINITY;
            let mut tx = 0.0;
            let mut ty = 0.0;
            for p in players.iter() {
                let dx = p.pos.x - e.body.pos.x;
                let dy = p.pos.y - e.body.pos.y;
                let d2 = dx * dx + dy * dy;
                if d2 < best_d2 {
                    best_d2 = d2;
                    tx = dx;
                    ty = dy;
                }
            }
            let (ax, ay) = if best_d2.is_finite() && best_d2 > 1.0 {
                let d = best_d2.sqrt();
                (tx / d * ENEMY_ACCEL, ty / d * ENEMY_ACCEL)
            } else {
                (0.0, 0.0)
            };
            integrate(&mut e.body, ax, ay, STEP);
            // Cap enemy speed below players so ramming is always possible.
            let sp = (e.body.vel.x * e.body.vel.x + e.body.vel.y * e.body.vel.y).sqrt();
            if sp > ENEMY_SPEED {
                let s = ENEMY_SPEED / sp;
                e.body.vel.x *= s;
                e.body.vel.y *= s;
            }
        }

        // 2. Enemy ↔ wall.
        for e in self.enemies.iter_mut() {
            if e.alive {
                resolve_walls(&mut e.body);
            }
        }

        // 3. Enemy ↔ wall pillars (static).
        for e in self.enemies.iter_mut() {
            if !e.alive {
                continue;
            }
            for w in self.walls.iter_mut() {
                resolve_circle_circle(&mut e.body, w);
            }
        }

        // 4. Enemy ↔ player. Damage the enemy on a hard hit; always exchange
        //    impulse so the collision feels physical.
        for e in self.enemies.iter_mut() {
            if !e.alive {
                continue;
            }
            for (i, pb) in players.iter_mut().enumerate() {
                let mut enemy_body = e.body;
                let mag = resolve_circle_circle(pb, &mut enemy_body);
                e.body = enemy_body;
                if mag > 0.0 {
                    player_impulse[i] = player_impulse[i].max(mag);
                    if mag > ENEMY_HIT_IMPULSE {
                        e.hp -= 1;
                        if e.hp <= 0 {
                            e.alive = false;
                        }
                    }
                }
            }
        }

        player_impulse
    }

    /// Check item pickups against the given player bodies. Returns the number
    /// collected this tick (for score/VFX). Cheap: O(items · players), both small.
    pub fn collect_items(&mut self, players: &[Body]) -> u32 {
        let mut collected = 0;
        for item in self.items.iter_mut() {
            if item.collected {
                continue;
            }
            for p in players.iter() {
                let dx = p.pos.x - item.x;
                let dy = p.pos.y - item.y;
                let rr = PLAYER_R + ITEM_R;
                if dx * dx + dy * dy <= rr * rr {
                    item.collected = true;
                    collected += 1;
                    break;
                }
            }
        }
        collected
    }

    /// True once every enemy is cleared.
    pub fn enemies_cleared(&self) -> bool {
        self.enemies.iter().all(|e| !e.alive)
    }

    /// True if the given point is inside the exit zone.
    pub fn in_exit(&self, x: f64, y: f64) -> bool {
        let dx = x - self.exit.x;
        let dy = y - self.exit.y;
        dx * dx + dy * dy <= self.exit.r * self.exit.r
    }
}

/// Pick an exit anchor that rotates through corners by level index, jittered.
fn exit_anchor(index: u8, rng: &mut StdRng) -> (f64, f64) {
    let margin = 220.0;
    let corners = [
        (ARENA_W - margin, ARENA_H - margin),
        (margin, margin),
        (ARENA_W - margin, margin),
        (margin, ARENA_H - margin),
    ];
    let (bx, by) = corners[(index as usize) % corners.len()];
    let jx = rng.gen_range(-60.0..60.0);
    let jy = rng.gen_range(-60.0..60.0);
    (bx + jx, by + jy)
}

/// Playable placement bounds (inset arena rectangle).
#[derive(Clone, Copy)]
struct Bounds {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

/// Rejection-sample a point inside `bounds` that clears all keepout circles and
/// previously placed entities by `clearance`. Returns `None` if it can't find a
/// spot in a bounded number of tries (level just ends up slightly sparser,
/// which is fine and keeps generation O(1) worst case).
fn sample_point(
    rng: &mut StdRng,
    bounds: Bounds,
    clearance: f64,
    keepouts: &[(f64, f64, f64)],
    placed: &[(f64, f64, f64)],
) -> Option<(f64, f64)> {
    for _ in 0..24 {
        let x = rng.gen_range(bounds.min_x..bounds.max_x);
        let y = rng.gen_range(bounds.min_y..bounds.max_y);

        let mut ok = true;
        for &(kx, ky, kr) in keepouts {
            let dx = x - kx;
            let dy = y - ky;
            if dx * dx + dy * dy < (kr + clearance) * (kr + clearance) {
                ok = false;
                break;
            }
        }
        if ok {
            for &(px, py, pr) in placed {
                let dx = x - px;
                let dy = y - py;
                let min_d = pr + clearance;
                if dx * dx + dy * dy < min_d * min_d {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Some((x, y));
        }
    }
    None
}
