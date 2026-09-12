//! Shared physics model — MUST stay in sync with the frontend `physics.ts`.
//! Constants are the single source of truth from spec §0.

pub const ARENA_W: f64 = 1800.0;
pub const ARENA_H: f64 = 1200.0;
pub const PLAYER_R: f64 = 22.0;
pub const MAX_SPEED: f64 = 460.0;
pub const ACCEL: f64 = 1650.0;
pub const FRICTION: f64 = 2.35;
pub const RESTITUTION: f64 = 0.82;
pub const STEP: f64 = 1.0 / 60.0;
pub const WALL_PAD: f64 = 28.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub pos: Vec2,
    pub vel: Vec2,
    pub r: f64,
    pub is_static: bool,
}

impl Body {
    pub fn new_player(x: f64, y: f64) -> Self {
        Body {
            pos: Vec2 { x, y },
            vel: Vec2 { x: 0.0, y: 0.0 },
            r: PLAYER_R,
            is_static: false,
        }
    }

    pub fn new_pillar(x: f64, y: f64, r: f64) -> Self {
        Body {
            pos: Vec2 { x, y },
            vel: Vec2 { x: 0.0, y: 0.0 },
            r,
            is_static: true,
        }
    }
}

/// Fixed-timestep integrator (spec §1.1).
pub fn integrate(body: &mut Body, ax: f64, ay: f64, dt: f64) {
    body.vel.x += ax * dt;
    body.vel.y += ay * dt;

    let speed = (body.vel.x * body.vel.x + body.vel.y * body.vel.y).sqrt();
    if speed > MAX_SPEED {
        let s = MAX_SPEED / speed;
        body.vel.x *= s;
        body.vel.y *= s;
    }

    let damp = (-FRICTION * dt).exp();
    body.vel.x *= damp;
    body.vel.y *= damp;

    body.pos.x += body.vel.x * dt;
    body.pos.y += body.vel.y * dt;
}

/// Wall collision (spec §1.2). Returns the impact magnitude.
pub fn resolve_walls(body: &mut Body) -> f64 {
    let min_x = WALL_PAD + body.r;
    let max_x = ARENA_W - WALL_PAD - body.r;
    let min_y = WALL_PAD + body.r;
    let max_y = ARENA_H - WALL_PAD - body.r;

    let mut impact = 0.0_f64;

    if body.pos.x < min_x {
        body.pos.x = min_x;
        if body.vel.x < 0.0 {
            impact = impact.max(-body.vel.x);
            body.vel.x = -body.vel.x * RESTITUTION;
        }
    } else if body.pos.x > max_x {
        body.pos.x = max_x;
        if body.vel.x > 0.0 {
            impact = impact.max(body.vel.x);
            body.vel.x = -body.vel.x * RESTITUTION;
        }
    }

    if body.pos.y < min_y {
        body.pos.y = min_y;
        if body.vel.y < 0.0 {
            impact = impact.max(-body.vel.y);
            body.vel.y = -body.vel.y * RESTITUTION;
        }
    } else if body.pos.y > max_y {
        body.pos.y = max_y;
        if body.vel.y > 0.0 {
            impact = impact.max(body.vel.y);
            body.vel.y = -body.vel.y * RESTITUTION;
        }
    }

    impact
}

/// Circle–circle collision (spec §1.3). B may be static (pillar).
/// Mutates A always; mutates B only when dynamic. Returns collision magnitude.
pub fn resolve_circle_circle(a: &mut Body, b: &mut Body) -> f64 {
    let dx = b.pos.x - a.pos.x;
    let dy = b.pos.y - a.pos.y;
    let dist_sq = dx * dx + dy * dy;
    let r_sum = a.r + b.r;

    if dist_sq >= r_sum * r_sum {
        return 0.0;
    }
    let dist = dist_sq.sqrt();
    if dist == 0.0 {
        // Coincident — skip to avoid divide-by-zero (spec §1.3 step 1).
        return 0.0;
    }

    let nx = dx / dist;
    let ny = dy / dist;
    let overlap = r_sum - dist;

    // Positional correction.
    if b.is_static {
        a.pos.x -= nx * overlap;
        a.pos.y -= ny * overlap;
    } else {
        let half = overlap * 0.5;
        a.pos.x -= nx * half;
        a.pos.y -= ny * half;
        b.pos.x += nx * half;
        b.pos.y += ny * half;
    }

    // Velocity response.
    let rvx = b.vel.x - a.vel.x;
    let rvy = b.vel.y - a.vel.y;
    let rvn = rvx * nx + rvy * ny;

    if rvn >= 0.0 {
        // Separating — positional correction only.
        return overlap;
    }

    let mass_factor = if b.is_static { 1.0 } else { 0.5 };
    let impulse = -(1.0 + RESTITUTION) * rvn * mass_factor;

    a.vel.x -= impulse * nx;
    a.vel.y -= impulse * ny;
    if !b.is_static {
        b.vel.x += impulse * nx;
        b.vel.y += impulse * ny;
    }

    overlap.max(impulse)
}

/// The 5 static pillars (spec §0).
pub fn pillars() -> Vec<Body> {
    vec![
        Body::new_pillar(520.0, 360.0, 48.0),
        Body::new_pillar(1280.0, 360.0, 48.0),
        Body::new_pillar(520.0, 840.0, 48.0),
        Body::new_pillar(1280.0, 840.0, 48.0),
        Body::new_pillar(900.0, 600.0, 36.0),
    ]
}
