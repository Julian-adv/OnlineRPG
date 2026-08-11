# Agent Client 기획

AI agent가 인간 플레이어와 함께 게임 월드에서 플레이할 수 있도록 하는 agent 전용 클라이언트 시스템.

## 개요

인간 클라이언트는 3D 그래픽과 마우스/키보드 입력을 사용하지만, agent 클라이언트는 텍스트 기반 인터페이스로 게임에 참여한다. Agent가 캐릭터를 조종하며 인간 플레이어와 섞여서 플레이한다.

이 문서는 운영자가 서버 머신에서 돌리는 NPC(Rica, Karl) 운용을 다룬다. 외부 사용자가 자기 머신·자기 구글 계정·자기 LLM 구독으로 에이전트를 돌리는 설계는 [REMOTE_AGENT_CLIENT.md](REMOTE_AGENT_CLIENT.md) 참고.

## 아키텍처

```
LLM Agent        <-->   Agent Client (상주 프로세스)   <-->   Game Server
(의도/전략)              (실행/네비게이션)                      (월드 시뮬레이션)
"대장간으로 가"           A* pathfinding                       좌표 기반 이동
"몬스터 공격"             타겟팅 + 스킬 로직                    전투 처리
"플레이어에게 인사"        채팅 메시지 전송                      메시지 브로드캐스트
```

### 레이어 구분

- **LLM**: 고수준 의사결정 (전략, 대화, 판단)
- **Agent Client**: 저수준 실행 (pathfinding, 상태 관리, 이벤트 수집)
- **Game Server**: 월드 시뮬레이션, 권한 검증

## 프로젝트 구조

모노레포 내 별도 패키지로 구성한다. 서버와 공유하는 타입/프로토콜 정의를 직접 import할 수 있고, 서버 프로토콜 변경 시 동기화가 용이하다.

```
OnlineRPG/
├── client/               # 기존 인간 클라이언트 (Svelte + Threlte)
├── agent-client/         # agent 전용 클라이언트
│   ├── src/
│   │   ├── ws.rs              # WebSocket 통신
│   │   ├── orchestrator.rs    # 다중 NPC 세션 관리
│   │   ├── state.rs           # 월드 상태 관리
│   │   ├── dungeon.rs         # 던전 레이아웃/계단 Y/문 (shared 생성기 사용)
│   │   ├── driver/            # LLM 드라이버 (프롬프트, 행동, 이동, 전투)
│   │   ├── llm_scheduler.rs   # 우선순위 큐 + 동시 호출 제한
│   │   └── watch.rs           # 로컬 관전 패널 (읽기 전용)
│   └── Cargo.toml
├── server/               # 게임 서버
├── shared/               # 서버-클라이언트 공유 타입
├── tools/                # 개발 도구
├── data/                 # 게임 데이터
└── doc/                  # 문서
```

## 통신 프로토콜

### WebSocket + JSON

Agent 클라이언트는 WebSocket으로 서버와 통신하되, binary가 아닌 JSON 텍스트를 사용한다.

- LLM이 직접 읽고 생성할 수 있는 형태
- 디버깅 용이
- Agent 수가 수천 단위가 아닌 이상 성능 문제 없음

기존 인간 클라이언트가 binary 프로토콜을 사용한다면, 서버에 JSON endpoint를 별도로 추가한다.

### 대안: Binary 프로토콜 + 클라이언트 측 텍스트 변환

서버와의 통신은 기존 인간 클라이언트와 동일한 binary 프로토콜을 그대로 사용하고, Agent 클라이언트 내부에서 binary ↔ 텍스트 변환 레이어를 두어 LLM과는 텍스트로 통신하는 방식도 가능하다.

- **서버 수정 불필요**: 기존 binary 프로토콜을 그대로 사용하므로 서버에 JSON endpoint를 별도로 추가할 필요가 없음
- **프로토콜 일관성**: 인간 클라이언트와 Agent 클라이언트가 서버 입장에서 동일하게 취급됨. 서버가 클라이언트 종류를 구분할 필요 없음
- **기존 인프라 재사용**: 로드밸런싱, 인증, rate limiting 등 기존 인프라를 그대로 활용 가능

이 경우 Agent 클라이언트의 구조는 다음과 같다:

```
서버 ←──binary──→ [Agent Client: 변환 레이어] ←──text──→ LLM
```

