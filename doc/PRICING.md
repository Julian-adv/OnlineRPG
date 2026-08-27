# Pricing System (물가)

서버 전체 골드 총량의 증감을 관측해 상인 소비재 가격을 자동 조정하는 시스템. 2026-08-27 초안, 구현 전. [ECONOMY.md](ECONOMY.md)의 화폐·상인 거래 위에 얹는다.

## 목표

- **완만한 인플레이션**을 유지한다. 골드 총량이 조금씩 늘어나는 것이 건강한 상태이며(플레이어가 부유해지는 감각), 폭증(파우셋 폭주)과 감소(사냥 의욕 저하)는 둘 다 막는다.
- 지표는 둘로 나눠 생각한다.
  - **측정 대상**: 활성 캐릭터(직전 30일 내 접속) **1인당 골드** `M = active_gold / active_characters`. 신규 유입으로 총량이 늘어도 1인당이 그대로면 물가는 움직이지 않는다.
  - **조작 대상**: 상인 소비재 구매가에 곱하는 **물가 지수** `P` (기본 1.0).
- 목표치는 서버 설정에 **게임일당 증가율**로 적는다. 조정은 20게임일마다이므로 ×20으로 환산해 쓴다.

## 골드 흐름 정리

| 종류 | 경로 | 비고 |
|------|------|------|
| faucet | 상인에게 판매 (`sellRatePercent`) | 비거주 상인은 골드를 무에서 발행 |
| faucet | 낚시 동전 (`coin_catch`), 던전 상자 | |
| faucet | NPC 급여 | `walletCap`으로 자체 잠김 |
| sink | 상인에게서 구매 | 비거주 상인은 골드를 소멸 |
| 중립 | 플레이어 간 거래, 팁 모자, 거주 NPC(Karl 등) 거래 | 지갑 간 이동만 |

NPC도 서버에서는 일반 캐릭터라 `characters.gold` 행을 가지며, `M`에 모두 포함한다. NPC 지갑에 쌓인 골드(팁 등)는 추후 NPC가 다시 유통시키는 기능으로 돌릴 예정이라 따로 빼지 않고, 스냅샷의 `npc_gold`로 비중만 본다.

## 측정: 시간별 스냅샷

로그 역산은 소비에 로그가 없어 부정확하다. 대신 서버가 **매시 정각** 스냅샷을 DB에 기록한다.

```sql
CREATE TABLE gold_snapshots (
  ts         INTEGER PRIMARY KEY,   -- unix seconds
  total_gold INTEGER NOT NULL,      -- SUM(characters.gold), 전체
  characters INTEGER NOT NULL,      -- 전체 캐릭터 수
  npc_gold   INTEGER NOT NULL,      -- 공식 NPC 지갑 합
  active_gold INTEGER NOT NULL,     -- 활성 캐릭터(30일 내 접속)의 지갑 합
  active_characters INTEGER NOT NULL  -- 활성 캐릭터 수
);
```

- 주기 저장(flush) 직후에 찍어 메모리의 미저장 골드까지 반영한다. `SUM(gold)` 한 번은 동시 접속 5,000명(누적 캐릭터는 그 몇 배)에서도 무시할 비용이다.
- 한 달 이상 접속하지 않은 캐릭터의 골드는 유통 골드가 아니다 — 떠난 유저의 지갑이 `M`을 부풀리고, 복귀하면 갑자기 늘어난 것처럼 보인다. `characters`에 `last_seen_at` 컬럼을 추가(32초 주기 저장 시 갱신, 입장 직후 dirty 표시 — 마지막으로 온라인이었던 시각)하고, 조정 지표는 `active_gold / active_characters`를 쓴다. 활성 기준(30일)은 설정값.
- 스냅샷은 물가 조정과 무관하게 **먼저 배포해 몇 주간 실측**한다. 목표 증가율은 실측치를 보고 정한다.

## 조정: Serin의 삭이 되는 저녁

