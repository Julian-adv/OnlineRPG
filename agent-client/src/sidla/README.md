# SIDLA — Scenario Interaction Data Link Architecture

An experimental control protocol for LLM-driven agents. Off by default: nothing
here compiles into a default build.

LLM 기반 에이전트를 위한 실험적 제어 프로토콜입니다. 기본값은 비활성이며, 기본
빌드에는 이 디렉터리의 코드가 **컴파일되지 않습니다**.

> **Reference / 참조**
> SIDLA (Scenario Interaction Data Link Architecture for Computing Resource
> Optimization and Deterministic AI Model and AI Agent Control, and Control
> Method Using the Same) — KR 10-2026-0065736, filed 2026-04-11.
> IPC A63F 13/45; CPC A63F 2300/51, G06N 3/0455, H04L 49/9057, H04L 69/26.
> Contributed to this project by the applicant.
> 출원인이 본 프로젝트에 기여한 것입니다.

---

## Why / 배경

**EN.** An LLM asked "what do you do?" in prose answers in prose, and prose can
say anything: an action that does not exist, a target that was never in sight, a
field the situation forbids. `driver/execute.rs` already meets this by dropping
any reply it cannot parse — which trades a hallucinated action for an NPC that
stands still for a turn.

SIDLA narrows the channel instead. World state goes out as packets rather than
sentences, decisions come back as packets, and every packet is checked against
the field matrix its header declares before anything reaches the game.

**KO.** 자연어로 "무엇을 하겠는가"를 물으면 자연어로 답이 돌아오고, 자연어는 무엇이든
말할 수 있습니다. 존재하지 않는 행동, 시야에 없던 대상, 해당 상황에서 금지된 필드까지
말입니다. 현재 `driver/execute.rs`는 파싱할 수 없는 응답을 폐기하는 방식으로 이에
대응하는데, 이는 환각 행동 대신 **한 턴 동안 멈춰 있는 NPC**를 얻는 거래입니다.

SIDLA는 대신 채널 자체를 좁힙니다. 월드 상태는 문장이 아닌 패킷으로 나가고, 의사결정도
패킷으로 돌아오며, 모든 패킷은 게임에 도달하기 전에 자신의 헤더가 선언한 필드 매트릭스에
대해 검증됩니다.

---

## How / 동작 구조

```
SharedState ──encode──► A (PPLI) + B (Track) frame ──► provider
                                                          │
                            C (Engage) / D (Mission) ◄─────┘
                                        │
                                 schema::validate
                                 ├─ admitted ─► decode ─► {thought, actions[]}
                                 └─ rejected ─► fsm::decide ─► decode ─► same
```

**EN.** Three barriers stand between the model and the game:

1. Control fields deserialize from integers only, so a word where an enum
   belongs fails to parse (`packet.rs`).
2. Each header fixes which fields must, may and must not appear; a packet
   carrying a field outside its purpose is rejected whole (`schema.rs`).
3. A rejected packet is answered by a deterministic ladder over the same world,
   so refusing a reply costs behaviour rather than removing it (`fsm.rs`).

What reaches the game is therefore always a packet the schema admits. The
guarantee is structural — it does not depend on the model cooperating, and it
holds for a model that returns nothing intelligible at all.

Determinism follows from the same arrangement: the uplink is a pure function of
the state snapshot and the downlink a pure function of the packets and that
snapshot, so an unchanged world and an unchanged reply produce byte-identical
commands. Where variety is wanted it is added afterwards by `shuffle.rs`, seeded
so a run stays reproducible.

**KO.** 모델과 게임 사이에 세 개의 장벽이 있습니다.

1. 제어 필드는 정수만 역직렬화하므로, 열거형 자리에 단어가 오면 파싱 자체가 실패합니다
   (`packet.rs`).
2. 각 헤더가 필수·선택·금지 필드를 고정하므로, 목적을 벗어난 필드를 실은 패킷은 통째로
   폐기됩니다 (`schema.rs`).
