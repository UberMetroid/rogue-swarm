use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
    _carrier_speed: f32,
    boid_separation_distance: f32,
    _boid_alignment_distance: f32,
    _boid_cohesion_distance: f32,
    _max_boids: usize,
}

#[derive(Resource)]
struct Score {
    wave: u32,
    score: u64,
}

#[derive(Resource)]
struct SwarmTarget(pub [f32; 2]);

#[derive(Resource)]
struct SpawnerState {
    last_alien_spawn: Instant,
    last_asteroid_spawn: Instant,
}

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
        let cell = ((pos[0] / self.cell_size) as i32, (pos[1] / self.cell_size) as i32);
        self.grid.entry(cell).or_insert(Vec::new()).push(entity);
    }

    fn get_neighbors(&self, pos: [f32; 2]) -> Vec<Entity> {
        let cell = ((pos[0] / self.cell_size) as i32, (pos[1] / self.cell_size) as i32);
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
        }
    }
}

fn carrier_movement_system(
    mut carrier_query: Query<(&mut Position, &mut Velocity), With<Carrier>>,
    config: Res<GameConfig>,
) {
    if let Ok((mut pos, mut vel)) = carrier_query.get_single_mut() {
        pos.0[0] += vel.0[0];
        pos.0[1] += vel.0[1];
        pos.0[0] = pos.0[0].clamp(0.0, config.map_size);
        pos.0[1] = pos.0[1].clamp(0.0, config.map_size);
        
        // Friction
        vel.0[0] *= 0.96;
        vel.0[1] *= 0.96;
    }
}

fn spawner_system(
    mut commands: Commands,
    mut spawner_state: ResMut<SpawnerState>,
    config: Res<GameConfig>,
    carrier_query: Query<&Position, With<Carrier>>,
) {
    let now = Instant::now();
    
    // Spawn 1 asteroid every 1 second
    if now.duration_since(spawner_state.last_asteroid_spawn).as_secs_f32() > 1.0 {
        spawner_state.last_asteroid_spawn = now;
        let x = (rand::random::<f32>() * 0.8 + 0.1) * config.map_size;
        let y = (rand::random::<f32>() * 0.8 + 0.1) * config.map_size;
        commands.spawn((Asteroid, Position([x, y])));
    }

    // Spawn 1 alien every 2 seconds, slightly away from carrier
    if now.duration_since(spawner_state.last_alien_spawn).as_secs_f32() > 2.0 {
        spawner_state.last_alien_spawn = now;
        
        let cpos = carrier_query.get_single().map(|p| p.0).unwrap_or([config.map_size/2.0, config.map_size/2.0]);
        let angle = rand::random::<f32>() * std::f32::consts::PI * 2.0;
        let dist = 400.0;
        
        let mut ax = cpos[0] + angle.cos() * dist;
        let mut ay = cpos[1] + angle.sin() * dist;
        ax = ax.clamp(0.0, config.map_size);
        ay = ay.clamp(0.0, config.map_size);
        
        commands.spawn((Alien, Position([ax, ay]), Velocity([0.0, 0.0])));
    }
}



