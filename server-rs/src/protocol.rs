//! Wire protocol (spec §2). JSON over WebSocket, tagged by field `t`.

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
    /// is ready, the campaign auto-starts (spec: campaign extension).
    #[serde(rename = "ready")]
    Ready { ready: bool },
}

/// Server → Client messages.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "t")]
pub enum ServerMsg {
    #[serde(rename = "welcome")]
    Welcome { id: String, tick: u32 },
    #[serde(rename = "state")]
    State {
        tick: u32,
        players: Vec<PlayerState>,
        /// Present only while a campaign is active or being set up. Omitted
        /// entirely in the plain free-play case so existing clients are
        /// unaffected (spec: campaign extension, backward-compatible).
        #[serde(skip_serializing_if = "Option::is_none")]
        campaign: Option<CampaignState>,
    },
    /// Phase-transition notification for one-shot UI/audio cues (level begin,
    /// victory). The authoritative detail always rides on `state.campaign`;
    /// this is just an edge trigger.
    #[serde(rename = "campaign")]
    Campaign { phase: String, level: u8 },
    #[serde(rename = "player_join")]
    PlayerJoin {
        id: String,
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

#[derive(Serialize, Clone, Debug)]
pub struct PlayerState {
    pub id: String,
    pub name: String,
    pub color: String,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub seq: u32,
    /// Campaign ready flag. Meaningful in the Lobby phase; harmless otherwise.
    pub ready: bool,
}

/// Snapshot of campaign state, attached to `state` while a campaign is active.
#[derive(Serialize, Clone, Debug)]
pub struct CampaignState {
    /// "lobby" | "playing" | "victory".
    pub phase: String,
    /// 0-based level index while playing.
    pub level: u8,
    /// Total levels in a run.
    pub levels: u8,
    /// Static walls for the current level (empty outside "playing").
    pub walls: Vec<WallState>,
    /// Living + dead enemies (dead ones flagged so clients can fade them out).
    pub enemies: Vec<EnemyState>,
    /// Pickups; collected ones flagged.
    pub items: Vec<ItemState>,
    /// The shared exit zone players gather in to advance.
    pub exit: ZoneState,
    /// Count of players currently standing inside the exit zone.
    pub in_zone: u8,
    /// Total players in the room (denominator for ready/in-zone UI).
    pub total: u8,
}

#[derive(Serialize, Clone, Debug)]
pub struct WallState {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct EnemyState {
    pub x: f64,
    pub y: f64,
    pub r: f64,
    pub hp: i32,
    pub alive: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ItemState {
    pub x: f64,
    pub y: f64,
    pub collected: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ZoneState {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}
