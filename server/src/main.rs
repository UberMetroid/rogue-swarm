use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use bincode;
use futures_util::{SinkExt, StreamExt};
use shared::{BroadcastState, PlayerInput};
use tokio::sync::{broadcast, mpsc};

struct InputReceiver(pub mpsc::Receiver<PlayerInput>);

struct StateBroadcaster(pub broadcast::Sender<Arc<Vec<u8>>>);

struct ActivePlayerResource(pub Arc<Mutex<u64>>);

struct GameState {
    active_player_id: Arc<Mutex<u64>>,
    tick_counter: Arc<Mutex<u64>>,
    client_ids: Arc<Mutex<Vec<u64>>>,
    input_sender: mpsc::Sender<PlayerInput>,
    state_broadcaster: broadcast::Sender<Arc<Vec<u8>>>,
}

// Simple entity types
#[derive(Debug, Clone, Copy, PartialEq)]
enum EntityKind {
    Carrier,
    Boid,
    Alien,
    Asteroid,
}

#[derive(Debug, Clone)]
struct Entity {
    kind: EntityKind,
    pos: [f32; 2],
    vel: [f32; 2],
}

struct SwarmTarget([f32; 2]);

struct GameConfig {
    map_size: f32,
    boid_speed: f32,
    boid_separation_distance: f32,
}

struct Score {
    wave: u32,
    score: u64,
}

struct SpawnerState {
    last_alien_spawn: Instant,
    last_asteroid_spawn: Instant,
}

struct SpatialHash {
    cell_size: f32,
    grid: HashMap<(i32, i32), Vec<usize>>,
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

    fn insert(&mut self, id: usize, pos: [f32; 2]) {
        let cell = (
            (pos[0] / self.cell_size) as i32,
            (pos[1] / self.cell_size) as i32,
        );
        self.grid.entry(cell).or_insert_with(Vec::new).push(id);
    }

    fn get_neighbors(&self, pos: [f32; 2]) -> Vec<usize> {
        let cell = (
            (pos[0] / self.cell_size) as i32,
            (pos[1] / self.cell_size) as i32,
        );
        let mut neighbors = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ids) = self.grid.get(&(cell.0 + dx, cell.1 + dy)) {
                    neighbors.extend(ids.iter().copied());
                }
            }
        }
        neighbors
    }
}

