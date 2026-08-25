# 절차적 지형 생성 시스템 (Procedural Terrain Generation)

이 문서는 OnlineRPG의 32km × 32km 오픈월드를 절차적으로 생성하기 위한
시스템의 설계를 정의한다. 기존 [MAP_DESIGN.md](MAP_DESIGN.md)의 런타임
지형 표현(타일, 하이트맵, 스플랫맵 포맷)은 그대로 유지하고, 이 문서는
**그 아래 파일들을 어떻게 만드는가**를 다룬다.

## 1. 목표

- 32,768 m × 32,768 m 월드 전체를 단일 시드로부터 결정론적으로 생성.
- 시드를 바꿔 가며 오프라인에서 반복 생성 → 만족스러운 결과를 선택
  → 최종 에셋(하이트맵, 스플랫맵, 초목/나무 데이터)을 디스크에
  베이크하는 **반복형 워크플로우**.
- 대륙/바다, 산지/평야, 하천, 정착지, 도로 등 **전역 피처**가 존재하는
  맵. 단순 fBm noise만으로는 "어디든 비슷한" 지형이 되기 쉬움.
- 기존 런타임 지형 로더(`terrain` crate의 `TerrainIO`, `HeightSampler`,
  클라이언트 `terrainHeightManager`)와 **포맷 호환**.

## 2. 핵심 설계: 2단 해상도

전체 맵을 풀 해상도(32,768² ≈ 10억 셀)로 한꺼번에 생성하면 메모리와
시간 비용이 막대하다 (hydraulic erosion 같은 단계는 GPU compute로도
분 단위). 대신 두 단계로 나눈다:

### 2.1 저해상도 전역 맵 (4,096 × 4,096)

- 셀당 8m × 8m. 32 MB uint16 heightmap + 보조 레이어.
- **전역 구조**를 여기서 결정한다: 대륙 모양, 산지 분포, 하천 네트워크,
  정착지 위치, 도로. 즉 "여러 타일에 걸친" 피처는 전부 이 단계에서.
- 이 해상도에서는 hydraulic erosion, flow accumulation, A* pathfinding
  같은 무거운 연산이 모두 저렴하게 돌아간다.

### 2.2 고해상도 타일 베이킹 (1m/px)

- 타일별 65 × 65 하이트맵 vertex + 64 × 64 스플랫맵 cell
  (기존 [MAP_DESIGN.md](MAP_DESIGN.md) 포맷 그대로).
- 전역 맵을 bilinear 샘플링해서 base height을 얻고, 여기에 **국소 detail
  noise**(octave 추가)를 합성.
- 전역 맵이 가리키는 하천 polyline이 타일을 통과하면 거기서 채널을
  carve. 도로도 마찬가지.
- 스플랫맵은 지형 경사, biome, 하천/해안 거리, 도로 유무 등을 기반으로
  각 셀의 primary/secondary 텍스처와 blend를 결정.

## 3. 파이프라인 단계

전역 맵은 아래 순서로 필드가 쌓인다. 각 단계는 이전 단계의 출력을 읽고
자기 필드를 채운다. 단계 결과는 모두 `GlobalMap` 구조에 누적.

| # | 단계 | 출력 필드 | 기법 |
|---|------|-----------|------|
| 1 | 대륙/바다 마스크 | `continent_potential`, `land_mask`, `sea_level_potential` | fBm + 반경 edge falloff + quantile threshold |
| 2 | 고도 레이어링 | `elevation` (f32 meters) | 단일 fBm (X-wrap) + Y-border 산맥 wall + config hotspots/carves. 능선/계곡은 Phase 3 erosion 이 만든다. |
| 3 | Hydraulic erosion | `elevation` 갱신 | dandrino grid-field 시뮬레이션 (rain → 정규화 gradient → semi-Lagrangian 이웃 샘플 → capacity 기반 침식/퇴적 → forward advect → gaussian slippage → velocity → evaporate). 1024² 다운샘플로 돌리고 결과를 4096²로 업샘플 |
| 4 | 하천 추출 | `flow_accumulation`, `rivers: Vec<Polyline>` | Flow field 계산 → 임계값 이상 셀을 trace해 polyline으로 |
| 5 | 정착지 배치 | `settlements: Vec<Settlement>` | Poisson-disk + 지형 적합도 스코어 (해안/강변/평야 가산점) |
| 6 | 도로 망 | `roads: Vec<Polyline>` | 정착지 쌍 간 A*, 비용 = 경사 + 강/늪 페널티 |
| 7 | (런타임) 타일 베이크 | 타일별 hmap/splat 파일 | 전역 맵 샘플 + detail noise + 강/도로 carve + biome → splat |
| 8 | 초목/나무 배치 | 타일별 vegetation, tree data | biome + slope + 수원 거리 기반 density |

