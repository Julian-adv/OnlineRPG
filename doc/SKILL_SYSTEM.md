# Skill System Design

> Status: **Proposed**. This document defines the intended gameplay contract.
> Implementation and balance changes should be reviewed separately after the
> design is accepted.

OpenMMO uses a small, use-based progression model: characters improve skills by
completing legitimate actions that the server accepts and resolves. Ultima
Online is inspiration for that broad idea, not a rules specification. OpenMMO
keeps its own combat, character-level, persistence, protocol, and client model.

## Goals

- Reward real gameplay instead of client-declared training events.
- Keep skill definitions explicit and data-authored.
- Use one progression, persistence, and messaging model across skill families.
- Keep bonuses small enough that equipment and character attributes remain
  meaningful.
- Make browser, agent, and server behavior describe the same rules.
- Add one bounded vertical slice at a time so balance can be measured before
  another family expands.

## Non-goals

The initial system does not add a total skill-point cap, skill locks, decay,
trainers, skill books, random gain chance, difficulty-based gain, diminishing
returns, PvP training, target-dummy training, class restrictions, active combat
abilities, or skill-based critical hits. Those require separate designs.

Equipment condition, repair, and resale are defined in
[DURABILITY.md](DURABILITY.md). Using a finished product does not automatically
train the profession that might eventually make that product.

## Shared progression model

All trained skills use one stable wire identifier and one sparse progress map.
Skill levels run from 0 through 30. Reaching level `n` costs `100 × n²` XP for
that level, and the cumulative threshold is the sum of those per-level costs.
XP clamps at the level-30 threshold.

A missing map entry means level 0 with 0 XP. The first valid award creates the
entry, even when the award is not enough to reach level 1. This avoids creating
rows for skills a character has never used.

The proposed shared skill set is:

| Domain     | Skill identifiers                                                            |
| ---------- | ---------------------------------------------------------------------------- |
| Gathering  | `fishing`                                                                    |
| Weapons    | `one_handed_sword`, `dagger`, `spear`                                        |
| Defense    | `shield`                                                                     |
| Treatment  | `healing`                                                                    |
| Body armor | `padded_armor`, `leather_armor`, `mail_armor`, `plate_armor`, `hybrid_armor` |

Adding another identifier is a protocol and gameplay decision. Unknown item
metadata must fail validation rather than silently creating a new skill.

## Server authority

The server owns every input that determines a skill award:

- the acting character and authoritative equipment;
- target existence, life state, floor, range, and ownership;
- cooldown or action-window acceptance;
- hit, miss, damage, healing, and kill outcomes;
- the mapped skill, current progress, bonus, and XP amount.

The client may request an action, but it never supplies the trained skill,
result, level, bonus, or XP. Rejected, stale, duplicated, out-of-range,
cross-floor, or otherwise invalid requests award nothing.

XP is attached to a resolved gameplay event, not to an animation packet,
inventory click, product consumption, or client prediction.

## Explicit item mappings

Skills are assigned by item metadata, never inferred from names, icons, model
paths, material strings, or damage dice.

| Metadata                           | Purpose                                                                                |
| ---------------------------------- | -------------------------------------------------------------------------------------- |
| `weaponSkill`                      | Main-hand weapon accuracy family                                                       |
| `defenseSkill`                     | Shield or primary body-armor proficiency                                               |
| `useSkill`                         | Skill used by an authoritative item action, such as Bandage treatment                  |
| `armorConstruction`                | Padded, Leather, Mail, Plate, or Hybrid body-armor identity                            |
| `equipmentKind` / `equipmentLayer` | Distinguish body armor, shields, clothing, accessories, held items, and primary layers |

Startup validation must reject mappings whose slot, equipment kind, layer,
construction, dice, Guard, or protection profile is incompatible with the
declared skill.

## Shared bonus bands

Weapon accuracy, Shield Guard, primary-armor Guard, and Bandage treatment use
the same small four-band progression:

| Skill level | Bonus |
| ----------- | ----: |
| 0–4         |    +0 |
| 5–14        |    +1 |
| 15–24       |    +2 |
| 25–30       |    +3 |

The meaning of the bonus depends on the skill family. A weapon skill adds to
the attack roll, Shield and armor skills add to Guard, and Healing adds HP to a
valid Bandage treatment. No initial skill adds directly to weapon damage.

## Weapon skills

### Mappings and combat profiles

| Skill            | Initial mapped items                                   | Reach | Authoritative attack window |
| ---------------- | ------------------------------------------------------ | ----: | --------------------------: |
| One-Handed Sword | Iron Sword, Worn Iron Sword, Goblin Sword, Small Sword |   2 m |                     1.533 s |
| Dagger           | Dagger                                                 |   2 m |                     1.533 s |
| Spear            | Spear                                                  |   3 m |                     2.467 s |

The server captures the equipped main-hand definition before resolution. The
browser and agent use the same authored profile for chase range and presentation,
but only the server accepts the action window.

The attack formula is:

```text
character-level attack bonus
+ Strength modifier
+ weapon enchantment
+ mapped weapon-skill bonus
```

Damage remains the weapon dice plus Strength modifier and enchantment. Weapon
skill does not change damage in the initial design.

### Weapon XP

Accepted server-resolved attacks award:

| Outcome      | Total XP |
| ------------ | -------: |
| Miss         |        5 |
| Hit          |       10 |
| Killing blow |       20 |

