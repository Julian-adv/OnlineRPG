# Dungeon corridor wall candidates

PBR wall textures (Poly Haven, exported to GLB from `.blend`) that were
trialed as the dungeon **corridor** wall texture. `rock_wall_10_1k` was
chosen; the others are the runners-up, kept here for later.

The runner-up `.glb`s in this folder are **not referenced by any code** —
they ship in the build but load nothing until wired up. To switch the
corridor wall to one of them:

1. Point `DUNGEON_CORRIDOR_WALL_TEXTURE_IDX` at it in
   `client/src/lib/utils/dungeon-geometry.ts`, and update the matching
   `HOUSING_TEXTURES` entry (`glb: 'dungeon/<name>'` / `label`) in
   `client/src/lib/utils/housing-textures.ts`.

Active dungeon-only textures in this folder:

- `rock_wall_10_1k.glb` — corridor walls (`DUNGEON_CORRIDOR_WALL_TEXTURE_IDX`)
- `wooden_garage_door_1k.glb` — entrance door (`DUNGEON_DOOR_TEXTURE_IDX`)
- `rusty_metal_grid_1k.glb` — key-locked doors (`DUNGEON_LOCKED_DOOR_TEXTURE_IDX`)

Room walls use `housing/medieval_blocks_03_1k` (`DUNGEON_WALL_TEXTURE_IDX`),
shared with the housing system — unrelated, and not in this folder.
