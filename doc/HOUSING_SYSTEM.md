# Housing System — Modular Room-Based Architecture

## Overview

유저가 방(Room)을 자유롭게 조합하여 집을 짓는 모듈러 하우징 시스템.
벽/바닥/지붕 텍스쳐 커스터마이즈, 문/창문 배치, 최대 4층 지원.
집 안에 들어가면 앞벽+지붕이 숨겨져 내부가 보인다.

리퍼런스:
  - https://www.youtube.com/watch?v=5jvJUPrmS18&t=5s
  - https://www.youtube.com/watch?v=zBVPcr7VjyQ

플레이어가 설계도로 집을 짓는 흐름은 [HOUSE_BUILDING.md](HOUSE_BUILDING.md).

## Data Model

### HouseData

```rust
pub struct HouseData {
    pub id: String,
    pub owner_id: String,
    pub origin: Position,          // 월드 좌표 (1m 그리드 스냅)
    pub rooms: Vec<RoomData>,
    pub passability: Vec<PassabilityGrid>,  // 셀 기반 통행 가능 여부
}
```

### RoomData

```rust
pub struct RoomData {
    pub room_type: RoomType,        // Normal | Stairwell
    pub roof_type: RoofType,        // Flat | Gabled | Steep
    pub roof_ridge_dir: RoofRidgeDir, // Auto | X | Z
    pub stair_reversed: bool,       // 계단 오름 방향 반전
    pub local_x: i32,              // house origin 기준 오프셋 (미터)
    pub local_z: i32,
    pub size_x: u8,                // 1~6m (1~2m는 복도/벽장용)
    pub size_z: u8,                // 1~6m
    pub floor_level: u8,           // 0~3 (1층~4층)
    pub floor_texture: u8,         // 텍스쳐 카탈로그 인덱스
    pub roof_texture: u8,
    pub wall_height: f32,          // 기본 3m
    /// 벽은 1m 세그먼트 배열 (예: 5m 북벽 → 5개 WallConfig)
    pub wall_north: Vec<WallConfig>,  // length = size_x
    pub wall_south: Vec<WallConfig>,  // length = size_x
    pub wall_east: Vec<WallConfig>,   // length = size_z
    pub wall_west: Vec<WallConfig>,   // length = size_z
}
```

### WallConfig

```rust
pub struct WallConfig {
    pub variant: WallVariant,
    pub texture: u8,
    pub is_open: bool,          // 문/창문 열림 상태 (기본 false)
}

pub enum WallVariant {
    Solid,
    WithDoor,
    WithWindow,
    Open,           // 인접 방 연결 또는 계단 공간
}
```

### PassabilityGrid

```rust
pub struct PassabilityGrid {
    pub floor_level: u8,
    pub origin_x: i32,         // house local 좌표 기준 그리드 원점
    pub origin_z: i32,
    pub width: u8,             // X 셀 수
    pub depth: u8,             // Z 셀 수
    pub cells: Vec<u8>,        // N=1, E=2, S=4, W=8 비트마스크
}
```

- 방 크기: 1~6m (템플릿 세트, 복도 6×1·6×2 포함), 배치 그리드: 1m 단위 스냅
- 벽은 1m 세그먼트 단위: 5m 북벽 → `wall_north` 길이 5
- 인접 방 공유 면: 배치 시 양쪽 `Open`으로 자동 설정. 내부 벽을 세우려면 한쪽만 `Solid`/`WithDoor`로 바꾸고 맞은편은 `Open` 유지 (에디터가 강제, 벽 한 겹). 서버는 검증하지 않음
- N층 y 오프셋 = N × (wall_height + FLOOR_THICKNESS)

## Wall System

### Wall Segment Layout

벽은 1m 세그먼트로 분할. 코너 겹침 방지를 위해 `WALL_THICKNESS` 만큼 축소:

```
segW = (wallSpan - WALL_THICKNESS) / numSegs
```

### Solid Wall