배포 시점에 맞추면 배포가 없는 날은 조정이 멈추므로, 배포와 분리해 서버가 자동으로 한다. 시점은 실시간 하루(=게임 8일, 뜬금없는 숫자)가 아니라 **작은 달 Serin(주기 20게임일)이 하루 종일 어두운 날의 일몰 후** (`is_serin_dark_day`: 게임일 % 20 == 14, 클라이언트 위상 공식을 서버 `celestial.rs`에 이식해 테스트로 검증) — 실시간 약 2.5일마다. 상인 회의(아래 연출)가 이 시각에 열리고, 회의 종료와 함께 새 가격이 적용된다. 실시간 시계 위를 미끄러지므로 어떤 회의는 낮에, 어떤 회의는 한밤에 열려 플레이어가 우연히 마주친다.

`tick_npc_salaries`의 롤오버 패턴을 그대로 쓴다 (부팅 직후 첫 틱은 기록만, 재시작은 조정 원인이 되지 않음). 마지막 회의 날짜는 `pricing_state`에 저장해 재시작에도 같은 날 두 번 열리지 않는다.

```
growth = (M_now - M_prev) / M_prev        -- M = active_gold / active_characters, 직전 회의 스냅샷 대비
target = targetDailyGrowth × 경과 게임일    -- 서버가 꺼져 회의를 건너뛰면 그만큼 늘어난 목표
P_new  = P * (1 + k * (growth - target))  -- k: 반응 계수 (초기 0.5)
P_new  = clamp(P_new, P * 0.9, P * 1.1)   -- 회의당 최대 ±10%
P_new  = clamp(P_new, P_min, P_max)       -- 전체 범위 (초기 0.9 ~ 2.0)
```

`P`는 정수 퍼센트(`index_percent`, 기본 100)로 저장·전송한다. 첫 회의(직전 측정값 없음)와 활성 캐릭터가 0명인 회의는 측정값만 기록한다.

방향: **골드가 목표보다 많이 늘었으면 가격을 올린다** (싱크 강화), 적게 늘었거나 줄었으면 내린다.

`M`이 작은 초기에는 한 명의 큰 거래로도 가격이 움직이지만, 그것도 재미다("어제 살걸", "다음 삭까지 기다릴까"). 최소 총량 조건 없이 항상 조정하고, 요동은 회의당 스텝 제한이 막는다.

### 설정 ([data-src/world.json](../data-src/world.json))

```json
"pricing": {
  "targetDailyGrowth": 0.002,
  "gain": 0.5,
  "maxStepPerMeeting": 0.1,
  "indexMin": 0.9,
  "indexMax": 2.0,
  "activeDays": 30
}
```

### 영속화

`P`는 `pricing_state` 테이블(단일 행: `index_percent`, `last_meeting_day`, `m_prev`)에 저장하고, 조정마다 `pricing_history(ts, game_day, m_prev, m_now, growth, index_before, index_after)`에 한 줄 남긴다. history는 튜닝 자료이자 아래 연출의 "회의록"이 된다.

## 적용 범위와 불변식

