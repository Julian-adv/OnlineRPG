# Item Tiers & Dungeon Drop Design

던전 티어별 방어구 세트 파밍 설계 (2026-07-30 논의). **기존 아이템·기존 던전 범위의 재배치와 독립 롤 드랍, 신규 슬롯 3종(hands·back·shirt, 프로토콜 v12), leather_gloves·leather_boots·iron_helmet·iron_gauntlets 구현됨 (2026-07-31)** — 나머지 신규 아이템·신규 던전·월드 드랍 확장은 미구현.

핵심 컨셉: **세트는 인접한 두 티어에 걸쳐 드랍된다.** 티어 N 던전에서 일부 파츠를, 티어 N+1 던전에서 나머지(몸통 등 핵심 파츠)를 모아 완성한다. 완성한 세트가 그다음 티어에 도전할 체급이 된다.

## 티어 로드맵

| 티어 | 던전 | 드랍 | 상태 |
|------|------|------|------|
| 1 | Old Crypt | 가죽 세트 일부 (투구·바지·벨트) | 티어 조정 필요 |
| 2 | Orc Warrens | 가죽 세트 완성 (몸통·장갑·부츠) + 체인 세트 일부 (철 부츠·철 투구) | 티어 조정 필요 |
| 3 | (신규 던전) | 체인 세트 완성 (체인 메일·건틀릿) + 판금 세트 일부 (부츠·그리브) + 기본 망토 | 던전 미구현 |
| 4 | (신규 던전) | 판금 세트 완성 (흉갑·투구·건틀릿) | 던전 미구현 |
| 5 | (신규 던전) | ring_of_protection | 던전 미구현 |

특수 망토(투명·보호 등)·셔츠·amulet_of_life_saving·ring_of_regeneration은 던전 풀에 넣지 않는다 — **월드 드랍 전용 희소템**으로 유저 간 거래의 축을 만든다 (아래 참조).

재배치 적용됨: leather_armor 1→2, chain_mail 2→3 (티어 3 던전이 생기기 전까지 미드랍), iron_boots는 티어 2 유지. 무기(iron_sword 등)는 chestTier 미지정으로 체스트에서 빠졌다 — 몬스터 무기 드랍·상점 경로로만.

## 세트 구성

### 티어 1–2: 가죽 세트

| 슬롯 | 아이템 | guard | 티어 | 상태 |
|------|--------|-------|------|------|
| head | leather_helmet | 1 | 1 | 있음 |
| pants | leather_pants | 1 | 1 | 있음 |
| belt | leather_belt | 0 | 1 | 있음 (아이콘은 sword.png placeholder) |
| chest | leather_armor | 2 | 2 | 있음 |
| hands | leather_gloves | 1 | 2 | 있음 |
| boots | leather_boots | 1 | 2 | 있음 |

세트 guard 합계 ≈ 6.

### 티어 2–3: 체인 세트

| 슬롯 | 아이템 | guard | 티어 | 상태 |
|------|--------|-------|------|------|
| boots | iron_boots | 2 | 2 | 있음 |
| head | iron_helmet | 2 | 2 | 있음 |
| chest | chain_mail | 5 | 3 | 있음 (티어 3 던전 생기기 전까지 미드랍) |
| hands | iron_gauntlets | 2 | 3 | 있음 (티어 3 던전 생기기 전까지 미드랍) |

세트 guard 합계 ≈ 9~11.

### 티어 3–4: 판금 세트

| 슬롯 | 아이템 | guard | 티어 | 상태 |
|------|--------|-------|------|------|
| pants | plate_greaves | 3 | 3 | **신규 애셋 필요** |
| boots | plate_boots | 2 | 3 | **신규 애셋 필요** |
| chest | breastplate | 7 | 4 | 있음 |
| head | plate_helmet | 3 | 4 | **신규 애셋 필요** |
| hands | plate_gauntlets | 2 | 4 | **신규 애셋 필요** |

세트 guard 합계 ≈ 15~17. 신규 파츠의 guard 값은 구현 시 확정.

### 장신구

효과 장신구는 NetHack 계열로 간다 (효과 수치는 구현 시 확정). 기존 환금템 3종(copper_earring, silver_necklace, gold_ring)은 chestTier 미지정으로 체스트 풀에서 빠지고 환금·상점 용도로만 남는다.

