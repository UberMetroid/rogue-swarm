use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BroadcastState {
    pub carrier_pos: [f32; 2],
    pub boid_positions: Vec<[f32; 2]>,
    pub wave: u32,
    pub score: u64,
    pub active_player_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerInput {
    pub carrier_direction: Option<[f32; 2]>,
    pub swarm_target: Option<[f32; 2]>,
    pub player_id: u64,
}
