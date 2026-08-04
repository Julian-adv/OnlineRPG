# Skill System

OpenMMO uses a small, use-based progression model: a character improves a
trained skill by completing legitimate server-resolved actions that use it.
Ultima Online is inspiration for that broad idea, not a rules specification.
OpenMMO keeps its own combat, level, persistence, protocol, and client model.

## Shared model

All trained skills use `SkillId`, `SkillProgress`, and `Skills` from
`shared/src/skills.rs`. Skill levels run from 0 through 30. Reaching level `n`
costs `100 × n²` XP for that level; `skill_xp_for_level` supplies the cumulative
threshold. XP clamps at the level-30 threshold.

The map is sparse. A fresh character has no row for an untrained skill, and a
missing entry reads as level 0 with 0 XP. The first valid XP award creates an
entry even when its total is still below level 1. Fishing, One-Handed Sword,
Dagger, Spear, Shield, Healing, Leather Armor, Mail Armor, Plate Armor, and
Padded Armor, and Hybrid Armor coexist in the same generic map.

## Phase 1: One-Handed Sword

The wire id is `one_handed_sword`, displayed as **One-Handed Sword**. Items are
classified explicitly by the optional `weaponSkill` field in
`data-src/items.csv`; names, model paths, and damage dice are not used to infer
the skill.

Mapped items:

- `iron_sword`
- `worn_iron_sword`
- `goblin_sword`
- `small_sword`

In Phase 1, Dagger, spear, torch variants, fishing rod, and unarmed attacks were
not mapped. At startup, a `weaponSkill` assignment must deserialize to a known
`SkillId`, be supported by the current phase, belong to a main-hand weapon, and
have positive `NdM` damage dice.

### Accuracy

One-Handed Sword affects the attack roll only:

| Skill level | Attack bonus |
| ----------- | -----------: |
| 0–4         |           +0 |
| 5–14        |           +1 |
| 15–24       |           +2 |
| 25–30       |           +3 |

The shared weapon-skill bonus function clamps inputs above 30 and is exported
through WASM so the browser shows the same value the server uses.
The player formula is:

```text
level attack bonus + Strength modifier + weapon enchant + weapon skill bonus
```

Skill does not change damage. Damage remains weapon dice plus Strength modifier
plus weapon enchant.

### XP

The provisional Phase 1 award is centralized by attack outcome:

```text
accepted resolved miss = 5 XP
accepted resolved hit  = 10 XP
killing blow           = 20 XP total
```

Equivalently: `5 + (hit ? 5 : 0) + (killing_blow ? 10 : 0)`.

The server grants XP only when the attacker exists and is alive, the target is
alive and on the same floor, melee range is valid, the per-player attack window
is atomically accepted, the captured authoritative main-hand item maps to
One-Handed Sword, and the server resolves the attack. Invalid, stale,
out-of-range, cross-floor, dead-attacker, cooldown-rejected, and duplicate kill
requests grant no XP. Only the transition from positive HP to zero earns the
kill portion.

Player attack cadence is authoritative and per attacker, not per target. The
server uses a monotonic 1.533-second window based on the visible `slash1`
animation. Player, target, floor, and range are checked before the window is
claimed; switching targets cannot bypass it. Cooldown state is removed with the
player.

## Persistence and protocol

One-Handed Sword uses the existing `character_skills` rows, generic dirty set,
batch save, logout/session-replacement detach, shutdown flush, login load, and
unknown-row-preserving upsert. It has no table, character column, or save path
of its own.

The existing private `SkillsUpdate` and `SkillXpGained` messages carry all
skills. Protocol version 27 identifies Hybrid Armor and the strict handshake
refuses older clients. Protocol 26 introduced Padded Armor, protocol 25
introduced Plate Armor, protocol 24 introduced Mail Armor, protocol 19
introduced Leather Armor, protocol 18 introduced Healing, and protocol 17
introduced Shield.

## Browser and agent parity

The browser reuses the existing skills store and Character Skills tab. It only
renders trained map entries, shows level/progress, displays shared accuracy,
Guard, or treatment bonuses, marks level 30 as max with a complete bar, and
labels mapped item tooltips with the relevant action skill. Character Stats
uses the exact server-authored Guard and summarizes the equipped primary body
armor's mapped skill and authored physical profile; Broken armor remains named
but is explicitly inactive.