**주의:** 4번 하천은 3번 erosion의 flow accumulation 결과물에서 파생된다.
별도 "강 파기" 단계가 아니라 erosion이 자연스럽게 만든 흐름을 이름 붙이는
것. 유저 원안의 1→2→3→4는 이렇게 3+4가 하나의 erosion pass로 통합된다.

## 4. 공유 Rust 코드 배치

생성 로직은 전부 `shared/src/worldgen/` 에 둔다. 이유:

- 서버는 MMO 런타임에서 타일을 lazy-baking 하거나 지형 검증을 해야
  할 수 있음.
- 클라이언트 에디터에서 "현재 시드의 전역 맵 미리보기" 같은 기능을
  붙이려면 WASM으로 같은 코드를 쓰는 것이 편함 (기존 XP/pathfinding
  WASM 바인딩 패턴과 동일).
- 오프라인 도구는 이 crate를 단순히 의존하는 얇은 래퍼가 된다.

현재 구조:

```
shared/src/worldgen/
  mod.rs
  config.rs            # WorldGenConfig (TOML 직렬화)
  noise.rs             # 결정론적 Perlin + fBm (외부 crate 없음)
  global_map.rs        # GlobalMap 누적 구조
  grid.rs              # BFS/min-heap helpers (crate-internal)
  growth.rs            # Eden 성장 기반 대륙 시드 (Phase 1 sub-pass)
  continent.rs         # Phase 1 (대륙/바다 마스크)
  elevation.rs         # Phase 2 (고도 + Y-border 산맥 wall + hotspot/carve)
  erosion.rs           # Phase 3 (dandrino hydraulic erosion)
  rivers.rs            # Phase 4 (flow → polyline + meander/distributary 후처리)
  coasts.rs            # 해안 polyline (Marching Squares on land_mask)
  settlements.rs       # Phase 5 (해안/강변/평야 fitness + greedy min-spacing)
  roads/               # Phase 6 (MST + K-NN A*; merge_parallel_runs +
                       #          merge_parallel_interiors; bridge snap)
  vector_features.rs   # polyline 공유 유틸 (Chaikin, RiverSegment, projection)
  vegetation.rs        # Phase 8 (tree V1 + grass V3 per-tile binary)
  grass_patches.rs     # warped-Voronoi grass patch field
  tile_bake/           # Phase 7 (per-tile bake)
    mod.rs             # 오케스트레이션 + BakeContext
    constants.rs       # TILE_DIM, VERTS_PER_SIDE, HEIGHT_BIAS/STEP,
                       # DETAIL_*, HILLS_*, RIVER_* 상수 (베이크 단계 튜닝)
    context.rs         # BakeContext — 전역 → 타일 샘플링 캐시
    heightmap.rs       # 65×65 heightmap 샘플 + flow-aware 강 carve
    splatmap.rs        # 64×64 V2 splatmap (priority ladder)
    river_field.rs     # RFD1 (River Field Data v1; surfaceY + flowDir) 바이너리 (강 셰이더 입력)
    river_geom.rs      # 공유 width/taper/fan-arc 계산 (heightmap·splatmap·
                       # bridges·river_field 가 모두 같은 폭 공식을 보도록)
    bridges.rs         # bridge deck rectangle flatten + 모델 선택 (stone vs wood_long)
    settlement_flatten.rs # 정착지 영역 평탄화
```