Equivalently, the award is `5 + (hit ? 5 : 0) + (killing blow ? 10 : 0)`.
Only the authoritative transition from positive HP to zero earns the killing
blow portion. Switching targets cannot bypass the per-attacker action window.

## Shield

Only explicitly mapped off-hand shields with positive item Guard activate the
Shield skill. Torches, weapons, accessories, and other armor do not.

Effective Guard is:

```text
DEX-derived base Guard
+ Guard from equipped items
+ active Shield skill bonus
+ active primary-armor skill bonus
```

Item Guard and trained Guard are separate terms and each is applied once.
Removing the mapped shield removes only its item contribution and Shield skill
bonus.

An accepted monster attack against a living defender trains Shield as follows:

| Outcome        |  XP |
| -------------- | --: |
| Monster misses |  10 |
| Monster hits   |   5 |

A successful avoidance teaches more, but a failed defense still teaches. The
initial slice adds no block animation, active parry request, damage reflection,
or Shield-specific mitigation packet.

## Healing

Healing means applying treatment. It does not mean consuming any item that
restores HP.

Only Bandages map to `healing` through `useSkill`. A valid initial action is
self-treatment by a connected, living, injured character who owns the mapped
Bandage instance. The server consumes one Bandage atomically before applying
the authoritative roll.

A Bandage restores `2d4 + Healing bonus` HP, capped at maximum HP. XP equals
the HP actually restored after the cap. Treating a one-point injury therefore
awards 1 XP. Full-health, defeated, missing-item, unusable-item, and duplicated
requests award nothing and must not consume a Bandage.

Healing Potions, fish, and other finished products may restore HP through their
own item effects, but drinking or eating them awards no Healing XP and receives
no Healing bonus. A Healing Potion remains a `6d4` product. Future Alchemy may
govern manufacturing without turning potion drinking into treatment training.

Party treatment, resurrection, magical healing, treatment channels, crafting,
and profession restrictions are outside the initial Healing slice.

## Body-armor proficiencies

Body armor uses construction-specific skills rather than Light/Heavy buckets.
The initial mappings are:

| Construction | Skill           | Initial primary chest |
| ------------ | --------------- | --------------------- |
| Padded       | `padded_armor`  | Padded Battle Robe    |
| Leather      | `leather_armor` | Leather Armor         |
| Mail         | `mail_armor`    | Chain Mail            |
| Plate        | `plate_armor`   | Breastplate           |
| Hybrid       | `hybrid_armor`  | Brigandine Coat       |

The explicitly mapped, functional primary chest anchors the active armor skill.
Helmets, gloves, gauntlets, pants, greaves, boots, ordinary clothing, and other
layers do not activate a second armor skill. Mixed equipment therefore cannot
create several armor-skill awards from one hit.

An armored robe or coat may qualify when its metadata explicitly says it is
primary body armor with a mapped construction. Garment form or a word in the
item name is never sufficient. An ordinary Traveler's Robe remains clothing.

Each active mapped armor skill uses the shared +0/+1/+2/+3 Guard bands. Item
Guard, authored slash/pierce/blunt protection, and trained Guard are separate
values. Construction identifies proficiency and repair compatibility; it does
not implicitly derive mitigation numbers.

An accepted, landed monster hit awards 5 XP to the one active armor skill. A
miss awards 0 because the blow did not reach the armor. Rejected attacks,
replayed cooldown requests, unmapped gear, ordinary clothing, and Broken armor
award nothing. If a bonus threshold is crossed, clients receive one final
authoritative Guard value containing all simultaneously active item, Shield,
and primary-armor terms.

`bodyCoverage` may describe head, torso, arms, hands, legs, and feet, but it is
not a skill trigger. Hit-location rolls, weighted regional mitigation,
multi-layer occupancy, casting penalties, and construction-specific movement
rules require separate designs.

## Equipment burden is not a skill

Equipped weight may use Strength-based capacity to select an unburdened, light,
medium, or heavy movement band. Bag contents, armor skill level, and a future
Athletics skill do not modify that band in the initial design. Movement burden
does not train a skill.

## Durability is not a skill action

Broken primary armor cannot activate or train its armor proficiency until it is
repaired. Wear, repair-kit use, selling, and buyback grant no armor, repair,
appraisal, trading, tailoring, or smithing XP. Future crafting skills may
produce maintenance supplies, but consuming a finished kit remains equipment
maintenance. See [DURABILITY.md](DURABILITY.md).

## Persistence and protocol

All skills share one sparse character-skill table, generic dirty tracking,
batch save, logout and shutdown flush, and login load. A new skill does not add
a character column or a family-specific save path.

Generic private messages carry full skill snapshots and XP deltas. A protocol
version must change whenever a new wire identifier or message shape would make
an older client unsafe. The handshake must reject incompatible clients rather
than allowing partial interpretation.

Persistence should preserve unknown rows so a temporarily older server does
not erase progress it cannot interpret.

## Browser and agent parity

Both clients consume the same generic snapshots and deltas. They may display
trained entries, level progress, the relevant bonus, mapped item metadata, and
the exact authoritative Guard value.

The agent stores routine skill and Guard updates as state-only information so
every XP event does not wake the language model. Neither client computes a
trusted outcome or chooses which skill receives XP.

## Balance and change control

The XP awards, bands, ranges, and action windows above are initial values. Any
balance change should use aggregate, non-identifying telemetry and receive a
separate review. Expanding a family, changing what trains it, or adding a new
bonus is a new vertical slice—not an automatic consequence of adding an item.
