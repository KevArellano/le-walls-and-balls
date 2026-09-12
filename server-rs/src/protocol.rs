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
}

/// Server → Client messages.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "t")]
pub enum ServerMsg {
    #[serde(rename = "welcome")]
    Welcome { id: String, tick: u32 },
    #[serde(rename = "state")]
    State { tick: u32, players: Vec<PlayerState> },
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
}
