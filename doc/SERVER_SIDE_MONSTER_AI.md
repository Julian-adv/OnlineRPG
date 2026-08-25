# Server-Side Monster AI (설계안)

상태: **C 구현됨 (2026-08-25)** — `server/src/game_state/monster_ai.rs`, 플래그 `serverMonsterAi`. 동접 20명대라 현 서버 용량으로 충분. 며칠~몇 주 메트릭을 보고 5,000명 감당이 안 되면 A(별도 머신에서 뇌 틱, `brains`+`apply` 경계를 IPC로 분리)를 구현한다. 관련: [NPC_MONSTER_AI.md](NPC_MONSTER_AI.md), [MONSTER_SEPARATION.md](MONSTER_SEPARATION.md), [REPEAT_FARMING.md](REPEAT_FARMING.md), [COMBAT.md](COMBAT.md), [AGENT_CLIENT.md](AGENT_CLIENT.md), [RUNTIME_PERFORMANCE.md](RUNTIME_PERFORMANCE.md)

## 1. 문제

현재 몬스터의 뇌(`shared/src/monster_ai/`)는 **소유자 클라이언트**에서 돈다. 서버는 스폰·소유자 지정·결과 검증만 한다 (`server/src/game_state/monster.rs`, `combat.rs`).

서버가 이미 막는 것:

| 항목 | 검증 위치 |
|---|---|
| 데미지 조작 | `MonsterAttack`에 데미지 없음, 서버가 roll (`combat.rs:707`) |
| 사망 위조 | 클라의 `Dead` 상태 거부 (`monster.rs:469`) |
| 순간이동 | 속도 토큰버킷 `run_speed×1.2`, 상한 12 m (`monster.rs:12-17`) |
| 공중/지하 | 지면 Y 허용오차 0.25 m |
| 원거리 저격 | 사거리 + 도달성 검사 (`combat.rs:779`) |
| 비소유자 조작 | 무시 → 2 s 후 release |

**서버가 못 막는 것 = 의도(intent).** 변조 클라이언트는

1. **뇌를 안 돌린다** → 소유 몬스터가 영원히 가만히 서 있다 → 무위험 파밍 (가장 쉽고 가장 치명적)
2. **속도 예산 안에서 몰이/주차** → 몬스터를 벽에 박아두거나 한 곳에 모아둔다
3. **`target_player_id` 선택** → 특정 유저만 집중 공격(그리핑), 반대로 자신은 절대 안 때리게
4. **도망/복귀 억제** → flee/return 상태를 안 보내 leash를 무시

모두 "정상 범위의 메시지"라 통계적 감지 외엔 방법이 없고, 그 감지 역시 게임 가능하다.

## 2. 목표 / 비목표

**목표**
- 몬스터의 이동·표적·공격 결정을 서버가 내린다. 클라이언트는 관전자.
- 기존 클라이언트(웹·agent-client)와 **프로토콜 호환** — 배포 순서 강제 없음.
- 5,000 동접 예산 안에서 동작 (§7).

**비목표**
- NPC 대화/LLM 계층은 agent-client에 남는다. agent-client는 몬스터 소유만 잃는다.
- 스폰 흐름(REPEAT_FARMING) 불변.
- 플레이어 자신의 치트(스피드핵 등)는 별도 문제.

## 3. 대안 비교

### A. 외부 컨트롤러 프로세스 무리 (사용자 제안 1)

agent-client 비슷한 프로세스 N개를 띄워 모든 몬스터를 소유.

- 프로토콜 벽: 1 WS = 1 `Player`이고 `Player`는 위치와 AOI(32 m)를 가진다. 헤드리스 컨트롤러가 맵 전체 몬스터의 주변 상태를 받으려면 **새 컨트롤러 프로토콜**을 만들어야 한다 — 그 시점에 이미 재설계다.
- 추가로 네트워크 홉(결정 지연 +RTT), 프로세스 수 산정, 장애 시 failover, 컨트롤러 간 몬스터 분배까지 새로 생긴다.
- 얻는 것은 서버 프로세스와 CPU 격리뿐. 다른 방법으로도 가능(§8).