변환 레이어가 binary 메시지를 LLM이 이해할 수 있는 텍스트(자연어 또는 JSON)로 직렬화하고, LLM의 텍스트 응답을 다시 binary 명령으로 변환하는 역할을 한다. 단, 변환 레이어의 구현 및 유지보수 비용이 추가되며, 프로토콜 변경 시 변환 로직도 함께 업데이트해야 하는 점은 고려해야 한다.

### 대안 2: 기존 인간 클라이언트에 LLM 연결 기능 내장

별도의 Agent 전용 클라이언트를 만들지 않고, 기존 인간용 클라이언트 자체에 LLM 연결 기능을 추가하는 방식이다. 클라이언트가 게임 상태를 텍스트로 요약하여 LLM에 전달하고, LLM의 응답을 클라이언트 내부에서 조작 명령으로 변환하여 실행한다.

```
서버 ←──기존 프로토콜──→ [인간 클라이언트 + LLM 연결 모듈] ←──text──→ LLM
                              ↑ 기존 UI/렌더링 그대로 동작
```

- **기존 클라이언트 코드베이스 재사용**: 게임 상태 파싱, 렌더링, 입력 처리 등 이미 구현된 로직을 그대로 활용. 별도 클라이언트를 밑바닥부터 만들 필요 없음
- **LLM 행동 실시간 관찰 가능**: 기존 UI가 그대로 동작하므로, LLM이 무엇을 보고 어떤 행동을 하는지 화면으로 직접 엿볼 수 있음. 디버깅과 행동 튜닝에 유리
- **인간 ↔ LLM 전환 용이**: 같은 클라이언트에서 인간이 직접 조작하다가 LLM에게 제어를 넘기거나, LLM이 플레이하는 것을 인간이 중간에 개입하는 하이브리드 운용이 가능

단, 기존 클라이언트가 UI/렌더링 등 무거운 의존성을 갖고 있다면 headless 환경에서의 대량 배포에는 적합하지 않을 수 있다. 관찰·디버깅 목적이나 소수의 Agent 운용에 적합한 방식이다.

### 대안 3: MCP 서버를 통한 LLM 브릿지

별도의 "LLM용 중간 서버"를 두는 방식이다. 이 중간 서버가 게임 서버와는 기존 binary 프로토콜로 통신하면서, LLM 측에는 MCP(Model Context Protocol) 서버로서 동작한다. 하나의 중간 서버가 여러 LLM Agent를 동시에 서빙할 수 있다.

```
게임 서버 ←──binary──→ [MCP 브릿지 서버] ←──MCP──→ LLM Agent 1
                            ↕                      LLM Agent 2
                       여러 Agent 동시 관리          LLM Agent N
```

- **LLM 사용자 측 프로세스 불필요**: Agent를 운용하려는 LLM 사용자가 별도 클라이언트 프로세스를 띄울 필요 없이, MCP 프로토콜로 중간 서버에 접속하면 바로 게임에 참여 가능
- **중앙 집중 관리**: 하나의 브릿지 서버가 다수의 LLM Agent 세션을 관리하므로, 모니터링·로깅·rate limiting 등을 한 곳에서 처리 가능
- **표준 프로토콜 활용**: MCP를 사용하므로 다양한 LLM 클라이언트(Claude, GPT 등)가 별도 어댑터 없이 연결 가능

단, 중간 서버가 단일 장애 지점(SPOF)이 될 수 있으며, 게임 서버와 LLM 사이에 홉이 하나 추가되므로 지연이 늘어날 수 있다. 또한 중간 서버 자체의 개발·운영 비용이 발생한다.

### 서버 → Agent Client 메시지 예시

```json
{
  "type": "world_update",
  "description": "당신은 마을 광장에 서 있습니다. 북쪽에 대장간, 동쪽에 여관이 보입니다.",
  "nearby_entities": [
    { "id": "player_42", "name": "용사김", "type": "player", "distance": 5.2, "direction": "북동" },
    { "id": "npc_blacksmith", "name": "대장장이 볼칸", "type": "npc", "distance": 12.0, "direction": "북" }
  ],
  "available_actions": ["move", "talk", "inspect", "use_skill"],
  "position": { "x": 128.5, "y": 0, "z": 64.3 }
}
```

### Agent Client → 서버 메시지 예시

