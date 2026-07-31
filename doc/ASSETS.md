# Assets

Asset source links, generation notes, and production workflows are split by topic.

Binary assets (`*.glb`, `*.mp3`, `*.m4a`) are not stored in git. They live in the
[jake-song-openmmo/onlinerpg-assets](https://huggingface.co/datasets/jake-song-openmmo/onlinerpg-assets)
Hugging Face dataset; `assets.lock` pins the exact revision.

- Download: `bash tools/fetch-assets.sh` (idempotent, verifies checksums)
- Publish changes (maintainer): `bash tools/push-assets.sh`, then commit the updated `assets.lock`
- Contributing assets in a PR: upload via a Hugging Face community PR on the dataset
  repo (any HF account), and reference it from the GitHub PR

History up to 2026-07-31 used git LFS; checking out old commits needs
`GIT_LFS_SKIP_SMUDGE=1` once the GitHub LFS quota lapses. `.blend` source files
were removed earlier and remain in LFS history (`git checkout 25b222b1 -- assets/`).

- [Environment](./assets/environment.md)
- [Characters](./assets/characters.md)
- [Monsters](./assets/monsters.md)
- [Items](./assets/items.md)
- [Props and Buildings](./assets/props.md)
- [Terrain](./assets/terrain.md)
- [Animation](./assets/animation.md)
- [Blender](./assets/blender.md)
- [UI](./assets/ui.md)
- [Music](./assets/music.md)
- [Sound Effects](./assets/sfx.md)

See also:

- [Animation pipeline and mapping rules](./ANIMATION.md)
