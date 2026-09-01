---
name: simplify-small
description: Review a working-tree diff, commit range, pull request, or other specified code changes for duplicated code, unnecessary abstractions, avoidable performance waste, and overly complex conditionals; then apply only minimal behavior-preserving refactors and run relevant tests. Use when the user asks to simplify, clean up, or safely refactor existing changes without altering behavior.
---

# Simplify

Review the requested changes, make only clearly justified simplifications, and verify them with relevant tests. Preserve observable behavior and keep the resulting diff as small as practical.

## Workflow

1. Read the repository instructions that apply to every file in scope.
2. Determine the review scope from the user's request. If no range is specified, inspect the current working-tree diff and status. Preserve unrelated user changes.
3. Read the surrounding implementation and existing tests before editing. Understand contracts, side effects, error behavior, and local conventions.
4. Look specifically for:
   - duplicated logic that can be consolidated locally without creating a broad abstraction;
   - abstractions, wrappers, helpers, or indirection that no longer earn their complexity;
   - repeated computation, needless allocation, redundant I/O, or unnecessary traversal with an evident cost;
   - deeply nested, repeated, or hard-to-follow conditions that can be expressed more directly.
5. Filter every candidate through the safety rules below. Do not edit merely to express a stylistic preference.
6. Apply the smallest coherent patch that resolves the confirmed issues. Avoid opportunistic cleanup outside the reviewed changes.
7. Re-read the final diff for accidental behavior changes and unnecessary churn.
8. Run the narrowest relevant existing tests, expanding only when risk or repository guidance warrants it. Follow repository-specific rules about validation timing and commands.
9. Report what was simplified, what tests ran and their results, and any remaining concern. If no safe improvement exists, leave the code unchanged and say so.

## Safety Rules

- Preserve public APIs, return values, errors, side effects, evaluation order, and externally visible state.
- Treat concurrency, async ordering, resource lifetime, caching, numeric edge cases, and short-circuit evaluation as behavior-sensitive.
- Prefer deletion and local simplification over introducing new shared infrastructure.
- Extract shared code only when duplication is substantive and the resulting abstraction is simpler than the repeated code.
- Make performance changes only when waste is evident from the code or measurements; avoid speculative optimization.
- Do not combine the refactor with formatting churn, renames, dependency changes, or unrelated fixes.
- Do not weaken, delete, or rewrite tests solely to make the refactor pass.
- Stop and explain rather than guessing when behavior cannot be preserved confidently.

## Testing

- Use existing focused tests for the affected behavior first.
- Add or adjust tests only when needed to preserve an uncovered contract and doing so remains within the user's requested scope.
- If tests cannot run, record the exact command and blocker. Never claim unrun tests passed.
- Respect repository instructions that reserve full validation or pre-commit checks for commit time; targeted behavior tests are distinct unless those instructions say otherwise.
