# OnlineRPG

An online RPG game supporting up to 10,000 concurrent connections

## Tech Stack

**Client:**
- Svelte + TypeScript
- Three.js (Threlte)
- Socket.io-client
- Vite

**Server:**
- Rust
- Tokio (async runtime)
- tokio-tungstenite (WebSocket)
- serde (JSON serialization)

## Development Setup

### Running the Server
```bash
cd server
cargo run
```

### Running the Client
```bash
cd client
npm install
npm run dev
```

## How to Connect

1. Ensure the server is running on `localhost:8080`
2. Run the client on `localhost:5173`
3. Access the game through your browser

## Features

- **Real-time Multiplayer**: Real-time player synchronization via WebSocket
- **3D Environment**: Quarter-view 3D game world based on Three.js
- **Chat System**: Real-time chat functionality
- **Player Movement**: Character control via mouse/keyboard

## Architecture

- **Client**: Svelte component-based UI + Three.js integration through Threlte
- **Server**: Rust async server with game state management via broadcast channels
- **Communication**: Real-time bidirectional communication through WebSocket