| 아이템 | 효과 | 획득처 | 상태 |
|--------|------|--------|------|
| ring_of_protection | guard 부여 | 티어 5 던전 (20% 롤) | **신규 아이콘 필요** |
| amulet_of_life_saving | 사망 1회 방지 후 소모 | **월드 드랍 전용** | **신규 아이콘 필요** |
| ring_of_regeneration | HP 지속 재생 | **월드 드랍 전용** | **신규 아이콘 필요** |

amulet_of_life_saving·ring_of_regeneration은 성능이 강력해 확정 파밍에서 제외 — 특수 망토와 같은 월드 드랍 트랙(거래템)으로만 푼다.

### 망토·셔츠 (신규 슬롯)

| 슬롯 | 아이템 | 획득처 | 상태 |
|------|--------|--------|------|
| back | 기본 망토 | 티어 3 던전 | **신규 애셋 필요** — 체인 등장 시점에 맞춰 배치 |
| back | 특수 망토 (투명·보호 등) | **월드 드랍 전용** | **신규 애셋 필요** + 효과 시스템 설계 필요 |
| shirt | 셔츠 | **월드 드랍 전용** | **신규 애셋 필요** — 악세서리성 희소템 |

## 드랍 설계

### 체스트 풀 규칙 (변경안)

현재: `장비 슬롯 있음 + basePrice ≥ 2000 + chestTier ≤ 던전 티어` (chestTier 미지정 시 기본 1).

문제: 가격 하한 2000 때문에 leather_helmet(1,500)·leather_belt(800)가 어느 티어에서도 안 나온다 — "세트 완성" 컨셉과 충돌.

**변경: 가격 하한을 없애고, `chestTier`가 명시된 아이템만 풀에 넣는다** (opt-in). 티어별 세트가 명시적 설계가 됐으므로 가격으로 암묵 필터링할 이유가 없다. `DEFAULT_CHEST_TIER` 폴백과 `CHEST_ITEM_MIN_PRICE`는 제거.

### 드랍 방식 (변경안: 아이템별 독립 롤)

- 던전당 시그니처(확정) 드랍 1개 + 풀의 각 아이템을 **아이템별 확률로 독립 롤**.
- 시그니처는 독립 롤에서 제외해 중복을 막는다.
- 골드는 `깊이 × 500 ~ 깊이 × 1500` (현행 유지).

균등 2~3개 뽑기는 풀 크기에 따라 개별 확률이 흔들리고 하위 티어 이월템이 상위 풀을 희석한다. 독립 롤은 파츠별 확률이 풀 크기와 무관하므로 기대 파밍 횟수를 직접 설계할 수 있다.

### 드랍 확률: 던전당 기대 ~5회

목표: **해당 티어에서 처음 나오는 파츠를 전부 모으는 데 평균 ~5회** (운 나쁘면 더 걸릴 수 있음 — 10회 초과 확률 ≈4~6%). 필요 파츠 K개가 각각 확률 p로 독립 드랍될 때의 완성 기대 횟수 기준: K=1 → p 20%, K=2 → p 30%, K=3 → p 33%, K=4 → p 37%.

| 던전 | 시그니처 (확정) | 독립 롤 파츠 (회당 p) | 완성 기대 |
|------|----------------|----------------------|-----------|
| Old Crypt (T1) | leather_helmet | leather_pants·leather_belt 각 30% | ≈4.7회 |
| Orc Warrens (T2) | leather_armor | leather_gloves·leather_boots·iron_boots·iron_helmet 각 37% | ≈5.0회 |
| 티어 3 던전 | chain_mail | iron_gauntlets·plate_greaves·plate_boots·기본 망토 각 37% | ≈5.0회 |
| 티어 4 던전 | breastplate | plate_helmet·plate_gauntlets 각 30% | ≈4.7회 |
| 티어 5 던전 | (시그니처 없음) | ring_of_protection 20% | ≈5.0회 |

- 하위 티어 이월템(chestTier < 던전 티어)은 각 10% 보너스 롤 — 놓친 파츠를 상위 던전에서 메꿀 수 있고, 독립 롤이라 신규 파츠 확률을 희석하지 않는다.
- 확률 상수는 아이템 데이터에 `chestChance`로 명시하고, 완성 기대 횟수는 테스트로 고정(시뮬레이션 또는 닫힌식 검증).