fn ai_and_collision_system(
    mut commands: Commands,
    mut alien_query: Query<(Entity, &mut Position, &mut Velocity), With<Alien>>,
    mut boid_query: Query<(Entity, &mut Position, &mut Velocity), With<Boid>>,
    asteroid_query: Query<(Entity, &Position), With<Asteroid>>,
    carrier_query: Query<&Position, With<Carrier>>,
    mut score: ResMut<Score>,
    swarm_target: Res<SwarmTarget>,
    mut spatial_hash: ResMut<SpatialHash>,
    config: Res<GameConfig>,
) {
    // Boid flocking logic (moved from boid_flocking_system)
    spatial_hash.clear();
    let mut positions = HashMap::new();
    // Copy positions first for the hash map to avoid double mut borrow
    for (entity, pos, _) in boid_query.iter() {
        spatial_hash.insert(entity, pos.0);
        positions.insert(entity, pos.0);
    }

    for (entity, mut pos, mut vel) in boid_query.iter_mut() {
        let neighbors = spatial_hash.get_neighbors(pos.0);
        let mut separation = [0.0, 0.0];
        let mut count_sep = 0;

        for &neighbor in &neighbors {
            if neighbor == entity { continue; }
            if let Some(npos) = positions.get(&neighbor) {
                let dx = pos.0[0] - npos[0];
                let dy = pos.0[1] - npos[1];
                let dist_sq = dx*dx + dy*dy;

                if dist_sq > 0.0 && dist_sq < config.boid_separation_distance.powi(2) {
                    let dist = dist_sq.sqrt();
                    separation[0] += dx / dist;
                    separation[1] += dy / dist;
                    count_sep += 1;
                }
            }
        }

        if count_sep > 0 {
            separation[0] /= count_sep as f32;
            separation[1] /= count_sep as f32;
            vel.0[0] += separation[0] * 0.2;
            vel.0[1] += separation[1] * 0.2;
        }

        let target_dir = [swarm_target.0[0] - pos.0[0], swarm_target.0[1] - pos.0[1]];
        let target_dist = (target_dir[0].powi(2) + target_dir[1].powi(2)).sqrt();
        if target_dist > 0.0 {
            // Speed up significantly if far away
            let pull = if target_dist > 200.0 { 0.15 } else { 0.05 };
            vel.0[0] += target_dir[0] / target_dist * pull;
            vel.0[1] += target_dir[1] / target_dist * pull;
        }

        let speed = (vel.0[0].powi(2) + vel.0[1].powi(2)).sqrt();
        if speed > config.boid_speed {
            vel.0[0] *= config.boid_speed / speed;
            vel.0[1] *= config.boid_speed / speed;
        }

        // Move boids
        pos.0[0] += vel.0[0];
        pos.0[1] += vel.0[1];
    }

    // Alien AI logic (moved from alien_ai_system)
    if let Ok(carrier_pos) = carrier_query.get_single() {
        for (_, mut apos, mut vel) in alien_query.iter_mut() {
            let dx = carrier_pos.0[0] - apos.0[0];
            let dy = carrier_pos.0[1] - apos.0[1];
            let dist = (dx*dx + dy*dy).sqrt();
            if dist > 0.0 {
                vel.0[0] += (dx / dist) * 0.02;
                vel.0[1] += (dy / dist) * 0.02;

                // Max speed
                let speed = (vel.0[0].powi(2) + vel.0[1].powi(2)).sqrt();
                if speed > 1.0 {
                    vel.0[0] *= 1.0 / speed;
                    vel.0[1] *= 1.0 / speed;
                }
            }

            apos.0[0] += vel.0[0];
            apos.0[1] += vel.0[1];
        }
    }

    // Collision logic (moved from collision_system)
    let mut asteroids_harvested = 0;

    // Boids kill Aliens
    for (alien_entity, apos, _) in alien_query.iter() {
        for (_, bpos, _) in boid_query.iter() {
            let dx = bpos.0[0] - apos.0[0];
            let dy = bpos.0[1] - apos.0[1];
            if dx*dx + dy*dy < 100.0 { // 10.0 dist
                commands.entity(alien_entity).despawn();
                score.score += 10;
                break;
            }
        }
    }

    // Boids harvest Asteroids
    for (asteroid_entity, apos) in asteroid_query.iter() {
        for (_, bpos, _) in boid_query.iter() {
            let dx = bpos.0[0] - apos.0[0];
            let dy = bpos.0[1] - apos.0[1];
            if dx*dx + dy*dy < 100.0 {
                commands.entity(asteroid_entity).despawn();
                score.score += 5;
                asteroids_harvested += 1;
                break;
            }
        }
    }

    // Spawn 10 boids per harvested asteroid around carrier
    if asteroids_harvested > 0 {
        if let Ok(cpos) = carrier_query.get_single() {
            for _ in 0..(asteroids_harvested * 10) {
                let dx = (rand::random::<f32>() - 0.5) * 20.0;
                let dy = (rand::random::<f32>() - 0.5) * 20.0;
                commands.spawn((
                    Boid,
                    Position([cpos.0[0] + dx, cpos.0[1] + dy]),
                    Velocity([0.0, 0.0])
                ));
            }
        }
    }

    // Aliens kill Carrier (Game over reset)
    if let Ok(cpos) = carrier_query.get_single() {
        let mut hit = false;
        for (alien_entity, apos, _) in alien_query.iter() {
            let dx = cpos.0[0] - apos.0[0];
            let dy = cpos.0[1] - apos.0[1];
            if dx*dx + dy*dy < 400.0 { // 20.0 dist
                hit = true;
                break;
            }
        }

        if hit {
            println!("CARRIER DESTROYED! Game Over.");
            // Next tick, game will be handled by UI, but here we just reset score
            score.score = 0;
            score.wave = 1;

            // Kill all aliens
            for (alien_entity, _, _) in alien_query.iter() {
                commands.entity(alien_entity).despawn();
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
    let carrier_pos = carrier_query.get_single().map(|p| p.0).unwrap_or([500.0, 500.0]);
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

    thread::spawn(move || {
        let mut app = App::new();
        app.insert_resource(GameConfig {
            map_size: 1000.0,
            boid_speed: 6.0,
            _carrier_speed: 3.0,
            boid_separation_distance: 10.0,
            _boid_alignment_distance: 50.0,
            _boid_cohesion_distance: 50.0,
            _max_boids: 10000,
        });
        app.insert_resource(Score { wave: 1, score: 0 });
        app.insert_resource(SwarmTarget([500.0, 500.0]));
        app.insert_resource(SpawnerState {
            last_alien_spawn: Instant::now(),
            last_asteroid_spawn: Instant::now(),
        });
        app.insert_resource(InputReceiver(input_receiver));
        app.insert_resource(StateBroadcaster(state_broadcaster));
        app.insert_resource(SpatialHash::new(20.0));
        app.insert_resource(ActivePlayerResource(active_player_id));

        app.world_mut().spawn((Carrier, Position([500.0, 500.0]), Velocity([0.0, 0.0])));
        
        // Spawn initial 100 boids
        for _ in 0..100 {
            app.world_mut().spawn((
                Boid,
                Position([500.0 + (rand::random::<f32>()-0.5)*100.0, 500.0 + (rand::random::<f32>()-0.5)*100.0]),
                Velocity([0.0, 0.0])
            ));
        }

        app.add_systems(Update, player_input_system);
        app.add_systems(Update, carrier_movement_system);
        app.add_systems(Update, spawner_system);
        app.add_systems(Update, ai_and_collision_system);
        app.add_systems(Update, broadcast_system);

        let mut schedule = Schedule::default();
        schedule.add_systems((
            player_input_system,
            carrier_movement_system,
            spawner_system,
            ai_and_collision_system,
            broadcast_system,
        ).chain());

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
        .nest_service("/pkg", tower_http::services::ServeDir::new("/client/pkg"))
        .with_state(game_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:7903").await.unwrap();
    println!("Server running on http://0.0.0.0:7903");

    axum::serve(listener, app).await.unwrap();
}

async fn index_handler() -> impl IntoResponse {
    match std::fs::read_to_string("/client/index.html") {
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
        
        // If they are the first, make them active
        let mut active = state.active_player_id.lock().unwrap();
        if state.client_ids.lock().unwrap().len() == 1 {
            *active = id;
        }
        
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
                                    if input.player_id == active_id || input.player_id == 0 { // 0 fallback
                                        let _ = state.input_sender.try_send(input);
                                    }
                                }
                            }
                            Message::Binary(bytes) => {
                                if let Ok(input) = bincode::deserialize::<PlayerInput>(&bytes) {
                                    let active_id = *state.active_player_id.lock().unwrap();
                                    // Let client push their input if active
                                    let _ = state.input_sender.try_send(input);
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