### B. 주변 유저 위임 + 서버 감시견 (사용자 제안 2 = 현행)

지금 모델. 어뷰저 판별은 행동 휴리스틱뿐:
- "사거리 안 표적이 있는데 cooldown×k 동안 공격 없음 → 강제 재할당"
- "idle 상태로 X초 이상 → 재할당"

한계: 재할당 받은 사람도 어뷰저일 수 있고(파티로 몰려다니면 100%), 휴리스틱은 "공격 직전에 한 번씩 헛스윙" 같은 최소 동작으로 우회된다. **임시방편**으로는 며칠 안에 넣을 수 있으니 §9에 병행안으로 둔다.

### C. 서버 프로세스 안에서 뇌를 돌린다 (**권장**)

`shared/src/monster_ai`는 이미 WASM(웹)과 native(agent-client) 양쪽에서 돈다. 서버도 `shared` crate를 링크하고, 필요한 입력을 전부 갖고 있다:

| 뇌의 입력 | agent-client에서 | 서버에서 |
|---|---|---|
| `NearbyPlayer[]` | AOI 스냅샷 | `SpatialCell` 조회 (`game_state/mod.rs:44`) |
| `NearbyMonster[]` (분리 셀) | AOI 스냅샷 | 같은 셀 조회 |
| `PathProvider` | `PassabilityCache` | 동일 타입 (`game_state/passability.rs`) — 집·던전 포함 |
| 행동트리 JSON | 파일 로드 | 동일 |
| hit/death 이벤트 | 서버 메시지 수신 | 전투 코드에서 직접 호출 |

참고 구현: `agent-client/src/monster_ai.rs:211 tick_all` — 서버 드라이버는 이 함수의 복사본에서 시작하면 된다.

## 4. 설계 (C)

### 4.1 구조

```
game_state
  ├── monsters: HashMap<id, Monster>        (기존)
  ├── brains:   HashMap<id, MonsterBrain>   (신규, shared::monster_ai)
  └── behavior_trees                        (신규, 부팅 시 1회 로드)

run_ticks (main.rs:88)
  └── monster_ai tick  — 200 ms, panic-guarded  (신규; player movement 틱과 동일 주기)
```

### 4.2 틱

```
for each spatial cell shard in this tick's slice:
    players  = players_in_cells(cell ± 1)      → NearbyPlayer[]
    monsters = live monsters in cell ± 1       → NearbyMonster[]
    for brain in cell:
        cmds = brain.tick_with_behavior_tree(200ms, players, monsters, tree, passability, rng)
        apply(cmds)
```

`apply`:
- `AiCommand::Move` → `Monster.position/state/target_position` 갱신 후 `MonsterMoved` 브로드캐스트. **속도·지면 검증 생략** (서버 자신의 결정).
- `AiCommand::Attack` → 기존 `MonsterAttack` 핸들러의 **소유권 검사 이후** 부분을 함수로 분리해 호출 (cooldown·사거리 판정·roll은 그대로 재사용, 결과 회귀 방지).

전투 훅: 플레이어가 몬스터를 때리면 `brain.handle_hit_with_behavior_tree`, 사망 시 `brain.handle_death` + brain 제거.

### 4.3 소유권의 의미 변화

레지스트리의 `owner_id`는 **스폰 캡 장부로 그대로 남는다** (`OwnedIds`, handoff, `tick_monster_ownership`의 despawn). 바뀌는 것은 클라이언트에 보이는 면뿐:

| 항목 | 플래그 on |
|---|---|
| 와이어의 `owner_id` (`MonsterSpawned`/`MonsterMoved`) | `wire_monster`/`wire_owner`로 항상 `None` → 어떤 클라도 뇌를 만들지 않음 |
| `MonsterAssigned`, handoff의 release(`MonsterRemoved`+`MonsterSpawned`) | 송신 안 함 (뇌를 관리하는 메시지) |
| 가시성용 `MonsterRemoved` (despawn, AOI 이탈, parked) | 유지 |
| `MonsterMove` 핸들러 | 즉시 return — 토큰버킷/지면 검사는 코드로 남고 실행 안 됨 |
| `MonsterAttack` 핸들러 | 즉시 return. 장부상 소유자라도 표적을 고를 수 없음 |
| `MonsterProvoked` | 소유자에게 보내는 대신 서버 뇌에 `handle_hit(false, 0)` |

플래그 off면 전부 기존 동작. 안정화 후 클라 뇌 경로를 지울 때 이 표의 왼쪽 열을 함께 삭제한다.

### 4.4 클라이언트 호환 — 핵심 카드

웹 클라는 `ensureBrain`에서 `monster.ownerId !== currentPlayer.id`이면 뇌를 만들지 않는다 (`client/src/lib/managers/monsterManager.ts:216`). agent-client도 `MonsterAssigned`를 받아야 brain을 만든다. 따라서 서버가 **배정을 멈추고 직접 움직이기만 하면** 기존 클라이언트는 자동으로 관전자가 된다 — 이미 비소유 몬스터를 `MonsterMoved`로 보간해 그리고 있다.

→ **프로토콜 버전 범프 없음, 서버 단독 배포 가능**, 클라의 뇌 코드 삭제는 나중에.

`Monster.owner_id`와 `MonsterMoved.owner_id`는 이미 `Option<PlayerId>` (`shared/src/entity.rs:199`, `messages.rs:950`)이므로 `None` 송신에 직렬화 변경이 없다. 구현 전 확인 항목: 웹 클라가 `ownerId: null`인 몬스터를 보간 경로로 그리는지 (dev에서 눈으로 1회 확인).

### 4.5 설정 플래그

`data-src/world.json`의 `"serverMonsterAi": true|false` (컴파일 시 포함 — 바꾸면 재빌드; 키가 없으면 true). false면 현행 그대로 → 문제 시 즉시 롤백. 테스트에서는 기본 off, `enable_server_monster_ai()`로 켠다. 안정화 후 플래그와 클라 소유 경로를 제거.

## 5. 롤아웃

1. 서버: `brains` + 틱 + `apply` + 전투 훅, 플래그 off. 기존 테스트 통과.
2. 서버 테스트: 스폰→추격→공격→사망 시나리오를 클라 없이 통과 (`spawn_soak_tests` 패턴).
3. dev에서 플래그 on, 웹 클라·agent-client 각각 미배포 상태로 접속해 관전 동작 확인.
4. prod 플래그 on (서버만 배포).
5. 클라 `monster_ai` WASM 바인딩·`monsterManager` 뇌 코드, agent-client `monster_ai.rs` 삭제. `MonsterMove`/`MonsterAssigned` 메시지 삭제는 프로토콜 범프와 함께.

## 6. 성능 (5,000 동접)

**상한**: 5,000 × 8 = **40,000 마리**. 실제는 훨씬 적다 — 스폰은 이동 거리 비례이고 AOI를 벗어나면 despawn되므로, 활성 플레이어당 평균 2-4마리로 보면 10-20k.

**틱당 비용** (native, 200 ms 주기 = 5 Hz):
- BT 평가 + 이동 적분: 수 µs. 40k × 5 µs = 200 ms/s = **코어 0.2개**.
- A\* 재탐색: 비싼 연산. 뇌 내부 `PATH_RECALC_MS = 500`이고 추격 중일 때만. 추격 비율 20% 가정 → 40k × 0.2 × 2/s = 16k 탐색/s. 탐색 1회 100 µs-1 ms면 **코어 1.6-16개** — 여기가 병목.