3. 폐기된 패킷은 동일한 월드에 대한 결정론적 판정 계단으로 응답합니다. 즉 응답을
   거부하는 대가는 **행동의 소실이 아니라 행동의 대체**입니다 (`fsm.rs`).

따라서 게임에 도달하는 것은 항상 스키마가 허용한 패킷입니다. 이 보장은 **구조적**입니다.
모델의 협조에 의존하지 않으며, 모델이 전혀 해독 불가능한 응답을 반환하는 경우에도
유지됩니다.

결정론성도 같은 배치에서 따라옵니다. 업링크는 상태 스냅샷의 순수 함수이고, 다운링크는
패킷과 그 스냅샷의 순수 함수이므로, 월드와 응답이 동일하면 바이트 단위로 동일한 명령이
생성됩니다. 다양성이 필요한 경우 `shuffle.rs`가 검증 이후에 시드 기반으로 부여하므로,
실행 전체는 여전히 재현 가능합니다.

---

## Measured / 실측

**EN.** From the test suite, on this codebase. Tokens are estimated at four
characters each, and only ever used to compare two renderings of the same
content.

**KO.** 본 코드베이스의 테스트에서 실측한 값입니다. 토큰은 4문자 = 1토큰으로 근사했으며,
동일 내용의 두 표현을 비교하는 용도로만 사용합니다.

| Measurement / 측정 항목 | Result / 결과 |
| :--- | :--- |
| Determinism / 결정론성 | 1,000 encodings of one world → 1 distinct frame / 동일 월드 1,000회 인코딩 → 프레임 1종 |
| Uplink, 1 entity / 업링크 1개체 | prose 32 tok → compact 25 tok (-22 %) |
| Uplink, 64 entities / 업링크 64개체 | prose 1,229 tok → compact 1,150 tok (-6 %) |
| Downlink / 다운링크 | envelope 66 tok → packet 15 tok (-77 %) |
| Adversarial corpus / 적대적 입력 | 21 malformed replies → 0 reached the game, 0 turns lost / 21종 비정상 응답 → 게임 도달 0건, 턴 소실 0건 |

**EN.** The uplink saving is modest and shrinks as the world fills up:
`format_world_state` is already terse, so re-encoding it as packets buys little.
**The saving is on the downlink**, because the model stops narrating its
reasoning to reach a decision. Note also that the JSON wire is *more* expensive
than prose (2,295 tok at 64 entities); `wire = "compact"` is the setting that
pays, and `json` is for reading logs.

Do not read the -77 % as the specification's +85 % efficiency figure verified —
that figure compares against a full natural-language exchange, and this is one
measurement of one channel on one codebase.

**KO.** 업링크 절감은 작고, 월드가 채워질수록 줄어듭니다. `format_world_state`가 이미
간결하므로 패킷으로 재인코딩해도 얻는 것이 적습니다. **절감은 다운링크에서 발생**합니다.
모델이 결정에 도달하기 위해 추론을 서술하지 않게 되기 때문입니다. 또한 JSON 와이어는
자연어보다 오히려 **비쌉니다** (64개체에서 2,295토큰). 이득이 있는 설정은
`wire = "compact"`이고, `json`은 로그 판독용입니다.

-77 %를 명세서의 +85 % 효율 수치가 검증된 것으로 읽어서는 안 됩니다. 해당 수치는 전체
자연어 교환을 대상으로 하며, 여기 실측은 하나의 코드베이스에서 한 채널을 측정한 값입니다.

---

## Headers / 헤더 규격

After the J-series message families a tactical data link uses.
전술 데이터링크의 J-series 메시지 계열을 차용했습니다.

