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
d20 굴림 + attack_bonus > target_guard  →  명중
d20 굴림 + attack_bonus ≤ target_guard  →  빗나감
```

- d20 범위: 1~20
- `guard`가 곧 명중 목표값이다.
- 기본 `attack_bonus = level / 2` (내림)
- 플레이어 근접 공격의 명중 보너스는 `level / 2 + STR modifier + weapon enchant + weapon skill bonus`다.
- 현재 `weapon skill bonus`는 명시적으로 매핑된 One-Handed Sword, Dagger, Spear에 적용된다: 레벨 0–4/+0, 5–14/+1, 15–24/+2, 25–30/+3.
- Sword/Dagger는 `slash1`, 사거리 2m, 재공격 1.533초를 사용한다. Spear는 `slash3`, 사거리 3m, 재공격 2.467초를 사용하며 서버가 장착 무기 기준으로 검증한다.
- 유효 Guard는 `기본 Guard + 모든 장착 아이템 Guard + 장착 중인 Shield 스킬 보너스 + 활성 primary armor 스킬 보너스`다. Wooden/Raven Shield와 Leather Armor의 아이템 Guard는 각각 한 번만 더하고, 각 스킬의 레벨 0–4/+0, 5–14/+1, 15–24/+2, 25–30/+3 보너스를 별도 항으로 한 번만 적용한다.
- Shield XP는 서버가 승인·해결한 몬스터 공격에만 발생한다. 방패 장착 상태에서 회피(몬스터 miss)는 10 XP, 피격은 5 XP이며, 사거리·층·소유권·생존·쿨다운 검사를 통과하지 못한 요청은 0 XP다.
- Leather Armor XP는 `armorConstruction: leather`, `equipmentLayer: primary`, `defenseSkill: leather_armor`가 명시된 chest 아이템을 장착하고 서버가 승인한 몬스터 공격에 실제로 맞았을 때만 5 XP다. 빗나간 공격, 다른 가죽 파츠만 장착한 상태, Padded/Mail/Plate/Hybrid chest, 일반 robe, 거절·중복 요청은 훈련되지 않는다.
- 피해 보너스는 기존대로 `STR modifier + weapon enchant`다. 스킬 레벨은 피해를 변경하지 않는다.
- 몬스터는 `attackBonus`가 정의되어 있으면 그 값을 쓰고, 없으면 레벨 기반 기본값을 쓴다.

Guard 판정 뒤에는 물리 경감 vertical slice가 적용된다. 서버가 공격을 `untyped`, `slash`, `pierce`, `blunt` 중 하나로 확정한다. primary Padded construction은 slash 1 / blunt 2, primary Leather construction은 slash 1 / pierce 1 / blunt 1, primary Mail construction은 slash 2 / pierce 1, primary Plate construction은 slash 3 / pierce 3 / blunt 1, primary Hybrid construction은 slash 2 / pierce 2 / blunt 2를 경감한다. 다섯 construction 모두 untyped은 경감하지 않고 Mail은 blunt도 경감하지 않으며, 양수 raw hit은 항상 최소 1 피해를 준다. 통합된 upstream 장비 기준은 `leather_armor` Guard 2, `chain_mail` Guard 5, `breastplate` Guard 7을 유지하면서 typed mitigation을 추가 channel로 적용한다. `padded_battle_robe`는 Guard 0, `brigandine_coat`는 Guard 2의 별도 상점 대안이다. 이 조합의 총 방어력은 playtest 대상이다. Leather Armor 스킬의 Guard band와 landed-hit XP 규칙은 그대로 유지되며 Mail, Plate, Hybrid는 별도 스킬을 만들거나 훈련하지 않는다. 장비의 kind/layer/form과 `armorConstruction`은 데이터·검증·툴팁에 적용되고, mitigation은 장착한 chest의 primary construction에만 적용된다. 다른 부위 파츠는 해당 파츠의 Guard만 제공한다. multi-layer occupancy/coverage, construction별 추가 부담, 마법 간섭, body armor 렌더링은 [ARMOR_SYSTEM.md](ARMOR_SYSTEM.md)의 후속 단계다.

프로토콜 v23에서 primary chest body armor는 인스턴스별 내구도를 가진다.
서버가 승인한 몬스터의 실제 명중만 방어 판정에 사용된 동일한 chest 인스턴스의
내구도를 1 낮춘다. 0이 된 방어구는 장착과 무게는 유지하지만 Guard, 물리 경감,
Leather Armor 활성화를 모두 잃는다. 장착 중 교체된 다른 인스턴스, miss, 사거리·층·
소유권·쿨다운에서 거절된 요청은 닳지 않는다. Cloth kit는 Padded, Leather kit는
Leather, Metal kit는 Mail/Plate, Hybrid kit는 Hybrid chest를 수리한다. 일치하는
Repair Kit는 각각 20 / 30 / 45 / 50 condition을 복구하고 최대치를 넘지 않는다.
잘못된 family의 kit는 소비되지 않고 기존 내구도도 바꾸지 않는다. condition 표시는
75% 초과 Pristine, 50% 초과 Worn, 25% 초과 Damaged, 0% 초과 Critical, 0 Broken이다.
이 단계는 표시용이며 양수 condition의 방어 성능은 동일하고 Broken에서만 꺼진다.
완제품 kit 사용은 스킬 XP를 지급하지 않는다.
전투 중이거나 쓰러진 상태에서는 수리할 수 없으므로 전투 도중 즉시 방어력을
되돌리는 소비품으로 사용되지는 않는다.

프로토콜 v21의 장비 부담은 별도의 이동 규칙이다. 가방 무게는 소지 한도에만
영향을 주고, 장착 무게는 `STR × 15` 대비 비율로 Unburdened / Light /
Medium / Heavy 단계와 3.0 / 2.7 / 2.4 / 2.1 m/s 속도를 결정한다. 서버 이동
budget, 브라우저 예측, 에이전트 이동 pacing이 모두 서버가 보낸 같은 값을
사용한다. 이 규칙은 Guard, 물리 경감, 방어구 스킬 XP를 변경하지 않는다.

### 대미지 롤 (Damage Roll)

명중 시에만 굴린다.

```
대미지 = dice notation 파싱 후 합산
예) "2d6" → d6 두 번 굴려 합산 (2~12)
```

주사위 표기법: `{count}d{sides}` (예: `1d6`, `2d8`, `3d4`)

물리 피해 처리 순서:

```text
raw damage
→ equipped weapon damageType 우선
→ 없으면 monster natural damageType
→ 둘 다 없으면 untyped
→ primary armor construction 경감
→ final damage (양수 raw hit은 최소 1)
```

- 검·단검은 slash, 창은 pierce, 횃불은 blunt다.
- Padded는 slash 1 / blunt 2, Leather는 slash·pierce·blunt 각각 1,
  Mail은 slash 2 / pierce 1, Plate는 slash 3 / pierce 3 / blunt 1,
  Hybrid는 slash·pierce·blunt 각각 2를 경감한다.
- 프로토콜 v20의 `PlayerAttacked`는 `damage_type`을 보낸다.
- `MonsterAttackedPlayer`는 `damage_type`, `raw_damage`,
  `mitigated_damage`, 최종 `damage`를 함께 보내 브라우저와 에이전트가
  서버 판정을 그대로 표시한다.

- 구현: [server/src/game/combat.rs](../server/src/game/combat.rs)

---

## Guard (GUARD)

NetHack의 AC를 반전시킨 방어 수치이자 명중 목표값. **높을수록 방어력이 좋다.**

- 캐릭터: 생성 시 DEX 기반 공식으로 계산 (위 섹션 참고)
- 몬스터: `data-src/monsters.csv`에 정의하고 `data/monsters.json`으로 생성
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
2. 서버가 플레이어/생존/타겟/층/2m 범위를 검증한 뒤 공격자별 1.533초 monotonic cooldown을 원자적으로 claim한다.
3. 서버 인벤토리의 main-hand 아이템 정의를 한 번 해석해 피해 주사위, enchant, `weaponSkill`, 스킬 레벨/명중 보너스를 캡처한다.
4. 서버에서 히트 롤: `roll_attack(player_attack_bonus, monster_guard, weapon_damage)`
5. 명중 시 몬스터 HP 차감 후 결과를 브로드캐스트 (`PlayerAttacked`)
6. HP가 0이 되면 `MonsterDead` 브로드캐스트, 30초 후 제거
7. 매핑된 One-Handed Sword의 정상 처리 결과에 miss 5 / hit 10 / killing blow 20 total 스킬 XP를 지급한다.

잘못된 타겟, 이미 죽은 타겟, 다른 층, 범위 밖, 죽은 공격자, 게임에 없는 플레이어, cooldown 중인 요청은 공격 주사위/피해/스킬 XP를 만들지 않는다. cooldown은 타겟이 아니라 공격자 기준이므로 타겟 교체로 우회할 수 없다.

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

Server → Client (direct):
  PlayerAttackRejected { monster_id, reason }
  SkillsUpdate         { skills }
  SkillXpGained        { skill, xp_amount, total_xp, new_level, leveled_up }
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
