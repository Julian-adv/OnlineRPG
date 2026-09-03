# House Building: 설계도 기반 플레이어 건축

> 상태: 설계안, 미구현 (2026-09-04)

플레이어가 설계도(blueprint) 아이템과 재료를 소비해 집을 짓는 흐름을 정의한다.
집의 데이터 모델·렌더링·충돌은 [HOUSING_SYSTEM.md](HOUSING_SYSTEM.md)를 그대로
쓴다. 이 문서는 "누가, 무엇을 내고, 어디에, 어떤 검증을 거쳐" 집이 생기는지만
다룬다.

## 목표

- 플레이어가 에디터 없이 클릭 몇 번으로 집을 가진다.
- 집의 형태는 운영자가 에디터로 만든 것만 허용한다. 검증 대상이 고정 템플릿이라
  서버가 확인할 것은 위치뿐이다.
- 집은 골드·재료 싱크가 된다 ([ECONOMY.md](ECONOMY.md)).
- 5,000명이 각자 집을 가져도 맵과 서버가 버틴다.

## 범위 밖

플레이어의 방 편집·증축, 부지(land deed) 거래, 집 내부 인스턴스, 회전, 가구
포함 설계도, 집 판매·양도. 모두 후속 단계.

## 방식 비교

| 방식 | 장점 | 단점 |
|------|------|------|
| **A. 설계도 + 재료 (채택)** | 에디터가 그대로 콘텐츠 파이프라인. 플레이어용 UI 최소. 검증 범위 작음 | 형태 선택지가 운영자 작업량에 비례 |
| B. 플레이어용 자유 에디터 | 자유도 최대 | UI·검증·그리핑 대응 비용 큼. 흉한 집 양산 |
| C. 부지 구매 + NPC 시공 | 세계관에 맞음 | A 위에 얹는 연출일 뿐, 독립 방식이 아님 |

A로 시작한다. B는 나중에 "내 집만 편집" 권한으로 A 위에 얹을 수 있고, C는
A의 획득 경로(상인 NPC가 설계도를 판다) 안에 이미 있다.

## 플레이 흐름

```text
[운영자] 에디터로 집을 짓는다 ──► /save_blueprint <template_id> (집 안에서)
                                          │ 템플릿 파일 저장
                                          ▼
[플레이어] 상인에게 설계도 구매 ──► 가방에서 설계도 사용
                                          │ 발밑 1m 앞에 풋프린트 미리보기
                                          ▼
                              위치 확정 클릭 ──► 서버 검증 실패 ──► 사유 메시지, 아이템 유지
                                          │ 통과
                                          ▼
                       설계도·재료 소비 ──► HouseSpawned 브로드캐스트 ──► 나무 제거
```

## 데이터

### 템플릿 파일

`data/housing-templates/<template_id>.json`. 서버 소유 파일, git 미추적
(집 파일과 같은 취급). `HouseData`에서 `id`·`owner_id`·`origin`을 뺀 나머지
(`rooms`, `passability`)를 저장한다. 문·창문 `is_open`은 false로 정규화.

```rust
pub struct HouseTemplate {
    pub template_id: String,     // [a-z0-9_]+, 파일명
    pub name: String,            // 표시 이름
    pub rooms: Vec<RoomData>,
    pub passability: Vec<PassabilityGrid>,
    pub cost: BuildCost,         // 저장 시 공식으로 산출, 파일에서 수정 가능
    pub source_house_id: String, // 유래 추적용
}
```

서버는 부팅 시 전부 메모리에 올린다. 템플릿은 수십 개 규모라 비용은 없다.

### 설계도 아이템

`items.csv`에 정의는 `house_blueprint` 하나만 둔다 (category `blueprint`,
`consumable=true`, `stackable=false` — 인스턴스마다 다른 집이라 겹치면 안 됨,
`wool_cape`와 같음). 어느 집인지는 인스턴스가 든다:

```rust
pub struct ItemInstance {
    ...
    #[serde(default)]
    pub template_id: Option<String>,   // house_blueprint 전용
}
```

`cape_color`와 같은 방식이다. 인벤토리 툴팁은 `template_id`로 템플릿 이름·
방 수·층수·재료 비용을 보여준다. `template_id`가 없는 설계도는 사용 시 거부.