| Header | Purpose / 목적 | Required / 필수 | Optional / 선택 | Forbidden / 금지 |
| :--- | :--- | :--- | :--- | :--- |
| A (PPLI) | own state and position / 자체 상태·위치 | SUB, STA, LOC | HP | IFF, REL, ACT, TAR, OBJ, MSG |
| B (Track) | observation and identification / 관측·피아식별 | SUB, TAR, IFF | REL | STA, ACT, OBJ, LOC, HP, MSG |
| C (Engage) | action and interaction / 행동·상호작용 | SUB, TAR, ACT | MSG | IFF, STA, REL, OBJ, LOC, HP |
| D (Mission) | objective change / 상위 목표 변경 | SUB, OBJ | TAR | IFF, STA, REL, ACT, LOC, HP, MSG |

**EN.** Splitting position (A) from identification (B) is what keeps the field
sets disjoint: where an entity is, is its own report; how we read it, is ours.

**KO.** 위치(A)와 피아식별(B)을 분리한 것이 필드 집합을 서로소로 유지하는 핵심입니다.
개체가 어디 있는지는 그 개체 자신의 보고이고, 그것을 어떻게 식별하는지는 관측자의 몫입니다.

---

## Dictionary / 데이터 사전

| Field | Values / 값 |
| :--- | :--- |
| `IFF` | 0 Unknown, 1 Friend, 2 Hostile, 3 Neutral |
| `STA` | 0 Idle, 1 Moving, 2 Engaged, 3 Panic, 4 Dead |
| `ACT` | 0 None, 1 Talk, 2 Attack, 3 Gift, 4 Flee |
| `OBJ` | 0 None, 1 Patrol, 2 Search, 3 Defend, 4 Escort, 5 Ambush, 6 Raid, 7 Charge, 8 Exterminate |
| `REL` | -100 - +100 integer |
| `SUB` / `TAR` | entity identifier / 개체 식별자 |
| `LOC` | zone name, or `[x, y, z]` |
| `HP` | 0 - 100 integer (extension / 확장) |
| `MSG` | spoken line, `ACT = 1` only (extension / 확장) |

**EN.** `HP` and `MSG` are domain extensions, not part of the core dictionary: a
game needs to know how hurt a target is, and a talking NPC needs somewhere to
put the line it speaks. Both are confined to one header.

A scenario designer controls behaviour by editing this table, not by editing a
prompt.

**KO.** `HP`와 `MSG`는 핵심 사전이 아닌 도메인 확장입니다. 게임은 대상이 얼마나 다쳤는지
알아야 하고, 말하는 NPC는 대사를 담을 자리가 필요합니다. 둘 다 각각 하나의 헤더에만
허용됩니다.

시나리오 기획자는 프롬프트를 고치는 대신 **이 표를 관리**하여 행동을 통제합니다.

### Identifiers / 식별자

**EN.** `SUB` and `TAR` carry the identifier the engine itself uses — a monster
by its instance id (`monster_slime_00c1`), a character by the name the server
answers to. `encode.rs` fills them from the live world, and `decode.rs` refuses
any identifier that was not in the frame it sent. The specification's worked
example uses illustrative character names; those are not the wire format.

**KO.** `SUB`와 `TAR`에는 엔진이 실제로 사용하는 식별자가 들어갑니다. 몬스터는 인스턴스
id(`monster_slime_00c1`), 캐릭터는 서버가 인식하는 이름입니다. `encode.rs`가 실시간
월드에서 이를 채우고, `decode.rs`는 자신이 보낸 프레임에 없던 식별자를 거부합니다.
명세서의 실시예에 등장하는 캐릭터 이름은 어디까지나 예시이며, 와이어 포맷이 아닙니다.

---

## Enabling it / 활성화 방법

**EN.** Two switches, both of which must be on. The compile-time one exists so a
default build carries no risk from an experiment.

**KO.** 두 개의 스위치가 모두 켜져야 합니다. 컴파일 타임 스위치는 기본 빌드가 실험 기능의
위험을 전혀 지지 않도록 하기 위한 것입니다.

