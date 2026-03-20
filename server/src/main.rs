use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;

use bevy::prelude::*;
use axum::{
    extract::{
        ws::{WebSocket, WebSocketUpgrade, Message},
        State,
    },
    routing::get,
    Router,
    response::IntoResponse,
    http::StatusCode,
};
use futures_util::{SinkExt, StreamExt};
use shared::{BroadcastState, PlayerInput};
use bincode;
use tokio::sync::{mpsc, broadcast};

#[derive(Resource)]
struct InputReceiver(pub mpsc::Receiver<PlayerInput>);

#[derive(Resource)]
struct StateBroadcaster(pub broadcast::Sender<Arc<Vec<u8>>>);

#[derive(Resource)]
struct ActivePlayerResource(pub Arc<Mutex<u64>>);

struct GameState {
    active_player_id: Arc<Mutex<u64>>,
    tick_counter: Arc<Mutex<u64>>,
    client_ids: Arc<Mutex<Vec<u64>>>,
    input_sender: mpsc::Sender<PlayerInput>,
    state_broadcaster: broadcast::Sender<Arc<Vec<u8>>>,
}

#[derive(Component)]
struct Carrier;

#[derive(Component)]
struct Boid;

#[derive(Component)]
struct Alien;

#[derive(Component)]
struct Asteroid;

#[derive(Component)]
struct Position(pub [f32; 2]);

#[derive(Component)]
struct Velocity(pub [f32; 2]);

#[derive(Resource)]
struct GameConfig {
    map_size: f32,
    boid_speed: f32,
    carrier_speed: f32,
    boid_separation_distance: f32,
    boid_alignment_distance: f32,
    boid_cohesion_distance: f32,
    max_boids: usize,
}

#[derive(Resource)]
struct Score {
    wave: u32,
    score: u64,
}

#[derive(Resource)]
struct SwarmTarget(pub [f32; 2]);

#[derive(Resource)]
struct SpatialHash {
    cell_size: f32,
    grid: HashMap<(i32, i32), Vec<Entity>>,
}

impl SpatialHash {
    fn new(cell_size: f32) -> Self {
        Self {
            cell_size,
            grid: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.grid.clear();
    }

    fn insert(&mut self, entity: Entity, pos: [f32; 2]) {
        let cell = self.get_cell(pos);
        self.grid.entry(cell).or_insert(Vec::new()).push(entity);
    }

    fn get_neighbors(&self, pos: [f32; 2]) -> Vec<Entity> {
        let cell = self.get_cell(pos);
        let mut neighbors = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(entities) = self.grid.get(&(cell.0 + dx, cell.1 + dy)) {
                    neighbors.extend(entities);
                }
            }
        }
        neighbors
    }

    fn get_cell(&self, pos: [f32; 2]) -> (i32, i32) {
        ((pos[0] / self.cell_size) as i32, (pos[1] / self.cell_size) as i32)
    }
}

fn player_input_system(
    mut input_receiver: ResMut<InputReceiver>,
    mut swarm_target: ResMut<SwarmTarget>,
    mut carrier_query: Query<&mut Velocity, With<Carrier>>,
) {
    while let Ok(input) = input_receiver.0.try_recv() {
        if let Some(target) = input.swarm_target {
            swarm_target.0 = target;
        }

        if let Ok(mut vel) = carrier_query.get_single_mut() {
            if let Some(dir) = input.carrier_direction {
                vel.0[0] += dir[0] * 0.5;
                vel.0[1] += dir[1] * 0.5;
            }
            vel.0[0] *= 0.98;
            vel.0[1] *= 0.98;
        }
    }
}

fn carrier_movement_system(
    mut carrier_query: Query<(&mut Position, &Velocity), With<Carrier>>,
    config: Res<GameConfig>,
) {
    if let Ok((mut pos, vel)) = carrier_query.get_single_mut() {
        pos.0[0] += vel.0[0];
        pos.0[1] += vel.0[1];
        pos.0[0] = pos.0[0].clamp(0.0, config.map_size);
        pos.0[1] = pos.0[1].clamp(0.0, config.map_size);
    }
}

