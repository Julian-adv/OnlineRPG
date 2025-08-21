# Repository Guidelines

## Project Structure & Module Organization
- Root: `client` (Svelte + Vite + TS) and `server` (Rust).
- Client code: `client/src` with `lib/components`, `lib/network`, `lib/stores`, `lib/types`, assets in `src/assets` and static files in `public`.
- Server code: `server/src` with modules `main.rs`, `connection.rs`, `game_state.rs`, `types.rs`.

## Build, Test, and Development Commands
- Client dev: `cd client && npm install && npm run dev` — start Vite dev server.
- Client build: `npm run build` — production build; `npm run preview` to serve it.
- Client quality: `npm run lint`, `npm run lint:fix`, `npm run format`, `npm run format:check`, `npm run check` (type + svelte-check).
- Server build/run: `cd server && cargo build` or `cargo run` — starts WebSocket server on `127.0.0.1:8080`.
- Server tests: `cargo test` — runs Rust unit/integration tests (add as needed).

## Coding Style & Naming Conventions
- Client: TypeScript, Svelte 5. Lint via ESLint (`eslint.config.ts`) and format via Prettier (`.prettierrc`). Use 2-space indent. Components in `PascalCase.svelte`, files in `kebab-case.ts` where applicable.
- Server: Rust 2021 edition. Prefer `snake_case` for files/modules and `CamelCase` for types. Use `tracing` for logs.

## Testing Guidelines
- Client: No test runner configured yet; prefer adding Vitest for unit tests under `client/src` with `*.test.ts` when introducing logic-heavy modules.
- Server: Add `#[test]` or `#[tokio::test]` in module files; keep tests small and deterministic. Place broader tests in `server/tests/` if needed.

## Commit & Pull Request Guidelines
- Commits: Use imperative, present-tense summaries (e.g., "Add HeightmapTerrain component"). Group related changes; keep scope focused.
- PRs: Include purpose, linked issues, and usage notes. For UI changes, add screenshots or short clips from the Vite preview. Ensure `npm run lint` (client) and `cargo build` (server) pass.

## Security & Configuration Tips
- Default server bind: `127.0.0.1:8080` in `server/src/main.rs`. Adjust if exposing externally and review firewall rules.
- Avoid committing large binary assets in `client/public` unless necessary; prefer Git LFS if assets grow.