- 메인 면: `segW × wallHeight × WALL_THICKNESS`
- 프레임 geometry (텍스쳐 `fitSegment: true` + 나무 텍스쳐 존재 시):
  - **가로 빔**: 바닥에서 40% 높이, 벽 높이의 5% 두께
  - **좌우 기둥**: 세그먼트 폭의 10%, 전체 벽 높이
  - **하단 스트립**: 벽 높이의 5%
  - **X자 대각선**: 하단~가로 빔 영역에 교차 대각선 2개

### Door Wall

개구부 `0.8m × 2.2m`, 세그먼트 중앙 배치:

- 좌우 솔리드 스트립: `(segW - 0.8) / 2`
- 프레임 (fitSegment 시):
  - 좌우 기둥: `segW × 10%` 폭, 전체 높이
  - 헤더 빔: 개구부 상단에 `FRAME_DIAG_THICKNESS(0.06m)` 두께

**문짝 패널**:

- 크기: `DOOR_WIDTH × (DOOR_HEIGHT - FRAME_DIAG_THICKNESS/2) × WALL_THICKNESS`
- 힌지: `THREE.Group` 피벗, 개구부 왼쪽 가장자리에 위치
- 프레임 기둥 겹침 방지: `inset = max(0, pillarW - (segW - openW) / 2)` 만큼 힌지를 안쪽으로 이동
- 패널 Z 오프셋: 벽의 **내부 면**에 배치 (`isFront === isNS`에 따라 부호 결정)
- 회전 각도:
  - 닫힘: `closedAngle` = 0 (NS벽) 또는 π/2 (EW벽)
  - 열림: `openAngle = closedAngle - π/2`

### Window Wall

개구부 `0.8m × 1.0m`, 바닥에서 `1.2m` 높이에 배치:

- 하단 스트립: `0.8m × 1.2m`
- 상단 스트립: 개구부 위 나머지 높이
- 프레임 (fitSegment 시):
  - 좌우 기둥, 하단 스트립, 헤더, 가로 빔 + X자 대각선

**창문 셔터 (좌우 2개)**:

- 크기: `(WINDOW_WIDTH/2) × panelH × (WALL_THICKNESS/2)` — 문보다 얇은 두께
- `panelH`: 프레임 가로빔 상단 ~ 헤더 하단 사이 (겹침 방지)
- 닫힌 상태: 패널이 개구부를 덮음 (좌반+우반)
- 열린 상태: 집 바깥쪽으로 90° 회전
  - `openAngle = closedAngle + outwardSign × side × π/2`
  - `outwardSign = isFront ? 1 : -1` (벽의 외부 방향)
- 힌지: 문과 동일한 기둥 겹침 보정 적용, 내부 면에 배치

### Corner Pillars

- NS벽이 담당 (중복 방지)
- 크기: `FRAME_DEPTH × wallHeight × FRAME_DEPTH`
- fitSegment 텍스쳐 사용 시에만 생성

## Door & Window Interaction

### 열기/닫기 흐름

1. E키 → 플레이어 근처(2m) 문 또는 창문 탐색 (`findNearestDoor`)
2. 서버에 `ToggleDoor` 전송 (낙관적 토글 없음)
3. 서버가 `WithDoor` 또는 `WithWindow` variant 검증 후 토글
4. **모든 플레이어**에게 `DoorToggled` 브로드캐스트 — 서버 권위적 상태
5. 클라이언트: `handleDoorToggled`로 서버 상태 적용
   - **문**: passability edge 비트도 업데이트 (열린 문 = 통과 가능)
   - **창문**: passability 업데이트 없음 (열려도 통과 불가)
6. 애니메이션: 게임 루프에서 피벗 rotation.y를 `closedAngle ↔ openAngle` lerp
   - 속도: `DOOR_SWING_SPEED = π rad/s` (~0.5초에 90°)

### syncDoorStates

geometry hash에서 `isOpen` 제외 비교 → isOpen만 변경된 경우 geometry rebuild 없이 상태만 동기화.
창문 패널 2개가 같은 segmentIndex를 공유하므로 둘 다 동일 isOpen 값을 받음.

