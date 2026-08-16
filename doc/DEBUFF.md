# Debuff System (디버프)

플레이어에게 걸리는 시한 상태이상을 한곳에 모은 문서다. 현재는 식중독과 출혈 두 가지이며, 이후 추가되는 디버프도 여기에 정리한다. 수치는 `data-src/debuffs.csv`에서 관리하고, 정의 로드는 `server/src/debuff_defs.rs`, 서버 로직은 `server/src/game_state/debuff.rs`, 와이어 타입은 `shared/src/debuff.rs`에 있다. 배고픔 자체는 [HUNGER.md](HUNGER.md), HP 회복 규칙은 [COMBAT.md](COMBAT.md)를 참고한다.

## 공통 규칙

- 디버프는 종류별로 하나만 유지한다. 같은 디버프에 다시 걸리면 중첩하지 않고 지속시간을 처음부터 다시 센다.
- 서로 다른 디버프는 동시에 걸릴 수 있으며, 배수는 곱해서 적용한다. 예를 들어 쇠약(×0.75)과 식중독(×0.6)이 겹치면 이동 속도는 0.45다.
- 시간이 지나거나 사망·리스폰하면 사라진다. 접속 종료 시에도 사라지며 DB에 저장하지 않는다.
- 걸림·해제는 전투 로그와 HUD의 디버프 아이콘, 캐릭터 상태 창의 효과 카드로 알린다. 남은 시간은 해당 플레이어에게만 보낸다.
- 확률 판정과 지속·틱 처리는 모두 서버가 한다.
- 공식 NPC와 에이전트 플레이어에는 배고픔과 같은 면제 플래그를 적용한다.

## debuffs.csv

| 열 | 뜻 |
|----|----|
| `id` | 디버프 식별자. 다른 CSV에서 이 값으로 참조한다. |
| `name` | 표시 이름 |
| `chance` | 발동 조건이 성립했을 때 걸릴 확률(%) |
| `durationSecs` | 지속시간(초) |
| `dps` | 1초마다 입는 피해. 비어 있으면 피해 없음 |
| `moveMult` / `attackMult` / `carryMult` | 이동 속도·공격 속도·최대 하중 배수. 비어 있으면 1 |
| `drainMult` | 포만도 활동 소비 배수. 비어 있으면 1 |
| `blocksRegen` | true면 HP 자연 회복이 멈춘다 |

현재 값:

| id | name | chance | durationSecs | dps | moveMult | attackMult | carryMult | drainMult | blocksRegen |
|----|------|--------|------------|-----|----------|------------|-----------|-----------|-------------|
| food_poisoning | Food Poisoning | 70 | 300 | | 0.6 | 0.6 | 0.6 | 4 | true |
| bleed | Bleeding | 35 | 8 | 1 | | | | | true |

아이콘과 문구는 클라이언트의 `debuffPresentation.ts`에 둔다. CSV에는 게임 수치만 적는다 (쉼표 금지, `data/debuffs.json`은 생성물).

확률은 디버프별로 하나만 둔다. 같은 디버프를 여러 출처가 서로 다른 확률로 걸어야 하는 상황이 오면 출처 쪽 CSV에 덮어쓰기 열을 추가한다.

## 출처 연결

- **몬스터 공격**: `monsters.csv`의 `hitDebuff` 열에 디버프 id를 적으면 명중할 때마다 해당 확률로 건다. 놀은 `hitDebuff=bleed`.
- **음식**: `items.csv`의 `useDebuff` 열. 날생선 5종은 `useDebuff=food_poisoning`.

## 식중독 (food_poisoning)

날생선을 먹으면 70% 확률로 5분간 걸린다. 이동 속도·공격 속도·최대 하중 ×0.6, 포만도 활동 소비 4배, HP 자연 회복 정지. HP에 직접 피해는 없다. 구운 생선은 걸리지 않는다. 자세한 배경은 [HUNGER.md](HUNGER.md)의 "날생선과 식중독" 절을 참고한다.

## 출혈 (bleed)

발톱·이빨류 몬스터의 명중 시 35% 확률로 8초간 걸린다. 1초마다 1의 피해를 입어 총 8의 피해이며, 지속 중에는 HP 자연 회복이 멈춘다. 다시 맞아 또 걸리면 지속시간만 8초로 갱신된다.

- 출혈 피해로 사망할 수 있다. 처치자는 없으며 일반 사망과 같이 처리한다.
- 출혈 틱은 `last_combat_at`을 갱신하지 않고 음식 HP 회복(`FoodRegeneration`)도 취소하지 않는다. 출혈 중 음식을 먹어 버티는 것은 의도된 대응이다.
- 붕대처럼 출혈을 즉시 멈추는 아이템은 아직 없다. 추가한다면 `items.csv`에 `curesDebuff` 열을 두는 방식을 검토한다.

## 후보

- 독(poison): 출혈과 같은 dps형. 지속이 길고 dps가 낮은 구성으로 거미·뱀류에 사용.
- 화상(burn): 짧고 dps가 높은 구성.
- 둔화(slow): `moveMult`만 쓰는 구성. 얼음·거미줄류.
- 기절(stun): 행동 불가는 지금 열로 표현할 수 없어 새 열이 필요하다.
