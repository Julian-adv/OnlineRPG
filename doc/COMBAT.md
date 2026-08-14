# Combat System

NetHack/D&D 스타일의 스탯 기반 전투 시스템. 모든 전투 계산은 서버에서 처리한다.

## 캐릭터 스탯 (Attributes)

6개의 기본 능력치. 범위는 3~18.

| 스탯 | 약자 | 설명 |
|------|------|------|
| Strength     | STR | 근접 공격력, 장비 제한 |
| Dexterity    | DEX | 명중, 회피, 원거리 공격 |
| Constitution | CON | HP 보너스, 체력 |
| Intelligence | INT | 마법 효과, 스킬 |
| Wisdom       | WIS | 회복력, 저항력 |
| Charisma     | CHA | NPC 반응, 거래 |

### 스탯 생성: 클래스 선택 → 4d6 roll → 클래스 보정 → 72 리밸런싱

1. 클래스를 먼저 선택한다.
2. 각 능력치마다 주사위 4개(d6)를 굴려 가장 낮은 값을 제외한 3개를 합산한다.
3. 클래스별 스탯 보정을 적용한다.
4. 6개 스탯의 합계를 72로 리밸런싱한다. 합계가 72 미만이면 낮은 스탯을 올리고, 초과하면 높은 스탯을 낮춘다. 각 스탯은 3~18 범위를 벗어날 수 없다.

```
예) 3, 5, 2, 4 → 2 제외 → 3+5+4 = 12
```

리밸런싱이 보정 이후에 적용되므로, 총합 72가 항상 보장된다.

- 구현: [server/src/game/character_attributes.rs](../server/src/game/character_attributes.rs)

### 클래스별 스탯 보정 (Class Stat Adjustments)

NetHack/D&D 스타일로, 클래스마다 고유한 능력치 보정을 적용한다. 보정을 먼저 적용한 뒤 72로 리밸런싱하므로, 총합 72가 항상 보장된다.

| 클래스 | STR | DEX | CON | INT | WIS | CHA |
|--------|-----|-----|-----|-----|-----|-----|
| Barbarian (M) | +3 | 0 | +2 | -2 | -2 | -1 |
| Barbarian (F) | +2 | +1 | +1 | -2 | -1 | -1 |
| Caveman (M) | +2 | 0 | +2 | -2 | 0 | -2 |
| Caveman (F) | +1 | +1 | +1 | -2 | +1 | -2 |
| Knight (M) | +1 | -1 | +1 | -1 | 0 | 0 |
| Knight (F) | 0 | 0 | 0 | -1 | +1 | 0 |
| Valkyrie | +2 | +1 | +1 | -1 | -2 | -1 |
| Ranger | +1 | +2 | 0 | -1 | 0 | -2 |
| Samurai | +1 | 0 | +2 | -1 | 0 | -2 |
| Monk | -1 | +2 | 0 | -1 | +2 | -2 |
| Priest | -1 | -1 | +1 | -1 | +3 | -1 |
| Archaeologist | -1 | +1 | 0 | +2 | +1 | -3 |
| Healer | -2 | -1 | +1 | +1 | +2 | -1 |
| Rogue | -1 | +3 | 0 | +1 | -1 | -2 |
| Wizard | -2 | 0 | -1 | +3 | +2 | -2 |
| Tourist | -1 | 0 | -1 | +1 | -1 | +2 |

**히든 클래스 (NPC 전용, 플레이어 선택 불가)**

| 클래스 | STR | DEX | CON | INT | WIS | CHA |
|--------|-----|-----|-----|-----|-----|-----|
| Merchant | -2 | 0 | -1 | +1 | -1 | +3 |
| Guard | +2 | 0 | +2 | -2 | -1 | -1 |

```
예) Barbarian, 롤 후 STR=12 → 12 + 3 = 15
    Wizard, 롤 후 STR=12 → 12 - 2 = 10
```

적용 순서:
1. 4d6 drop lowest로 6개 스탯 생성
2. 클래스 보정 적용
3. 합계 72로 리밸런싱 (3~18 범위 유지)
4. 최종 DEX로 GUARD 계산

### 캐릭터 Guard 계산 (생성 시)

캐릭터를 생성할 때, 최종 `DEX` (클래스 보정 적용 후)로 `GUARD`를 계산해 저장한다.

```
dex_mod = (DEX - 10) / 2
GUARD = clamp(10 + dex_mod, 1, 20)
```

- 현재 구현은 Rust 정수 나눗셈을 사용하므로 0 쪽으로 버림된다.
- 현재 스탯 범위(DEX 3~18) 기준, 실제 캐릭터 GUARD 범위는 대략 7~14다.

