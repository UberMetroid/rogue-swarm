# Rogue Swarm

A browser-based, single-player PvE action-strategy game built with Rust. One player controls the game while connected web clients spectate the live run in real-time.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Digital Arcade Cabinet                  │
├─────────────────────────────────────────────────────────────┤
│  Carrier (Player 1)  │  Spectators (Players 2-N)         │
│  WASD + Mouse        │  Read-only view                   │
│  sends PlayerInput   │  receive BroadcastState            │
└──────────┬───────────┴──────────────┬──────────────────────┘
           │                          │
           ▼                          ▼
┌──────────────────────────────────────────────────────────────┐
│                      Server (Axum + Bevy)                   │
│  Port 7903: HTTP (index.html + WASM) + WebSocket /ws      │
│  - Headless Bevy runs 60 TPS simulation on dedicated thread│
│  - Boid physics for 10,000+ nanobots via spatial hashing   │
│  - Binary broadcast via bincode                             │
└──────────────────────────────────────────────────────────────┘
```

## Tech Stack

| Component | Technology |
|-----------|------------|
| Language | Rust (Cargo workspace) |
| Frontend | Bevy Engine (WASM) + HTML5 Canvas 2D |
| Backend | Axum web server + headless Bevy (separate thread) |
| Networking | WebSockets (tokio-tungstenite) |
| Serialization | bincode |
| Container | Docker (pre-built binary) |

## Project Structure

```
rouge-swarm/
├── Cargo.toml           # Workspace manifest
├── client/             # Bevy WASM frontend + HTML5 Canvas
│   ├── Cargo.toml
│   ├── index.html      # JS WebSocket client + render loop
│   └── src/
│       └── lib.rs      # WASM rendering, state management
├── server/             # Axum + headless Bevy backend
│   ├── Cargo.toml
│   └── src/
│       └── main.rs     # Server, game loop, broadcast
└── shared/             # Shared types
    ├── Cargo.toml
    └── src/
        └── lib.rs      # BroadcastState, PlayerInput
```

## Quick Start

### Docker (Recommended)

The easiest way to run the game server is using the pre-built Docker container. The container includes everything needed (the server binary, HTML, and WASM files) and requires no volume mounts for standard play.

```bash
docker run -d -p 7903:7903 --name rogue-swarm ghcr.io/ubermetroid/rogue-swarm:latest
# Open http://localhost:7903
```

#### Running on Unraid / NAS

When configuring the container on Unraid or other Docker UI managers:
- **Repository:** `ghcr.io/ubermetroid/rogue-swarm:latest`
- **Network Type:** Bridge
- **Port mapping:** Host `7903` to Container `7903` (TCP)
- **Volumes:** None required for the standard game.

#### Local Development with Docker

If you are modifying the WASM client locally and want to test it using the Docker server without rebuilding the entire image, you can bind-mount your local directories into the container:

```bash
# 1. Build the WASM client locally first
wasm-pack build --target web -d ../pkg client

# 2. Run the Docker container, mounting your local files over the container's files
docker run -p 7903:7903 \
  -v $(pwd)/client/index.html:/client/index.html:ro \
  -v $(pwd)/pkg:/client/pkg:ro \
  ghcr.io/ubermetroid/rogue-swarm:latest
```

### From Source

```bash
# Build WASM client
wasm-pack build --target web -d ../pkg client

# Build & run server
cargo build --release -p server
./target/release/server

# Open http://localhost:7903
```

## Controls

- **WASD** — Move the Carrier (has momentum/inertia)
- **Mouse** — Direct the nanobot swarm toward targets

## Gameplay

- **Harvest asteroids** — Move your swarm near yellow asteroid circles to harvest them. Each harvested asteroid spawns **10 new nanobots** around your Carrier, growing your swarm!
- **Attack aliens** — Direct your cyan nanobot swarm at red alien squares. Boids destroy aliens on contact (+10 score).
- **Survive** — If an alien touches your blue Carrier, the game resets.

## Development

### Prerequisites

- Rust 1.77+
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- `wasm-pack`: `cargo install wasm-pack`

### Build Commands

```bash
# Build WASM client
wasm-pack build --target web -d ../pkg client

# Build server
cargo build --release -p server

# Run server
cargo run -p server
```

## GitHub Actions

Pushes to `master` automatically:

1. Build server binary (Linux x86_64)
2. Build client WASM
3. Build & push Docker image to `ghcr.io`

Docker image:
```
ghcr.io/ubermetroid/rogue-swarm:latest
ghcr.io/ubermetroid/rogue-swarm:<sha>
```

## License

MIT
