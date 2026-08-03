---
name: game-login
description: Automate the OpenMMO game login process — open the dev server in real Chrome, sign in with Google, and enter the game with the default-selected character. Use this skill whenever the user says "game login", "log into the game", "start the game", "connect to OpenMMO", or "connect to BottleField".
---

# OpenMMO Game Login

Automates the login flow for the OpenMMO online RPG against the dev server using browser automation.

Auth is Google Sign-In (GSI), not a password form — see [dc3e2467].

## Browser surface: real Chrome only

Use the `mcp__claude-in-chrome__*` tools. **The in-app Browser pane (`mcp__Claude_Browser__*`) cannot complete this login.** GSI falls back to `ux_mode=popup`, and the pane returns `null` from `window.open` even with a real user gesture (`navigator.userActivation.isActive: true`), so the console fills with `[GSI_LOGGER]: Failed to open popup window ... Maybe blocked by the browser?` and `auto_select` retries forever.

The pane does expose FedCM (`IdentityCredential`), so the very first attempt can succeed through the native account chooser — but once FedCM enters its per-origin cooldown, every later attempt drops to the blocked popup path. Do not rely on it.

## Steps

1. **Open the game in Chrome** — `tabs_context_mcp` (`createIfEmpty: true`), then `navigate` to `https://localhost:10004/`.
   - `https` is the tunnelled dev server. A plain local `npm run dev` serves `http://localhost:10004/`; GSI accepts that too, since localhost is exempt from its HTTPS requirement.

2. **Wait for sign-in** — `auto_select: true` means returning users are signed in automatically with no click. Screenshot and check whether the character select screen already appeared.

3. **Click the Google button if still on the login screen** — the pill button reads "Sign in as <name>" / "<name>(으)로 로그인". Clicking it uses the existing Chrome Google session.
   - **If Google asks for a password or a new account, stop and hand back to the user.** Never type credentials.

4. **Wait for character select** — ~3 seconds. The first character is selected automatically, so no manual selection is needed.

5. **Click Start** — enters the game with the default-selected character.

6. **Confirm game loaded** — wait ~5 seconds for the world to load.

7. **Turn on torch** — `read_page` with `filter: "interactive"` to find "Toggle Torch Point Light", then click it. The TORCH button in the FPS debug panel turns orange when active.

8. **Take screenshot** — confirm entry with torch enabled.

## Notes

- Login screen setup lives in [LoginScreen.svelte](../../../client/src/lib/components/LoginScreen.svelte) (`googleId.initialize` with `auto_select`/`itp_support`/`use_fedcm_for_prompt`, then `prompt()` + `renderButton`).
- `VITE_GOOGLE_CLIENT_ID` must be set, or the screen shows "VITE_GOOGLE_CLIENT_ID is not configured" instead of a button.
- Character cards and models are rendered inside a WebGL canvas via Threlte (not DOM), so they can't be targeted by selectors. If a specific character must be picked, dispatch `pointermove`/`pointerdown`/`pointerup` on the canvas with the correct client coordinates AND override `offsetX`/`offsetY` on the synthetic event (Threlte's raycaster reads `offsetX/Y`, which are 0 on synthetic events by default). Without this override, the raycast lands at the top-left and hits the wrong character.
- The torch button is in the FPS debug panel (top-left). Find it by its accessible name rather than clicking by coordinates.