`tile_bake.rs` 가 `tile_bake/` 디렉터리로 분리된 이유: 강·다리·정착지 평탄화·
스플랫맵·heightmap 이 각각 독립한 한 페이즈가 되어 단일 파일이 비대해졌고,
정점/픽셀 루프 비용 분석을 모듈별로 가능하게 하려는 의도. RFD1 출력은
`river_field.rs` 가 단독 책임 — [RIVER_SYSTEM.md](RIVER_SYSTEM.md) 참조.
강 폭/테이퍼/델타 fan 공식은 `river_geom.rs` 가 단일 소스 — 같은 vertex 에 대해
heightmap carve, splatmap sand band, bridge 모델 선택, RFD1 surface 가 동일한
값을 보장한다.

## 5. 오프라인 도구: `tools/terrain-gen`

Rust 바이너리 crate (워크스페이스 새 멤버). `shared::worldgen`에
의존하며, 자체적으로 이미지 출력과 최종 에셋 베이킹을 담당.

### 5.1 명령 구조

```
terrain-gen preview      --seed <N>             [--out <dir>]
terrain-gen bake         --seed <N>             [--out <dir>] [--region-{x,z}-{min,max} <i>]
terrain-gen render-map-world --seed <N> --legacy-source <dir> --out <dir>
terrain-gen inspect-tile --seed <N> --tile-x <TX> --tile-z <TZ>
terrain-gen probe-point  --seed <N> --at <X,Z>  [--at <X,Z> ...]
```

- `preview`: 전역 맵만 생성하고 PNG 여러 장 출력 (elevation, biome,
  rivers overlay, settlements + roads, coasts). 수초 내 완료. 반복 튜닝용.
  `--out` 기본값은 `data/terrain/worldgen_preview` → 실제 PNG 는
  `data/terrain/worldgen_preview/<seed-hex>/` 에 떨어진다.
- `bake`: 전역 맵 + 지정 region 범위 내 타일 파일을 `data/terrain/` 포맷에
  맞춰 디스크에 쓴다. `--out` 기본값은 `data/terrain`. 디폴트 region 범위
  (`-16..=15` X/Z) 는 32 km 월드 전체 = 32×32 regions × 256 tiles =
  **262,144 tiles**. 수 분~ 수십 분 걸리며 rayon 병렬화. bake 끝에서 같은
  `--out` 아래 `worldgen_preview/<seed-hex>/` 에도 preview PNG 를 함께
  떨군다 (preview 명령과 동일 위치, 중복 방지).
- `inspect-tile` / `probe-point`: 디버깅 (§12.3 참조).

### 5.2 config / 파라미터 노출

`WorldGenConfig` 자체는 `serde` 직렬화 가능하지만, 현재 CLI 에 TOML 로더는
없다. 튜닝 표면은 두 곳:

1. **CLI 플래그** (`GenArgs` in `tools/terrain-gen/src/main.rs`) — seed,
   해상도, 대륙 fBm, 침식 sim_res, 정착지/강 mouth 등 자주 만지는 값만
   노출. 나머지 필드는 `WorldGenConfig::default()` 값.
2. **`shared/src/worldgen/config.rs` defaults + `tile_bake/constants.rs`**
   — Phase 3b 의 plain band, 침식 상수, 베이크 단계 detail/hills/river
   튜닝은 코드 상수. 변경 시 리빌드.

### 5.3 출력물

**Preview 모드** (기본 `data/terrain/worldgen_preview/<seed_hex>/`,
`--out <dir>` 지정 시 `<dir>/<seed_hex>/`):
- `01_potential.png` — Phase 1 continent potential field
- `01_land_sea.png`, `01_land_sea_shifted.png` — land mask + X-wrap 검증
- `02_elevation.png`, `02_elevation_hypso.png` — grayscale + 색상 매핑
- `03_rivers.png` — hypso 위 하천 polyline overlay
- `04_settlements.png` — 정착지 라벨 overlay
- `05_roads.png` — 도로망 overlay
- `06_coasts.png` — 추출된 coast polyline overlay
- `meta.json` — 사용된 config 요약, 관측 통계 (실제 sea_ratio, 마을 수 등)