```
cargo run -p agent-client --features sidla
```

```toml
# agent-client/data/config.toml
[sidla]
enabled = true
wire = "compact"     # or "json", easier to read in a log / 로그 가독성 우선 시 "json"
shuffle = false      # vary interchangeable objectives across turns / 등가 목표 변주
shuffle_seed = 1397705793
log_frames = false   # log every frame and rejected reply at debug level
```

**EN.** The table may also go under a `[[npcs]]` entry to opt one agent in.
Without the `sidla` feature the table is ignored, so a config file is safe to
share across both builds.

**KO.** 이 테이블은 `[[npcs]]` 항목 아래에 두어 특정 에이전트 하나만 적용할 수도 있습니다.
`sidla` 피처가 없으면 테이블은 무시되므로, 하나의 설정 파일을 두 빌드에서 공유해도
안전합니다.

---

## Files / 파일 구성

| File | Role / 역할 |
| :--- | :--- |
| `packet.rs` | header taxonomy, integer-only field enums, the `Packet` they assemble into / 헤더 분류, 정수 전용 열거형, 패킷 구조체 |
| `schema.rs` | the field matrix, `validate`, `parse_frame`, the violation taxonomy / 필드 매트릭스, 검증, 위반 분류 |
| `wire.rs` | JSON and compact renderings; frame splitting that tolerates fences and prose / 두 가지 렌더링, 프레임 분해 |
| `encode.rs` | `SharedState` to packets, with no natural-language step / 상태를 패킷으로, 자연어 단계 없음 |
| `decode.rs` | validated packets to the driver's existing action envelope / 검증된 패킷을 기존 액션 봉투로 |
| `fsm.rs` | the deterministic ladder a rejected reply falls back to / 폐기 시 결정론적 폴백 |
| `shuffle.rs` | seeded variation, applied after validation / 검증 이후 시드 기반 변주 |
| `backend.rs` | the `LlmBackend` decorator that runs a turn / 한 턴을 수행하는 데코레이터 |

---

## Integration notes / 통합 메모

**EN.** The layer sits behind `driver::LlmBackend` and emits the same
`{thought, actions[]}` JSON the driver already parses, so `driver/*` is
untouched. It wraps outermost — after `TimeoutBackend` and `WatchedBackend` — so
the spectator panel records the data link frames rather than the envelope they
were translated into. `experimental_layers` in `orchestrator.rs` is the single
switch point.

The uplink replaces the prose world state but keeps the driver's
`=== EVENTS ===` section verbatim. A player's chat line is not a control field
and has no packet form; only the machine-actionable channel is constrained.

**KO.** 이 계층은 `driver::LlmBackend` 뒤에 위치하며 드라이버가 이미 파싱하는
`{thought, actions[]}` JSON을 그대로 생성하므로 `driver/*`는 손대지 않았습니다.
`TimeoutBackend`와 `WatchedBackend` 다음의 **최외곽**에서 감싸므로, 관전 패널에는
변환된 봉투가 아니라 데이터링크 프레임이 기록됩니다. `orchestrator.rs`의
`experimental_layers`가 유일한 스위치 지점입니다.

업링크는 자연어 월드 상태를 대체하지만 드라이버의 `=== EVENTS ===` 구간은 그대로
유지합니다. 플레이어의 채팅 한 줄은 제어 필드가 아니며 패킷 형태가 없습니다. 제약 대상은
기계가 실행하는 채널뿐입니다.

---

## Known limits / 알려진 한계

**EN.**

- `REL` is derived from `IFF` (Friend +50, Hostile -100, otherwise 0). The game
  has no relationship store yet; `encode::rel_from_iff` is the one place to
  change when it does.
- A `D` packet installs an objective, but the game has no standing-objective
  system, so `decode` translates it into a single immediate action. Patrol and
  search both become one waypoint.
