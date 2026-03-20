# Rogue Swarm

A browser-based, single-player PvE action-strategy game built with Rust. One player controls the game while connected web clients spectate the live run in real-time.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Digital Arcade Cabinet                  │
├─────────────────────────────────────────────────────────────┤
│  Carrier (Player 1)  │  Spectators (Players 2-N)           │
│  WASD + Mouse        │  Read-only view                     │
│  sends PlayerInput   │  receive BroadcastState             │
└──────────┬───────────┴──────────────┬──────────────────────┘
           │                          │
           ▼                          ▼
┌──────────────────────────────────────────────────────────────┐
│                      Server (Axum + Bevy)                    │
│  Port 8080: HTTP (index.html + WASM) + WebSocket /ws       │
│  - Headless Bevy runs 60 TPS simulation                      │
│  - Boid physics for 10,000+ nanobots                         │
│  - Binary broadcast via bincode                              │
└──────────────────────────────────────────────────────────────┘
```

## Tech Stack

| Component | Technology |
|-----------|------------|
| Language | Rust (Cargo workspace) |
| Frontend | Bevy Engine → WebAssembly |
| Backend | Axum web server + headless Bevy |
| Networking | WebSockets (tokio-tungstenite) |
| Serialization | bincode |

## Project Structure

```
rouge-swarm/
├── Cargo.toml           # Workspace manifest
├── client/              # Bevy WASM frontend
│   ├── Cargo.toml
│   ├── index.html
│   └── src/
│       ├── lib.rs       # Game rendering, input, WS client
│       └── main.rs
├── server/              # Axum + headless Bevy backend
│   ├── Cargo.toml
│   └── src/
│       └── main.rs      # Server, game loop, broadcast
└── shared/              # Shared types
    ├── Cargo.toml
    └── src/
        └── lib.rs       # BroadcastState, PlayerInput
```

## Build

### Prerequisites

- Rust 1.77+
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- `wasm-pack`: `cargo install wasm-pack`

### Server

```bash
cargo build --release -p server
./target/release/server
# Server runs on http://0.0.0.0:7903
```

### Client (WASM)

```bash
wasm-pack build --target web -d ../pkg client
```

### Docker

```bash
# Build locally
docker build -t rogue-swarm .

# Run
docker run -p 7903:7903 rogue-swarm
```

## GitHub Actions

Pushes to `master` automatically:

1. Build server binary (Linux x86_64)
2. Build client WASM
3. Build & push Docker image to `ghcr.io`

Docker image available at:
```
ghcr.io/<owner>/rogue-swarm:latest
```

## Gameplay

- **WASD**: Move the Carrier (no weapons, only scoop & spawner)
- **Mouse**: Direct the nanobot swarm to attack aliens or harvest asteroids
- **Spectators**: Watch the live run at `/ws`

## License

MIT