The agent-client sends the same target-only attack request, follows the same
animation cadence, accepts generic skill snapshots and deltas, and stores them
without waking the LLM for every XP event. It also stores `GuardUpdated` as
state-only data and exposes the exact effective Guard beside its semantic
equipment summary without waking the LLM for the update itself. Neither client
supplies equipment, skill level, outcome, bonus, Guard, or XP to the server.

## Phase 1 limits

Phase 1 adds no skill-based damage, critical hits, attack-speed bonus, special
attacks, PvP training, training targets, difficulty scaling, diminishing
returns, random gain chance, total skill cap, locks, decay, trainers, skill
books, weapon requirements, or future skill placeholders. Fishing balance and
character XP/level rules are unchanged. Future work belongs in
`SKILL_SYSTEM_ROADMAP.md` and requires a separately approved vertical slice.

## Phase 2: Stabilization

Phase 2 begins with opt-in aggregate server reporting rather than another
skill or an immediate balance change. The report measures outcomes by skill
band, target difficulty, configured monster type, and client kind, along with
attack cadence, rejection counts, skill messages, new rows, and save batches.
It contains no player identifiers and is disabled by default.

The baseline, report flag, field definitions, and review procedure are in
`doc/SKILL_BALANCE.md`. Phase 1 XP, accuracy, and cooldown values remain
provisional until gameplay data supports a reviewed adjustment.

## Phase 3: Dagger

Phase 3 adds exactly one weapon skill with wire id `dagger` and display name
**Dagger**. Only the existing `dagger` item maps to it. During Phase 3, Spear,
torches, fishing rod, and unarmed attacks remained unmapped.

Dagger reuses the generic XP curve, sparse persistence rows, private protocol
messages, Skills tab, item tooltip, combat profile, cooldown, and aggregate
balance reporting. Accepted misses, hits, and killing blows provisionally use
the same 5/10/20 XP awards and +0/+1/+2/+3 accuracy ladder as One-Handed Sword;
damage remains the Dagger item's `1d4` plus Strength and enchantment.

The item currently reuses `weapons/sword.glb` and the existing `slash1`
animation, so it keeps the same 2-meter melee validation and 1.533-second
authoritative cadence. This is a content-backed provisional choice, not a
promise that every future weapon family shares the same animation or range.

The server captures the authoritative main-hand definition before resolving an
attack and awards XP only to its explicit skill. The browser and agent-client
consume the same generic snapshots/deltas; neither can choose the awarded
skill. Aggregate reports include a per-skill breakdown without player identity.

Phase 3 was started under an explicit project-owner override of Phase 2's
representative-data completion gate. That override permits this vertical slice;
it does not approve balance changes or mark the Phase 2 review complete.

## Phase 4: Spear

Phase 4 adds the `spear` wire skill and maps only the existing `spear` item.
The pinned `weapons/spear.glb` asset uses the dedicated `slash3` combat clip.
Asset sampling places its forward tip strike at about 1.060 seconds; the full
clip is 2.467 seconds. Browser hit, miss, and damage presentation use those
Spear timings.

Spear has a server-authoritative 3-meter melee reach and 2.467-second attack
window. The browser click/chase controller and agent chase/attack loop read the
same shared profile. Sword and Dagger retain `slash1`, 2 meters, and 1.533
seconds. Range and cadence are captured with the authoritative equipped weapon
before each server-resolved attack.

Spear provisionally reuses the +0/+1/+2/+3 accuracy ladder and 5/10/20 XP
awards. Skill does not add damage: the Spear item still deals `1d6` plus
Strength and enchantment. Sparse persistence, generic messages, Skills UI,
tooltips, no-identity metrics, and agent snapshots/deltas require no
family-specific storage or protocol path.

## Phase 5: Shield

Phase 5 adds one defensive skill with wire id `shield` and display name
**Shield**. The existing `wooden_shield` and `raven_shield` items map through
the explicit optional `defenseSkill` field. A mapping is valid only for
off-hand armor with positive item Guard; torches, helmets, rings, and other
armor remain unmapped.

Effective Guard stays readable and is calculated once by the server:

```text
base attribute Guard + every equipped item's Guard + active Shield skill bonus
```