fn boid_flocking_system(
    mut boid_query: Query<(Entity, &Position, &mut Velocity), With<Boid>>,
    swarm_target: Res<SwarmTarget>,
    mut spatial_hash: ResMut<SpatialHash>,
    config: Res<GameConfig>,
) {
    spatial_hash.clear();
    let mut positions = HashMap::new();
    for (entity, pos, _) in boid_query.iter() {
        spatial_hash.insert(entity, pos.0);
        positions.insert(entity, pos.0);
    }

    for (entity, pos, mut vel) in boid_query.iter_mut() {
        let neighbors = spatial_hash.get_neighbors(pos.0);
        let mut separation = [0.0, 0.0];
        let mut count_sep = 0;

        for &neighbor in &neighbors {
            if neighbor == entity { continue; }
            if let Some(npos) = positions.get(&neighbor) {
                let dist = ((pos.0[0] - npos[0]).powi(2) + (pos.0[1] - npos[1]).powi(2)).sqrt();
                if dist < config.boid_separation_distance {
                    separation[0] += (pos.0[0] - npos[0]) / dist;
                    separation[1] += (pos.0[1] - npos[1]) / dist;
                    count_sep += 1;
                }
            }
        }

        if count_sep > 0 {
            separation[0] /= count_sep as f32;
            separation[1] /= count_sep as f32;
            vel.0[0] += separation[0] * 0.1;
            vel.0[1] += separation[1] * 0.1;
        }
        
        let target_dir = [swarm_target.0[0] - pos.0[0], swarm_target.0[1] - pos.0[1]];
        let target_dist = (target_dir[0].powi(2) + target_dir[1].powi(2)).sqrt();
        if target_dist > 0.0 {
            vel.0[0] += target_dir[0] / target_dist * 0.05;
            vel.0[1] += target_dir[1] / target_dist * 0.05;
        }
        
        let speed = (vel.0[0].powi(2) + vel.0[1].powi(2)).sqrt();
        if speed > config.boid_speed {
            vel.0[0] *= config.boid_speed / speed;
            vel.0[1] *= config.boid_speed / speed;
        }
    }
}

fn alien_ai_system(
    mut alien_query: Query<(&Position, &mut Velocity), With<Alien>>,
    carrier_query: Query<&Position, With<Carrier>>,
) {
    if let Ok(carrier_pos) = carrier_query.get_single() {
        for (apos, mut vel) in alien_query.iter_mut() {
            let dir = [carrier_pos.0[0] - apos.0[0], carrier_pos.0[1] - apos.0[1]];
            let dist = (dir[0].powi(2) + dir[1].powi(2)).sqrt();
            if dist > 0.0 {
                vel.0[0] = dir[0] / dist * 0.5;
                vel.0[1] = dir[1] / dist * 0.5;
            }
        }
    }
}

fn collision_system(
    mut commands: Commands,
    boid_query: Query<(Entity, &Position), With<Boid>>,
    alien_query: Query<(Entity, &Position), (With<Alien>, Without<Boid>)>,
    asteroid_query: Query<(Entity, &Position), (With<Asteroid>, Without<Boid>, Without<Alien>)>,
    carrier_query: Query<&Position, With<Carrier>>,
    mut score: ResMut<Score>,
) {
    for (_boid_entity, bpos) in boid_query.iter() {
        let mut aliens_to_remove = Vec::new();
        for (alien_entity, apos) in alien_query.iter() {
            let dist = ((bpos.0[0] - apos.0[0]).powi(2) + (bpos.0[1] - apos.0[1]).powi(2)).sqrt();
            if dist < 5.0 { 
                aliens_to_remove.push(alien_entity);
                score.score += 10;
            }
        }
        for entity in aliens_to_remove {
            commands.entity(entity).despawn();
        }
    }

    for (_boid_entity, bpos) in boid_query.iter() {
        let mut asteroids_to_remove = Vec::new();
        for (asteroid_entity, apos) in asteroid_query.iter() {
            let dist = ((bpos.0[0] - apos.0[0]).powi(2) + (bpos.0[1] - apos.0[1]).powi(2)).sqrt();
            if dist < 5.0 {
                asteroids_to_remove.push(asteroid_entity);
                score.score += 5;
            }
        }
        for entity in asteroids_to_remove {
            commands.entity(entity).despawn();
        }
    }

    if let Ok(cpos) = carrier_query.get_single() {
        for (_alien_entity, apos) in alien_query.iter() {
            let dist = ((cpos.0[0] - apos.0[0]).powi(2) + (cpos.0[1] - apos.0[1]).powi(2)).sqrt();
            if dist < 10.0 {
                // Game over
            }
        }
    }
}

fn broadcast_system(
    boid_query: Query<&Position, With<Boid>>,
    alien_query: Query<&Position, With<Alien>>,
    asteroid_query: Query<&Position, With<Asteroid>>,
    carrier_query: Query<&Position, With<Carrier>>,
    score: Res<Score>,
    active_player_res: Res<ActivePlayerResource>,
    broadcaster: Res<StateBroadcaster>,
) {
    let carrier_pos = carrier_query.get_single().map(|p| p.0).unwrap_or([0.0, 0.0]);
    let boid_positions: Vec<[f32; 2]> = boid_query.iter().map(|p| p.0).collect();
    let alien_positions: Vec<[f32; 2]> = alien_query.iter().map(|p| p.0).collect();
    let asteroid_positions: Vec<[f32; 2]> = asteroid_query.iter().map(|p| p.0).collect();
    let active_id = *active_player_res.0.lock().unwrap();

    let broadcast = BroadcastState {
        carrier_pos,
        boid_positions,
        alien_positions,
        asteroid_positions,
        wave: score.wave,
        score: score.score,
        active_player_id: active_id,
    };

    let data = bincode::serialize(&broadcast).unwrap();
    let _ = broadcaster.0.send(Arc::new(data));
}