## Stairwell System

### 계단 방향

- 긴 축 방향으로 오름: `sizeZ >= sizeX`이면 Z축, 아니면 X축
- `stairReversed: true` → 오름 방향 180° 반전 (entry ↔ exit 위치 교환)

### Geometry

```
totalRise = wallHeight + FLOOR_THICKNESS
stairRun = max(sizeX, sizeZ) - LANDING_DEPTH × 2
stepCount = round(totalRise / 0.25)
stepHeight = totalRise / stepCount
stepDepth = stairRun / stepCount
```

- **Entry 랜딩** (하층): `LANDING_DEPTH(0.5m)` 깊이 평탄 영역
- **Exit 랜딩** (상층): 동일
- **계단 스텝**: entry~exit 사이 균등 분할
- 인접 방의 솔리드 벽이 있으면 `WALL_THICKNESS` 만큼 geometry 인셋

### Y 오프셋 계산

플레이어 위치에 따라 계단 위 Y 높이 계산 (`getStairwellYOffset`):
- entry 랜딩 위 → `baseY + FLOOR_THICKNESS/2`
- exit 랜딩 위 → `baseY + totalRise + FLOOR_THICKNESS/2`
- 스텝 위 → 위치 비율에 따라 선형 보간

## Jetty (Floor Overhang)

상층 바닥이 하층 벽보다 바깥으로 돌출되는 중세 건축 기법:

```
FLOOR_OVERHANG_PER_LEVEL = 0.15m
floorOverhang(floorLevel) = floorLevel × 0.15
```

| 층 | 돌출 |
|----|------|
| 1F (level 0) | 0m |
| 2F (level 1) | 0.15m |
| 3F (level 2) | 0.30m |
| 4F (level 3) | 0.45m |

### Floor Cell 확장

- **코너 셀**: 두 방향으로 overhang 만큼 확장
- **가장자리 셀**: 한 방향으로 확장
- **내부 셀**: 1m×1m 유지

### Stairwell Hole Punching

상층 바닥이 하층 계단실 위에 있으면:
- 계단실 풋프린트에 1m² 구멍 생성
- 구멍 주변에 **overhang strip** 추가 (폭 = `overhang + WALL_THICKNESS`)
  - 코너 부분은 strip 연장으로 간극 방지

## Roof System

### Flat Roof

- 단순 평면: `(sizeX + overhang×2) × (sizeZ + overhang×2)` + `ROOF_OVERHANG(0.3m)` 처마

### Gabled Roof (맞배지붕)

지붕 스팬은 방 단위가 아니라 **footprint** 단위로 계산 (`computeRoofSpans`): 같은 `층|지붕타입|벽높이|용마루방향` 방들을 1m 셀로 래스터화한 뒤 최대 직사각형으로 재구성. 내부를 여러 방으로 쪼개도 지붕은 방 하나일 때와 같다.

- 용마루 방향: 명시값 > 직사각형 footprint를 이루는 방들은 면적 가중 다수결로 통일 > 그 외 방은 자기 긴 축, 폭 ≤ 2m 방(복도)은 가장 긴 변을 공유하는 이웃 방향
- 용마루 직각 방향 깊이가 `MAX_ROOF_SPAN_DEPTH`(12m)를 넘으면 M자로 분할 (분할선은 3m 이내의 방 경계에 스냅)
- 회귀 테스트 `house-geo-roof-spans.test.ts`: 실서버 하우스 스냅샷과 동일한 출력 보장

- **두 경사면**: 용마루에서 만남, 두께 `WALL_THICKNESS`
- **용마루 방향**: `roofRidgeDir` (auto = 긴 축 방향)
- **박공벽**: 양쪽 끝 삼각형 벽
- **박공 빔**: 삼각형 하단에 `0.12m` 두께 빔 (나무 텍스쳐 시)
- **박공 창문**: 용마루 높이 > 1.0m이면 자동 생성 (`0.6m × 0.7m`)
  - 중심: 용마루 높이의 38%
  - 마진: 가장자리에서 0.25m 이상

