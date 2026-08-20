# NPC & Monster AI Architecture

## 대원칙

**서버는 client와 agent-client를 구분하지 않는다.** WebSocket으로 오가는 프로토콜은 완전히 동일하다. 서버 입장에서 NPC든 PC든 모두 같은 `Player`이고, 같은 `ClientMessage`/`ServerMessage`를 주고받는다.

## WS 스케일링

- 서버는 연결 상한 없음. 연결당 비용: tokio task 1개 (~1-2KB) + broadcast receiver
- Broadcast는 `tokio::broadcast` 사용, 메시지 직렬화 1회 + zero-copy 전달
- **100+ 연결 문제없음**, 10K도 OS 튜닝으로 가능
- 현재 프로토콜은 1 WS = 1 캐릭터 강제 → **NPC당 1 WS 유지**

## 아키텍처: Hybrid Orchestrator

```
Orchestrator Process
  ├── WS Connection 1 (NPC "경비병")  → SharedState_1 → 소유 몬스터
  ├── WS Connection 2 (NPC "상인")    → SharedState_2 → 소유 몬스터
  ├── WS Connection 3 (NPC "의뢰인")  → SharedState_3 → 소유 몬스터
  │
  ├── Monster AI Engine (game loop tick, 각 연결의 소유 몬스터를 독립 구동)
  │     └── 상태머신: Idle → Walk/Run → Attack (chase) → Hit → Flee → Return → Idle
  │     └── 몬스터 소유는 WS 연결(=player) 단위, 서버 전체 상한 공유
  │
  └── LLM Scheduler
        ├── Priority queue (urgent 이벤트 우선)
        ├── max_concurrent: 1~3 동시 LLM 호출
        ├── request_timeout_secs: 호출당 상한 (기본 120, 슬롯 점유 방지)
        └── NPC별 개별 system prompt + 대화 기억
```

### 몬스터 소유 모델

서버가 몬스터 스폰을 결정하고 소유자를 지정한다 (치팅 방지). 클라이언트는 할당받은 몬스터의 AI(이동/공격)만 담당한다.

**서버→클라이언트 스폰 흐름**:
1. 서버가 스폰 규칙에 따라 스폰 필요성 판단 (10초 주기 tick, 플레이어당 한 틱에 한 마리)
2. `ServerMessage::SpawnMonsterRequest { monster_type }` → 소유자 클라이언트에 전달
3. 클라이언트가 반경 내 유효한 위치를 선정 (물/절벽/실내 회피)
4. `ClientMessage::RequestSpawnMonster { monster_type, position, rotation }` → 서버에 전송
5. 서버가 위치를 검증하고 몬스터 생성
6. `ServerMessage::MonsterAssigned { monster }` → 소유자에게 직접 전송
7. `ServerMessage::MonsterSpawned { monster }` → 전체 브로드캐스트
8. 클라이언트는 할당된 몬스터에 대해 `MonsterMove`/`MonsterAttack` 전송

이 규칙은 웹 클라이언트와 agent-client 모두 동일하게 적용.

**소유권 자가 치유**: 핸드오프는 실제 직전 소유자에게 `MonsterRemoved`를 보내고(경합에서 계획이 어긋나도), 그 소유자가 아직 AOI 안이면 `MonsterSpawned`로 관전자 시점을 복원한다. 소유권 변경 2초 유예 이후의 비소유자 `MonsterMove`는 무시 대신 release(`MonsterRemoved`[+`MonsterSpawned`])로 응답해 낡은 brain을 내리게 하며, 클라이언트도 `MonsterMoved.owner_id`가 자신이 아니면 brain을 제거한다.

## 2계층 AI 시스템

| 계층 | 대상 | 방식 | 비용 |
|------|------|------|------|
| Deterministic | 몬스터 (patrol, chase, attack) | 상태머신 | 0원 |
| Deterministic | NPC 전투 (chase, attack loop) | 기존 tick_combat | 0원 |
| LLM | NPC 대화, 고수준 판단 | per-NPC LLM call | $$$ |

## 3계층 프롬프트 시스템

