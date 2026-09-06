# Agent Guidelines

- Avoid comments in code where possible; only write them when truly necessary, keeping them short and concise.
- If you find long or verbose comments in existing code, rewrite them to be short and concise, or remove them where possible.
- When adding a new asset, record its source in the matching `doc/assets/` file, with the license (and for AI/paid tools, the tier + generation date). Mark entries that fall out of use with **[미사용]**.
- Before production deployment, read [doc/DEPLOY_NOTES.md](doc/DEPLOY_NOTES.md) and carry out any pending operator-data transfers recorded there.

## Python

- When running Python in this repository, use the project virtual environment at `.venv`.
- Prefer `.venv\Scripts\python.exe` for direct Python commands.
- Prefer `uv pip install ...` for installing Python packages into the active project environment.

## Pre-Commit Validation

- For simple frontend edits such as text, color, or spacing changes, skip Prettier, `npm run check`, and `npm run lint`. For more substantial frontend changes, run `npm run check` and `npm run lint`.
- For Rust changes, run `cargo fmt` and `cargo check`.
