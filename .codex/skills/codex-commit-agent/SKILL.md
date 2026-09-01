---
name: codex-commit-agent
description: Run the repository commit workflow safely and consistently when the user explicitly asks to commit changes or create a save-point commit. Detect changed project areas, run required quality checks, draft an imperative English commit message, and commit immediately without asking for duplicate message approval.
---

# Codex Commit Agent

## Overview

Validate changed areas, stage the intended files, and create a clean commit after required checks pass.
Treat an explicit user request to commit as authorization for `git commit`; do not ask again solely to approve an assistant-drafted message. A statement that work is complete is not authorization unless it also asks for a commit.

## Workflow

1. Inspect current changes.
- Run `git status --short`.
- Build the changed-file set from staged, unstaged, and untracked files.
- If no changes exist, stop and report that there is nothing to commit.

2. Detect which project areas need checks.
- If any file is under `client/`, run client checks in `client/`.
- If files are under `tools/<tool-name>/`, run checks in each changed `tools/<tool-name>/`.
- If any file is under `server/`, run server checks in `server/`.

3. Run quality checks.
- Preferred path: run `./.codex/skills/codex-commit-agent/scripts/validate.sh`.
- Equivalent manual checks:
  - `client/` and each changed `tools/<tool-name>/`: `npm run format`, `npm run lint`, `npm run check`
  - `server/`: `cargo fmt`, `cargo check`
- If a check fails, stop and report the failing command and actionable errors.

4. Review commit contents.
- Run `git status --short` again.
- Review `git diff --staged` (or `git diff` if nothing is staged yet).
- Stage intended files with `git add ...` (avoid staging unrelated changes).

5. Draft and commit.
- Draft a concise English commit message in imperative present tense.
- Keep the title under 72 characters.
- Use a user-provided message when one was supplied; otherwise choose the message from the staged diff.
- Once checks pass and the staged files match the requested scope, run `git commit -m "<message>"` immediately.
- If commit fails, report the exact failure and next action.
- Push only when the user also requested a push, publish, or remote workflow.

## Commit Message Rules

- Use English only.
- Start with a verb: `Add`, `Fix`, `Update`, `Refactor`, `Remove`, `Improve`.
- Describe what changed clearly and specifically.
- Keep it scoped to staged files.

## Failure Handling

- Never commit when required checks fail.
- Ask before committing only when the staged scope is ambiguous, includes unrelated changes, or the user explicitly requested message review.
- If formatting updates files, include those changes in the reviewed/staged set before commit.