| 계층 | 파일 | 내용 | 갱신 |
|------|------|------|------|
| Template | `templates/guard.txt` | 역할 공통 프롬프트 (경비병의 일반 행동 규칙) | 개발자가 수동 |
| Instance | `instances/karl.txt` | 개체 고유 정보 (이름, 나이, 성격, 말투, 배경) | 개발자가 수동 |
| Memory | `memory/karl.txt` | 게임 내 경험 기억 (만난 사람, 사건, 감정) | LLM이 자동 갱신 |

LLM 호출 시 system prompt = `template + instance + memory` 순서로 결합.
Memory는 LLM 응답에 `memory_update` 필드를 추가하여 자동 갱신 (append 또는 요약 교체).

## LLM Scheduler 규칙

1. Urgent 이벤트 (NPC에게 말 걸기, 공격받음) → 즉시 호출
2. 동시 최대 N개 LLM 호출 (설정 가능)
3. Non-urgent NPC는 라운드로빈
4. 이벤트 없는 NPC는 스킵 (idle 시 1시간 간격)
5. 전투 액션은 LLM 대기 없이 deterministic 실행

## Config 구조

```toml
server = "ws://localhost:10006"

[[npcs]]
account = "npc_guard"
character_name = "경비병 카렌"
template_prompt = "data/prompts/templates/guard.txt"
instance_prompt = "data/prompts/instances/karl.txt"
memory_file = "data/prompts/memory/karl.txt"
llm = "openrouter"

[[npcs]]
account = "npc_merchant"
character_name = "상인 리카"
template_prompt = "data/prompts/templates/merchant.txt"
instance_prompt = "data/prompts/instances/rica.txt"
memory_file = "data/prompts/memory/rica.txt"
llm = "openrouter"

# 몬스터 스폰은 서버가 결정 — 클라이언트 config에서 제거
# 서버 config에서 스폰 규칙 정의 (어떤 몬스터, 몇 마리, 어디에)

[llm_scheduler]
max_concurrent = 2
min_interval_secs = 5
```

## 구현 Phase

### Phase 1: Monster AI Module ✅ 구현 완료

**서버 측:**
- 스폰 규칙 시스템: `world.json`의 `ambientSpawns` 배열로 규칙 정의 (타입, 최대 거리)
- `tick_monster_spawns()`: 10초 주기로 스폰 필요성 판단, `SpawnMonsterRequest`를 클라이언트에 전송
  - 한 틱에 플레이어당 최대 한 건만 요청하고, 타입은 규칙 목록을 섞은 순서로 훑어 고른다.
    던전이 밀집 전투를 맡으므로 필드는 `maxMonstersPerPlayer`로 성기게 유지한다
  - 지상 스폰은 플레이어의 마을 거리가 (몬스터 레벨 − 1) × 70m 이상일 때만
    요청한다 (`min_ambient_town_distance`, [COMBAT.md](COMBAT.md)의 거리 게이트).
    레벨은 `monsters.csv`에서 오므로 규칙마다 따로 적지 않는다
- 클라이언트가 위치를 선정하여 `RequestSpawnMonster`로 응답하면, 서버가 위치 검증 후 생성
- `MonsterAssigned` → 소유자에게 직접 전송, `MonsterSpawned` → 전체 브로드캐스트
- 전투 검증: 서버가 공격 판정(hit/miss, 데미지 roll), 쿨다운 체크, HP 관리, 사망 처리
- 사망 몬스터 30초 후 자동 제거 (`MonsterRemoved`)
- 몬스터 ID 형식: `m{owner_number}_{spawn_count}`

**클라이언트 측 (web client + agent-client 동일 로직):**
- `monsterManager.ts` (TS) / `monster_ai.rs` (Rust) 양쪽 모두 behavior tree 런타임을 사용
- 내부 상태: `idle` → `walk`/`run` → `attack` (chase) → `hit` → `flee` → `return` → `idle` / `dead`
  - **idle**: 1초 간격 체크, 30% 확률로 이동 전환
  - **walk/run**: A* 경로 탐색 + waypoint 추적, 도착 시 50% idle / 50% 새 이동
  - **attack**: `chaseRange` 내 타겟 추적, 500ms 간격 경로 재계산, `attackRange` 도달 시 공격
  - **hit**: ~800ms 스태거 후 → HP 30% 이하면 flee, 아니면 attack 복귀
  - **flee**: 공격자 반대 방향으로 runSpeed 도주, 공격자와의 거리가 안전거리(`safeDist`, 기본 chaseRange+5m) 밖이 될 때까지 계속(경로 소진 시 재계산, `maxDurationMs` 기본 15초 페일세이프). 공격자 위치를 모르면 스폰 지점으로 도주. 네트워크 상태: `run`
  - **return**: 스폰 지점으로 walkSpeed 복귀, 도착(5m 이내) 시 idle. 네트워크 상태: `walk`
  - **dead**: AI 중단, 서버가 30초 후 제거