```
ridgeHeight = halfShort × ROOF_PITCH
ROOF_PITCH = { gabled: 0.8, steep: 1.4 }
```

### Roof Suppression

상층 방이 하층 방을 완전 커버 시 하층 지붕 생략 (상층 바닥이 대체).

## Multi-Floor

### 상수

```
MAX_FLOOR_LEVEL = 3          // 0~3 (4층까지)
FLOOR_THICKNESS = 0.1m
DEFAULT_WALL_HEIGHT = 3.0m
```

### 층별 Y 좌표

```
floorYBase(level) = level × (wallHeight + FLOOR_THICKNESS)
```

| 층 | Y |
|----|---|
| 1F | 0 |
| 2F | 3.1 |
| 3F | 6.2 |
| 4F | 9.3 |

### Visibility

층별 `front`/`back` 그룹 분리:
- **front**: 남벽 + 서벽 + 지붕 (플레이어 inside 시 숨김)
- **back**: 북벽 + 동벽 + 바닥 (항상 표시)
- 플레이어 층 이상의 front + back 모두 숨김
- 오쏘그래픽 카메라 (pitch 45°, yaw -45°) 기준 앞벽 = 남+서

## Wall Collision (Cell-Based Passability)

### 개요

1m 셀 기반 통행 가능 여부 시스템. 각 셀에 N(1)/E(2)/S(4)/W(8) 4비트로 edge 차단 저장.

### Build

집 건축/편집 시 `buildPassability(house)` → HouseData에 포함하여 서버 저장.

- 층별 별도 그리드
- 벽 세그먼트 순회: `variant !== 'open'`이면 해당 셀 edge 비트 set
- 양쪽 셀 모두 비트 set (안쪽 + 바깥쪽 인접 셀)
- 저장 시 정적 구조 기준 (모든 문은 닫힌 상태로 취급)

### Stairwell Passability

1F stairwell을 1F/2F 두 grid에 모두 등록:
- **entry 층**: entry 랜딩만 side wall skip, exit 랜딩 포함 측면+끝 blocked
- **exit 층**: exit 랜딩만 side wall skip, entry 랜딩 포함 측면+끝 blocked
- `stairReversed` 시 물리적 위치 반전 반영

### Runtime

- 집 로드 시 저장된 passability 그리드 사용 (없으면 fallback 계산)
- 문 상태 overlay: `updateDoorEdge`로 해당 edge 비트 O(1) flip
- 창문은 passability 변경 없음 (열려도 통과 불가)

### Movement Check

`isMovementBlocked(fromX, fromZ, toX, toZ, y)`:
1. house AABB fast rejection
2. Y로 해당 floor grid 매칭
3. world → house local → grid cell 좌표 변환
4. X축/Z축 각각 셀 edge 교차 검사
5. `WALL_HALF_THICKNESS(0.3m)` proximity buffer

### Server-Owned Storey & Y

서버는 클라가 보낸 층/Y를 그대로 믿지 않는다 (`validated_house_floor`, `surface_ground_y`):

- 층 변경은 한 번에 한 층, 두 층을 잇는 stairwell footprint(±1 cell, 레그 길이 ≤ 2×run)에 닿는 레그에서만 허용. 그 외는 층 유지 + 스냅(`PositionCorrected`).
- 예외: 들고 있는 층의 바닥이 발밑에 전혀 없으면(방이 편집으로 삭제됨) 0층으로 내려오는 변경은 허용.
- 저장 Y: stairwell 위면 램프 높이(`getStairwellYOffset`와 동일 공식), 층 grid 위면 그 층의 `y_base`, 던전 입구 램프면 램프 높이. 열린 지형에서는 보고 Y 유지 (서버에 다리 데크 모델이 없고, grid 밖에서는 Y를 소비하는 서버 로직도 없음).
- 관리자(trusted)와 공식 NPC(`is_official_npc`)는 층 검증을 건너뜀 (벽 충돌과 동일).