**Bake 모드** (`data/terrain/` 또는 지정 경로) — 팔레트는 전역
(`shared/palette.json`) 이므로 per-region meta 파일은 없다:
- `height/r±xx_±zz/h_±xxxxx_±zzzzz.bin` — uint16 65×65
- `splat/r±xx_±zz/s_±xxxxx_±zzzzz.bin` — 64×64×4 bytes (V2)
- `river-field/r±xx_±zz/r_±xxxxx_±zzzzz.bin` — RFD1 (강 있는 타일만)
- `trees/r±xx_±zz/t_±xxxxx_±zzzzz.bin` — Phase 8 tree V1
- `grass/r±xx_±zz/g_±xxxxx_±zzzzz.bin` — Phase 8 grass V3
- `minimap/r±xx_±zz.png` — 1024px shaded-relief map tile
- `minimap/{128,256,512}/r±xx_±zz.png` — 월드맵 줌별 LOD
- `objects/r±xx_±zz.json` — region 단위 오브젝트 목록 (현재 bridge placements)
- `worldgen.json` — 시드, config, 정착지/도로 목록 (게임 서버가
  로드할 수 있도록)
- `worldgen_preview/<seed_hex>/` — bake 중 같은 시드의 §5.3 preview PNG 도 함께 dump (preview 명령의 기본 출력 경로와 동일)

## 6. 반복 워크플로우

1. `terrain-gen preview --seed 12345` 실행.
2. `data/terrain/worldgen_preview/0000000000003039/*.png` (= seed 12345 hex)
   열어서 확인.
3. 마음에 안 들면 시드 바꾸거나 config 조정 → goto 1.
4. 마음에 들면 `terrain-gen bake --seed 12345` (`--out` 기본 `data/terrain`).
5. 게임 실행. 기존 `TerrainIO`가 파일을 그대로 로드.

전체 월드를 다시 베이크하지 않고 기존 height/splat/tree 데이터로 한 지역만
확인하려면 다음 명령을 사용한다.

```powershell
cargo run -p terrain-gen --release -- render-map-region `
  --terrain data/terrain --region-x=-2 --region-z=4 `
  --out data/terrain/worldmap_preview/r-02_+04-relief.png
```

서비스용 LOD까지 해당 지역에 다시 쓰려면 `render-map-pyramid`를 사용한다.

```powershell
cargo run -p terrain-gen --release -- render-map-pyramid `
  --terrain data/terrain --out data/terrain --region-x=-2 --region-z=4
```

판타지 월드맵은 gameplay terrain 전체를 다시 베이크하지 않고 다음 명령으로
별도 생성한다.

```powershell
cargo run -p terrain-gen --release -- render-map-world `
  --seed 42 --legacy-source data/terrain/minimap `
  --out data/terrain/minimap-fantasy