- `ACT = Gift` maps to `open_trade`, the nearest thing the game offers.
- The inference side of the referenced architecture — a deterministic latent
  state encoder (DGAE-WL) and a sub-2-bit quantised engine — is not implemented
  here. This client calls external providers, so determinism is enforced at the
  output boundary instead.

**KO.**

- `REL`은 `IFF`에서 파생합니다 (Friend +50, Hostile -100, 그 외 0). 게임에 관계 저장소가
  아직 없기 때문이며, 도입 시 `encode::rel_from_iff` 한 곳만 교체하면 됩니다.
- `D` 패킷은 목표를 설치하지만 게임에 상시 목표 시스템이 없으므로 `decode`가 즉시 행동
  하나로 변환합니다. Patrol과 Search는 모두 하나의 경로점이 됩니다.
- `ACT = Gift`는 게임이 제공하는 가장 가까운 기능인 `open_trade`로 매핑됩니다.
- 참조 아키텍처의 추론 측면 — 결정론적 잠재 상태 인코더(DGAE-WL)와 2 bit 이하 양자화
  엔진 — 은 여기 구현되지 않았습니다. 이 클라이언트는 외부 제공자를 호출하므로, 결정론성은
  대신 **출력 경계에서** 강제됩니다.

---

## Where DGAE-WL would fit / DGAE-WL 접목 지점

**EN.** Not implemented, recorded so the seam is not lost. A deterministic
Gaussian autoencoder trained with a wristband loss maps a state vector to a
fixed latent coordinate — identical input, identical coordinate, no sampling.
Three places in this client could use one:

1. **Turn deduplication.** `encode::encode` already produces a stable frame; the
   latent coordinate of that frame is a cheap cache key. An agent whose world
   has not meaningfully changed could reuse its last decision instead of
   spending a provider call, which is the dominant cost at fleet scale.
2. **Frame compression.** The uplink currently sends per-entity packets. A
   latent coordinate for the local situation would replace the bulk of them,
   holding the context window flat as entity count grows.
3. **Behaviour-tree selection.** `monster_ai.rs` picks a tree by monster type. A
   latent coordinate over the tactical situation is a natural index for choosing
   among trees deterministically, keeping the property that makes trees
   debuggable in the first place.

All three want the encoder trained on recorded frames from this client, which is
a separate piece of work from the protocol.

**KO.** 구현하지 않았으나 접목 지점을 잃지 않도록 기록합니다. Wristband loss로 학습한
결정론적 가우스 오토인코더는 상태 벡터를 고정 잠재 좌표로 매핑합니다. 동일 입력 → 동일
좌표, 샘플링 없음. 이 클라이언트에서 활용 가능한 지점은 세 곳입니다.

1. **턴 중복 제거.** `encode::encode`는 이미 안정적인 프레임을 생성하므로, 그 프레임의
   잠재 좌표는 저렴한 캐시 키가 됩니다. 월드가 유의미하게 바뀌지 않은 에이전트는 제공자
   호출을 소모하는 대신 직전 결정을 재사용할 수 있습니다. 함대 규모에서는 이 호출 비용이
   지배적입니다.
2. **프레임 압축.** 현재 업링크는 개체별 패킷을 보냅니다. 국소 상황에 대한 잠재 좌표가
   그 대부분을 대체하면, 개체 수가 늘어도 컨텍스트 윈도우가 평탄하게 유지됩니다.
3. **행동 트리 선택.** `monster_ai.rs`는 몬스터 타입으로 트리를 고릅니다. 전술 상황에 대한
   잠재 좌표는 트리를 결정론적으로 선택하는 자연스러운 인덱스이며, 트리를 디버깅 가능하게
   만드는 성질을 그대로 유지합니다.

세 가지 모두 이 클라이언트에서 기록한 프레임으로 인코더를 학습시키는 작업을 필요로 하며,
이는 프로토콜과는 별개의 작업입니다.