예시:

| DEX | dex_mod | GUARD |
|-----|---------|-------|
| 8   | -1      | 9     |
| 10  | 0       | 10    |
| 14  | +2      | 12    |
| 18  | +4      | 14    |

---

## HP 계산

레벨 1 기준: `max_hp = HD_max + con_mod + 종족 보너스`

```
con_mod = (CON - 10) / 2
```

- `con_mod`는 정수 나눗셈을 사용해 0 쪽으로 버림된다.

### 클래스 Hit Die (HD)

| 클래스 | HD |
|--------|----|
| Knight, Barbarian, Caveman, Valkyrie | d10 |
| Ranger, Samurai, Monk, Priest | d8 |
| Archaeologist, Healer, Rogue, Wizard | d6 |
| Tourist | d4 |

### 종족 보너스

| 종족 | 보너스 |
|------|--------|
| Dwarf | +4 |
| Human | +2 |
| Elf, Gnome, Orc | +1 |

**레벨 1 예시:** Human Knight, CON 14  
`HD_max(10) + con_mod(+2) + 종족 보너스(+2) = 14 HP`

---

## HP 재생 (Regeneration)

NetHack과 D&D의 자연 회복 시스템에서 영감을 받은 시간 기반 자동 회복 시스템.

### 회복 주기

- **16초(2 Ticks):** 서버의 기본 게임 시간 틱(8초) 두 번마다 회복이 발생한다.
- 고전적인 "기다림"의 느낌을 주기 위해 리듬은 8초(Clock Sync)를 유지하되 회복 주기는 16초로 설정하였다.

### 회복량 공식

회복량은 **기본 회복량(1)**에 캐릭터의 **레벨(Level)**과 **건강(CON)** 보정치를 더해 결정된다.

```
con_mod = (CON - 10) / 2
regeneration_amount = max(1, 1 + floor(Level / 5) + con_mod)
```

- `con_mod`는 정수 나눗셈을 사용해 0 쪽으로 버림된다.
- 최소 회복량은 **1 HP**로 보장된다.
- **예시 (레벨 6, CON 12 기준):**
    - `1(기본) + 1(레벨 6/5) + 1(CON 12 보정) = 3 HP`

### 회복 조건

- 캐릭터가 **살아있는 상태**(`health > 0`)여야 한다.
- 현재 체력이 **최대 체력보다 낮아야**(`health < max_health`) 한다.
- **비전투 상태:** 마지막 공격 또는 피격으로부터 **10초 이상** 경과해야 한다.
- **허기:** 쇠약(Weak) 또는 식중독 상태면 회복이 멈춘다 ([HUNGER.md](HUNGER.md)).

- 구현: [server/src/game_state/mod.rs](../server/src/game_state/mod.rs) (메서드: `tick_regeneration`)

---

### 레벨업 시 Max HP 증가 (하이브리드 룰)

- 레벨 2부터 적용
- HD를 굴린 뒤 최소 50% 보장, 그 다음 `con_mod`를 더한다

```
roll = dX
min_roll = X / 2
hp_gain = max(roll, min_roll) + con_mod
max_hp += hp_gain
```

**예시 (전사 계열 d10):**  
`roll = 3` → `min_roll = 5` → `hp_gain = 5 + con_mod`

- 구현: [server/src/game/character_hp.rs](../server/src/game/character_hp.rs)

---

## 전투 공식

### 히트 롤 (Hit Roll)

```
굴림 합계 + attack_bonus > target_guard  →  명중
굴림 합계 + attack_bonus ≤ target_guard  →  빗나감
```

