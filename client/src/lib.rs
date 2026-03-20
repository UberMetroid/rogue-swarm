use wasm_bindgen::prelude::*;
use bevy::prelude::*;
use shared::{BroadcastState, PlayerInput};

#[wasm_bindgen(start)]
pub fn main() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init_with_level(log::Level::Info).unwrap();
}

#[derive(Resource)]
pub struct NetworkState {
    pub received_state: Option<BroadcastState>,
    pub player_id: u64,
}

#[wasm_bindgen]
pub fn run() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(TransformPlugin)
        .insert_resource(NetworkState {
            received_state: None,
            player_id: 0,
        })
        .add_systems(Update, network_system)
        .run();
}

fn network_system(mut state: ResMut<NetworkState>) {
    // Stub - websocket data will be pushed here via JS callbacks
    if state.received_state.is_some() {
        // Render state
    }
}