**완화**
- 재탐색은 `target_move_threshold(3 m)`로 이미 게이트됨 → 표적이 서 있으면 0.
- `MAX_SLOT_PATH_TRIES=6`, `DETOUR_MAX_NODES=300` 등 예산은 뇌에 이미 있음.
- 틱 예산: 매 틱 셀 단위로 돌리다 예산(초기값 40 ms) 소진 시 중단, 다음 틱에 이어서. 뇌마다 마지막 틱 시각을 기억해 delta는 실제 경과 시간. 부하가 낮으면 매 틱 전부 돌고, 폭주 시에만 자동으로 늦춰진다. 셀 순서는 플레이어 시야 안 셀 우선 → 늦어지는 건 화면 밖 몬스터.
- 같은 셀의 뇌는 항상 같은 틱에 돌린다(분리 셀이 이웃의 최신 포즈를 봐야 함). 뇌 틱 간격은 500 ms 이하 유지 — `PATH_RECALC_MS`·`NETWORK_SYNC_INTERVAL_MS`(500)를 넘으면 타이머가 "매 틱"으로 퇴화.
- 고정 샤드 수 K(틱마다 1/K 그룹만)는 위의 동적 예산으로 대체. K=2(400 ms)까지는 체감 없지만 그 이상은 공격 타이밍이 어긋난다.
- 메트릭: 틱 시간 p50/p99, brain 수, 탐색 횟수/s를 `RUNTIME_PERFORMANCE.md` 항목에 추가.

### 6.1 A\* 부하 줄이기

위 16k 탐색/s는 최악 가정. 순서대로:

1. **측정** — 메트릭 먼저, 아래는 수치가 나온 뒤 필요한 만큼만.
2. **표적별 경로 공유** — 같은 플레이어를 쫓는 몬스터들은 표적이 같다. 표적당 역방향 A\*(또는 chase range 25 m 안의 flow field)를 500 ms에 1회 계산하고 추격자 전원이 읽는다. 비용이 몬스터 수가 아니라 *추격당하는 플레이어 수*에 비례. 슬롯 후보 탐색(`MAX_SLOT_PATH_TRIES`)도 필드에서 거리만 읽으면 된다.
3. **부분 재탐색** — 표적이 움직여도 경로 꼬리만 갱신.
4. **노드 상한** — 추격 경로에도 `DETOUR_MAX_NODES`류 상한, 초과 시 partial path 진행.

**클라이언트에 A\* 위임 (마지막 카드)**: 경로 *검증*은 O(경로 길이)로 싸므로 불가능한 경로는 걸러진다. 그러나 요청 상대가 표적 자신(=어뷰저)이 되고, "경로 없음"·무응답·과도한 우회는 각각 직선 통과 검사, 타임아웃, `path_len ≤ k×직선거리` 상한으로 막되 결국 서버 A\* fallback이 필요하다. 즉 정직한 클라가 많을 때의 최적화일 뿐 어뷰저가 있는 셀에선 서버가 계산한다. RTT만큼 추격 반응도 늦는다. 1-4로 부족할 때만 검증+fallback 형태로 추가.

**대역폭**: 변화 없음. `MonsterMoved`는 지금도 서버가 AOI 브로드캐스트하고, 뇌의 `NETWORK_SYNC_INTERVAL_MS = 500`이 그대로 적용된다. 업링크(`MonsterMove` 수신)는 오히려 사라진다.

**메모리**: `MonsterBrain` ~ 수백 B + 경로 벡터. 40k × 1 KB = 40 MB. 무시 가능.

## 7. 락/동시성

현재 틱 루프들은 각각 `game_state` 락을 잡는다. AI 틱이 40k 뇌를 한 락 안에서 돌리면 200 ms 주기의 플레이어 이동 틱이 밀린다. 셀 샤드 단위로 락을 짧게 잡고 놓는다: 샤드 스냅샷(players/monsters 복사) → 락 해제 → 뇌 틱 → 락 재획득 → apply. 뇌는 `game_state` 밖의 별도 맵(`Mutex<HashMap>`)에 두면 스냅샷 단계 외엔 `game_state` 락이 필요 없다.