The base attribute Guard is still the character-roll value derived from DEX;
Shield does not recalculate DEX or replace that term.

The item value and trained value are separate terms. A Wooden Shield contributes
its item Guard +1 exactly once; Shield levels 0–4/5–14/15–24/25–30 add
+0/+1/+2/+3 while a mapped shield is equipped. Removing or replacing the
shield removes only the trained modifier. `GuardUpdated` carries the exact
resolved number on join, equipment changes, and skill-bonus thresholds.

Shield training is owned by the server's monster-attack resolution. After the
monster ownership, alive, floor, reach, and per-monster cooldown gates accept
an attack against a living defender, a mapped equipped shield earns 10 XP when
the monster misses and 5 XP when it hits. A failed defense still teaches, but
an out-of-range, cross-floor, stale, unowned, cooldown-replayed, or unmapped
request earns nothing. The requesting browser or agent supplies neither the
shield, Guard, result, skill level, nor XP amount.

Shield reuses the generic sparse rows, dirty persistence, snapshots/deltas,
Skills UI, tooltip metadata, and no-identity balance report. It adds no block
animation, damage reduction, reflected damage, active parry packet, PvP
training, or second defensive family.

## Phase 6: Healing

Phase 6 adds one noncombat skill with wire id `healing` and display name
**Healing**. Healing represents applying treatment, not consuming a finished
medical product. Only the Bandage maps through the explicit optional
`useSkill` field. A valid mapping must be a consumable `bandage` with positive
dice. Healing Potions keep their existing instant-heal path but have no
`useSkill`; fish likewise heal without training. Scrolls and other consumables
remain unmapped.

A valid Healing action requires a connected, living, injured player and the
mapped Bandage instance in that player's bag. This slice supports
self-treatment only. The server atomically consumes one Bandage before
applying its authoritative roll, so concurrent duplicate `UseItem` packets
cannot reuse an instance. Full-health, defeated, missing, unusable, and
unmapped item requests award no Healing XP.

A Bandage restores `2d4` base HP. Training adds a small flat modifier:

| Skill level | Healing bonus |
| ----------- | ------------: |
| 0–4         |         +0 HP |
| 5–14        |         +1 HP |
| 15–24       |         +2 HP |
| 25–30       |         +3 HP |

Healing cannot exceed max HP. XP equals the HP actually restored after that
cap, so treating a one-point scratch earns 1 XP rather than the uncapped dice
roll. The existing generic curve converts that XP into levels. A capped skill
still permits Bandage use but stops adding XP or messages. A Healing Potion
always uses only its own `6d4` roll regardless of Healing level and awards no
Healing XP.

Healing reuses sparse skill rows, dirty persistence, snapshots/deltas, the
Skills tab, agent state, tooltip metadata, and aggregate no-identity metrics.
It adds no party-target treatment, resurrection, spellcasting, treatment
channel, crafting, potion recipes, class restriction, or additional
profession. Magical healing remains a future spell-system concern; Alchemy
may later govern potion manufacture without turning potion drinking into
Healing training.

## Armor vertical slices: Padded, Leather, Mail, Plate, and Hybrid Armor

Construction-specific armor skills do not introduce Light/Heavy buckets.
Worn body armor is classified by `armorConstruction` as `padded`, `leather`,
`mail`, `plate`, or `hybrid`; shields, clothing, and accessories have no
body-armor construction. Equippable items also declare `equipmentKind` and
`equipmentLayer`, while garments declare `garmentForm`.

The first slice maps the `leather_armor` chest to wire id `leather_armor` and
display name **Leather Armor**. Protocol v24 adds `chain_mail` through wire id
`mail_armor` and display name **Mail Armor**. Protocol v25 adds `breastplate`
through wire id `plate_armor` and display name **Plate Armor**. Protocol v26
adds `padded_battle_robe` through wire id `padded_armor` and display name
**Padded Armor**. Protocol v27 adds `brigandine_coat` through wire id
`hybrid_armor` and display name **Hybrid Armor**. A shared mapping binds each
skill to its matching construction, and startup validation rejects a wrong
kind, slot, primary layer, construction, or an item with neither Guard nor
physical protection. Future construction values still require an explicit,
approved mapping rather than becoming skills automatically.

