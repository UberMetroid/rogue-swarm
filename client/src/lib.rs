use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main() {
    std::panic::set_hook(Box::new(console_error_panic_hook::console_error));
    console_log::init_with_level(log::Level::Info).unwrap();
}

pub mod game {
    use bevy::prelude::*;
    use bincode;
    use shared::{BroadcastState, PlayerInput};

    pub struct NetworkState {
        pub ws: Option<web_sys::WebSocket>,
        pub received_state: Option<BroadcastState>,
        pub player_id: u64,
    }

    pub fn run() {
        App::build()
            .add_plugins(MinimalPlugins)
            .add_plugins(TransformPlugin)
            .add_plugins(RenderPlugin::default())
            .add_system(network_system)
            .run();
    }

    fn network_system(mut state: ResMut<NetworkState>) {
        if let Some(ws) = &state.ws {
            if let Ok(data) = ws.recv() {
                if let Ok(broadcast) = bincode::deserialize::<BroadcastState>(&data) {
                    state.received_state = Some(broadcast);
                }
            }
        }
    }
}