## Texture System

### Texture Catalog

17개 텍스쳐, 각각 PBR(albedo, normal, ORM) 맵 로드:

| 그룹 | 텍스쳐 |
|------|--------|
| Stone | Stone, Brick |
| Marble | Marble |
| Wood | 다수 (판자, 셔터, 통나무 등) |
| Roof | Clay (2종), Grey, Reed |
| Other | Red Brick, Medieval, Sandstone, Plaster & Wood |

### fitSegment Flag

- `true`: UV를 세그먼트 단위로 0→1 클램프 (타일링 없음)
- 프레임 geometry (기둥, 빔, X자 대각선) 활성화
- 코너 필러 활성화
- `ClampToEdgeWrapping` 사용

### Material Pool

- 텍스쳐 인덱스별 1개 `MeshStandardMaterial` 캐시
- WebGPU 제약: 텍스쳐별 개별 material 인스턴스 필요

## Rendering

### Mesh Construction

- 방별 geometry를 집 단위로 merged geometry 생성 (draw call 최소화)
- 텍스쳐×면방향별 merge → 집 1개당 ~4~5 draw call
- 문짝/창문 셔터만 별도 Mesh (애니메이션 필요)

### Front / Back Separation

| Group | 포함 메쉬 | 플레이어 inside 시 |
|-------|----------|-------------------|
| `front` | 남벽, 서벽, 지붕 | Y를 OFFSCREEN_Y로 이동 |
| `back` | 북벽, 동벽, 바닥 | 항상 표시 |
| `interior` | 인접 방과 공유하는 벽 (벽선별 그룹) | 벽이 실제로 플레이어를 가릴 때만 반투명: 카메라 방향 시선은 남/서로 1m 갈 때 1m 오르므로 `0 < 거리 < 벽높이`이고 교차점이 벽 세그먼트 범위(±0.6m) 안일 때 (`interiorWallOccludes`). 반투명 시 나무 기둥·빔·X 브레이스(`decor`)는 숨김 |

멀티패스 렌더링(refraction/reflection) 시에는 모든 벽 visible 유지.

## Network Protocol

### ClientMessage

```rust
ToggleDoor { house_id, room_index, wall_dir, segment_index }
```

문과 창문 모두 동일 메시지 사용 (서버에서 variant 검증).

### ServerMessage

```rust
HouseSpawned { house: HouseData },
HouseUpdated { house: HouseData },
HouseRemoved { house_id: String },
HousesInArea { houses: Vec<HouseData> },  // 청크 진입 시 전송
DoorToggled { house_id, room_index, wall_dir, segment_index, is_open }
```

## Server Storage

- 파일 기반: `data/housing/r{cx}_{cz}/{house_id}.json`
- REST 엔드포인트:
  - `GET /api/housing/area/{cx}/{cz}` — 청크 내 모든 집
  - `POST /api/housing` — 생성 (ID 서버 할당)
  - `PUT /api/housing/{id}` — 수정
  - `DELETE /api/housing/{id}` — 삭제
- 서버 검증: 인접 벽 유효성, 겹침 검사, 소유자 권한, 상층 floor support

## Constants

