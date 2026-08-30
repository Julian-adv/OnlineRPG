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

## Instrument UI

- `client/public/textures/ui/instrument/mandolin-ornament.webp` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier not exposed), 2026-08-30; project-owned generated asset. Source 1983×793 PNG, color-keyed, cropped, and scaled to a transparent 1600×323 WebP for the free-play HUD
- **[미사용·삭제]** `client/public/textures/ui/instrument/mandolin-emblem.webp` — OpenAI Codex built-in ImageGen using the mandolin ornament as a material and palette reference, workspace-provided tier (exact tier not exposed), 2026-08-30; project-owned generated asset. Source 1254² PNG, background-keyed and scaled to a transparent 256² WebP; removed from the free-play HUD in favor of clean negative space, then deleted from the repository on 2026-08-30

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

## NPC 거래 초상화

- `client/public/portraits/karl.webp` — 경비병 Karl 초상화; OpenAI ChatGPT 이미지 생성, ChatGPT Pro tier, 2026-06-12; project-owned. 원본(배경 있는 1122×1402 PNG)은 `../images/characters/karl-portrait.png`; 배포본은 커밋 edf3acd7의 배경 제거된 1122² PNG를 축소한 것 (원본에서 어떻게 정사각형으로 다듬었는지는 기록 없음)
- `client/public/portraits/rica.webp` — 상인 Rica 초상화; OpenAI ChatGPT 이미지 생성, ChatGPT Pro tier, 2026-06-10; project-owned. 원본(배경 있는 1300×1210 PNG)은 `../images/characters/rica-portrait.png`, 배경 제거·정사각 크롭한 1210² PNG는 커밋 aa29e30f에 남아 있다 (비율 왜곡 없음)
- `client/public/portraits/wick.webp` — 야간 상인 Wick의 거래 창 초상화; OpenAI ChatGPT 이미지 생성, ChatGPT Pro tier, 2026-08-27; project-owned. 원본(배경 있는 1122×1402 PNG)은 `../images/characters/wick-portrait.png`; 배포본은 그 위쪽 정사각 크롭(`crop=1122:1122:0:0`)을 배경 제거한 것
- `client/public/portraits/miriel.webp` — 여관 메이드 Miriel의 거래 창 초상화; 2026-09-02 추가, 생성 도구·등급 미기록(원본 메타데이터에는 Paint.NET 5.1.12 편집 흔적만 있음); project-owned. 원본(배경 제거된 1230² PNG)은 `../images/characters/miriel-portrait.png`; 배포본은 512² WebP q88, alpha 유지, 비율 그대로
- `client/public/portraits/cocoly.webp` — 여관 메이드 Cocoly의 거래 창 초상화; 2026-09-02 추가, 생성 도구·등급 미기록(원본 메타데이터에는 Paint.NET 5.1.12 편집 흔적만 있음); project-owned. 원본(배경 제거된 1056² PNG)은 `../images/characters/cocoly-portrait.png`; 배포본은 512² WebP q88, alpha 유지, 비율 그대로

세 장 모두 512² WebP q88(알파 유지)로 축소해 배포한다 (2026-08-27) — 비율은 건드리지 않고 크기만 줄인다. `TradeWindow.svelte`가 폭 160px로 그리므로 1122~1210² 원본은 3배 넘게 과했다. 합계 4.8MB → 194KB. 파일명이 곧 `traderId`라 `/portraits/{traderId}.webp` 규칙으로 자동 해석된다 (새 초상화 추가 시 코드 변경 불필요).
