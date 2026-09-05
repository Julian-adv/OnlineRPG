---
name: codex-commit-agent
description: Run the repository commit workflow when the user explicitly asks to commit changes or create a save-point commit. Upload changed binary assets to Hugging Face and update assets.lock before committing, run required quality checks, and commit with an imperative English message without duplicate approval.
---

# Codex Commit Agent

## Overview

Publish changed binary assets, validate changed areas, and commit the intended files with the updated asset lock.
Treat an explicit user request to commit as authorization for `git commit`; do not ask again solely to approve an assistant-drafted message. A statement that work is complete is not authorization unless it also asks for a commit.
Per the repository owner's instruction, a commit request also authorizes the Hugging Face upload needed for that commit's assets. Do not ask for separate upload approval within that scope.

## Workflow

1. Inspect current changes.
- Run `git status --short`.
- Build the changed-file set from staged, unstaged, and untracked files.
- Also inspect relevant git-ignored binary assets and source files against the paths and SHA-256 hashes in `assets.lock`; Git status alone cannot detect their changes.
- If neither Git changes nor asset changes exist, stop and report that there is nothing to commit.

2. Upload changed assets before committing.
- When the task adds, modifies, moves, or removes HF-managed assets, complete their upload and update `assets.lock` before `git commit`. Include source assets under `assets/`, excluding `.blend1` backups, as well as game binaries under `client/public/`.
- Read `tools/push-assets.sh` and review the upload scope. It replaces remote `client/**` and `assets/**` trees; a partial local checkout must not delete unrelated remote assets. A locally missing file is not an intentional deletion.
- Use `bash tools/push-assets.sh` only with a complete, reviewed local asset tree. Otherwise upload only the intended changes, preserve unrelated remote files and lock entries, and regenerate the affected lock entries.
- Pin the exact successfully uploaded HF revision and record each changed file's SHA-256 hash in `assets.lock`. Verify the files at that revision match the intended contents; do not pin a revision where the assets are absent.
- Stage `assets.lock` with the code, data, and documentation referencing those assets. Keep git-ignored binaries on HF; do not force-add them to Git.
- If upload, authentication, or remote verification fails, stop before committing and report the failure. Do not leave required assets local-only and call the commit complete.
- Skip upload when there are no HF-managed asset changes.

3. Detect which project areas need checks.
- If any file is under `client/`, run client checks in `client/`.
- If files are under `tools/<tool-name>/`, run checks in each changed `tools/<tool-name>/`.
- If any file is under `server/`, run server checks in `server/`.

4. Run quality checks.
- Preferred path: run `./.codex/skills/codex-commit-agent/scripts/validate.sh`.
- Equivalent manual checks:
  - `client/` and each changed `tools/<tool-name>/`: `npm run format`, `npm run lint`, `npm run check`
  - `server/`: `cargo fmt`, `cargo check`
- If a check fails, stop and report the failing command and actionable errors.

5. Review commit contents.
- Run `git status --short` again.
- Review `git diff --staged` (or `git diff` if nothing is staged yet).
- Stage intended files with `git add ...` (avoid staging unrelated changes).

6. Draft and commit.
- Draft a concise English commit message in imperative present tense.
- Keep the title under 72 characters.
- Use a user-provided message when one was supplied; otherwise choose the message from the staged diff.
- Once required asset publication and checks pass and the staged files match the requested scope, run `git commit -m "<message>"` immediately.
- If commit fails, report the exact failure and next action.
- Push Git commits only when the user also requested a push, publish, or remote workflow. Required HF asset uploads happen before the commit as described above.

## Commit Message Rules

- Use English only.
- Start with a verb: `Add`, `Fix`, `Update`, `Refactor`, `Remove`, `Improve`.
- Describe what changed clearly and specifically.
- Keep it scoped to staged files.

## Failure Handling

- Never commit when required asset uploads, remote verification, or quality checks fail.
- Ask before committing only when the staged scope is ambiguous, includes unrelated changes, or the user explicitly requested message review.
- If formatting updates files, include those changes in the reviewed/staged set before commit.
