//! Wire protocol (spec §2). JSON over WebSocket, tagged by field `t`.
//!
//! v2 (egress-optimized) contract. Compared to v1 this:
//!   - keys per-tick player state on a small per-room integer index (`i`)
//!     instead of the 36-char UUID, and rounds positions/velocities to whole
//!     world units with short field names;
//!   - sends static level geometry (walls + exit) ONCE via `level`, not every
//!     tick;
//!   - sends enemies/items as per-tick DELTAS (only what changed), with the
//!     full set delivered on level start via `level`;
//!   - broadcasts `state` at a reduced cadence (see STATE_HZ in main.rs) while
//!     the simulation still runs at 60Hz.

use serde::{Deserialize, Serialize};

/// Client → Server messages.
#[derive(Deserialize, Debug)]
#[serde(tag = "t")]
pub enum ClientMsg {
    #[serde(rename = "join")]
    Join {
        room: String,
        name: String,
        color: String,
    },
    #[serde(rename = "input")]
    Input { x: f64, y: f64, seq: u32 },
    #[serde(rename = "leave")]
    Leave,
    #[serde(rename = "ping")]
    Ping { ts: u64 },
    /// Toggle this player's campaign ready flag. When every player in the room
    /// is ready, the campaign auto-starts.
    #[serde(rename = "ready")]
    Ready { ready: bool },
}

/// Server → Client messages.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "t")]
pub enum ServerMsg {
    /// Assigns this client its player id and small per-room index.
    #[serde(rename = "welcome")]
    Welcome { id: String, idx: u8, tick: u32 },
    // NOTE: the per-tick `state` message is NOT a JSON variant. It is the only
    // hot-path message and is sent as a WS **binary** frame — see
    // `Room::state_frame` and spec §2.7. All variants here are JSON text.
    /// One-shot static level geometry. Sent on level start (broadcast) and to a
    /// newcomer who joins mid-level. Not repeated per tick.
    #[serde(rename = "level")]
    Level {
        index: u8,
        levels: u8,
        walls: Vec<WallState>,
        enemies: Vec<EnemyFull>,
        items: Vec<ItemFull>,
        exit: ZoneState,
    },
    /// Phase-transition edge trigger for UI/audio cues (victory, lobby reset).
    /// Level starts are signalled by `level`; this covers the rest.
    #[serde(rename = "campaign")]
    Campaign { phase: String, level: u8 },
    #[serde(rename = "player_join")]
    PlayerJoin {
        id: String,
        idx: u8,
        name: String,
        color: String,
    },
    #[serde(rename = "player_leave")]
    PlayerLeave { id: String },
    #[serde(rename = "pong")]
    Pong { ts: u64 },
    #[serde(rename = "bump")]
    Bump {
        id: String,
        other: String,
        impulse: f64,
    },
}

// Per-tick player + campaign state is NOT modeled here — it is encoded directly
// as a binary frame in `Room::state_frame` (spec §2.7). The JSON structs below
// are only used by the one-shot `level` message.

#[derive(Serialize, Clone, Debug)]
pub struct WallState {
    pub x: i32,
    pub y: i32,
    pub r: i32,
}

/// Full enemy record sent once in `level`.
#[derive(Serialize, Clone, Debug)]
pub struct EnemyFull {
    pub i: u8,
    pub x: i32,
    pub y: i32,
    pub r: i32,
    pub hp: i32,
}

/// Full item record sent once in `level`.
#[derive(Serialize, Clone, Debug)]
pub struct ItemFull {
    pub i: u8,
    pub x: i32,
    pub y: i32,
}

#[derive(Serialize, Clone, Debug)]
pub struct ZoneState {
    pub x: i32,
    pub y: i32,
    pub r: i32,
}