The primary chest anchors the active armor skill. Leather boots, gloves, pants,
and helmets do not activate Leather Armor; Plate helmets, gauntlets, greaves,
and boots do not activate either Mail or Plate Armor. Mixed pieces therefore
cannot create several armor-skill events from one hit. The Padded Battle Robe is
an armored robe and maps Padded Armor despite having no item Guard; the ordinary
Traveler Robe remains clothing with no physical skill. The Brigandine Coat is
body armor and maps Hybrid Armor because of its authored Hybrid construction;
the word “coat” and garment form alone never establish skill eligibility.

Authored `bodyCoverage` records whether a garment spans head, torso, arms,
hands, legs, or feet, but it does not select or train a skill. A Traveler's Robe
and Padded Battle Robe both span torso/arms/legs while only the explicitly
constructed and mapped Padded item activates Padded Armor. Extremity coverage
therefore remains semantic foundation rather than a second skill trigger.

Any mapped armor skill adds one trained Guard term while its chest is
functional and equipped:

| Skill level | Guard bonus |
| ----------- | ----------: |
| 0–4         |          +0 |
| 5–14        |          +1 |
| 15–24       |          +2 |
| 25–30       |          +3 |

The item Guard and trained Guard are separate and each is applied once. Shield
may be active at the same time, but its item and skill terms remain separate.
The authoritative total is:

```text
DEX-derived base Guard
+ every equipped item's Guard
+ active Shield skill bonus
+ active primary-armor skill bonus
```

The active mapped armor skill trains only when an accepted server-resolved
monster attack lands on the living wearer. Each hit awards 5 XP; a miss awards
0 because the blow never reached the armor. Ownership, life, floor, reach, and
monster cooldown gates run first. Rejected, replayed, Broken, unmapped, and
ordinary-clothing cases award nothing. Swapping among Padded, Leather, Mail,
Plate, and Hybrid changes which single skill receives the next valid hit; it
never trains more than one. A bonus-band crossing emits one final
`GuardUpdated` after the generic XP delta, including any simultaneously active
Shield term.

The skill slice reuses sparse persistence, generic messages, the Skills tab,
item tooltips, agent state, and aggregate defense metrics. The later protocol
v20 physical-damage slice adds damage types plus Padded, Leather, Mail, Plate,
and Hybrid mitigation identities. The current primary chests now author their
exact `slashProtection`, `pierceProtection`, and `bluntProtection` values rather
than deriving numbers from construction. The Leather chest retains Guard 2 and
mitigates slash, pierce, and blunt by 1 each. Chain Mail retains Guard 5 and
mitigates slash 2 / pierce 1. Plate retains Guard 7 and mitigates slash 3 /
pierce 3 / blunt 1. Hybrid retains Guard 2 and mitigates slash, pierce, and
blunt by 2 each. The Padded Battle Robe, Leather, Chain Mail, and Breastplate
chests and Brigandine Coat map their matching skills without changing those
authored item profiles. A landed hit trains the one active mapped skill regardless of
its exact final-damage amount; the authoritative hit result owns training, not
product or repair-kit use. Hit location, casting penalties, future
construction-specific skills, class restrictions, and wearable body rendering
remain separate stages in [ARMOR_SYSTEM.md](ARMOR_SYSTEM.md).

Protocol v21 adds equipped-weight movement burden without adding or training a
skill. Strength changes the burden thresholds through carry capacity, but no
Armor, Athletics, or movement proficiency modifies the tier or speed. Bag
contents remain outside this calculation.

Protocol v23 adds per-instance condition to primary chest body armor. A broken
Padded, Leather, Mail, Plate, or Hybrid chest no longer activates or trains its
armor skill until repaired; construction-aware Cloth, Leather, Metal, and
Hybrid kits repair only their matching family by an authored 20, 30, 45, or 50 condition,
capped at maximum.
Pristine, Worn, Damaged, Critical, and Broken are condition labels rather than
skills or gradual protection modifiers. Using a finished kit is equipment
maintenance and does not create or train Smithing, Tailoring, or a generic
repair skill. The generic skill persistence and XP formulas are unchanged.
NPC resale value now consumes the same raw condition on a smooth 25–100% scale,
but selling and buyback also grant no XP and do not introduce Trading or
Appraisal as trained skills.
