# Environment Assets

## Grass

- grass: https://github.com/thebenezer/FluffyGrass — **[미사용]** 자체 grass 시스템(`grass-material.ts` 등)으로 대체됨

## Tree

- tree.glb & tree2.glb
  - https://sketchfab.com/3d-models/low-poly-tree-scene-free-89daa5e21f0d4f08a59dba0d566e88bd

## Terrain Textures

`client/public/textures/**/*.glb`는 배포본이다: 원본(Poly Haven 1k GLB)은 `assets/textures-src/`(HF 동기화)에 두고
`python3 tools/repack-material-glbs.py`로 재생성한다 — 샘플 메시·specular 제거, 모든 맵 WebP q90(노멀맵 포함; 렌더 A/B에서 무손실 대비 0.3/255 이내),
palette 전용 레이어는 아틀라스 크기인 512로 축소. 270 MB → 31 MB (2026-09-05).
white-cloud.jpg도 같은 폴더의 원본(5964px)을 2048px q85로 줄인 것.

- 자갈: https://polyhaven.com/a/gravel_floor — **[미사용]** 도로는 gravel_road 사용 (palette.json), 참조 없음
- 잔디: https://polyhaven.com/a/rocky_terrain_02
- 눈: https://polyhaven.com/a/snow_02
- 흙: https://polyhaven.com/a/red_laterite_soil_stones
- 모래: https://polyhaven.com/a/sandy_gravel_02
- 도로: https://polyhaven.com/a/gravel_road
- 절벽: https://polyhaven.com/a/rocky_trail
- 강바닥: https://polyhaven.com/a/ganges_river_pebbles
- 해안 모래: https://sketchfab.com/3d-models/fine-sand-material-6e54464d405a4c1e8bdb0f81e8d74db2 — **[미사용]** 참조 없음
- https://polyhaven.com/a/cobblestone_color — **[미사용]** 참조 없음
- https://polyhaven.com/a/grey_stone_path
- https://polyhaven.com/a/stone_pathway
- https://polyhaven.com/a/pavement_02 — **[미사용]** 참조 없음 (patterned_paving_02 사용)
- https://polyhaven.com/a/patterned_paving_02
- https://polyhaven.com/a/japanese_stone_wall — **[미사용]** 참조 없음
- https://polyhaven.com/a/rock_embedded_concrete — **[미사용]** 참조 없음
- marble_cliff_01 (Poly Haven) — **[미사용]** 초기 절벽 텍스처, rocky_trail로 교체됨
- cliff_side (Poly Haven) — **[미사용]** 참조 없음
- gray_rocks (Poly Haven) — **[미사용]** 참조 없음

## Cloud

- <a href="https://kr.freepik.com/free-photo/white-cloud_3816314.htm#fromView=keyword&page=1&position=8&uuid=159badd4-6cbe-4767-aec8-8a40d2789230&query=Sky+cloud">작가 lifeforstock 출처 Freepik</a>

## Sea

- sea https://www.filterforge.com/filters/4141.html — **[미사용]** 물 표면은 절차적 렌더 + waternormals(three.js examples); 4141 사용 흔적 없음
- sea foam https://www.filterforge.com/filters/13843.html — 폼 텍스처로 사용 중 (`textures/13843.png`, water-foam-gen.ts)
- waternormals (Three.js examples) https://github.com/mrdoob/three.js/tree/dev/examples/textures — 물 표면 노멀맵으로 사용 중 (`textures/waternormals.jpg`, scene-init.ts)

## References

- https://github.com/tigerabrodi/webgpu-vfx for reference — **[미사용]** 기법 참고용
- https://freestylized.com/material/moss_ground_01/ — **[미사용]** 참고용, 팔레트에 없음