```json
{ "type": "move", "x": 140.2, "y": 0, "z": 58.1 }
{ "type": "chat", "target": "player_42", "message": "안녕하세요!" }
{ "type": "use_skill", "skill": "attack", "target": "goblin_7" }
```

## 텍스트 월드 디스크립션

서버에 "텍스트 MUD 레이어"를 추가한다. 서버가 월드 상태를 알고 있으므로, 서버가 직접 텍스트 디스크립션을 생성하는 것이 일관성 있다.

### 디스크립션 내용

- 주변 환경 묘사 (지형, 건물, 날씨)
- 시야 내 엔티티 (플레이어, NPC, 몬스터)
- 가능한 행동 목록
- 최근 이벤트 로그 (누가 나타남, 누가 말함, 전투 결과 등)

### 예시

```
[환경] 당신은 해변에 서 있습니다. 파도 소리가 들립니다. 서쪽으로 마을이 보입니다.
[발견] 플레이어 '용사김'이 시야에 나타났습니다. (북동쪽, 약 15m)
[전투] 고블린이 당신을 공격했습니다. HP: 85/100
[채팅] 용사김: "파티 하실래요?"
```

## 네비게이션 시스템

LLM이 매 프레임 좌표를 결정하는 것은 비현실적이고 비용이 크다. Agent는 텍스트로 고수준 명령을 내리고, agent client가 pathfinding으로 실행한다.

### 흐름

1. LLM: `{ "action": "move_to", "destination": "대장간" }` 명령
2. Agent Client: "대장간"을 알려진 POI(Point of Interest) 좌표로 변환
3. Agent Client: A* 알고리즘으로 현재 위치 → 대장간 경로 계산
4. Agent Client: 경로를 따라 이동 명령을 서버에 순차 전송
5. Agent Client: 도착 시 LLM에 결과 보고

### Agent Client 담당

- A* pathfinding (서버에서 맵 데이터 수신)
- POI(관심 지점) 이름 → 좌표 변환
- 텍스트 명령 → 게임 액션 변환
- 상태 머신 관리 (이동 중, 전투 중, 대기, 대화 중 등)

### 던전 (`src/dungeon.rs`)

층 이동은 별도 액션이 아니라 **이동에 딸린 결과**다. LLM은 목표 층만 말하고
(`{"type": "move", "depth": 2}`, 0이면 지상 복귀), 진입·하강·문 열기는 mover가 처리한다.
계단 전환은 계단통이라는 기하학적 조건에서만 성립하므로, LLM이 "내려가라"를 따로 쏘는
구조는 shaft 전환 로직과 충돌한다.

레이아웃·통행성·계단 A*는 전부 `shared/src/dungeon`(서버가 쓰는 그 코드)에서 온다.
시작 시 `WorldCache::register_dungeons()`가 레지스트리의 모든 던전을 한 번 생성해 공유
캐시에 등록하면(서버 `init_passability`와 동일), 기존 A*가 지상→최하층까지 그대로 걷는다.
에이전트별로 갖지 않으므로 동시 접속 수와 무관하다.

이 모듈이 직접 갖는 건 두 가지다:

- **Y 모델** — 층 높이와 계단 램프 보간. 서버는 우리가 보낸 Y로 충돌 층을 정하고
  (`get_floor_at_position`) 선언한 층을 그 Y로 검증하므로(`validated_dungeon_floor`,
  허용 오차 2.5m / 층 간격 4m), 지상 Y를 유지한 채 층만 선언하면 즉시 되돌려진다.
  선언 층은 항상 Y에 가장 가까운 층으로 계산해 둘이 어긋나지 않게 한다.
- **문** — 인테리어 문은 기본 닫힘이고 실제로 하강 계단을 막는다(old_crypt 기준 5층 중
  3개 층). 경로가 막히면 mover가 갈 수 있는 가장 가까운 닫힌 문으로 걸어가
  `ToggleDungeonDoor`를 보내고 다시 경로를 찾는다. 서버의 문/소품 상태는
  `DungeonDoorsState`/`DungeonPropsState`로 받아 해당 층 셀을 재계산한다.

에이전트가 던전 층의 몬스터 AI를 위임받으면(`MonsterAssigned`) 그 몬스터의 Y도 지형이 아닌
층 높이로 스냅하고 `path_floor`를 층 인덱스로 세팅한다 — 안 그러면 몬스터 무리가 지표로
끌려 올라온다.

## LLM 연동 방향 (MCP에서 선회)