```

빠른 시각 반복은 같은 명령에 `--region-x-min=-2 --region-x-max=-2
--region-z-min=4 --region-z-max=4`를 붙인다. 결과가 맞으면 범위 옵션을 빼서
32×32 전 세계를 생성한다. 출력은 최상위 1024px 타일과 `128/`, `256/`,
`512/` LOD를 합쳐 총 4,096개이며, 텍스처 타일은 PNG 무손실 압축이 거의
듣지 않으므로 lossy WebP(quality 82, `r±xx_±zz.webp`)로 저장한다. 서버는
fantasy WebP를 우선 서빙하고 없으면 legacy PNG로 폴백하며, Content-Type은
파일 매직 바이트로 판별한다. 서버는 PNG 시절의 fantasy 타일은 더 이상 읽지
않으므로, PNG 세대 데이터 디렉터리에는 부분 렌더가 아니라 전 범위 렌더를
한 번 돌려야 한다(퍼블리시가 같은 슬롯의 옛 `.png`도 함께 지운다). 이 경로는 Phase 1–6 전역 필드와 기존
1024px minimap의 1m 의미 마스크만 사용한다. 월드맵 이미지 변경 때문에
262,144 gameplay tile의 height/splat/vegetation을 다시 쓰는 것은 느리고
플레이 데이터를 불필요하게 변경하므로 full `bake`를 실행하지 않는다.

v2 렌더러는 `world-atlas-guide.png`를 32 km 월드 전체에 bilinear 투영해
산맥, 숲 군락, 계곡과 연안이 지역 타일 경계에서 끊기지 않게 한다. 실제
육지/바다 판정은 raw terrain 또는 기존 minimap 의미 마스크가 계속
담당하므로 지형 가이드는 게임의 해안선이나 도시 좌표를 바꾸지 않는다.
가까운 줌의 미세 질감은 반복 가능한 lowland/forest/rock/ocean albedo와
실제 높이의 hillshade를 섞어 보강한다. 가이드 반영은 픽셀 단위 land-cover
게이트를 통과해야 한다: 물/뭍 일치에 더해, 뭍에서는 가이드의 숲 캐노피
판정(어두운 초록)과 실데이터 forest 강도가 일치해야 하므로 가이드가 실제
들판을 숲으로 덮지 못한다. 전역 가이드 비중은 1024/512/256/128
LOD에서 각각 46/64/80/92%로 높아져, 최대 확대에서는 로컬 디테일을 유지하고
기본 월드 줌에서는 한 장처럼 이어진 산맥과 숲 구성을 우선한다.

강과 도로는 지형 색상과 LOD downsample 단계에서 중복 합성하지 않는다.
각 LOD를 저장하기 직전에 한 번만 합성하며, 축척이 작을수록 flow와 경로
길이가 큰 주요 선만 남긴다. 클라이언트는 보이는 지역 이미지를 먼저 하나의
HiDPI offscreen atlas에 공통 정수 경계로 조립한 다음 전체 atlas를 한 번만
회전해 비동기 타일 사이의 대각선 틈을 방지한다.

지명은 minimap PNG에 굽지 않는다. `data-src/map_labels.csv`를 원본으로 두고
클라이언트가 HTML 레이어에 표시한다. 따라서 지형 LOD와 무관하게 철자와 월드
좌표가 유지되며, 줌 단계별 계층과 충돌 회피는
`client/src/lib/utils/worldMapLabelLayout.ts`에서 처리한다. 충돌 시 낮은 우선순위의
텍스트만 숨기고 정착지와 발견 지점의 마커 좌표는 이동하지 않는다.

preview는 수 초 안에 끝나야 반복 튜닝이 실용적임. 그래서 Phase 1-6은
저해상도 전역 맵에서만 동작하도록 최적화한다.

### 6.1 현재 사용 중인 bake 커맨드 (재현용)

`data/terrain`에 들어 있는 현재 월드는 아래 커맨드로 정확히 재현된다.
CLI의 모든 인자는 기본값 그대로이며, seed만 지정했다:

```
cargo run -p terrain-gen --release -- bake --seed 42
```

주요 파라미터:
- `--seed 42` — master seed. **CLI 기본값은 `7`** 이지만 현재 월드는 42 로
  베이크되어 있어 재현하려면 명시적으로 지정해야 한다. (config 의
  hand-tuned hotspots 도 seed 42 기준으로 잡혀 있음.)
- `--res 4096` (글로벌 맵 해상도, 8 m/cell)
- `--sea 0.30` (타겟 해수 비율 → 필터 슬립으로 실측 ~0.37)
- `--wavelength 700`, `--octaves 4`, `--gain 0.5` (대륙 fBm)
- `--continents 3`, `--gap 120`, `--islands 15`
- `--erosion-res 1024` (Phase 3 simulation grid; auto = `ceil(1.4 · sim_res)` iterations)
- `--settlements 60`, `--settlement-spacing 70`
- Region 범위: `x=[-16,+15] z=[-16,+15]` (32×32 regions = 262,144 tiles, 32 km 월드 전체)

`WorldGenConfig` 기본값 외에, CLI 에 노출되지 않은 **코드 상수**도
재현에 필요하다. seed 42 의 현재 튜닝 핵심 (저지대 부드럽게 + 해안 석호
방지 + 강 추출 범위):

| 위치 | 상수 | 현재 값 | 효과 |
|------|------|--------|------|
| `config.rs` (default) | `plain_band_pivot_m` / `plain_band_power` | 1200 m / 15.0 | Phase 3b 깊은 평지 압축 — 1200 m 미만 표고는 강한 power curve 로 0 쪽으로 끌어내려 플레이어가 걷는 평지 면적 확대. 1200 m 위 산악은 그대로. |
| `config.rs` (default) | `lowland_expansion_power` | 1.4 | post-erosion 표고에 `(h/peak)^p` 적용, 저지대 면적 추가 확장. |
| `config.rs` (default) | `min_land_height_m` / 노이즈 | 1.0 m, ±0.25 m | 강 surfaceY (0.5 m) 위로 육지 floor 유지 → 강 메시 depth-fade 가 자연 둑에서 종료. |
| `config.rs` (default) | `coast_beach_max_height_m` / `_distance_cells` | 0.1 m / 12 | 해안 12 cell (~96 m) 동안 자연 표고에서 0.1 m 로 ramp. `coast_cliff_fraction = 0.2` 만큼 절벽 유지. |
| `tile_bake/constants.rs` | `DETAIL_GAIN`, `DETAIL_FREQUENCY` | 0.5, 1/16 m | 베이크 단계 detail noise — 4 옥타브로 1 m vertex 간격까지 도달. |
| `tile_bake/constants.rs` | `HILLS_FREQUENCY` / `HILLS_AMPLITUDE_M` | 1/60 m / 5.0 | 모든 육지에 60 m 파장, ±2.5 m 구릉. Phase 3 의 24 m brush 가 60 m feature 를 평탄화하므로 베이크 단계에서 추가. |
| `tile_bake/constants.rs` | `HILLS_OCTAVES`, `HILLS_GAIN` | 3, 0.5 | 60 / 30 / 15 m 옥타브. |
| `tile_bake/constants.rs` | `HILLS_COASTAL_FADE_M`, `DETAIL_COAST_DAMP` | 3.0 m, 0.5 | base=0..3 m 구간 hills/detail 진폭 페이드 — 해안 석호 방지. |
| `rivers.rs` | `RIVER_PEAK_ELEVATION_FRAC` | 0.3 | Phase 4 river peak 후보 = `max_elevation_m × 0.3` 이상. |
| `tile_bake/constants.rs` | `RIVER_MOUTH_FAN_ARC_CELLS` / `RIVER_DELTA_BUFFER_ARC_CELLS` | 8.5 / 14.0 | mouth distributary fan 범위와, bridge/A* 가 강을 건너야 할 upstream buffer. 후자가 넓어야 다리가 fan apex 위쪽에서 강을 가로지름. |

재현 불가능한 요소는 없음 — 동일 커맨드 + 동일 코드에서 동일 바이트가
나온다 (seed 파생 규칙은 §9). 결과 요약은 `data/terrain/worldgen.json`의
`baked_at` / `measured_sea_ratio` / settlements / roads 배열로 확인.

## 7. 기존 시스템과의 통합

- **포맷**: 기존 `terrain` crate의 uint16 인코딩 (`height = value*0.05
  - 500`), 65×65 vertex layout, V2 splatmap 포맷을 그대로 따른다.
  변경 없음.
- **서버**: 기존 `HeightSampler`, `TerrainIO`가 그대로 파일을 읽는다.
  바뀌는 것 없음.
- **클라이언트 생성 UI 폐기**: 옛 `GenerateTerrainDialog.svelte` 는 제거
  되었고, `client/src/lib/terrain/` 에는 런타임 헬퍼만 남는다 (스플랫
  인코딩, 상수, 미니맵 헬퍼). 절차적 생성은 전적으로 Rust 파이프라인
  소관.
- **No-spawn zones, 정착지**: `worldgen.json`에 저장된 정착지 위치
  데이터는 기존 `NoSpawnZone` 메커니즘과 연계 가능 (자동으로 마을
  중심 반경을 no-spawn으로 등록).

## 8. 월드 경계 처리

- **동서(X축) 연결**: 월드는 X축으로 **원통 위상**이다. 오른쪽 가장자리를
  벗어나면 왼쪽 가장자리로 들어온다. 이를 위해 Phase 1의 대륙 noise는
  **X-periodic**으로 샘플링한다 (3D Perlin에서 X축을 원에 매핑:
  `(R·cosθ, y, R·sinθ)`, `θ = 2π·x/W`). 좌우 경계에서 대륙이 자연스럽게
  이어진다.
- **남북(Y축) 벽**: 위아래는 wrap이 geometry상 어려우므로 **통과 불가능한
  높은 산**으로 막는다. Phase 1에서는 이 경계에 대한 특별 처리 없이
  land가 자연스럽게 형성되게 두고, Phase 2에서 `y < margin` 또는
  `y > res - margin` 영역의 land 고도를 강하게 boost해 산맥 벽으로 만든다.
- **가장자리 바다 깊이**: 플레이어는 바다 깊이를 체감하지 않으므로 인위적
  bias 없음 (과거 시도에서 margin cliff 문제 발생 → 제거).

## 9. 결정론과 시드 파생

하나의 마스터 시드 `u64`에서 출발하여 각 단계가 독립 시드를 쓴다.
단계 시드 = `master_seed ^ PHASE_TAG` (`PHASE_TAG`는 단계별 상수).
이렇게 하면 한 단계의 알고리즘을 바꿔도 다른 단계가 재현 불가능해지지
않는다.

## 10. 미결정 / 추후 결정

- **지하 동굴/지하 구조**: 현 시스템 범위 밖. 하이트맵 위에 별도
  오브젝트로 배치 (기존 정책 그대로).
- **바이옴 구분**: 온도/습도 노이즈로 더 세분(타이가/사막/열대 등)할지.
  현재는 고도/경사/수원 거리만 쓴다.
- **해안 sub-cell smoothing**: §11.1 참조 — Marching Squares staircase
  잔존, `continent_potential` isocontour 로 대체 필요.

> 과거 미결정이었으나 결정됨: **하천 수원** — Phase 4 는 pit-fill + D8 flow
> 위에서 peak trace 로 수원을 찍는다 (peak + flow 결합). **도로 곡선화** —
> A* 출력 polyline 은 각진 채로 저장하고, 베이크 시
> `tile_bake::context::smooth_polylines` 가 Chaikin 으로 처리한다.

## 11. Vector feature distance (현재 구조)

강 / 도로 / 해안의 위치 정보는 모두 **polyline → world-space 세그먼트
거리** 로 통일됐다. 옛 방식은 4K 전역 맵의 raster mask(`river_mask`,
`dist_to_road`, `dist_to_sea`) 를 nearest lookup 해서 8 m 셀 lattice 가
최종 출력에 그대로 노출됐었다.

공통 구조 ([`vector_features.rs`](../shared/src/worldgen/vector_features.rs)):

1. 출처 polyline 을 **Chaikin smoothing** → 8 m vertex 가 곡선으로.
2. World-space 세그먼트로 분해. 타일 bake 시 `*_segments_near_tile(bbox, margin)`
   로 타일 bbox + margin 안에 걸친 세그먼트만 필터 — 타일당 수~수십 개.
3. `project_point_to_segment(px, pz, ax, az, bx, bz) → (d_sq, t)` 로 정점/픽셀
   에서 최근접 세그먼트와 projection 파라미터 `t∈[0,1]` 을 얻는다.
4. 정점별 attribute (width, flow_norm 등) 는 `t` 로 선형 보간.
5. 결과:
   - **강 splat/carve**: `tile_bake/heightmap.rs` 가 세그먼트 거리로 폭/깊이
     보간 carve. RFD1 ([RIVER_SYSTEM.md](RIVER_SYSTEM.md)) 도 같은 세그먼트
     세트를 본다.
   - **도로 splat**: priority ladder 의 도로 슬롯이 세그먼트 거리로 결정.
   - **해안 sand band**: `dist_to_sea` (BFS) 가 `coast_polyline` 세그먼트
     거리로 치환됨. `sample_coast_d_jittered` 의 fBm jitter hack 도 제거.

### 11.1 미해결 영역

- **해안 sub-cell smoothing**: `coasts.rs` 의 Marching Squares 는 binary
  `land_mask` 위에서 동작하므로 vertex 위치가 8 m 격자 ±0.5 셀에 고정된다.
  Chaikin smoothing 으로 corner 는 깎이지만 staircase 자체는 남아있음.
  진정한 sub-cell 정밀도를 얻으려면 `continent_potential` (continuous
  f32 field) 에 isocontour 를 적용해 두 셀의 linear interp 로 vertex 를
  결정해야 함 — 별개의 큰 작업이라 현재는 보류.
- **`dist_to_land`**: bathymetry 전용으로 유지. catmull-rom elevation
  샘플러가 셀당 4×4 이웃을 읽기 때문에 polyline 쿼리로 대체하면 bake
  시간이 폭증.

## 12. 현재 상태 요약

전체 Phase 1–8 모두 구현 완료 + 프로덕션 사용 중 (`data/terrain/` 에 seed 42
월드가 베이크되어 있음). 단계별 핵심:

| Phase | 모듈 | 핵심 알고리즘 |
|---|---|---|
| 1 — 대륙/바다 | `continent.rs` + `growth.rs` | Eden 성장 + union-find + island filter + isthmus cut. X-periodic. |
| 2 — 고도 | `elevation.rs` | Single fBm + Y-border 산맥 wall + config hotspots/carves. 능선/계곡은 Phase 3 erosion 이 만든다. |
| 3 — Erosion | `erosion.rs` | dandrino `simulation.py` 포팅. 1024² grid 에서 `ceil(1.4 · sim_res)` iter, 4096² 로 업샘플. |
| 4 — 강 | `rivers.rs` | Barnes 2014 pit-fill → D8 → peak trace + meander / distributary / self-overlap 후처리. **Phase 4b**: `elevation::seed_river_gap_mountains` 로 강 없는 저지대에 산 hotspot 추가 → flow + rivers 재추출. **Phase 4c**: `elevation::seed_small_island_hills` + `rivers::extract_small_island_rivers` 로 작은 섬에 구릉/강 추가. |
| 5 — 정착지 | `settlements.rs` | habitability 필드 (coast/river dist, slope) + 4-phase greedy (A: 강 drainage basin 별 1 정착지 / B: 내륙 평야 / C: 고립 섬 / D: coverage gap-fill). |
| 6 — 도로 | `roads/` | Prim MST + K-NN 추가 엣지 + 경사 페널티 A*. `merge_parallel_runs`, `snap_crossings_to_grid` 후처리로 bridge 위치 정렬. |
| 7 — 타일 베이크 | `tile_bake/` | 65×65 heightmap + 64×64 V2 splatmap + RFD1 강 field (강 있는 타일만). 강 carve = flow-aware depth/width, bed floor clamp. |
| 8 — 초목 | `vegetation.rs` | Phase 7 의 splatmap vegMeta 바이트(230–249) 를 읽어 tree V1 + grass V3 바이너리를 per-tile 출력. |

### 12.1 베이크 출력물

`bake` 명령은 region 범위 내 타일마다 다음을 쓴다 (팔레트는 전역
`shared/palette.json` 이라 per-region meta 파일 없음):

```
data/terrain/
  height/r±xx_±zz/h_±xxxxx_±zzzzz.bin       # 65×65 uint16
  splat/r±xx_±zz/s_±xxxxx_±zzzzz.bin        # 64×64×4 (V2)
  river-field/r±xx_±zz/r_±xxxxx_±zzzzz.bin  # RFD1 (강 있는 타일만)
  trees/r±xx_±zz/t_±xxxxx_±zzzzz.bin        # V1
  grass/r±xx_±zz/g_±xxxxx_±zzzzz.bin        # V3
  minimap/r±xx_±zz.png                      # 1024px shaded-relief map tile
  minimap/{128,256,512}/r±xx_±zz.png        # 월드맵 줌별 LOD
  objects/r±xx_±zz.json                     # region 단위 오브젝트 (현재 bridges)
  worldgen.json                             # seed/config/settlements/roads
  worldgen_preview/<seed_hex>/              # §5.3 preview PNGs 의 dump (preview 명령과 동일 경로)
```

### 12.2 Preview 출력물

```
data/terrain/worldgen_preview/<seed_hex>/     # 기본 위치 (--out 미지정 시)
  01_potential.png, 01_land_sea.png, 01_land_sea_shifted.png (X-wrap 검증)
  02_elevation.png, 02_elevation_hypso.png
  03_rivers.png, 04_settlements.png, 05_roads.png
  meta.json
```

### 12.3 디버깅 도구

- `terrain-gen inspect-tile --seed N --tile-x TX --tile-z TZ` — 지정 타일에
  영향을 주는 강 segment 들을 텍스트로 덤프. `naturalize_river_meanders` /
  `remove_polyline_self_overlaps` 출력 검증용.
- `terrain-gen probe-point --seed N --at X,Z [--at X,Z ...]` — 지정 world
  좌표에서 자연 표고, 가장 가까운 강 segment 파라미터 (t, 폭, flow_norm),
  carve 후 표고를 분해 출력. 강 carve depth/width 회귀 시 한 점을 picking
  해 검증하는 용도. `--at` 를 반복해 여러 점을 한 번에 probe.