#[tokio::main]
async fn main() {
    let (input_sender, input_receiver) = mpsc::channel(100);
    let (state_broadcaster, _) = broadcast::channel(16);
    
    let active_player_id = Arc::new(Mutex::new(0));

    let game_state = Arc::new(GameState {
        client_ids: Arc::new(Mutex::new(Vec::new())),
        active_player_id: active_player_id.clone(),
        tick_counter: Arc::new(Mutex::new(0)),
        input_sender,
        state_broadcaster: state_broadcaster.clone(),
    });

    // Spawn Bevy on a dedicated OS thread
    thread::spawn(move || {
        let mut app = App::new();
        app.insert_resource(GameConfig {
            map_size: 1000.0,
            boid_speed: 2.0,
            carrier_speed: 3.0,
            boid_separation_distance: 20.0,
            boid_alignment_distance: 50.0,
            boid_cohesion_distance: 50.0,
            max_boids: 10000,
        });
        app.insert_resource(Score { wave: 1, score: 0 });
        app.insert_resource(SwarmTarget([500.0, 500.0]));
        app.insert_resource(InputReceiver(input_receiver));
        app.insert_resource(StateBroadcaster(state_broadcaster));
        app.insert_resource(SpatialHash::new(50.0));
        app.insert_resource(ActivePlayerResource(active_player_id));

        app.world_mut().spawn((Carrier, Position([500.0, 500.0]), Velocity([0.0, 0.0])));

        app.add_systems(Update, player_input_system);
        app.add_systems(Update, carrier_movement_system);
        app.add_systems(Update, boid_flocking_system);
        app.add_systems(Update, alien_ai_system);
        app.add_systems(Update, collision_system);
        app.add_systems(Update, broadcast_system);

        let mut schedule = Schedule::default();
        schedule.add_systems(player_input_system);
        schedule.add_systems(carrier_movement_system);
        schedule.add_systems(boid_flocking_system);
        schedule.add_systems(alien_ai_system);
        schedule.add_systems(collision_system);
        schedule.add_systems(broadcast_system);

        loop {
            {
                let mut world = app.world_mut();
                schedule.run(&mut world);
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .with_state(game_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:7903").await.unwrap();
    println!("Server running on http://0.0.0.0:7903");

    axum::serve(listener, app).await.unwrap();
}

async fn index_handler() -> impl IntoResponse {
    match std::fs::read_to_string("client/index.html") {
        Ok(html) => (StatusCode::OK, html),
        Err(_) => (StatusCode::NOT_FOUND, "index.html not found".to_string()),
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<GameState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<GameState>) {
    let (mut writer, mut reader) = socket.split();
    
    let player_id = {
        let mut counter = state.tick_counter.lock().unwrap();
        let id = *counter;
        *counter += 1;
        state.client_ids.lock().unwrap().push(id);
        id
    };

    println!("Client {} connected", player_id);
    let mut rx = state.state_broadcaster.subscribe();

    loop {
        tokio::select! {
            msg = reader.next() => {
                match msg {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Text(text) => {
                                if let Ok(input) = bincode::deserialize::<PlayerInput>(text.as_bytes()) {
                                    let active_id = *state.active_player_id.lock().unwrap();
                                    if input.player_id == active_id {
                                        let _ = state.input_sender.try_send(input);
                                    }
                                }
                            }
                            Message::Binary(bytes) => {
                                if let Ok(input) = bincode::deserialize::<PlayerInput>(&bytes) {
                                    let active_id = *state.active_player_id.lock().unwrap();
                                    if input.player_id == active_id {
                                        let _ = state.input_sender.try_send(input);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => break, // Disconnected or error
                }
            }
            Ok(data) = rx.recv() => {
                let msg = Message::Binary(data.to_vec().into());
                if writer.send(msg).await.is_err() {
                    break;
                }
            }
        }
    }

    {
        state.client_ids.lock().unwrap().retain(|&id| id != player_id);
        if let Ok(mut active) = state.active_player_id.lock() {
            if *active == player_id {
                if let Some(&next) = state.client_ids.lock().unwrap().first() {
                    *active = next;
                    println!("New active player: {}", next);
                }
            }
        }
    }

    println!("Client {} disconnected", player_id);
}