한때 agent client 위에 MCP 서버를 얹어(`src/mcp.rs`, `list_characters`/`create_character`/
`enter_game`/`get_events`/`say`) 외부 LLM이 툴 호출로 에이전트를 **조종하게** 했다.
2026-03-30에 걷어냈다.

방향이 반대로 뒤집혔기 때문이다. 지금은 에이전트가 스스로 프롬프트를 만들어 백엔드
(claude / codex / openrouter / openai)를 **호출하러 나간다**. `driver/`가 관측을 프롬프트로 바꾸고
`llm_scheduler.rs`가 우선순위와 동시 호출을 관리한다. 외부에서 들어오는 제어 경로가
없으니 MCP 서버 표면도 필요 없다.

관찰 용도만 `watch.rs`의 읽기 전용 관전 패널로 남겼다 — 조종은 못 하고 보기만 한다.

### HTTP 백엔드 (`openai.rs`)

openrouter와 openai는 같은 chat completions 호출부를 쓴다. `openai.rs`의
`OpenAiInvoker`가 엔진이고, `openrouter.rs`는 URL이 고정된 `Endpoint`를 만들어
넘길 뿐이다. 새 게이트웨이를 붙일 때도 `Endpoint` 하나만 만들면 된다.

- API 키: openai 백엔드는 `OPENAI_COMPAT_API_KEY`를 본다. codex가 진짜 OpenAI
  키를 `OPENAI_API_KEY`로 받으므로, 그 키가 남의 엔드포인트로 새 나가지 않게
  변수를 분리했다.
- 이번 턴은 따로 만들어 두고 응답이 돌아온 뒤에야 히스토리에 커밋한다. 실패 시
  되돌리는 방식이면 타임아웃으로 future가 통째로 취소될 때 롤백이 아예 실행되지
  않는다 — user 메시지가 연속으로 남고, 역할 교대를 강제하는 chat template은 이를
  거절한다.
- `reqwest::Client`는 전 인보커가 공유한다(`static HTTP`). 엔드포인트별 설정이
  없으니 같은 엔드포인트를 쓰는 NPC들이 커넥션 풀을 나눠 쓴다.
- `reasoning_effort` 기본값은 `"none"`이다. 같은 라우터에서도 모델마다 수용 여부가
  달라(opencode.ai Zen의 deepseek-v4-pro는 거절, minimax-m3는 허용) `""`로 두면
  필드 자체를 생략한다.

### 호출 타임아웃 (`request_timeout_secs`, 기본 120)

백엔드별 설정이 아니라 스케줄러 설정이다(`max_concurrent` 옆, 루트 레벨). 지키는
대상이 특정 provider가 아니라 **슬롯**이기 때문이다: 응답 없는 엔드포인트든 먹통이
된 CLI 자식 프로세스든, 하나가 `max_concurrent`(기본 2) 중 하나를 영구히 잡으면 두
개만 물려도 모든 NPC가 멈춘다.

`llm_scheduler.rs`의 `TimeoutBackend`가 모든 백엔드를 감싼다. 백엔드는
`build_llm_backend()` 한 곳에서만 만들어지므로 claude / codex / openrouter / openai가
전부 덮인다. 감싸는 순서는 `WatchedBackend`(관전 패널) **안쪽** — 타임아웃이 평범한
에러로 올라와 패널에 `llm-error`로 남는다.

시간이 지나면 안쪽 future를 drop하는 것으로 일이 실제로 멈춘다. reqwest는 요청을
취소하고, stdio 백엔드(`claude.rs` / `codex.rs`)는 `kill_on_drop(true)`로 CLI를 띄우므로
자식 프로세스가 살아남지 않는다. 다만 CLI가 또 띄운 손자 프로세스까지는 죽이지
않는다(프로세스 그룹 kill이 아니다). `0`은 타임아웃 해제 — 디버거로 백엔드를
따라갈 때 쓴다.

## 구현 우선순위

1. **Agent Client 기본 구조** - 기존 binary 프로토콜로 서버에 접속하여 한 명의 PC를 조종하는 클라이언트. 서버 수정 없이 시작
2. **클라이언트 측 텍스트 변환 레이어** - 수신한 binary 게임 상태를 LLM이 읽을 수 있는 텍스트로 변환, LLM 응답을 binary 명령으로 변환
3. **네비게이션 시스템** - A* pathfinding, POI 매핑
4. **LLM 드라이버** - 프롬프트 생성 + 응답 파싱, 백엔드별 어댑터
5. **LLM 통합 테스트** - 실제 LLM으로 게임 플레이 테스트

