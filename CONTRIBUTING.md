# Contributing to OpenMMO

Contributions are welcome — this guide collects the practical details that are easy to discover only after CI fails or a PR collides with someone else's.

## Before you start

- [doc/TODO.md](doc/TODO.md) is the maintainer's backlog and the best source of work that is wanted. Verify against the current code that the item is still open — the list moves fast.
- Check [open pull requests](https://github.com/Julian-adv/OpenMMO/pulls) before starting: several contributors work in parallel, and an item can be claimed between one day and the next.
- For larger changes (new systems, UI, design decisions), open an issue describing the approach first.

## Development setup

Follow the [Development Setup](README.md#development-setup) section of the README. Two steps live outside git and are required for a working world: fetching binary assets (`bash tools/fetch-assets.sh`, re-run whenever `assets.lock` changes) and baking terrain (`cargo run -p terrain-gen --release -- bake --seed 42`, ~73 GB).

## Checks CI runs

CI ([.github/workflows/ci.yml](.github/workflows/ci.yml)) runs the checks below on every PR; running them locally before pushing saves a round-trip.

Rust, from the repo root:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Client, from `client/`:

```bash
npm test              # vitest
npm run check         # svelte-check + tsc
npm run lint          # eslint
npm run format:check  # prettier (npm run format to fix)
```

If you touched Rust code in `shared/`, run `npm run build:wasm` first so the client checks see the current WASM bindings — CI builds it before everything else.

## Pull requests

- Keep branch names short and descriptive (kebab-case).
- Write commit messages in the imperative mood and explain the intent, matching the existing history.
- In the PR body, describe what changed and how you tested it; if the work comes from `doc/TODO.md`, quote the item.
- On your first PR, a bot asks you to sign the [Contributor License Agreement](CLA.md) by leaving a comment — required before merging.

## Binary assets

3D models, music, and sounds are hosted on Hugging Face, not in git. See [doc/ASSETS.md](doc/ASSETS.md) for the dataset PR flow, and record each asset's origin and license in the matching `doc/assets/` file.