```
WALL_THICKNESS       = 0.1m
FLOOR_THICKNESS      = 0.1m
DEFAULT_WALL_HEIGHT  = 3.0m
MAX_FLOOR_LEVEL      = 3
LANDING_DEPTH        = 0.5m
ROOF_OVERHANG        = 0.3m
FLOOR_OVERHANG_PER_LEVEL = 0.15m
FRAME_PROTRUSION     = 0.04m
FRAME_DEPTH          = 0.18m  (WALL_THICKNESS + FRAME_PROTRUSION × 2)
FRAME_DIAG_THICKNESS = 0.06m
FRAME_BEAM_FRAC      = 0.05   (벽 높이의 5%)
FRAME_BEAM_Y_FRAC    = 0.40   (바닥에서 40%)
FRAME_SIDE_FRAC      = 0.10   (세그먼트 폭의 10%)
FRAME_BOTTOM_FRAC    = 0.05   (벽 높이의 5%)
DOOR_WIDTH           = 0.8m
DOOR_HEIGHT          = 2.2m
WINDOW_WIDTH         = 0.8m
WINDOW_HEIGHT        = 1.0m
WINDOW_BOTTOM        = 1.2m   (창턱 높이)
ROOF_PITCH_GABLED    = 0.8
ROOF_PITCH_STEEP     = 1.4
GABLE_WIN_W          = 0.6m
GABLE_WIN_H          = 0.7m
DOOR_SWING_SPEED     = π rad/s
WALL_HALF_THICKNESS  = 0.3m   (passability proximity)
```

## File Structure

| Path | Description |
|------|-------------|
| `shared/src/housing.rs` | HouseData, RoomData, WallConfig, PassabilityGrid 등 공유 타입 |
| `client/src/lib/types/housing.ts` | 클라이언트 타입 미러 |
| `client/src/lib/managers/housingManager.ts` | 집 로딩/캐싱, 문 토글, passability 관리 |
| `client/src/lib/managers/housing-queries.ts` | 문/창문 탐색, 방 감지 |
| `client/src/lib/managers/housing-passability.ts` | passability grid build/update |
| `client/src/lib/utils/house-geometry.ts` | 프로시저럴 geometry 생성, merged mesh 조립 |
| `client/src/lib/utils/house-geo-walls.ts` | 벽, 문짝, 창문 셔터, 프레임 geometry |
| `client/src/lib/utils/house-geo-floor.ts` | 바닥, overhang, stairwell hole |
| `client/src/lib/utils/house-geo-roof.ts` | 지붕 (flat, gabled, steep), 박공벽/창문 |
| `client/src/lib/utils/house-geo-stairwell.ts` | 계단 스텝, 랜딩 geometry |
| `client/src/lib/utils/house-geo-utils.ts` | 공유 상수, DoorMeshInfo, bakedGeo, mergedMeshes |
| `client/src/lib/utils/housing-textures.ts` | 텍스쳐 카탈로그, material pool |
| `client/src/lib/components/game-scene/GameSceneHousingLayer.svelte` | 하우징 렌더/애니메이션 레이어 |
| `client/src/lib/components/map-editor/HousingEditorPanel.svelte` | 건축 UI 패널 |
| `client/src/lib/components/map-editor/HousingEditorCursor.svelte` | 건축 에디터 커서 |
| `server/src/game_state/mod.rs` | 문/창문 토글 처리 |
| `server/src/housing/mod.rs` | 하우징 게임 로직 + 검증 |
| `server/src/housing/routes.rs` | REST 엔드포인트 |

## Implementation Phases

### Phase 1: Static House Rendering (MVP) ✅
### Phase 2: Server Integration ✅
### Phase 3: Building UI ✅
### Phase 4: Second Floor + Stairs ✅
### Phase 5: Optimization ✅

Merged geometry per house, draw call 최소화.

### Phase 6: Wall Collision ✅

셀 기반 passability grid로 구현. 이전의 line-segment intersection 방식에서 전환.

### Phase 7: Doors & Windows Interaction ✅

문짝 힌지 애니메이션, 창문 셔터 (좌우 2개, 바깥으로 열림), E키 상호작용, 네트워크 동기화, passability 연동.

### Phase 8: Third Floor+ ✅

`MAX_FLOOR_LEVEL = 3` (4층까지), visibility N층 일반화, floor support 검증, jetty overhang.

### Phase 9: Roof Connection

1. 인접 방의 지붕 교차선(valley line) 계산
2. 작은 방 지붕 끝단을 큰 방 경사면 높이에 맞춰 조정
3. Valley 부분에 이음새 삼각형 메쉬 추가
4. ridge direction이 다른 경우(직각 배치)의 교차선 처리