fn main() {
    let (input_sender, input_receiver) = mpsc::channel(100);
    let (state_broadcaster, _) = broadcast::channel(16);
    let active_player_id = Arc::new(Mutex::new(0u64));

    let game_state = Arc::new(GameState {
        client_ids: Arc::new(Mutex::new(Vec::new())),
        active_player_id: active_player_id.clone(),
        tick_counter: Arc::new(Mutex::new(0)),
        input_sender,
        state_broadcaster: state_broadcaster.clone(),
    });

    thread::spawn(move || {
        let config = GameConfig {
            map_size: 1000.0,
            boid_speed: 6.0,
            boid_separation_distance: 10.0,
        };

        let mut score = Score { wave: 1, score: 0 };
        let mut swarm_target = SwarmTarget([500.0, 500.0]);
        let mut spawner = SpawnerState {
            last_alien_spawn: Instant::now(),
            last_asteroid_spawn: Instant::now(),
        };

        // Entity storage: simple HashMap instead of ECS
        let mut entities: HashMap<usize, Entity> = HashMap::new();
        let mut next_entity_id: usize = 0;

        // Spawn carrier
        let carrier_id = next_entity_id;
        entities.insert(
            carrier_id,
            Entity {
                kind: EntityKind::Carrier,
                pos: [500.0, 500.0],
                vel: [0.0, 0.0],
            },
        );
        next_entity_id += 1;

        // Spawn initial 50 boids
        for _ in 0..50 {
            let id = next_entity_id;
            entities.insert(
                id,
                Entity {
                    kind: EntityKind::Boid,
                    pos: [
                        500.0 + (rand::random::<f32>() - 0.5) * 100.0,
                        500.0 + (rand::random::<f32>() - 0.5) * 100.0,
                    ],
                    vel: [0.0, 0.0],
                },
            );
            next_entity_id += 1;
        }

        let input_receiver = input_receiver;
        let mut input_receiver = Some(input_receiver);

        loop {
            // Handle input
            if let Some(ref mut rx) = input_receiver {
                while let Ok(input) = rx.try_recv() {
                    if let Some(target) = input.swarm_target {
                        swarm_target.0 = target;
                    }
                    if let Some(dir) = input.carrier_direction {
                        if let Some(e) = entities.get_mut(&carrier_id) {
                            e.vel[0] += dir[0] * 0.5;
                            e.vel[1] += dir[1] * 0.5;
                        }
                    }
                }
            }

            // Spawn asteroids every 1 second
            let now = Instant::now();
            if now
                .duration_since(spawner.last_asteroid_spawn)
                .as_secs_f32()
                > 1.0
            {
                spawner.last_asteroid_spawn = now;
                let id = next_entity_id;
                entities.insert(
                    id,
                    Entity {
                        kind: EntityKind::Asteroid,
                        pos: [
                            (rand::random::<f32>() * 0.8 + 0.1) * config.map_size,
                            (rand::random::<f32>() * 0.8 + 0.1) * config.map_size,
                        ],
                        vel: [0.0, 0.0],
                    },
                );
                next_entity_id += 1;
            }

            // Spawn aliens every 2 seconds
            if now.duration_since(spawner.last_alien_spawn).as_secs_f32() > 2.0 {
                spawner.last_alien_spawn = now;
                let carrier_pos = entities
                    .get(&carrier_id)
                    .map(|e| e.pos)
                    .unwrap_or([500.0, 500.0]);
                let angle = rand::random::<f32>() * std::f32::consts::PI * 2.0;
                let dist = 400.0;
                let id = next_entity_id;
                entities.insert(
                    id,
                    Entity {
                        kind: EntityKind::Alien,
                        pos: [
                            (carrier_pos[0] + angle.cos() * dist).clamp(0.0, config.map_size),
                            (carrier_pos[1] + angle.sin() * dist).clamp(0.0, config.map_size),
                        ],
                        vel: [0.0, 0.0],
                    },
                );
                next_entity_id += 1;
            }

            // Build spatial hash for boids
            let mut spatial_hash = SpatialHash::new(20.0);
            let boid_ids: Vec<usize> = entities
                .iter()
                .filter(|(_, e)| e.kind == EntityKind::Boid)
                .map(|(id, _)| *id)
                .collect();
            for &id in &boid_ids {
                if let Some(e) = entities.get(&id) {
                    spatial_hash.insert(id, e.pos);
                }
            }

            // Store boid positions for collision lookup
            let boid_positions: HashMap<usize, [f32; 2]> = entities
                .iter()
                .filter(|(_, e)| e.kind == EntityKind::Boid)
                .map(|(id, e)| (*id, e.pos))
                .collect();

            // Update boids
            for &id in &boid_ids {
                // Separation
                let neighbors = spatial_hash.get_neighbors(entities[&id].pos);
                let mut separation = [0.0f32; 2];
                let mut count = 0;

                for &nid in &neighbors {
                    if nid == id {
                        continue;
                    }
                    if let Some(np) = boid_positions.get(&nid) {
                        let dx = entities[&id].pos[0] - np[0];
                        let dy = entities[&id].pos[1] - np[1];
                        let dist_sq = dx * dx + dy * dy;
                        if dist_sq > 0.0 && dist_sq < config.boid_separation_distance.powi(2) {
                            let dist = dist_sq.sqrt();
                            separation[0] += dx / dist;
                            separation[1] += dy / dist;
                            count += 1;
                        }
                    }
                }

                if count > 0 {
                    separation[0] /= count as f32;
                    separation[1] /= count as f32;
                    entities.get_mut(&id).unwrap().vel[0] += separation[0] * 0.2;
                    entities.get_mut(&id).unwrap().vel[1] += separation[1] * 0.2;
                }

                // Swarm attraction
                let pos = entities[&id].pos;
                let dx = swarm_target.0[0] - pos[0];
                let dy = swarm_target.0[1] - pos[1];
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > 0.0 {
                    let pull = if dist > 200.0 { 0.15 } else { 0.05 };
                    let e = entities.get_mut(&id).unwrap();
                    e.vel[0] += (dx / dist) * pull;
                    e.vel[1] += (dy / dist) * pull;

                    // Speed limit
                    let speed = (e.vel[0] * e.vel[0] + e.vel[1] * e.vel[1]).sqrt();
                    if speed > config.boid_speed {
                        e.vel[0] *= config.boid_speed / speed;
                        e.vel[1] *= config.boid_speed / speed;
                    }

                    // Move
                    e.pos[0] += e.vel[0];
                    e.pos[1] += e.vel[1];
                }
            }

            // Update aliens (chase carrier)
            if let Some(carrier) = entities.get(&carrier_id) {
                let carrier_pos = carrier.pos;
                for (_, e) in entities
                    .iter_mut()
                    .filter(|(_, e)| e.kind == EntityKind::Alien)
                {
                    let dx = carrier_pos[0] - e.pos[0];
                    let dy = carrier_pos[1] - e.pos[1];
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > 0.0 {
                        e.vel[0] += (dx / dist) * 0.02;
                        e.vel[1] += (dy / dist) * 0.02;
                        let speed = (e.vel[0] * e.vel[0] + e.vel[1] * e.vel[1]).sqrt();
                        if speed > 1.0 {
                            e.vel[0] *= 1.0 / speed;
                            e.vel[1] *= 1.0 / speed;
                        }
                    }
                    e.pos[0] += e.vel[0];
                    e.pos[1] += e.vel[1];
                }
            }

            // Move carrier
            if let Some(e) = entities.get_mut(&carrier_id) {
                e.pos[0] += e.vel[0];
                e.pos[1] += e.vel[1];
                e.pos[0] = e.pos[0].clamp(0.0, config.map_size);
                e.pos[1] = e.pos[1].clamp(0.0, config.map_size);
                e.vel[0] *= 0.96;
                e.vel[1] *= 0.96;
            }

            // Collisions: boids vs aliens
            let mut aliens_to_kill = Vec::new();
            let boid_positions: HashMap<usize, [f32; 2]> = entities
                .iter()
                .filter(|(_, e)| e.kind == EntityKind::Boid)
                .map(|(id, e)| (*id, e.pos))
                .collect();

            for (aid, alien) in entities
                .iter()
                .filter(|(id, e)| e.kind == EntityKind::Alien && boid_positions.contains_key(id))
            {
                let neighbors = spatial_hash.get_neighbors(alien.pos);
                for &bid in &neighbors {
                    if let Some(bpos) = boid_positions.get(&bid) {
                        let dx = bpos[0] - alien.pos[0];
                        let dy = bpos[1] - alien.pos[1];
                        if dx * dx + dy * dy < 100.0 {
                            aliens_to_kill.push(*aid);
                            score.score += 10;
                            break;
                        }
                    }
                }
            }

            // Collisions: boids vs asteroids
            let mut asteroids_to_harvest = 0;
            let mut asteroids_to_kill = Vec::new();
            for (aid, asteroid) in entities
                .iter()
                .filter(|(id, e)| e.kind == EntityKind::Asteroid)
            {
                let neighbors = spatial_hash.get_neighbors(asteroid.pos);
                for &bid in &neighbors {
                    if let Some(bpos) = boid_positions.get(&bid) {
                        let dx = bpos[0] - asteroid.pos[0];
                        let dy = bpos[1] - asteroid.pos[1];
                        if dx * dx + dy * dy < 100.0 {
                            asteroids_to_kill.push(*aid);
                            asteroids_to_harvest += 1;
                            score.score += 5;
                            break;
                        }
                    }
                }
            }

            // Harvest asteroids spawns boids
            for _ in 0..(asteroids_to_harvest * 2) {
                if let Some(carrier) = entities.get(&carrier_id) {
                    let id = next_entity_id;
                    entities.insert(
                        id,
                        Entity {
                            kind: EntityKind::Boid,
                            pos: [
                                carrier.pos[0] + (rand::random::<f32>() - 0.5) * 20.0,
                                carrier.pos[1] + (rand::random::<f32>() - 0.5) * 20.0,
                            ],
                            vel: [0.0, 0.0],
                        },
                    );
                    next_entity_id += 1;
                }
            }

            // Apply deaths
            for aid in aliens_to_kill {
                entities.remove(&aid);
            }
            for aid in asteroids_to_kill {
                entities.remove(&aid);
            }

            // Alien vs carrier collision
            let mut game_over = false;
            if let Some(carrier) = entities.get(&carrier_id) {
                for (_, alien) in entities.iter().filter(|(_, e)| e.kind == EntityKind::Alien) {
                    let dx = carrier.pos[0] - alien.pos[0];
                    let dy = carrier.pos[1] - alien.pos[1];
                    if dx * dx + dy * dy < 400.0 {
                        game_over = true;
                        break;
                    }
                }
            }

            if game_over {
                println!("CARRIER DESTROYED! Game Over.");
                score.score = 0;
                score.wave = 1;
                entities.retain(|_, e| e.kind != EntityKind::Alien);
            }

            // Broadcast state
            let carrier_pos = entities
                .get(&carrier_id)
                .map(|e| e.pos)
                .unwrap_or([500.0, 500.0]);
            let boid_positions: Vec<[f32; 2]> = entities
                .iter()
                .filter(|(_, e)| e.kind == EntityKind::Boid)
                .map(|(_, e)| e.pos)
                .collect();
            let alien_positions: Vec<[f32; 2]> = entities
                .iter()
                .filter(|(_, e)| e.kind == EntityKind::Alien)
                .map(|(_, e)| e.pos)
                .collect();
            let asteroid_positions: Vec<[f32; 2]> = entities
                .iter()
                .filter(|(_, e)| e.kind == EntityKind::Asteroid)
                .map(|(_, e)| e.pos)
                .collect();

            let broadcast = BroadcastState {
                carrier_pos,
                boid_positions,
                alien_positions,
                asteroid_positions,
                wave: score.wave,
                score: score.score,
                active_player_id: *active_player_id.lock().unwrap(),
            };

            if let Ok(data) = bincode::serialize(&broadcast) {
                let _ = state_broadcaster.send(Arc::new(data));
            }

            thread::sleep(Duration::from_millis(16));
        }
    });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .nest_service("/pkg", tower_http::services::ServeDir::new("/client/pkg"))
        .with_state(game_state);

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("0.0.0.0:7903").await.unwrap();
        println!("Server running on http://0.0.0.0:7903");
        axum::serve(listener, app).await.unwrap();
    });
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
                                    if input.player_id == active_id || input.player_id == 0 {
                                        let _ = state.input_sender.try_send(input);
                                    }
                                }
                            }
                            Message::Binary(bytes) => {
                                if let Ok(input) = bincode::deserialize::<PlayerInput>(&bytes) {
                                    let _ = state.input_sender.try_send(input);
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => break,
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

    state
        .client_ids
        .lock()
        .unwrap()
        .retain(|&id| id != player_id);
    if let Ok(mut active) = state.active_player_id.lock() {
        if *active == player_id {
            if let Some(&next) = state.client_ids.lock().unwrap().first() {
                *active = next;
                println!("New active player: {}", next);
            }
        }
    }

    println!("Client {} disconnected", player_id);
}