정의를 템플릿마다 CSV에 추가하지 않는 이유: 템플릿은 운영 중 명령으로 생기고
CSV는 빌드 산출물이라 배포 없이는 늘릴 수 없다.

### 재료

현재 게임에 건축 재료 아이템이 없다. [GATHERING.md](GATHERING.md)(벌목)는
미구현이다. 따라서 재료는 두 단계로 간다:

- **Phase 1**: 골드만. 설계도 가격에 재료비까지 녹인다.
- **Phase 2**: 벌목 구현 후 `BuildCost.items`를 켠다. 설계도는 싸지고 재료가
  비용의 본체가 된다.

`BuildCost`는 저장 시 템플릿에서 공식으로 뽑는다. 운영자가 파일을 고쳐 덮어쓸
수 있다.

```rust
pub struct BuildCost {
    pub gold: u32,
    pub items: Vec<(String, u32)>,   // (item_def_id, quantity)
}
```

산출 공식(초안, 숫자는 튜닝 대상):

| 요소 | 재료 |
|------|------|
| 벽 세그먼트 1개 (`Solid`/`WithDoor`/`WithWindow`) | 목재 1 |
| 바닥 셀 1m² | 석재 1 |
| 지붕 셀 1m² (최상층 방) | 지붕재 1 (텍스쳐 그룹에 따라 갈대/점토) |
| 계단실 | 목재 4 |
| 층 하나 추가마다 | 위 합계 ×1.2 |

골드는 재료 합계 × 상점가 + 고정 설계비. 판매가는 `basePrice`로 두고 상점의
`P` 지수 적용은 기존 규칙을 따른다.

## 운영자 명령

`/save_blueprint <template_id> [name...]` (trusted 전용, 채팅 명령). 운영자가
서 있는 집을 대상으로 한다. 게임 안에서는 집 id가 보이지 않는다.

1. 운영자 위치가 든 집을 찾는다. 실외면 거부.
2. `rooms`·`passability`를 복사하고 `is_open`을 false로 정규화한다.
3. `BuildCost`를 산출한다.
4. 파일을 쓰고 메모리 카탈로그에 넣는다. 같은 `template_id`면 덮어쓴다.
5. 시스템 메시지로 방 수·층수·비용 요약을 돌려준다.

설계도 지급은 기존 아이템 지급 경로에 `template_id` 인자를 더한다. 상인 판매는
`merchants.csv`에 `house_blueprint:<template_id>` 형태로 적는다.

## 배치 검증 (서버 권위)

REST `POST /api/housing`은 관리자 전용이라 플레이어는 쓸 수 없다. WebSocket
메시지를 추가한다.

```rust
ClientMessage::PlaceHouse { instance_id: u64, origin: Position }
ServerMessage::HousePlaceRejected { reason: String }
```

성공 시 기존 `HouseSpawned`를 그대로 쓴다. 미리보기는 클라이언트가 템플릿
`rooms`로 로컬 렌더하고, 색으로 통과/거부를 표시한다. 클라이언트 판정은 힌트일
뿐이고 서버가 다시 전부 검사한다.

서버 순서:

1. 인스턴스가 `house_blueprint`이고 `template_id`가 카탈로그에 있는지.
2. 플레이어 상태: 사망·거래 중·전투 중 아님, 실외, `PLACEMENT_DISTANCE_M`
   안 (campfire kit의 `outdoor_placement` 재사용).
3. **소유 한도**: 캐릭터당 1채. `owner_id`로 집 인덱스를 조회한다.
4. **재료**: `BuildCost` 전부 보유. 확인 후 마지막에 소비 (검증 실패 시 손실
   없음).
5. `origin`을 1m 그리드에 스냅하고 템플릿 `rooms`를 붙여 `HouseData`를 만든다.
   `origin.y`는 서버 지형 높이에서 정한다. 클라 값을 믿지 않는다.