### 월드 드랍 (희소·거래템)

- 대상: **특수 망토** (투명·보호 등 — 종류별 개별 롤), **셔츠**, **amulet_of_life_saving**, **ring_of_regeneration**.
- 던전 체스트 풀에서 완전히 제외. 티어 4~5 지역·던전의 일반 몬스터 처치 시 **0.1~0.5%** 확률로 드랍 (상위 지역일수록 가중).
- 확정 파밍 경로가 없어 희소성이 유지되고, 유저 간 거래의 축이 된다. "기대 ~5회" 목표의 의도적 예외.
- 귀속(soulbound) 없음 — 거래 가능해야 컨셉이 성립.

### 티어별 시그니처 드랍

각 세트의 몸통(핵심) 파츠가 완성 티어의 시그니처.

| 던전 | 시그니처 |
|------|----------|
| Old Crypt | leather_helmet |
| Orc Warrens | leather_armor (기존 raven_shield는 무작위 풀로) |
| 티어 3 던전 | chain_mail |
| 티어 4 던전 | breastplate |
| 티어 5 던전 | 없음 — ring_of_protection은 20% 롤 (확정이면 1회 만에 끝나므로) |

### 방패 배치

- raven_shield(guard 2): 티어 2 풀 20% 롤 (세트 완성 목표에는 미포함).
- 장신구는 위 장신구 표를 따른다. leather_belt는 가죽 세트 소속으로 티어 1 풀.

## 선행 작업

1. ~~신규 슬롯 3종 추가 확정: hands·back·shirt~~ — **완료 (2026-07-31, 프로토콜 v12)**. 장비 UI에는 hands만 노출; back·shirt 칸은 해당 아이템 등장 시 추가. back은 캐릭터 부착 렌더링(리깅/흔들림) 작업이 여전히 필요.
2. 신규 애셋: leather_belt 아이콘(현재 sword.png placeholder로 드랍 중), 판금 파츠 4종, 기본 망토, 특수 망토, 셔츠, 장신구 아이콘 3종(protection·life_saving·regeneration) (Meshy/ChatGPT 생성, `doc/assets/items.md`에 기록). leather_gloves·leather_boots·iron_helmet·iron_gauntlets는 완료 (2026-07-31), chain_mail은 완료 (2026-08-01).
3. 특수 망토·장신구 효과 시스템 설계 — 투명은 서버 측 가시성 처리, 사망 방지·재생은 서버 전투 로직.
4. **유저 간 거래 시스템.** 현재 `trading.rs`는 NPC 상인뿐, P2P 거래 미구현 — 월드 드랍 희소템 컨셉의 전제.
5. 몬스터 월드 드랍 경로 — 사망 시 무기 드랍(`weapon_drop_chance`, `combat.rs`)이 이미 있으므로 이를 희소템 롤로 확장.
6. 신규 던전 3개 (티어 3, 4, 5). 던전 생성기 변경 시 골든 해시 게이트 준수.
7. `chest_tiers_gate_endgame_loot_by_dungeon` 테스트를 새 배치로 갱신.

## 구현 순서 (제안)

1. ~~풀 규칙 변경 (opt-in chestTier + 아이템별 독립 롤 `chestChance`) + 기존 아이템 티어 재배치 + 테스트 갱신~~ — **완료 (2026-07-31)**.
2. ~~신규 슬롯 3종 (hands·back·shirt) 추가 — 프로토콜 범프 + 장비 UI 개편 한 번에~~ — **완료 (2026-07-31, v12)**.
3. ~~leather_boots·leather_gloves 애셋 추가 → 가죽 세트 완성 (1–2), iron_helmet 추가 → 체인 하위 파츠(2) 배치~~ — **완료 (2026-07-31, 각 guard 1~2·티어 2·37% 롤)**.
4. 티어 3 던전 → 체인 세트 완성 (체인 메일·건틀릿) + 판금 하위 파츠 + 기본 망토. iron_gauntlets 애셋·데이터는 완료 (2026-07-31, 티어 3·37% 롤).
5. 티어 4 던전 → 판금 세트 완성.
6. 티어 5 던전 → ring_of_protection.
7. 월드 드랍 + 특수 망토·셔츠·상위 장신구(효과 시스템) + 유저 간 거래 — 희소템 경제는 마지막 단계로.
