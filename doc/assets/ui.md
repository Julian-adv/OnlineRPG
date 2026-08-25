# UI Assets

## Icon

- https://icon-sets.iconify.design/fa6-solid/people-group/
- https://icon-sets.iconify.design/icon-park-solid/backpack/
- https://icon-sets.iconify.design/fa6-solid/handshake-simple/ — social corner button in GameHud.svelte
- https://icon-sets.iconify.design/fa6-solid/face-smile/ — Emotes entry in the social flyout, GameHud.svelte
- GitHub mark (octicon mark-github, MIT) — inline SVG in LoginScreen.svelte

## World map

- `client/public/textures/ui/world-map/dark-wood.webp` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier not exposed), 2026-08-24; user target image used as the style reference; project-owned generated asset. Shipped as 768² WebP q88 (2026-08-26): the source 1254² PNG was 2.0 MB but is only drawn as a 220 px button tile and a 52 px header bar
- `client/public/textures/ui/world-map/ornate-frame.webp` — OpenAI Codex built-in ImageGen edit, workspace-provided tier (exact tier not exposed), 2026-08-24; user target image used as the style reference, then ffmpeg color-keyed to restore real alpha transparency; project-owned generated asset. Shipped as 1254² WebP q90 (2026-08-26), same resolution as the 1.3 MB source PNG
- Settlement crest marker + player self-marker — hand-authored inline SVG in WorldMapDialog.svelte (OpenAI Codex, 2026-08-24), original shapes with no external source

## Party UI design mockups

- `doc/design/party-ui/openmmo-current-game.jpg` — local OpenMMO gameplay capture from `localhost:10004`, 2026-08-05
- `doc/design/party-ui/concept-a-compact-tactical.png` — OpenAI Codex built-in ImageGen image edit/composite, workspace-provided tier (exact tier not exposed), 2026-08-05
- **[미사용]** `doc/design/party-ui/concept-a-v2-class-icon-leading-no-hp-numbers.png` — OpenAI Codex built-in ImageGen precise-object edit using a preview of the project-owned party class icons, workspace-provided tier (exact tier not exposed), 2026-08-05; replaced by exact SVG composites
- `doc/design/party-ui/concept-b-portrait-cards.png` — OpenAI Codex built-in ImageGen image edit/composite, workspace-provided tier (exact tier not exposed), 2026-08-05
- `doc/design/party-ui/concept-c-horizontal-combat-strip.png` — OpenAI Codex built-in ImageGen image edit/composite, workspace-provided tier (exact tier not exposed), 2026-08-05
- Inputs for the three mockups: the local gameplay capture and the existing `female_knight.png`, `ranger.png`, `female_priest.png`, and `rogue.png` character concept assets
- `doc/design/party-ui/concept-01-fixed-column.png` — deterministic local composite using the gameplay capture, project character concepts, and exact party SVG assets; project-owned, 2026-08-05
- **[미사용]** `doc/design/party-ui/concept-02-portrait-badge.png` — deterministic local composite using the gameplay capture, project character concepts, and exact party SVG assets; replaced by the inset-icon revision, project-owned, 2026-08-05
- `doc/design/party-ui/concept-02-portrait-inset-red-hp.png` — deterministic local composite using the gameplay capture, project character concepts, and exact party SVG assets; icon-only portrait inset with red HP bars, project-owned, 2026-08-05
- `doc/design/party-ui/concept-03-icon-tile.png` — deterministic local composite using the gameplay capture, project character concepts, and exact party SVG assets; project-owned, 2026-08-05
- `doc/design/party-ui/concept-04-class-rail.png` — deterministic local composite using the gameplay capture, project character concepts, and exact party SVG assets; project-owned, 2026-08-05
- `doc/design/party-ui/concept-05-portrait-side-tab.png` — deterministic local composite using the gameplay capture, project character concepts, and exact party SVG assets; project-owned, 2026-08-05

## Party class icons

- `client/public/icons/party/class-knight.svg` — original OpenMMO vector asset authored with OpenAI GPT Image 2, ChatGPT Pro tier, 2026-08-05; project-owned
- `client/public/icons/party/class-barbarian.svg` — original OpenMMO vector asset authored with OpenAI GPT Image 2, ChatGPT Pro tier, 2026-08-05; project-owned
- `client/public/icons/party/class-caveman.svg` — original OpenMMO vector asset authored with OpenAI GPT Image 2, ChatGPT Pro tier, 2026-08-05; project-owned
- `client/public/icons/party/class-valkyrie.svg` — original OpenMMO vector asset authored with OpenAI GPT Image 2, ChatGPT Pro tier, 2026-08-05; project-owned
- `client/public/icons/party/class-ranger.svg` — original OpenMMO vector asset authored with OpenAI Codex, workspace-provided tier (exact tier not exposed), 2026-08-05; project-owned
- `client/public/icons/party/class-priest.svg` — original OpenMMO vector asset authored with OpenAI GPT Image 2, ChatGPT Pro tier, 2026-08-05; project-owned
- `client/public/icons/party/class-rogue.svg` — original OpenMMO vector asset authored with OpenAI GPT Image 2, ChatGPT Pro tier, 2026-08-05; project-owned
- `client/public/icons/party/class-bard.svg` — original OpenMMO mandolin vector asset authored with OpenAI Codex, workspace-provided tier (exact tier not exposed), 2026-08-06; project-owned
- `client/public/icons/party/leader-crown.svg` — original OpenMMO vector asset authored with OpenAI Codex, workspace-provided tier (exact tier not exposed), 2026-08-05; project-owned
