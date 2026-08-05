# Durability and Repair Design

> Status: **Proposed**. This document defines the intended item-condition
> contract. Implementation should be reviewed separately after the design is
> accepted.

Durability is per-item-instance state. Definitions describe an item's maximum
condition and repair compatibility; each owned, equipped, dropped, or traded
instance carries its own remaining condition.

## Goals

- Make wear deterministic, server-authoritative, and easy to explain.
- Preserve valuable equipment instead of destroying it implicitly at zero.
- Create repair-product and economy hooks without prematurely adding crafting
  professions.
- Keep condition intact through every inventory and trade path.
- Show enough information for players and agents to make maintenance choices.
- Provide a narrow first slice that can later expand to weapons, shields, and
  other armor pieces through separate review.

## Non-goals

The first slice does not include weapon wear, Shield wear, extremity-armor wear,
death wear, environmental wear, repair quality, permanent maximum-durability
loss, random breakage, item destruction, repair skill XP, crafting recipes, or
gradual combat penalties between full and zero condition.

## First-slice scope

Only the five mapped primary chest body-armors are durable initially:

| Armor              | Construction | Maximum condition | Repair family | Basic kit capacity |
| ------------------ | ------------ | ----------------: | ------------- | -----------------: |
| Padded Battle Robe | Padded       |                40 | Cloth         |                +20 |
| Leather Armor      | Leather      |                60 | Leather       |                +30 |
| Chain Mail         | Mail         |                90 | Metal         |                +45 |
| Breastplate        | Plate        |               120 | Metal         |                +45 |
| Brigandine Coat    | Hybrid       |               100 | Hybrid        |                +50 |

The values are initial balance targets. Construction identifies armor
proficiency and repair compatibility, but it does not derive maximum condition,
protection, or kit capacity implicitly.

## Data model

Durable item definitions declare:

- `maxDurability`: a positive maximum condition;
- `repairFamily`: one explicit family compatible with repair products.

Finished repair-product definitions declare:

- the same `repairFamily` vocabulary;
- `repairAmount`: a positive amount restored by one product.

Each durable `ItemInstance` declares:

- `durability`: remaining condition from 0 through the definition maximum.

Durable equipment must be non-stackable. If stack handling is applied
generically, entries may merge only when item definition, enchantment, and
condition are identical.

Startup validation must reject a durable item without a compatible repair
family, a repair product without a positive capacity, an unsupported family,
or durability metadata on an incompatible item kind.

## Instance lifecycle

New durable instances start at the current definition maximum. The same raw
condition must survive:

- bag and equipped movement;
- equip and unequip;
- drop and pickup;
- resident-trader transfer;
- merchant sale and buyback;
- disconnect, reconnect, and database round trip.

Legacy database rows may have no stored condition. On login, a missing value is
hydrated to the current definition maximum; the next save makes it explicit.
If a later data change lowers a maximum, an older larger value clamps to the
new maximum on load.

Condition is never client-authored. The server sends raw values, and clients
derive presentation from the same shared rules.

## Wear event

An accepted, landed monster hit wears the same functional primary chest
captured in the authoritative defense snapshot by exactly 1 condition.

The hit uses the pre-wear snapshot for Guard, protection, and armor-skill
eligibility. If that hit reduces condition from 1 to 0, the armor becomes
Broken for subsequent events; the accepted hit is not recalculated midway.

The following events cause no wear:

- a monster miss;
- a rejected, stale, duplicate, out-of-range, cross-floor, or invalid attack;
- a client prediction without an accepted server result;
- a different item equipped after the server captured the resolved snapshot;
- player attacks, PvP, death, weather, or ordinary movement in the first slice.

## Broken behavior

At 0 condition, armor remains equipped and keeps its equipment weight. It is
not deleted. Until repaired, it contributes:

- no item Guard;
- no slash, pierce, or blunt protection;
- no primary-armor skill activation or XP eligibility.

Broken state must be visible in item tooltips, character defense summaries, and
agent state. Ordinary clothing remains non-combat regardless of condition.

## Condition labels

Labels are presentation bands over the raw integer value:

| Label    | Boundary              |
| -------- | --------------------- |
| Pristine | Above 75%             |
| Worn     | Above 50% through 75% |
| Damaged  | Above 25% through 50% |
| Critical | Above 0% through 25%  |
| Broken   | Exactly 0             |

The bands do not change combat performance. Every positive condition value is
fully functional in the first slice; only Broken disables the item. This avoids
hidden protection cliffs and keeps the initial balance observable.

## Repair action

A repair request targets the equipped primary chest and one owned finished
repair product. It is valid only when:

- the character is connected, alive, and not in combat;
- the chest is durable and below maximum condition;
- the repair product exists in the character's bag;
- the product and armor repair families match;
- the request can atomically consume exactly one product and update the same
  item instance.

A successful repair applies:

```text
new condition = min(current condition + repairAmount, maxDurability)
```

The product is consumed only when condition increases. A mismatched family,
full-condition item, defeated character, in-combat character, missing item,
duplicate request, or otherwise rejected action keeps both the product and
condition unchanged.

Using a finished kit awards no skill XP. Cloth, Leather, Metal, and Hybrid are
repair families, not automatically trained Tailoring, Leatherworking,
Smithing, or Repair skills.

## Economy and resale

Condition modifies NPC resale after the ordinary sell-rate and haggling
calculation. The condition percentage is:

```text
25 + floor(75 × current condition / maximum condition)
```

The final payout is the ordinary offer multiplied by that percentage, rounded
down, with the existing minimum positive payout preserved. Full-condition gear
keeps 100% of the ordinary offer. Broken gear keeps a 25% salvage floor.

The smooth raw-condition formula is independent of presentation labels, so
crossing Pristine, Worn, Damaged, or Critical does not create a price cliff.

A merchant buyback entry stores the exact final payout, enchantment, and
condition of the sold unit. Repurchasing costs that exact payout and restores
the same condition, making the undo gold-neutral. Resident-trader transfers
also preserve condition. Batched sell and buyback paths must follow the same
rules as single-item actions.

Selling, buying back, and repairing grant no armor, appraisal, trading, repair,
or profession XP.

## Protocol and persistence

The wire model carries optional raw condition on item instances and trade
entries so legacy data can be represented during migration. Once hydrated, a
durable item should normally have an explicit value.

Any protocol change must update the handshake version and reject incompatible
clients. Browser and agent clients display raw current/maximum condition and the
shared label; they do not send trusted condition values.

Persistence tests must cover bag, equipped, ground, resident trade, merchant
buyback, reconnect, legacy-null hydration, and over-maximum clamping.

## Acceptance criteria

The first slice is acceptable when tests demonstrate that:

1. Only accepted landed monster hits wear the resolved functional chest.
2. The hit that reaches zero resolves with the pre-wear defense snapshot, and
   later hits receive no defense or armor-skill activation from that chest.
3. Condition survives every inventory, persistence, ground, and trade path.
4. Each repair family accepts only its matching product and applies its authored
   bounded capacity atomically.
5. Rejected repair requests consume nothing.
6. Browser and agent labels match shared boundary rules.
7. Single and batched resale use the same smooth condition value.
8. Buyback restores the exact condition and exact adjusted payout.

## Future expansion

Weapons, shields, additional armor layers, profession-made repair products,
repair quality, field-repair restrictions, material costs, and environmental
wear should reuse the same instance-state and repair-family model. Each new wear
source or durable equipment category needs its own gameplay and balance review.

The skill interaction boundary is defined in
[SKILL_SYSTEM.md](SKILL_SYSTEM.md): wear and finished-product use are not skill
training events by themselves.