- `P`는 **상인 구매가**, 그중 **소비재**(consumable, 물약·숫돌·음식·주문서 등)에만 곱한다. 장비·염료 같은 내구재는 고정. 거주 NPC(Karl 등)의 재고 판매와 되사기(buyback, 받은 금액 그대로)도 지수 밖이다 — 거주 재고는 유한해서 차익이 캡 된다.
- **판매가(`basePrice × sellRatePercent`)는 `P`를 곱하지 않는다.** 판매가에도 곱하면 싱크와 함께 파우셋이 커져 제어 효과가 상쇄된다.
- 흥정 밴드([ECONOMY.md](ECONOMY.md#원칙-llm이-제안하고-서버가-집행한다))는 `P` 적용 **후** 가격에 계산한다.
- 머니 펌프 불변식을 `P` 포함으로 다시 세운다: `basePrice × P_min × 밴드최저 > basePrice × sellRate × 밴드최고`. sellRate 40%, 밴드 ±25%p면 `P_min × 0.75 > 0.65` → `P_min > 0.87`. **따라서 `indexMin`은 0.9 이상**이어야 한다. 서버가 부팅 시 설정을 검증한다.
- 가격은 정수로 반올림, 최소 1 코퍼.

## 프로토콜

클라이언트는 상점가를 아이템 정의에서 계산하므로 `ShopState`에 `price_index_percent`를 추가했다(프로토콜 v41; 거주 NPC는 항상 100). 서버 권위는 그대로 — 구매 검증은 서버가 `buy_base_price`에서 `P`를 곱해 한다. NPC 클라이언트에는 별도로 `PricingNotice`(지수·직전 변동·추세·다음 회의까지 일수, 프로토콜 v42)를 입장 시와 회의 직후에 보내고, agent-client는 이를 프롬프트의 `## Market` 섹션과 진열대 가격에 반영한다. 조정은 회의 때만이라 열린 거래창에는 다음 열 때 반영되면 충분하다.

## 연출: 상인 회의

가격 변경을 시스템 메시지가 아니라 **마을의 사건**으로 보여준다.

1. **MVP — 대화로 알리기** (구현 완료 2026-08-27): 조정 결과를 `PricingNotice`로 받아 agent-client의 상인 프롬프트에 매 턴 `## Market` 섹션으로 주입한다(`shop_info::market_prompt`). Rica와 Wick이 "삭 회의에서 물약값을 올리기로 했어, 요즘 금이 너무 풀려서" 같은 말을 자기 말투로 한다. 
2. **회의 장면** (구현 완료 2026-08-27): 스케줄 조건 `"at": "meeting"`(shared `schedule.rs` — Serin 어두운 날 && 일몰 후, 서버·agent 공통 판정)로 Rica와 Wick이 Rica 가게 1층에 모인다. 일과 목록의 마지막에 두어 취침 항목보다 우선하며, 서버의 수면 판정도 같은 조건을 써서 회의 중 거래가 "asleep"으로 거부되지 않는다. 자정에 날이 바뀌면 원래 일과로 돌아간다. 회의 중에는 `## Market` 섹션이 "지금 회의 중" 모드로 바뀌어 LLM이 다른 상인과 가격을 논의한다. 플레이어가 엿들으면 새 가격 방향을 먼저 아는 정보 콘텐츠가 되고, 달력·달 위상을 볼 이유가 생긴다.
3. **다음 회의 힌트** (구현 완료 2026-08-27): 서버는 최신 시간별 스냅샷으로 직전 회의 이후 `M`이 목표 대비 어느 쪽으로 가는지 안다(`pricing_notice`의 `trend`: 예상 스텝 ±1% 기준 rising/falling/steady). 이 추세를 상인 프롬프트에 주입하면 "요즘 금이 많이 돌아서 다음 삭엔 값이 오를 것 같네요" 같은 힌트 대사가 나온다. 삭 전에 뒤집힐 수 있으니 확정이 아닌 예감 톤으로 — 그 불확실성 자체가 "지금 살까, 기다릴까" 재미다.
4. **추후**: 상공회의소 건물, 회의록 게시판(`pricing_history` 열람), 플레이어 상인 클래스의 회의 참석(발언은 가능하되 결정은 서버).

## 구현 단계

1. `gold_snapshots` + 매시 기록. 실측만 하고 `P`는 1.0 고정. — 몇 주 관측. — **구현 완료 (2026-08-27)**: `record_gold_snapshot`(auth.rs), `tick_gold_snapshot`(pricing.rs), `pricing.activeDays`(world.json).
2. 설정, 서버 달 위상, 삭 조정 틱, `pricing_state/history`, `ShopState` 필드, 서버 구매 검증. — **구현 완료 (2026-08-27)**: `tick_pricing_meeting`·`next_index_percent`(pricing.rs), 불변식에 `indexMin` 반영(`band_invariant_holds`). `gain`은 history를 보며 튜닝.
3. 연출 1단계(프롬프트 주입) → 2단계(회의 일과) → 3단계(다음 회의 힌트). — **구현 완료 (2026-08-27)**, 프로토콜 v42.
4. 품목군(물약/음식/주문서)별 개별 `P` — 단일 `P`로 시작하고, 운영 상황을 보고 도입한다.