## NPC 정의와 배포 설정 (source of truth)

NPC가 *누구인지*는 git 추적되는 게임 데이터가 단일 진실 소스다 (2026-06-12 통일):

- **`data-src/npcs.csv`** — 전체 NPC 레지스트리. `id`, `npcName`, `class`(역할 = 프롬프트 템플릿 + 캐릭터 클래스), 선택적 거래 필드(`wishlist`, `wishlistRatePercent`, `salaryPerDay`, `walletCap`, `keepsakes`), 선택적 `loadout`(접속 시마다 부족분을 지급·장착하는 지급 장비 — 스타터 킷 대체, 어느 방향으로도 판매 불가). 스탯은 플레이어와 같은 롤 프로토콜을 쓴다: 레지스트리 NPC는 클래스 휴리스틱으로 만족할 때까지 리롤(`orchestrator.rs::roll_fits_class`), 서버가 별도 수치를 주지 않는다. 서버(`npc_defs.rs`), agent-client(`shop_info.rs`), 웹 클라이언트(`traderDefs.ts`)가 같은 생성물 `data/npcs.json`을 읽는다.
- **`agent-client/data/npcs/{id}/`** — 개체 디렉터리 컨벤션: `instance.txt`(개성), `memory.txt`(런타임 누적, gitignore), `schedule.json`(선택).
- **`agent-client/data/templates/{class}.txt`** — 역할별 행동 규칙.

`agent-client/data/config.toml`(gitignore, 호스트별)은 배포 결정만 담는다: 이 인스턴스에서 어떤 레지스트리 NPC를 띄울지(`[[npcs]] id = "karl"`), 계정/비밀번호, LLM 백엔드 선택, 타이밍 오버라이드. `id`가 있으면 `character_name`/`character_class`/프롬프트·스케줄 경로가 레지스트리와 디렉터리 컨벤션에서 파생되고(`main.rs::resolve_from_registry`), 명시 필드는 오버라이드로 동작한다. `id` 없는 항목은 예전처럼 전부 명시하면 되므로 임시(ad-hoc) NPC도 가능하다. 예시는 `config.toml.example` 참고.

## 추후 과제 (TODO)
- **코드에 박힌 프롬프트 문자열을 외부 텍스트 파일로 분리.** 역할 템플릿(`data/templates/*.txt`)과 달리 일부 프롬프트는 Rust 소스에 하드코딩되어 있어 문구를 다듬을 때마다 재컴파일이 필요하다. 대상:
  - `src/shop_info.rs` — 자동 생성되는 "## Your Shop" / "## Your Personal Trading" 섹션의 고정 문구 (동적 값은 placeholder 치환으로)
  - `src/driver/execute.rs`, `src/state.rs` — `[TradeFailed]`, `[DealFailed]`, `[OpenTrade]`, `[PlayerNearby]`, `[MoveFailed]` 등 합성 agent 이벤트 문구
  - `src/driver/prompt.rs` — `format_event`의 서버 이벤트 표현 문구, "What do you do?" 등 프롬프트 골격
  - 분리 위치는 기존 `data/templates/` 아래가 자연스럽다 (예: `data/templates/sections/`, `data/templates/events/`). 페르소나 튜닝이 코드 빌드 없이 텍스트 편집만으로 가능해지는 것이 목표. 단, 원격 watcher가 템플릿 변경에도 재시작하므로 핫리로드까지는 불필요.

## 낚시

Agent는 인간과 동일한 프로토콜로 낚시한다(`doc/FISHING.md`). 반사 동작(입질
시 자동 후킹, `auto_stance` 파이팅)은 클라이언트가 `src/state.rs`에서
처리하고, LLM은 시작과 중단만 결정한다:

```json
{"type": "fish", "x": 10.0, "z": -5.0}
{"type": "fish"}
{"type": "stop_fishing"}
```

낚싯대를 주 손에 장착해야 한다(`{"type": "use", "item":
"fishing_rod"}`). 결과는 `[Fishing]` 이벤트로 도착하고, 거부(낚싯대 없음,
물이 아님, 너무 멂)는 `[FishingError]`로 도착한다.
