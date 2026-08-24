# Terrain Assets

## Heightmap

- https://tangrams.github.io/heightmapper/#11.16667/34.4293/126.4164
- export PATH="$HOME/.local/bin:$PATH" && rm -rf data/terrain/height/r*/h_*.bin && find data/terrain/height/ -type d -empty -delete 2>/dev/null; uv run --with Pillow --with numpy tools/import_heightmap.py     client/public/textures/height_map.png     --min-height -7 --max-height 60     --origin-tile -29 -31     --terrain-dir data/terrain

## References

- https://blog.runevision.com/2026/03/fast-and-gorgeous-erosion-filter.html for reference

## Fantasy World Map Textures

- `tools/terrain-gen/assets/world-map/ocean.png` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier is not exposed), generated 2026-08-24 using the user-provided fantasy map as a style reference; project-owned generated asset.
- `tools/terrain-gen/assets/world-map/lowland.png` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier is not exposed), generated 2026-08-24 using the user-provided fantasy map as a style reference; project-owned generated asset.
- `tools/terrain-gen/assets/world-map/forest.png` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier is not exposed), generated 2026-08-24 using the user-provided fantasy map as a style reference; project-owned generated asset.
- **[미사용]** `tools/terrain-gen/assets/world-map/mountain.png` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier is not exposed), generated 2026-08-24 using the user-provided fantasy map as a style reference; project-owned generated asset. 완성된 봉우리와 그림자가 반복되는 문제로 `rock-albedo.png`로 교체했다.
- `tools/terrain-gen/assets/world-map/rock-albedo.png` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier is not exposed), generated 2026-08-24 in new-image mode as a seamless, unlit gray-brown granite/slate/scree albedo without peaks, roads, rivers, trees, text, or UI; project-owned generated asset.
- `tools/terrain-gen/assets/world-map/world-atlas-guide.png` — OpenAI Codex built-in ImageGen, workspace-provided tier (exact tier is not exposed), generated and terrain-only edited 2026-08-24 in edit/reference mode from the project minimap atlas with the user-provided fantasy map as the quality and art-direction reference. 대륙·해안·섬 배치를 유지하면서 북서광 2.5D 산맥, 연속 숲, 짙은 바다와 청록색 연안을 생성하고, 실제 월드 데이터와 중복되지 않도록 AI 가이드의 강·도로·문자·표식을 제거한 project-owned asset.

## Splat Map Texture GLB Export

- Plane의 크기는 상관없다. 코드에서 geometry는 무시하고 material의 텍스처만 추출한다.
  - `splatLayerLoader.ts`가 GLB를 로드한 뒤 첫 번째 `MeshStandardMaterial`에서 `map`, `normalMap`, `roughnessMap`, `metalnessMap`, `aoMap`만 꺼내 쓴다.
  - 터레인 geometry는 별도로 `PlaneGeometry(64, 64)`를 생성한다.
- 중요한 것은 Blender에서 material에 올바른 텍스처(albedo, normal, roughness 등)가 할당되어 있는지이다.
