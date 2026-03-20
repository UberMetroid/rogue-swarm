use std::sync::{Arc, Mutex};
use std::net::SocketAddr;

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
use tokio_tungstenite::{accept_async, tungstenite::Message as TungsteniteMessage};
use shared::{BroadcastState, PlayerInput};
use bincode;

struct ClientSession {
    player_id: u64,
    sender: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::stream::Stream<
                tokio::net::TcpStream,
                tokio::native_tls::TlsStream<tokio::net::TcpStream>,
            >,
            tokio_tungstenite::Protocol,
        >,
        TungsteniteMessage,
    >,
}

struct GameState {
    clients: Arc<Mutex<Vec<ClientSession>>>,
    active_player_id: Arc<Mutex<u64>>,
    tick_counter: u64,
}

#[tokio::main]
async fn main() {
    let game_state = Arc::new(Mutex::new(GameState {
        clients: Arc::new(Mutex::new(Vec::new())),
        active_player_id: Arc::new(Mutex::new(0)),
        tick_counter: 0,
    }));

    let app_state = game_state.clone();

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .with_state(app_state);

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
    State(state): State<Arc<Mutex<GameState>>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<Mutex<GameState>>) {
    let (writer, mut reader) = socket.split();
    let player_id = {
        let mut s = state.lock().unwrap();
        let id = s.tick_counter;
        s.tick_counter += 1;
        let client = ClientSession {
            player_id: id,
            sender: writer,
        };
        s.clients.lock().unwrap().push(client);
        id
    };

    println!("Client {} connected", player_id);

    while let Some(msg) = reader.next().await {
        if let Ok(msg) = msg {
            if msg.is_text() || msg.is_binary() {
                let bytes = msg.into_data();
                if let Ok(input) = bincode::deserialize::<PlayerInput>(&bytes) {
                    let s = state.lock().unwrap();
                    let active_id = *s.active_player_id.lock().unwrap();
                    if input.player_id == active_id {
                        println!("Active player {} sent input", active_id);
                    }
                }
            }
        } else {
            break;
        }
    }

    {
        let mut s = state.lock().unwrap();
        s.clients.lock().unwrap().retain(|c| c.player_id != player_id);
        if let Ok(mut active) = s.active_player_id.lock() {
            if *active == player_id {
                if let Some(next) = s.clients.lock().unwrap().first() {
                    *active = next.player_id;
                    println!("New active player: {}", next.player_id);
                }
            }
        }
    }

    println!("Client {} disconnected", player_id);
}

fn broadcast_state(state: &Arc<Mutex<GameState>>, broadcast: BroadcastState) {
    let data = bincode::serialize(&broadcast).unwrap();
    let mut s = state.lock().unwrap();
    let mut disconnected = Vec::new();

    for (i, client) in s.clients.lock().unwrap().iter_mut().enumerate() {
        let msg = TungsteniteMessage::Binary(data.clone());
        if client.sender.send(msg).await.is_err() {
            disconnected.push(i);
        }
    }

    for i in disconnected.into_iter().rev() {
        s.clients.lock().unwrap().remove(i);
    }
}