## 8. 리스크

| 리스크 | 대응 |
|---|---|
| 서버 CPU 회귀 | 플래그, 메트릭, 틱 예산. 최악엔 A 방식(외부 프로세스)으로 뇌 틱만 옮기는 것도 이 설계 위에서 가능 — `brains` 맵과 `apply`만 IPC로 분리 |
| 행동 드리프트 | 뇌 코드가 동일하므로 최소. 60 Hz→5 Hz 틱 차이는 agent-client가 이미 낮은 주기로 검증함 |
| 멀티 floor(집/던전) | `path_floor`는 `Monster.floor_level`에서 계산, agent-client와 동일 |
| `MONSTER_SEPARATION.md`의 "서버 변경 없음" 전제 무효 | 해당 문서 갱신 |
| agent-client NPC가 자기 몬스터를 못 보게 되는 것 | 없음 — NPC는 이제 그냥 플레이어이고 서버 뇌가 NPC도 `NearbyPlayer`로 본다 |

## 9. 병행 임시방편 (B)

C 구현 기간 동안 최소 비용 감시견을 넣을 수 있다:
- `Monster.last_client_tick_at`: `MonsterMove` 수신마다 갱신. 소유자가 AOI 안에 있는데 5 s 이상 수신 없음 → 다른 플레이어에게 재할당, 없으면 despawn.
- 사거리 안 표적 존재 + cooldown×3 동안 `MonsterAttack` 없음 → 같은 처리.
- 재할당 횟수를 계정별로 카운트해 로그 (어뷰저 후보 목록).

C가 들어가면 통째로 삭제되므로 단순하게 유지한다.

## 10. 구현 메모

- `tick_monster_ai` (200 ms): 로스터를 셀별 스냅샷 → 레지스트리 1회 순회로 (a) 죽은/사라진 몬스터의 뇌 제거 (b) AOI 안에 플레이어가 있는 살아있는 몬스터만 `active` (뇌가 없으면 생성) (c) 정지 몬스터 셀 맵. 그 다음 `passability_read()` 잡고 CPU 구간: 뇌마다 delta = 실제 경과(상한 1 s), 40 ms 예산 초과 시 커서를 남겨 다음 틱에 이어서. 락을 다 놓은 뒤 커맨드 적용.
- `Move` 적용: `expected_monster_move_y`로 지면 Y(지형/던전) → `set_position` → `fanout_monster_position_update` (AOI 진입/이탈 처리 재사용). 뇌의 Y도 갱신.
- `Attack` 적용: `monster_attack(None, ..)` — 기존 클라 경로와 같은 쿨다운·사거리·벽 검사.
- 훅: `broadcast_player_attack`의 명중/빗나감 모두 `brain_hit` (빗나감도 어그로), 사망 시 `brain_death`.
- 메트릭: 30 s마다 `info!("monster ai: brains .. active .. ticked/tick .. pathfinds/s .. commands/s .. over_budget .. worst ms")` → journald에서 `grep "monster ai:"`.
- 미해결/후속: 레지스트리 전체 순회가 틱마다 O(전체 몬스터). 지금 규모엔 무관, 40k에서 문제 되면 플레이어 셀 주변만 순회하도록. §6.1의 표적별 경로 공유는 메트릭 보고 결정.

## 11. 결정 (2026-08-25)

- C 진행. B(감시견)는 하지 않음.
- 틱 200 ms, 틱 예산 40 ms.
- 성능 측정 후 부족하면 A를 별도 서버 머신에서. `brains` 맵과 `apply`를 처음부터 모듈 경계로 분리해 둔다.
- 클라 뇌 코드 삭제는 다음 프로토콜 범프에 묶는다.