6. `validate_house` + `validate_house_neighbors` (기존).
7. 에디터에는 없던 규칙:
   - **경사**: 0층 풋프린트 셀의 지형 높이 최대-최소 ≤ 1.0m. 넘으면 "땅이
     고르지 않습니다".
   - **물·강·도로**: 풋프린트가 수면 아래이거나 강·도로 셀과 겹치면 거부.
   - **건축 금지 구역**: `NoSpawnZone`과 같은 형식의 `NoBuildZone`을 zone
     파일에 추가. 마을·던전 입구·공식 NPC 집 주변은 운영자가 에디터로 칠한다.
   - **가구·던전 입구·모닥불 겹침**: 풋프린트 + 2m 마진 안에 배치물이 있으면
     거부.
   - **집 간 거리**: 다른 집과 최소 3m. `validate_house_neighbors`의 겹침 검사를
     마진 포함으로 확장.
   - **주기 제한**: 캐릭터당 1분에 5회 시도. 미리보기 스팸이 아니라 확정 요청
     기준.
8. 설계도 1개 + 재료 소비, `write_house`, `passability_add_house`, 나무 제거,
   브로드캐스트. 기존 `create_house` 경로를 함수로 빼서 REST와 공유한다.

## 소유

`HouseData.owner_id`는 지금 필드만 있고 아무 의미가 없다 (기존 집 파일은
전부 `"ownerId": "local"`, 서버·클라 어디서도 읽지 않음). 이 기능부터 의미를
준다:

| 행위 | 소유자 | 타인 | 운영자 |
|------|--------|------|--------|
| 문·창문 토글 | O | O (Phase 1) | O |
| 문 잠금 | Phase 2 | - | O |
| 가구 배치 | Phase 2 | X | O |
| 철거 | O | X | O |
| 에디터 편집 | X | X | O |

- `owner_id`는 캐릭터 id 문자열. 운영자가 지은 집은 빈 문자열(공용). 기존
  `"local"` 값은 첫 배포 때 빈 문자열로 옮긴다.
- 서버는 `owner_id → house_id` 인덱스를 메모리에 둔다 (부팅 시 전체 집 로드,
  이미 `read_all_houses`가 있다).
- **철거**: `/demolish`를 자기 집 안에서 실행. 환급 없음 (Phase 1). 집 파일 삭제,
  `HouseRemoved` 브로드캐스트. 기존 `delete_house`와 공유.
- **방치 정리**: 소유자 `last_seen_at`이 90일 이전이면 일일 배치에서 철거.
  없으면 5,000명 규모에서 맵이 빈 집으로 찬다. 기준일은 `world.json` 설정값.
  공용 집(빈 `owner_id`)은 대상 아님.

## 성능

- 집 수 증가는 청크당 파일 수와 `HousesInArea` 페이로드로 직결된다. 5,000채면
  청크당 평균 수십 채, 페이로드는 이미 청크 단위라 문제없다. 청크 진입 시
  `read_chunk`가 디스크를 읽는 구조라면 메모리 캐시로 바꾼다 (별도 항목).
- 나무 제거는 지형 타일 쓰기를 동반한다. 집 생성은 드문 이벤트라 그대로 둔다.
- 검증 7번의 지형 높이 조회는 풋프린트 셀 수(≤ 32방 × 36셀)에 비례. 서버
  heightmap 조회 비용은 무시할 수준.

## 프로토콜

- `ClientMessage::PlaceHouse`, `ServerMessage::HousePlaceRejected` 추가.
- `ItemInstance.template_id` 추가 (`serde(default)`, 구 클라 호환).
- 툴팁용으로 `ServerMessage::HouseTemplateCatalog { templates: Vec<TemplateSummary> }`
  를 입장 시 1회 전송. 이름·방 수·층수·비용만. 방 데이터는 사용 시점에
  `HouseTemplateRooms { template_id, rooms }`로 따로 받는다 (미리보기용).
- 프로토콜 버전 상향.

## 구현 단계

1. **템플릿 저장** — `HouseTemplate`, 파일 IO, `/save_blueprint`, 비용 공식.
   테스트: 저장 후 다시 읽어 `rooms` 동일, `is_open` 정규화.
2. **설계도 아이템** — `house_blueprint` 정의, `template_id`, 툴팁, 지급 명령,
   상인 판매.
3. **배치** — `PlaceHouse`, 검증 규칙, `NoBuildZone`(에디터 포함), 미리보기.
   테스트: 경사·물·구역·거리·한도·재료 각 거부 사유.
4. **소유·철거·방치 정리** — 인덱스, `/demolish`, 일일 배치.
5. **재료 연동** — 벌목 구현 후 `BuildCost.items` 활성화.
6. 후속: 회전, 문 잠금, 소유자 가구 배치, 가구 포함 설계도.
