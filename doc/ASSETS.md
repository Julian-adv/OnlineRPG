# Assets

Asset source links, generation notes, and production workflows are split by topic.

Binary assets (`*.glb`, `*.mp3`, `*.m4a`) are not stored in git. They live in the
[jake-song-openmmo/onlinerpg-assets](https://huggingface.co/datasets/jake-song-openmmo/onlinerpg-assets)
Hugging Face dataset; `assets.lock` pins the exact revision.

- Download: `bash tools/fetch-assets.sh` (idempotent, verifies checksums; re-run after `assets.lock` changes)
- Publish changes (maintainer): `bash tools/push-assets.sh`, then commit the updated `assets.lock`

History up to 2026-07-31 used git LFS; checking out old commits needs
`GIT_LFS_SKIP_SMUDGE=1` once the GitHub LFS quota lapses. `.blend` source files
were removed earlier and remain in LFS history (`git checkout 25b222b1 -- assets/`).

## Contributing assets in a PR

Icons and other images under `client/public` are tracked in git — include them in
the GitHub PR directly. The flow below is only for the gitignored binaries
(`*.glb`, `*.mp3`, `*.m4a`, `*.blend`) and needs no write access to the dataset:

1. Place the file in the working tree (e.g. `client/public/models/...`) and test locally.
2. Upload it as a Hugging Face community PR from any free HF account
   (first path = local file, second = same path inside the dataset):

   ```bash
   hf upload jake-song-openmmo/onlinerpg-assets \
     client/public/models/armor/iron_helmet.glb \
     client/public/models/armor/iron_helmet.glb \
     --repo-type dataset --create-pr
   ```

3. Open the GitHub PR with the code/data changes, record the asset's source and
   license in the matching `doc/assets/*.md` file (AI/paid tools: tier +
   generation date), and link the HF PR in the description. Leave `assets.lock`
   alone — the revision it must pin only exists after the merge.

Maintainer merge: merge the HF PR, download the new file into the local tree, run
`bash tools/push-assets.sh` (identical content dedupes server-side, so this just
regenerates `assets.lock`), and commit the lock with the GitHub PR merge —
contributors receive the asset on their next `fetch-assets.sh`.

## Sources by topic

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