- `guard`가 곧 명중 목표값이다.
- 플레이어 `attack_bonus = level / 2`(내림) + STR modifier + 무기 인챈트
- 몬스터 `attack_bonus = level` — 플레이어보다 가파르다. 플레이어 guard는
  레벨이 아니라 장비로 오르기 때문이다(아래 "몬스터 공격보너스와 플레이어
  guard 밸런스"). `attackBonus`를 monsters.csv에 적으면 그 값이 우선하고,
  던전 깊이 스케일링은 그 값에 올라간 레벨만큼을 더한다.

#### 굴림 합계: 폭발 주사위 (Exploding d20)

d20을 굴린다. **20이 나오면 한 번 더 굴려서 더한다.** 그 굴림도 20이면 또
더한다 (최대 5회까지 — 합계 120이면 어떤 guard보다 높으므로 실질 무제한).

```
굴림이 13     → 합계 13
굴림이 20, 10 → 합계 30
굴림이 20, 20, 4 → 합계 44
```

합계 30이 나왔고 공격보너스가 +2라면 32가 되고, 목표 guard가 31 이하면
명중이다.

**왜 이렇게 하나.** "자연 20이면 무조건 명중"으로 두면 아무리 두꺼운 갑옷을
입어도 명중률이 정확히 5%에서 멈춘다. 그 5%는 밸런스 판단이 아니라 주사위
면이 20개라서 생긴 숫자일 뿐이다 — 코볼트가 판금 갑옷 플레이어를 스무 번에
한 번 때리게 된다. 폭발 주사위는 **모자란 만큼 확률이 줄어들되 0은 되지
않게** 한다. 자연 1은 특별 취급하지 않는다.

(폭발이 아예 없으면 guard 22 이상은 공격보너스 +2 이하 몬스터에게 수학적으로
무적이 된다.)

**확률.** 합계가 목표치 `t` 이상 나올 확률은 20점 구간마다 나눠서 본다.

- `t ≤ 20`: 그냥 d20 한 번이므로 `P(t) = (21 − t) / 20`
  (예: `P(15) = 6/20 = 30%` — 15,16,…,20 여섯 눈)
- `t > 20`: 첫 굴림이 반드시 20이어야 하고(1/20) 나머지를 같은 방식으로 다시
  본다 → `P(t) = P(t − 20) / 20`

즉 20점을 넘길 때마다 확률이 1/20로 꺾인다. 예를 들어 합계 30 이상은

```
P(30) = P(10) / 20 = (11/20) / 20 = 11/400 = 2.75%
```

`(21 − 30)/20`처럼 t가 20을 넘은 채로 첫 번째 식에 넣으면 안 된다 — 20을
넘는 순간 두 번째 식으로 넘어간다.

명중률로 정리하면:

| 몬스터 (보너스) | G15 | G21 | G24 | G30 | G40 | G50 |
|---|---:|---:|---:|---:|---:|---:|
| Kobold (+1) | 30% | 5% | 4.25% | 2.75% | 0.25% | 0.14% |
| Orc (+4) | 45% | 15% | 5% | 3.5% | 1% | 0.18% |
| Hobgoblin d10 (+8) | 65% | 35% | 20% | 4.5% | 2% | 0.23% |
| Orc Warlord (+10) | 75% | 45% | 30% | 5% | 2.5% | 0.25% |

guard 40짜리 플레이어를 코볼트가 한 번 맞히려면 평균 400회, 공격 쿨다운
1.9초 기준 13분을 때려야 한다. 무적은 아니되 사실상 무의미한 수준이다.

### 몬스터 공격보너스와 플레이어 guard 밸런스

플레이어 guard는 레벨이 아니라 획득한 방어구로 오른다. 그래서 몬스터
공격보너스는 컨텐츠가 상대할 **장비 단계**를 기준으로 맞춘다.

| 단계 | 장비 | 총 guard |
|------|------|---------:|
| 신규 (방어구 없음) | — | 10~12 |
| 상점 구비 | wooden_shield, leather_pants | 12~14 |
| Old Crypt 졸업 | + leather_helmet, leather_belt | 13~15 |
| Orc Warrens 졸업 | + leather_armor, iron_helmet, iron_boots, leather_gloves, raven_shield | 20~22 |
| 풀 플레이트 + 방패 | | 30~34 |

Orc Warrens 기준 명중률(입장 guard 15 / 졸업 guard 21):

| 깊이 | 몬스터 (레벨) | G15 | G21 |
|------|--------------|----:|----:|
| 1~2 | Kobold (1) | 30% | 5% |
| 3~4 | Goblin (2) | 35% | 5% |
| 5 | Orc (4) | 45% | 15% |
| 6~9 | Hobgoblin (6~7) | 55~60% | 25~30% |
| 10 | Hobgoblin (8) | 65% | 35% |
| 보스 | Orc Warlord (10) | 75% | 45% |

풀 플레이트(guard 30+)는 현재 어떤 몬스터에게도 5% 이하다. 깊이 스케일링은
레벨 20까지 지원하지만(`MAX_DEPTH = 20`) 실제 던전이 5·10층뿐이라 몬스터가
레벨 10에서 멈추기 때문 — 공식이 아니라 층수·신규 던전으로 풀 문제다.

### 대미지 롤 (Damage Roll)

명중 시에만 굴린다.

```
대미지 = dice notation 파싱 후 합산
예) "2d6" → d6 두 번 굴려 합산 (2~12)
```

주사위 표기법: `{count}d{sides}` (예: `1d6`, `2d8`, `3d4`)

- 구현: [server/src/game/combat.rs](../server/src/game/combat.rs)

---

## Guard (GUARD)

NetHack의 AC를 반전시킨 방어 수치이자 명중 목표값. **높을수록 방어력이 좋다.**

- 캐릭터: 생성 시 DEX 기반 공식으로 계산 (위 섹션 참고)
- 몬스터: `data-src/monsters.csv`에 정의하고 `data/monsters.json`으로 생성
- 장비: 착용 아이템의 `guard`와 방어구의 인챈트 +N을 더한다 ([ENCHANT.md](ENCHANT.md))
- 10이 기준점이다.

| GUARD | 의미 |
|-------|------|
| 0~7 | 무방비 / 매우 취약 |
| 8~9 | 약한 방어 |
| 10 | 보통 방어 |
| 11~13 | 단단한 방어 |
| 14+ | 중장갑 이상 |

> NetHack AC와의 대응: `GUARD = 10 − AC`
> (NetHack AC 0 → GUARD 10, AC -5 → GUARD 15)

---

## 몬스터 스탯 정의

몬스터는 [data-src/monsters.csv](../data-src/monsters.csv)에 정의하고, 빌드/개발 도구가 [data/monsters.json](../data/monsters.json)을 생성한다.

| 필드 | 타입 | 설명 |
|------|------|------|
| `health` | u32? | 최대 HP override. 비우면 레벨 기반 기본값 (`level d8` 평균 반올림) |
| `level` | u8 | 몬스터 레벨 (기본 HP/명중/피해/XP 계산에 사용) |
| `guard` | u8 | 명중 목표값. 높을수록 맞히기 어렵고, 10 초과분은 XP 보너스에 영향 |
| `attackBonus` | i32? | 몬스터 명중 보너스 override. 비우면 `level / 2` |
| `damageRoll` | string? | 대미지 주사위 override. 비우면 레벨 기반 기본값 |
| `behavior` | string | 몬스터 행동 트리 이름 (`data-src/behavior_trees.json`, 없으면 `brave` 사용) |
| `attackRange` | f32 | 근접 공격 가능 거리 |
| `chaseRange` | f32 | 플레이어 추적 시작 거리 |
| `attackCooldown` | u32 | 공격 간격 (밀리초) |

**현재 몬스터 예시 (SCP-939):**

```json
{
  "level": 3,
  "guard": 10,
  "behavior": "timid",
  "attackRange": 3,
  "chaseRange": 25,
  "attackCooldown": 4100
}
```

---

## 전투 흐름

### 플레이어 → 몬스터 공격

1. 클라이언트가 `PlayerAttack { monster_id }` 전송
2. 서버에서 히트 롤: `roll_attack(player_attack_bonus, monster_guard, weapon_damage)`
3. 결과를 전체 클라이언트에 브로드캐스트 (`PlayerAttacked`)
4. 명중 시 몬스터 HP 차감
5. HP가 0이 되면 `MonsterDead` 브로드캐스트, 30초 후 제거

### 몬스터 → 플레이어 공격

1. 클라이언트(몬스터 owner)가 `MonsterAttack { monster_id, target_player_id }` 전송
2. 서버에서 히트 롤: `roll_attack(monster_attack_bonus, player_guard, monster_damage)`
3. 결과를 전체 클라이언트에 브로드캐스트 (`MonsterAttackedPlayer`)
4. 명중 시 플레이어 HP 차감
5. HP가 0이 되면 `PlayerDead` 브로드캐스트

### 리스폰

- 클라이언트가 `RequestRespawn` 전송
- 서버에서 HP 0 확인 후 최대 HP로 회복, 원점(0,0,0)으로 이동
- `PlayerRespawned { player }` 브로드캐스트

---

## 경험치 (XP) 시스템

### 몬스터 처치 XP 공식

```
xp = 1 + level²  +  guard_bonus
```

**guard_bonus:**

| GUARD | 보너스 |
|-------|--------|
| 0 ~ 10 | 없음 |
| 11 | +2 |
| 12 | +4 |
| 13 | +6 |
| 10 + i | 2i |

일반 공식: `guard_bonus = max(guard - 10, 0) × 2`

**예시:**

| 몬스터 | level | GUARD | xp |
|--------|-------|-------|----|
| 약한 적 | 1 | 8 | 1 + 1 = **2** |
| 보통 적 | 3 | 10 | 1 + 9 = **10** |
| 강한 적 | 5 | 12 | 1 + 25 + 4 = **30** |
| 보스 | 8 | 13 | 1 + 64 + 6 = **71** |

### 레벨업 필요 XP

모든 레벨에 동일한 공식 적용: `XP(n) = 20 × 2^(n−2)` (n ≥ 2)

| 레벨 | 필요 누적 XP |
|------|-------------|
| 1 | 0 |
| 2 | 20 |
| 3 | 40 |
| 4 | 80 |
| 5 | 160 |
| 6 | 320 |
| 7 | 640 |
| 8 | 1,280 |
| 9 | 2,560 |
| 10 | 5,120 |
| 11 | 10,240 |
| 12 | 20,480 |
| 13 | 40,960 |
| 14 | 81,920 |
| 15 | 163,840 |
| 16 | 327,680 |
| 17 | 655,360 |
| 18 | 1,310,720 |
| 19 | 2,621,440 |
| 20 | 5,242,880 |
| 21 | 10,485,760 |
| 22 | 20,971,520 |
| 23 | 41,943,040 |
| 24 | 83,886,080 |
| 25 | 167,772,160 |
| 26 | 335,544,320 |
| 27 | 671,088,640 |
| 28 | 1,342,177,280 |
| 29 | 2,684,354,560 |
| 30 | 5,368,709,120 |

### 죽음 페널티 (Death Penalty)

사망 시, 현재 레벨 구간 XP의 15%를 차감한다.

```
level_start_xp = XP(L)
next_level_xp = XP(L + 1)
level_band = next_level_xp - level_start_xp
penalty = max(1, floor(level_band * 0.15))
new_xp = max(0, current_xp - penalty)
```

#### 레벨 하락 조건

사망 후 XP가 현재 레벨 시작 XP보다 작아지면 레벨을 1 내린다.

```
if new_xp < XP(L):
  L = max(1, L - 1)   // 1회 사망당 최대 1레벨 하락
```

#### 레벨 하락 시 XP 보정

레벨 하락이 발생하면, 하위 레벨 구간의 최소 30% 진행도는 보장한다.

```
lower_start_xp = XP(L)
lower_next_xp = XP(L + 1)
lower_band = lower_next_xp - lower_start_xp
recovery_floor = lower_start_xp + floor(lower_band * 0.30)
new_xp = max(new_xp, recovery_floor)
```

#### 레벨 하락 시 Max HP 보정

레벨 업/다운 반복에서 통계적 이득이 없도록, **레벨 다운 시 HP 감소량 분포를 레벨 업 증가량 분포와 동일하게** 한다.

```
con_mod = (CON - 10) / 2
hp_delta(HD, CON):
  roll = dHD
  min_roll = HD / 2
  return max(roll, min_roll) + con_mod

hp_loss = hp_delta(HD(class), CON)   // 레벨업과 동일 분포
new_max_hp = max(level1_max_hp, current_max_hp - hp_loss)
current_hp = min(current_hp, new_max_hp)
```

- 레벨이 내려가지 않은 경우에는 `max_hp`를 깎지 않는다.
- 통계적으로 `E(hp_gain) = E(hp_loss)`이므로, 레벨 업/다운 반복의 기대 순이득은 0이다.
- 클래스별 `E(max(roll, HD/2))`는 다음과 같다: d10=6.5, d8=5.25, d6=4.0, d4=2.75.

#### 예외 규칙

- 레벨 1에서는 레벨 하락이 발생하지 않는다.
- 1회 사망으로 연속 레벨 하락(2레벨 이상)은 발생하지 않는다.

---

## 네트워크 메시지

```
Client → Server:
  PlayerAttack { monster_id }
  MonsterAttack { monster_id, target_player_id }
  RequestRespawn

Server → Client (broadcast):
  PlayerAttacked   { player_id, monster_id, hit, roll, damage }
  MonsterAttackedPlayer { monster_id, player_id, hit, roll, damage }
  MonsterDead      { monster_id }
  PlayerDead       { player_id }
  PlayerRespawned  { player }
```

- 구현: [shared/src/lib.rs](../shared/src/lib.rs)

---

## 몬스터 AI 상태

클라이언트가 몬스터 AI를 처리하고, 공격 판정은 서버에 요청한다.

| 상태 | 설명 |
|------|------|
| `idle` | 대기 (30% 확률로 랜덤 이동) |
| `walk` | 이동 중 |
| `run` | 플레이어 추적 중 (chaseRange 이내) |
| `attack` | 공격 중 (attackRange 이내) |
| `hit` | 피격 경직 (~800ms) |
| `dead` | 사망 |

- 구현: [client/src/lib/managers/monsterManager.ts](../client/src/lib/managers/monsterManager.ts)