- **리쉬(leash)**: attack 중 스폰 지점에서 50m 초과 시 → return 전환
- chase 범위 초과 / 타겟 사망·소실 시에도 idle 대신 return (스폰 지점 복귀)
- 이동 거리 2~10, 거리 비례 walk/run 확률 (가까울수록 walk)
- WASM 기반 A* 경로 탐색, 물/절벽 회피
- `SpawnMonsterRequest` 수신 시 반경 내 유효 위치 선정하여 `RequestSpawnMonster` 응답
- `MonsterMove`/`MonsterAttack` 메시지로 서버에 동기화
- 원격 몬스터는 `targetPosition` 기반 보간 이동

### Phase 2: Orchestrator Refactor

- `agent-client/src/orchestrator.rs` 생성
- "connect → auth → enter game" 시퀀스를 `NpcConnection`으로 추출
- `[[npcs]]` 배열 config 지원
- HeightSampler, PassabilityCache 공유
- 단일 NPC config도 하위호환 유지

### Phase 3: LLM Scheduler

- `agent-client/src/llm_scheduler.rs` 생성
- NPC별 3계층 프롬프트 (template + instance + memory)
- Priority queue + concurrency limiter
- 기존 `classify_event()` 활용한 우선순위
- idle polling 시차 분산

### Phase 4: 관전 패널

- `agent-client/src/watch.rs` — 127.0.0.1 읽기 전용 HTTP 패널 (`watch_port`)
- NPC별 맵/이벤트 피드/LLM 턴 관찰. 제어 경로는 두지 않는다

## 핵심 파일

- `shared/src/lib.rs` — 프로토콜 타입 정의 (Monster, MonsterState, 메시지 variants)
- `server/src/game_state/monster.rs` — 서버 몬스터 스폰/소유/위치 동기화
- `server/src/game_state/combat.rs` — 서버 전투 판정 (플레이어↔몬스터)
- `server/src/monster_defs.rs` — 생성된 몬스터 정의 로드 (`data/monsters.json`)
- `server/src/world_config.rs` — 월드 설정 로드 (`data-src/world.json` 스폰 규칙)
- `server/src/connection.rs` — 메시지 라우팅 (RequestSpawnMonster, MonsterMove 등)
- `client/src/lib/managers/monsterManager.ts` — 클라이언트 몬스터 AI FSM (Phase 1 구현체)
- `client/src/lib/network/messageHandlers.ts` — 클라이언트 메시지 핸들러
- `client/src/lib/network/socket.ts` — 클라이언트 네트워크 전송 메서드
- `client/src/lib/types/Monster.ts` — 클라이언트 MonsterData 타입
- `data-src/monsters.csv` — 몬스터 정의 원본 데이터
- `data/monsters.json` — 생성된 몬스터 정의 데이터
- `data-src/world.json` — 월드/스폰 설정
- `agent-client/src/driver.rs` — LLM driver loop, combat tick (스케줄러가 대체 예정)
- `agent-client/src/state.rs` — SharedState 확장 (MonsterBrain 추가 예정)
- `agent-client/src/main.rs` — 세션 라이프사이클 (orchestrator 확장 예정)

## Option 비교 (참고)

### Option 1: NPC 1명당 1 LLM (현재 방식 스케일링)
- N개 agent-client 프로세스 각각 독립 실행
- **장점**: 구현 변경 없음, 완벽한 컨텍스트 격리, 장애 격리
- **단점**: 비용 N배, 중복 world state

### Option 2: 1 LLM이 여러 NPC 동시 조종
- 하나의 프롬프트에 모든 NPC 상태를 넣고 한번에 응답
- **장점**: 비용 효율, NPC 간 협동 가능
- **단점**: 성격 오염, 레이턴시 (전원 블로킹), 컨텍스트 폭발, 단일 장애점

### Option 3: Hybrid Orchestrator (채택)
- 위 아키텍처 참조
