use bincode;
use js_sys::Uint8Array;
use shared::{BroadcastState, PlayerInput};
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

// Use lazy_static or similar for a global singleton to pass data from JS to the Bevy App
use once_cell::sync::Lazy;

pub static LATEST_STATE: Lazy<Arc<Mutex<Option<BroadcastState>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

pub static PLAYER_ID: Lazy<Arc<Mutex<Option<u64>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));

#[wasm_bindgen(start)]
pub fn main() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init_with_level(log::Level::Info).unwrap_or(());
    log::info!("Rogue Swarm WASM module initialized.");
}

// Called from JS when a new WebSocket binary message arrives
#[wasm_bindgen]
pub fn update_state(data: Uint8Array) {
    let bytes = data.to_vec();
    if let Ok(state) = bincode::deserialize::<BroadcastState>(&bytes) {
        if let Ok(mut latest) = LATEST_STATE.lock() {
            *latest = Some(state);
        }
    } else {
        log::error!("Failed to deserialize BroadcastState from server");
    }
}

// Called from JS to set the player ID once connected
#[wasm_bindgen]
pub fn set_player_id(id: u64) {
    if let Ok(mut pid) = PLAYER_ID.lock() {
        *pid = Some(id);
    }
    log::info!("Player ID set to {}", id);
}

// Helper to construct PlayerInput bytes to send over WebSocket
#[wasm_bindgen]
pub fn create_input_bytes(
    player_id: u64,
    carrier_dir_x: f32,
    carrier_dir_y: f32,
    swarm_target_x: f32,
    swarm_target_y: f32,
    has_dir: bool,
) -> Uint8Array {
    let input = PlayerInput {
        player_id,
        carrier_direction: if has_dir {
            Some([carrier_dir_x, carrier_dir_y])
        } else {
            None
        },
        swarm_target: Some([swarm_target_x, swarm_target_y]),
    };

    let bytes = bincode::serialize(&input).unwrap_or_default();
    let array = Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(&bytes);
    array
}

// Pure JS rendering loop - much faster for 10k simple shapes than Bevy/WGPU overhead in browser
#[wasm_bindgen]
pub fn render_frame(
    ctx: &web_sys::CanvasRenderingContext2d,
    canvas_width: f64,
    canvas_height: f64,
) {
    let state = {
        if let Ok(latest) = LATEST_STATE.lock() {
            latest.clone()
        } else {
            None
        }
    };

    if let Some(state) = state {
        // Clear screen (slight trail effect)
        ctx.set_fill_style_str("rgba(10, 10, 18, 0.3)");
        ctx.fill_rect(0.0, 0.0, canvas_width, canvas_height);

        let map_size = 1000.0;
        let scale_x = canvas_width / map_size;
        let scale_y = canvas_height / map_size;

        // Draw Asteroids (yellow circles)
        ctx.set_fill_style_str("#cccc00");
        for pos in &state.asteroid_positions {
            ctx.begin_path();
            let _ = ctx.arc(
                pos[0] as f64 * scale_x,
                pos[1] as f64 * scale_y,
                4.0 * scale_x,
                0.0,
                std::f64::consts::PI * 2.0,
            );
            ctx.fill();
        }

        // Draw Aliens (red squares)
        ctx.set_fill_style_str("#ff3333");
        for pos in &state.alien_positions {
            let size = 6.0 * scale_x;
            ctx.fill_rect(
                (pos[0] as f64 * scale_x) - size / 2.0,
                (pos[1] as f64 * scale_y) - size / 2.0,
                size,
                size,
            );
        }

        // Draw Carrier (blue circle)
        ctx.set_fill_style_str("#3388ff");
        ctx.begin_path();
        let _ = ctx.arc(
            state.carrier_pos[0] as f64 * scale_x,
            state.carrier_pos[1] as f64 * scale_y,
            10.0 * scale_x,
            0.0,
            std::f64::consts::PI * 2.0,
        );
        ctx.fill();

        // Draw Boids (cyan pixels/tiny lines)
        ctx.set_fill_style_str("#00ffff");
        for pos in &state.boid_positions {
            // Draw a tiny 2x2 rect for speed instead of full arcs for 10k entities
            ctx.fill_rect(
                (pos[0] as f64 * scale_x) - 1.0,
                (pos[1] as f64 * scale_y) - 1.0,
                2.0,
                2.0,
            );
        }

        // Update UI info (using JS string return might be cleaner, but we can do it via DOM)
        update_dom_ui(state.wave, state.score, state.active_player_id);
    }
}

fn update_dom_ui(wave: u32, score: u64, active_player_id: u64) {
    if let Some(window) = web_sys::window() {
        if let Some(document) = window.document() {
            if let Some(el) = document.get_element_by_id("wave") {
                el.set_text_content(Some(&format!("Wave: {}", wave)));
            }
            if let Some(el) = document.get_element_by_id("score") {
                el.set_text_content(Some(&format!("Score: {}", score)));
            }

            let is_active = if let Ok(pid) = PLAYER_ID.lock() {
                if let Some(id) = *pid {
                    id == active_player_id
                } else {
                    false
                }
            } else {
                false
            };

            if let Some(el) = document.get_element_by_id("status") {
                let status_text = if is_active {
                    "ACTIVE PLAYER (WASD to move, Mouse to direct swarm)"
                } else {
                    &format!("SPECTATING (Player {} is active)", active_player_id)
                };
                el.set_text_content(Some(status_text));

                if is_active {
                    let _ = el.set_attribute("style", "color: #00ffaa; font-weight: bold;");
                } else {
                    let _ = el.set_attribute("style", "color: #aaa;");
                }
            }
        }
    }
}
