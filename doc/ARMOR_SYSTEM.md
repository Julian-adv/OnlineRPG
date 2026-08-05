# Armor System Architecture

This document defines the long-term armor foundation. It deliberately covers
more than the current game implements so new equipment can fit one coherent
model, while keeping each implementation phase small and testable.

The runtime now has five deliberately narrow, item-authored physical-mitigation
profiles. Every
equipped item's `guard` is added to the character's DEX-derived Guard, and an
active Shield plus one mapped Padded, Leather, Mail, Plate, or Hybrid Armor skill
may add a trained bonus. After a hit lands, the server resolves `untyped`,
`slash`, `pierce`, or `blunt` damage.
A Padded Battle Robe mitigates slash 1 and blunt 2; Leather Armor mitigates
slash, pierce, and blunt by 1 each. Chain Mail mitigates slash 2 and pierce 1
but not blunt. A Breastplate mitigates slash and pierce by 3 and blunt by 1. A
Brigandine Coat mitigates slash, pierce, and blunt by 2 each. None protects
against untyped damage, and every positive hit deals at least 1 final damage.
Per-instance durability, authored body-region identity, and deterministic
weighted aggregate coverage are active. Construction-specific burden and
multi-layer occupancy remain staged. Equipped-weight movement burden is active
as a separate vertical slice.

## Design rules

1. Classify equipment by explicit data, never by item name, display name, model
   path, or broad `material` alone.
2. Keep construction, garment form, equipment slot, layer, coverage, and loot
   set/tier independent. They answer different questions.
3. Keep all combat, burden, durability, and skill outcomes server-authoritative.
4. Give the browser client and agent-client the same actionable state and
   outcome information.
5. Activate the design in complete vertical slices. Do not add unused enums,
   protocol fields, database columns, or UI rows as placeholders.
6. Prefer tunable data to item-specific combat branches.
7. Do not count the same protection twice through Guard, mitigation, resistance,
   or a skill bonus.

## Classification model

An item may need values on several independent axes. “Robe,” “mail,” and
“plate” therefore do not belong in one flat armor enum.

| Axis               | Question answered                          | Examples                                                |
| ------------------ | ------------------------------------------ | ------------------------------------------------------- |
| Equipment kind     | What broad rules own the item?             | clothing, body armor, shield, accessory                 |
| Construction       | How is physical protection built?          | none, padded, leather, mail, plate, hybrid              |
| Layer              | Where is it worn relative to other gear?   | under, primary armor, outer, held, accessory            |
| Garment form       | What shape is the item?                    | shirt, robe, gambeson, hauberk, breastplate, greaves    |
| Equip slot         | Which inventory position does it occupy?   | shirt, chest, pants, head, hands, boots, back, off-hand |
| Coverage           | Which anatomical regions does it span?     | torso, head, arms, hands, legs, feet                    |
| Set/tier           | Where does it sit in progression and loot? | Leather set T1–2, Mail loadout T2–3, Plate set T3–4     |
| Protection profile | What happens after a hit lands?            | slash, pierce, blunt, wards                             |

### Equipment kind

- **Clothing** provides identity, visuals, utility, or magical effects. Ordinary
  clothing does not gain physical defense merely because it occupies a body
  slot.
- **Body armor** participates in the physical protection pipeline.
- **Shield** is held equipment with block/Guard behavior. It is not a body-armor
  construction.
- **Accessory** uses accessory effects and must not be inferred as armor because
  an item such as `ring_of_protection` grants Guard.

The runtime `equipmentKind` now distinguishes weapons, tools, clothing, body
armor, shields, and accessories. The older `category` remains because it owns
existing item-use and damage dispatch, but boot validation requires both fields
to agree.

### Armor construction

The canonical constructions are:

| Construction | Meaning                                                         | Initial gameplay identity                               |
| ------------ | --------------------------------------------------------------- | ------------------------------------------------------- |
| none         | Ordinary clothing with no physical armor structure              | no physical protection                                  |
| padded       | Quilted or layered textile armor                                | light protection; useful against impact                 |
| leather      | Purpose-built leather armor                                     | light, balanced physical protection                     |
| mail         | Interlinked metal rings                                         | strong against cuts; less complete against impact       |
| plate        | Shaped rigid metal plates                                       | broad physical protection with greater equipment burden |
| hybrid       | Two or more intentional constructions in one protective garment | authored profile; do not average by name                |

“Chain Mail” can remain the player-facing item name, but **mail** is the better
internal construction term. A jack of plate or coat-of-plates belongs under
`hybrid`, not `plate` merely because it contains metal. Scale and lamellar can
be added later when real content needs them; they should not be empty enum
members today.

Construction is not the same as `material`. The current `metal` value cannot
distinguish `chain_mail`, an iron helmet, and a steel breastplate, while a
hybrid garment may legitimately contain both textile and metal.

### Layer

The target layer meanings are:

- **under** — shirts, tunics, and future arming garments;
- **primary armor** — the main protective body layer;
- **outer** — robes, coats, cloaks, and capes when designed to cover another
  layer;
- **held** — shields;
- **accessory** — rings, belts, necklaces, and similar effects.

The current runtime registers `held`, `primary`, and `accessory`. `under` and
`outer` remain unregistered until compatible items, wearable assets, and an
occupancy consumer ship. `shirt` and `back` already exist on the wire, but their
panel cells remain hidden. Current robes occupy `chest` on the `primary` layer,
so equipping one replaces any chest armor. If a later design requires robes
over body armor, add an explicit outer-body occupancy rule; do not overload
`back`, which is reserved for cloaks and capes.

### Garment form and the robe rule

`robe` is a garment form, not a protection construction:

| Item concept         | Kind       | Construction                      | Likely layer/slot       | Physical rule                                              |
| -------------------- | ---------- | --------------------------------- | ----------------------- | ---------------------------------------------------------- |
| `traveler_robe`      | clothing   | none                              | primary/chest           | no physical Guard or mitigation                            |
| `padded_battle_robe` | body armor | padded                            | primary/chest           | slash 1 / blunt 2 mitigation; Padded Armor skill           |
| `brigandine_coat`    | body armor | hybrid                            | primary/chest           | balanced slash/pierce/blunt mitigation; Hybrid Armor skill |
| leather coat         | body armor | leather                           | primary/chest           | balanced slash/pierce/blunt protection                     |
| mail robe/hauberk    | body armor | mail                              | primary/chest           | cut-focused slash 2 / pierce 1 protection                  |
| enchanted robe       | clothing   | none                              | primary/chest initially | magical ward/effect, not physical armor skill              |
| cloak                | clothing   | none unless explicitly reinforced | outer/back              | utility, appearance, or authored ward                      |

This prevents appearance from silently deciding combat behavior. A magical
robe can have a ward without becoming “light armor,” and an armored robe can be
categorized by its actual construction.

### Coverage and occupancy

`EquipSlot` describes inventory occupancy; `bodyCoverage` describes anatomical
reach. They are related but not identical. A chest-slot hauberk covers torso,
arms, and legs, while a breastplate covers the torso. Coverage alone grants no
Guard, mitigation, or skill: an ordinary robe can span the same regions as an
armored robe while remaining clothing.

Phase Q establishes six canonical regions: `head`, `torso`, `arms`, `hands`,
`legs`, and `feet`. Every clothing or body-armor garment authors a non-empty,
canonically ordered set, while weapons, shields, and accessories must not use
the field. Form validation keeps helmets on head, gloves on hands, boots on
feet, leggings on legs, cuirasses on torso, and extended chest garments within
torso/arms/legs.

Phase R makes coverage a deterministic aggregate defense consumer. The shared
region budget is provisional:

| Region | Weight |
| ------ | -----: |
| Head   |    10% |
| Torso  |    40% |
| Arms   |    15% |
| Hands  |     5% |
| Legs   |    20% |
| Feet   |    10% |

Every region covered by at least one functional equipped body-armor item counts
once. Overlapping garments and extremity pieces therefore cannot stack the same
region, ordinary clothing contributes nothing, and Broken armor is excluded.
The active functional primary chest still owns the authored physical profile
and armor skill. Each positive channel is ceiling-scaled by aggregate coverage:

```text
effective protection
= ceil(authored primary-chest protection × coverage percent / 100)
```

Ceiling rounding preserves useful low integer profiles while fuller loadouts
can restore stronger Plate channels. A torso-only Breastplate has 40% coverage
and resolves its authored 3/3/1 profile as 2/2/1; the current Plate set covers
85% and restores 3/3/1. The current Mail loadout reaches 100% because its
Hauberk already spans torso, arms, and legs. Exact weights and rounding remain
playtest gates. Random hit locations still require separate animation,
messaging, balance, and monster-anatomy approval.

## Current item mapping

This table classifies the implemented catalog. Existing set pieces keep their
prior stats; the three merchant alternatives are new in the runtime-taxonomy
slice. “Set/loadout” follows [ITEM_TIERS.md](ITEM_TIERS.md); construction is the
item's physical build.

| Item(s)                         | Slot     | Coverage          | Current Guard | Weight | Construction   | Current set/loadout         |
| ------------------------------- | -------- | ----------------- | ------------: | -----: | -------------- | --------------------------- |
| `leather_helmet`                | head     | head              |             1 |      1 | leather        | Leather                     |
| `leather_armor`                 | chest    | torso             |             2 |      8 | leather        | Leather                     |
| `leather_gloves`                | hands    | hands             |             1 |    0.5 | leather        | Leather                     |
| `leather_pants`                 | pants    | legs              |             1 |      4 | leather        | Leather                     |
| `leather_boots`                 | boots    | feet              |             1 |      3 | leather        | Leather                     |
| `leather_belt`                  | belt     | —                 |             0 |      1 | none/accessory | Leather set accessory       |
| `chain_mail`                    | chest    | torso, arms, legs |             5 |     30 | mail           | Mail loadout                |
| `traveler_robe`                 | chest    | torso, arms, legs |             0 |      3 | none/clothing  | merchant clothing           |
| `padded_battle_robe`            | chest    | torso, arms, legs |             0 |      6 | padded         | merchant alternative        |
| `brigandine_coat`               | chest    | torso, arms       |             2 |     14 | hybrid         | merchant alternative        |
| `iron_helmet`                   | head     | head              |             2 |      4 | plate          | Mail loadout                |
| `iron_gauntlets`                | hands    | hands             |             2 |      2 | plate          | Mail loadout                |
| `iron_boots`                    | boots    | feet              |             2 |      6 | plate          | Mail loadout                |
| `breastplate`                   | chest    | torso             |             7 |     20 | plate          | Plate                       |
| `plate_helmet`                  | head     | head              |             3 |      5 | plate          | Plate                       |
| `plate_gauntlets`               | hands    | hands             |             3 |      3 | plate          | Plate                       |
| `plate_greaves`                 | pants    | legs              |             3 |      8 | plate          | Plate                       |
| `plate_boots`                   | boots    | feet              |             3 |      7 | plate          | Plate                       |
| `wooden_shield`, `raven_shield` | off-hand | —                 |          1, 2 | 6 each | shield         | independent shield track    |
| `ring_of_protection`            | ring     | —                 |             1 |    0.1 | accessory      | independent accessory track |

The current “Mail set” is therefore a progression loadout, not a pure mail
construction set: it combines a mail chest with plate extremity pieces. It also
has no dedicated pants item. Both facts are valid for the current loot plan but
must remain explicit when armor behavior is introduced.

Current full-loadout totals are:

- Leather body pieces: Guard 6, weight 16.5; the belt adds weight 1 and Guard 0.
- Mail loadout: Guard 11, weight 42.
- Plate set: Guard 19, weight 43.

Mail and Plate now have nearly equal armor-only weight but different Guard and
typed mitigation profiles. Their resulting progression remains a playtest gate.

Every functional primary chest body armor authors a complete
`slashProtection` / `pierceProtection` / `bluntProtection` triple. Construction
still identifies build, proficiency, repair family, and reporting; it no longer
silently supplies combat numbers.

The first armor-skill slice maps only the `leather_armor` primary chest to
Leather Armor. Protocol v24 adds the second mapping from the `chain_mail`
primary chest to Mail Armor. Protocol v25 maps the `breastplate` primary chest
to Plate Armor. Protocol v26 maps the Guard-0 `padded_battle_robe` primary chest
to Padded Armor because its construction supplies real physical mitigation.
Protocol v27 maps the `brigandine_coat` primary chest to Hybrid Armor. Other
leather pieces and every Plate extremity cannot activate or train a proficiency
by themselves; their functional body regions contribute to weighted aggregate
coverage. Garment form alone never activates a
skill: an ordinary coat would still require explicit body-armor construction
and `defenseSkill` metadata.

## Defense pipeline

### Implemented pipeline

```text
validate attack
→ d20 + attack bonus versus effective Guard
→ roll raw damage on hit
→ resolve authoritative physical damage type
→ apply the active primary chest item's authored protection profile
→ clamp every positive hit to at least 1 final damage
→ subtract final damage from HP
```

```text
effective Guard
= DEX-derived base Guard
+ every equipped item's Guard
+ active Shield skill bonus
+ active mapped primary-armor skill bonus
```

The active profiles are intentionally small:

| Primary item (construction) | Slash | Pierce | Blunt | Untyped |
| --------------------------- | ----: | -----: | ----: | ------: |
| Padded Battle Robe (padded) |     1 |      0 |     2 |       0 |
| Leather Armor (leather)     |     1 |      1 |     1 |       0 |
| Chain Mail (mail)           |     2 |      1 |     0 |       0 |
| Breastplate (plate)         |     3 |      3 |     1 |       0 |
| Brigandine Coat (hybrid)    |     2 |      2 |     2 |       0 |
| none                        |     0 |      0 |     0 |       0 |

`padded_battle_robe` moved from Guard 2 to Guard 0 when this profile became
active. The upstream economy rebalance keeps Leather Armor at Guard 2, Chain
Mail at Guard 5, and Breastplate at Guard 7; typed mitigation is an additional
item-authored channel on top of those tier baselines. This combined protection
is intentionally called out for playtesting rather than silently lowering the
upstream values during integration. The server publishes raw, mitigated, and
final damage in the combat outcome. There are still no random hit locations,
elemental resistances, casting penalties, stealth penalties, player mana, or
player stamina.

Mail's slash 2 / pierce 1 profile retains blunt as its deliberate weakness;
the mapped Chain Mail chest also activates Mail Armor. Plate provides broad
slash and pierce protection with blunt 1 as the relative weakness of rigid
plate; the mapped Breastplate chest also activates Plate Armor. Only the
equipped functional primary chest supplies mitigation and may activate an armor
skill; plate helmets, gauntlets, greaves, and boots retain their item Guard
without duplicating a chest profile or activating Plate Armor.

`brigandine_coat` migrated from Guard 4 to Guard 2. Its overlapping rigid plates
and textile backing form an authored Hybrid profile that mitigates slash,
pierce, and blunt by 2 each. The remaining Guard represents deflection while
the migrated points become reliable typed protection. The name and coat form
alone grant no armor behavior; the explicitly mapped Brigandine Coat activates
and trains Hybrid Armor.

### Target pipeline

```text
validate attack and snapshot authoritative equipment
→ resolve accuracy/evasion
→ resolve shield block or deflection
→ roll raw typed damage
→ union functional equipped body-armor coverage
→ scale the primary chest's authored physical profile
→ apply physical mitigation
→ apply resistances and magical wards
→ clamp and apply final damage to HP
→ apply durability wear from the resolved outcome
→ emit one authoritative outcome and progression event
```

This pipeline separates three defensive ideas:

- **Evasion/deflection** decides whether the attack lands. DEX-derived defense
  belongs here; selected armor and shields may contribute only when explicitly
  budgeted to do so.
- **Block** is an equipped-shield outcome, not a synonym for every miss.
- **Mitigation** reduces a landed hit according to construction and coverage.

During migration, Guard remains the visible hit target while mitigation is
introduced one construction at a time. Each item must have one documented stat
budget so its old Guard value is not simply retained at full strength and then
duplicated as mitigation.

Armor must not make every hit zero. The shared physical resolver therefore
caps mitigation below the positive raw hit and guarantees at least 1 final
damage. Boundary and monotonicity tests cover raw values 0 through 100. Future
stacking or percentage resistance still needs separately approved caps and
representative gameplay data.

## Damage and protection vocabulary

The physical damage vocabulary is deliberately small:

- **untyped** — neutral fallback for legacy, unknown, or non-physical metadata;
- **slash** — cutting attacks;
- **pierce** — concentrated points and projectiles;
- **blunt** — impact and crushing attacks.

Elemental and magical channels should be added only with the spells, monsters,
hazards, wards, and UI that consume them. “Magic” should not become one dumping
ground for unrelated fire, cold, poison, and spiritual effects.

Current weapon metadata maps swords and daggers to slash, spears to pierce,
and torches to blunt. An equipped monster weapon wins over its natural attack
type; SCP-939 supplies an explicit pierce natural attack; missing metadata
falls back to untyped. The server never infers a type from an item or monster
name.

Construction communicates a default identity, while each primary chest authors
its exact protection numbers:

- padded favors impact cushioning;
- leather provides modest balanced protection;
- mail strongly addresses cuts but transfers more impact;
- plate provides broad physical protection, with blunt remaining its relative
  weakness;
- hybrid combines rigid and cushioning layers for balanced protection above
  Leather's baseline.

All five current primary armor items have active numbers. Unknown or legacy
attacks use the documented untyped fallback and receive no current physical
mitigation.

Magical wards and elemental resistances are separate modifier channels. An
enchanted robe, protective ring, shield, or plate item may all grant a ward
without changing construction. The existing item-instance `enchant` field is
weapon-specific in code and persistence; it should not be repurposed for armor
without an approved generic modifier and migration design.

## Equipment burden and movement

The game continues to enforce total carried weight:

```text
bag weight + equipped weight <= STR × 15
```

Equipped items count toward the cap. Protocol v21 also separates equipped load
from bag contents and makes movement the first burden consumer:

- **carried weight** — whether the character can own/pick up the load;
- **equipped load** — how worn gear affects action performance;
- **construction burden** — rigidity, noise, heat, and casting interference
  that raw kilograms alone do not express.

The server compares equipped weight with the character's `STR × 15` carry
capacity and publishes the tier, equipped weight, capacity, and effective speed:

| Equipped load / capacity | Tier       | Movement speed |
| ------------------------ | ---------- | -------------: |
| ≤ 20%                    | Unburdened | 3.0 m/s (100%) |
| > 20% and ≤ 35%          | Light      |  2.7 m/s (90%) |
| > 35% and ≤ 50%          | Medium     |  2.4 m/s (80%) |
| > 50%                    | Heavy      |  2.1 m/s (70%) |

Bag weight affects pickup and ownership limits but never movement speed. All
equipped items count, including weapons, shields, accessories, and clothing;
quantity does not multiply an equipped item because equipment slots hold one
instance. At Strength 10, the current armor-only examples are:

| Loadout             | Equipped weight | Tier       |   Speed |
| ------------------- | --------------: | ---------- | ------: |
| Padded Battle Robe  |               6 | Unburdened | 3.0 m/s |
| Leather body pieces |            16.5 | Unburdened | 3.0 m/s |
| Mail loadout        |              42 | Light      | 2.7 m/s |
| Plate set           |              43 | Light      | 2.7 m/s |
| Brigandine Coat     |              14 | Unburdened | 3.0 m/s |

A weapon or shield can push any example into the next band, and higher Strength
raises the thresholds. The current Mail/Plate weight relationship is therefore
visible gameplay data and remains a deliberate playtest question.

The server uses the published speed for its movement budget. The browser uses
it for click and keyboard movement and scales acceleration/deceleration by the
same factor. The agent client uses it to pace path and chase steps and includes
the tier and cause in its world state. Equip, unequip, load, and reconnect all
refresh the value.

Construction burden—rigidity, noise, heat, and casting interference—remains a
separate future hook rather than an unreported multiplier. Possible later
consumers include dodge/evasion, stamina, stealth/noise, spell casting,
recovery time, and swimming. There is no general player stamina or mana system
today, so armor must not invent isolated armor-only resources.

The active movement consumer follows these rules:

1. compute it on the server from the authoritative equipment snapshot;
2. use gradual thresholds rather than hard class restrictions by default;
3. publish the effective value and cause to both clients;
4. validate movement or action timing against the same value the UI reports;
5. define how buffs, debuffs, skills, and temporary equipment changes compose.

This leaves room for a wizard to wear plate at a meaningful cost instead of
forbidding it solely by class, while still allowing later content to impose an
explicit requirement when justified.

## Durability, repair, and crafting foundation

Durability is an item-instance concern, not an item-definition-only stat. The
first vertical slice implements it for primary chest body armor:

- maximum durability and an explicit repair family on the armor definition;
- a positive `repairAmount` capacity on each finished repair-kit definition;
- current durability on each non-stackable item instance;
- authoritative wear triggers and a bounded loss formula;
- broken-item behavior that never destroys valuable gear without an explicit
  product decision;
- repair costs, material sinks, vendor/crafting ownership, and trade behavior;
- save migration and round-trip tests for bag, equipped, dropped, traded, and
  legacy items;
- tooltip, inventory, agent-state, and combat-outcome visibility.

`maxDurability` is definition data while `durability` travels with each item
through the bag, equipment, ground, resident trade, merchant buyback, and
database. Legacy rows use SQL `NULL`; login hydrates those rows to the current
definition maximum and the next save makes the value explicit. Values above a
later reduced maximum clamp safely on load.

An accepted landed monster hit wears the same functional chest instance from
the defense snapshot by one point. Misses, rejected requests, duplicate
cooldown requests, and gear swapped during resolution do not wear an item. At
zero condition, armor stays equipped and keeps its weight but contributes no
Guard, construction mitigation, or armor-skill activation. Four finished
products cover explicit repair families: Cloth repairs Padded, Leather repairs
Leather, Metal repairs Mail and Plate, and Hybrid repairs Hybrid. Their authored
capacities are +20, +30, +45, and +50 condition respectively. A matching kit is
consumed only when it raises damaged equipped chest armor by its capacity,
capped at the definition maximum. A mismatched kit, rejected request, or
already-full use keeps the product and condition unchanged.

Condition is labeled Pristine above 75%, Worn above 50% through 75%, Damaged
above 25% through 50%, Critical above zero through 25%, and Broken at zero.
These bands are shared by browser and agent presentation and do not add gradual
combat penalties: armor remains fully functional until Broken. Protocol v23
already carries the raw values, so the derived labels and definition-authored
capacity do not change the wire shape. Repairs are refused while defeated or in
combat, so a kit is maintenance rather than an instant defensive consumable.
Using a finished kit grants no skill XP; future crafting professions may produce
supplies without turning product use into a skill action.

NPC resale value is a separate smooth condition consumer. After the normal
sell-rate and haggling calculation, durable gear receives
`25 + floor(75 × current / max)` percent of that offer. Full gear therefore
keeps full value, Broken gear keeps a 25% salvage floor, and the presentation
bands do not create price cliffs. Merchant buyback records that final payout
exactly and returns the same instance condition, preserving a gold-neutral undo.
Resident transfers likewise preserve condition. Selling, buying back, and
repairing finished products do not train an armor, repair, appraisal, or
profession skill.

Shield wear, weapon wear, death wear, repair quality, and environmental wear
remain separate events and need separate approval. Crafting professions should
consume construction/repair metadata rather than identify items by string
prefixes.

## Appearance and equipment synchronization

Body armor currently has ground models and inventory presentation, but the
live `Player` snapshot broadcasts only main-hand identity and torch state.
`PlayerModel` attaches main-hand/off-hand objects; equipped head, chest, hands,
pants, boots, shirt, and back items are not synchronized or rendered on player
bodies.

A wearable-appearance phase therefore needs:

- a public equipment appearance snapshot plus equipment-change delta;
- one coordinated protocol-version bump for shared, browser, and agent clients;
- serialization round-trip coverage, especially because `Player` uses
  positional MessagePack encoding;
- local and remote rendering parity;
- a mapping from item appearance ids to rig-compatible wearable assets;
- deterministic layer order, clipping rules, gender/class body compatibility,
  hiding rules, and fallback visuals;
- semantic equipment data for agents even though they do not render meshes.

An item's existing ground `worldModel` is not automatically a wearable mesh.
Wearable assets must be validated on the supported character rigs before a
slot is presented as visually complete.

## Skills and proficiency

Padded Armor, Leather Armor, Mail Armor, Plate Armor, and Hybrid Armor are approved
construction-specific proficiencies. Each trains only when a server-approved
monster hit lands while its explicitly mapped primary chest is functional and
equipped. A hit grants 5 XP; a miss grants none. All five use the +0/+1/+2/+3
Guard bands once, separately from item Guard and Shield. Padded demonstrates
that physical mitigation can establish eligibility even when item Guard is 0.
The chest anchor and shared skill-to-construction mapping prevent mixed
extremity pieces from activating multiple armor skills from one attack.

No Light Armor, Heavy Armor, Tailoring, or Smithing skill is registered. Future
construction metadata must still be classified explicitly and does not promise
that every construction automatically receives a skill.

Any approved defensive proficiency must define:

- the exact eligible items through explicit metadata;
- the server-resolved action that earns XP;
- whether a miss, block, mitigated hit, durability event, or repair trains it;
- one owner for each outcome so Shield and armor cannot both claim the same
  defensive value accidentally;
- an effect that modifies one stage of the pipeline exactly once;
- anti-spam, weak-target, cooldown, and repeated-target rules;
- browser/agent observability and balance metrics.

Merely wearing armor, standing idle, drinking a potion, or sending repeated
packets is not a valid training action.

## Future authoring contract

The conceptual item-definition shape is shown below to make dependencies
visible. Fields enter runtime only in the phase that consumes and validates
them.

```text
existing:
  category, equipSlot, material, armorConstruction, equipmentKind,
  equipmentLayer, garmentForm, bodyCoverage, weight, guard, worldModel, chestTier,
  slashProtection, pierceProtection, bluntProtection

classification consumer:
  setId (only if set mechanics need it; loot documentation may be enough)

shared coverage consumer:
  body-region weights

combat consumer:
  wards/resistances

burden consumer:
  equippedBurden and/or construction traits

durability/repair consumer:
  maxDurability
  repairFamily
  repairAmount

crafting consumer:
  material requirements

appearance consumer:
  wearableAppearanceId
```

Validation must reject contradictory combinations—for example, a shield in a
ring slot or `equipmentKind: clothing` with unexplained physical mitigation—
while allowing intentional hybrids through explicit overrides.

## Staged implementation

### A. Architecture and catalog audit — completed

- Establish the independent taxonomy and classify current items.
- Record current technical boundaries and open decisions.
- This phase originally made no runtime, protocol, persistence, skill, or
  balance change.

### B. Runtime taxonomy and primary-layer content — completed

- `equipmentKind` and `equipmentLayer` classify every equippable item;
  `garmentForm` classifies every garment.
- `armorConstruction` covers real padded, leather, mail, plate, and hybrid body
  armor content and is consumed by boot validation, tooltips, and agent data.
- `traveler_robe`, `padded_battle_robe`, and `brigandine_coat` establish the
  clothing/robe, padded/robe, and hybrid/coat distinctions.
- Current garments use one `primary` slot layer. Chest robes and coats replace
  other chest gear, and tests verify that swapping them cannot leak a mapped
  armor skill or bonus.
- Generated-data parity tests cover server, browser, and agent definitions.

### C. Wearable appearance and layer UX

- Expose `shirt` and `back` only with shippable items and assets.
- Synchronize body equipment and render local/remote appearance.
- Update protocol, agent state, fallbacks, asset documentation, and smoke tests
  together.

### D. Physical damage and mitigation vertical slice

Completed profiles:

- weapons and natural attacks carry slash, pierce, blunt, or neutral untyped
  identities;
- the shared resolver produces authoritative raw, mitigated, and final damage;
- Padded supplies the first aggregate profile, migrated its robe's Guard, and
  did not register a skill during the mitigation phase;
- Leather supplies balanced typed protection, migrated one chest Guard point,
  and continues to compose with its existing proficiency without changing XP;
- Mail supplies cut-focused protection, migrated two chest Guard points, and
  did not register a skill during the mitigation phase;
- Plate supplies broad protection with a Blunt weakness, migrated three chest
  Guard points, and did not register a skill during the mitigation phase;
- Hybrid supplies balanced mixed-material protection, migrated two Brigandine
  Coat Guard points, and did not register a skill during the mitigation phase;
- protocol v20 publishes the type and mitigation breakdown to browser and
  agent clients;
- browser item tooltips and agent equipped-item summaries read the active
  protection values, so item behavior is visible before combat;
- deterministic, property, server-integration, generated-data parity, and
  client tests cover the slice.

Protection authoring is completed in phase O, and Phase Q adds body-region
identity. Weighted coverage remains a separate follow-up rather than an implied
combat behavior.

### E. Equipment burden consumer — movement completed

- Movement is the sole active consumer; no stamina, casting, stealth, or dodge
  penalties were added.
- Carried weight and equipped load are separate, and protocol v21 synchronizes
  equipped weight, capacity, tier, and effective speed.
- Server movement validation, browser prediction, agent pacing, UI, reconnect,
  and equipment changes consume the same value.
- Accessibility and class/build impact remain explicit playtest gates before
  adding another consumer or construction-specific modifiers.

### F. Durability, repair, and economy — first chest-armor slice completed

- Primary chest body armor has definition-authored maximum condition and
  per-instance remaining condition.
- Accepted landed monster attacks wear the resolved instance; broken armor is
  retained but loses Guard, mitigation, and proficiency activation.
- A consumed repair product provides the first authoritative repair and
  material sink.
- Nullable legacy migration plus bag, equip, loot, ground, resident trade,
  merchant buyback, and reconnect paths preserve condition.
- Browser and agent clients expose condition through protocol v23.
- Crafting links remain deferred until a real profession slice is approved.

### G. First armor proficiency slice — Leather Armor implemented

- The mapped leather chest activates one Leather Armor skill through the
  generic skill system; other leather pieces alone do not.
- Only landed, accepted monster hits train it, while Shield keeps its separate
  miss/hit training rule.
- The trained Guard term is applied once and publishes the final combined Guard
  after threshold changes.
- Mail, Plate, Padded, and Hybrid are approved later as phases K, L, M, and N;
  a different proficiency shape still requires separate review.

### H. Construction-aware repair economy — completed

- Every durable primary chest and every repair kit declares one validated
  `repairFamily`.
- Cloth repairs Padded, Leather repairs Leather, Metal repairs Mail and Plate,
  and Hybrid repairs Hybrid.
- Rica sells four separate products; they intentionally share the generic repair
  icon while names, materials, prices, tooltips, and agent summaries distinguish
  their purpose.
- The server checks the equipped armor family atomically before consumption, so
  mismatches preserve both the kit and existing damage.
- Finished-kit use grants no XP and does not register Smithing, Tailoring, or a
  generic repair skill. This slice owns family matching; capacity is the next
  completed slice below, while quality, crafted inputs, and profession ownership
  remain separate.

### I. Bounded repair capacity and condition UX — completed

- `repairAmount` gives each finished kit an explicit positive capacity: Cloth
  20, Leather 30, Metal 45, and Hybrid 50.
- A valid use applies `min(current + repairAmount, maxDurability)` atomically;
  one basic Metal kit therefore repairs the same amount of Mail or Plate rather
  than silently scaling to the target's larger maximum.
- Pristine, Worn, Damaged, Critical, and Broken bands use shared integer
  boundaries and appear beside raw condition in browser and agent summaries.
- Bands are informational. Guard, mitigation, and mapped proficiency activation stay
  unchanged at every positive condition and turn off only at Broken.
- Capped restoration, family mismatch, full-condition, combat, defeated, and
  no-XP behavior have server integration coverage. Kit quality tiers, crafting,
  and profession ownership remain unapproved.

### J. Condition-aware NPC valuation — completed

- A shared integer function maps durable items smoothly from a 25% Broken
  salvage floor to 100% value at full condition and clamps over-max input.
- The server applies condition after the ordinary NPC sell rate and haggled
  modifier; non-durable and legacy missing-condition items keep full value.
- The browser previews the authoritative formula per item, and agent prompts
  explain the same rule to merchants, residents, and customers.
- Merchant buyback stores the adjusted payout and exact durability, so undo is
  gold-neutral. Resident inventory transfers preserve the same instance state.
- The multiplier can only reduce a sale payout, so the existing merchant
  buy/sell anti-arbitrage invariant remains conservative. It awards no skill XP.

### K. Second armor proficiency slice — Mail Armor completed

- Protocol v24 adds `mail_armor` as a generic persisted/wire skill and maps only
  the `chain_mail` primary chest through explicit `defenseSkill` metadata.
- A shared skill-to-construction mapping validates Leather against Leather and
  Mail Armor against Mail; mismatched kind, slot, layer, construction, or Guard
  fails item-definition loading.
- Accepted landed monster hits grant 5 XP to the one mapped active armor skill;
  misses, rejected attacks, other constructions, and Broken Mail grant none.
- Mail Armor reuses the +0/+1/+2/+3 Guard ladder, authoritative combined-Guard
  updates, sparse persistence, aggregate defense metrics, browser UI, agent
  state, tooltip metadata, and durability/repair paths.
- Chain Mail keeps Guard 5, slash 2 / pierce 1 mitigation, weight 30, maximum
  condition 90, and Metal repair ownership. Plate extremities do not activate
  the skill; Plate, Padded, and Hybrid Armor are approved separately in phases
  L, M, and N.

### L. Third armor proficiency slice — Plate Armor completed

- Protocol v25 adds `plate_armor` as a generic persisted/wire skill and maps
  only the `breastplate` primary chest through explicit `defenseSkill` metadata.
- The shared mapping validates Plate Armor against Plate construction with the
  same kind, chest slot, primary layer, and positive-Guard requirements.
- Accepted landed monster hits grant 5 XP to Plate Armor while a functional
  Breastplate is active; misses, rejected attacks, extremity-only loadouts,
  other constructions, and Broken Plate grant none.
- Plate Armor reuses the +0/+1/+2/+3 Guard ladder, authoritative combined-Guard
  updates, sparse persistence, aggregate defense metrics, browser UI, agent
  state, tooltip metadata, and durability/repair paths.
- Breastplate keeps Guard 7, slash 3 / pierce 3 / blunt 1 mitigation, weight 20,
  maximum condition 120, and Metal repair ownership. Plate helmets, gauntlets,
  greaves, and boots remain stat-only; Padded and Hybrid Armor are approved
  separately in phases M and N.

### M. Fourth armor proficiency slice — Padded Armor completed

- Protocol v26 adds `padded_armor` as a generic persisted/wire skill and maps
  only the `padded_battle_robe` primary chest through explicit `defenseSkill`
  metadata. Robe remains a garment form; `traveler_robe` stays ordinary clothing.
- Armor-skill validation now accepts either positive Guard or real typed
  physical protection after kind, chest slot, primary layer, and exact
  construction checks. Shield validation still requires positive Guard.
- Accepted landed monster hits grant 5 XP to Padded Armor; misses, rejected
  attacks, ordinary robes, other constructions, and Broken Padded armor grant
  none.
- Padded Armor reuses the +0/+1/+2/+3 Guard ladder, authoritative combined-Guard
  updates, sparse persistence, aggregate defense metrics, browser UI, agent
  state, tooltip metadata, and durability/repair paths.
- Padded Battle Robe keeps Guard 0, slash 1 / blunt 2 mitigation, weight 6,
  maximum condition 40, and Cloth repair ownership. Hybrid Armor is approved
  separately in phase N.

### N. Fifth armor proficiency slice — Hybrid Armor completed

- Protocol v27 adds `hybrid_armor` as a generic persisted/wire skill and maps
  only the `brigandine_coat` primary chest through explicit `defenseSkill`
  metadata. Coat remains a garment form rather than a skill category.
- The shared mapping validates Hybrid Armor against Hybrid construction with
  the same body-armor kind, chest slot, primary layer, and protection checks.
- Accepted landed monster hits grant 5 XP to Hybrid Armor; misses, rejected
  attacks, ordinary clothing, other constructions, and Broken Hybrid armor
  grant none.
- Hybrid Armor reuses the +0/+1/+2/+3 Guard ladder, authoritative combined-Guard
  updates, sparse persistence, aggregate defense metrics, browser UI, agent
  state, tooltip metadata, and durability/repair paths.
- Brigandine Coat keeps Guard 2, slash 2 / pierce 2 / blunt 2 mitigation, weight
  14, maximum condition 100, and Hybrid repair ownership.

### O. Item-authored physical protection — completed

- `slashProtection`, `pierceProtection`, and `bluntProtection` move the five
  active primary-chest profiles from a hardcoded construction table into item
  definitions without changing any current Guard or mitigation value.
- Startup validation requires a complete triple with at least one positive
  channel on primary chest body armor and rejects protection fields on ordinary
  clothing, shields, accessories, or extremity pieces.
- Server combat snapshots the functional equipped chest's exact profile. A
  Broken, removed, or ordinary-clothing chest supplies zero mitigation.
- Browser tooltips and agent equipment summaries consume the same generated
  fields. Construction remains independent metadata for skills, repairs, and
  aggregate metrics rather than an implicit stat lookup.
- Shared boundary tests preserve the minimum-one-damage rule. Generated-data,
  server profile/combat, browser, and agent tests protect the authoring path.
  No protocol or persistence bump is required because combat outcomes already
  carry raw, mitigated, and final damage and item definitions are build data.

### P. Active defense profile observability — completed

- The agent client now retains the server-authored `GuardUpdated` value as
  state-only input and reports it as the exact hit target in world state. Guard
  refreshes do not wake the LLM or become duplicate event prose.
- The browser Character Stats pane continues to use that authoritative Guard
  and now presents the equipped primary body armor's name, mapped defense
  skill, and authored slash/pierce/blunt profile without requiring a tooltip.
- Empty chest slots and ordinary clothing report no physical armor profile. A
  Broken body armor profile remains identifiable but is marked inactive because
  it supplies no item Guard, physical mitigation, or armor-skill training.
- The browser projection is generated from the same item definitions and
  per-instance condition already received for inventory. It does not reproduce
  Guard math or resolve combat locally.
- Focused agent and browser store tests cover state-only Guard replacement,
  ordinary clothing exclusion, a functional Hybrid profile, and a Broken
  Padded profile. No combat balance, wire shape, persistence, or protocol bump
  is required.

### Q. Authored garment body coverage — completed

- Shared data defines `head`, `torso`, `arms`, `hands`, `legs`, and `feet` as
  stable body-region identities. `bodyCoverage` stores canonical semicolon-
  delimited region sets in item source data without adding them to the network
  protocol.
- All 17 current clothing/body-armor garments author coverage. Cuirasses are
  torso-only; helmets, gloves, leggings, and boots map to their matching region;
  Hauberk and Robe forms span torso/arms/legs; Brigandine Coat spans torso/arms.
- Server boot validation requires non-empty, unique, canonically ordered
  coverage on garments, rejects it on held/accessory gear, and checks each set
  against `garmentForm`.
- Browser tooltips and agent equipment summaries expose the same generated
  coverage. Ordinary clothing can cover regions without gaining construction,
  Guard, mitigation, durability, or an armor skill.
- Coverage remains structural metadata. It neither changes the current
  aggregate primary-chest mitigation nor activates skills on extremity pieces;
  weighted aggregate coverage and random hit locations remain separate,
  playtest-gated phases. No protocol, persistence, or combat balance bump is
  required.

### R. Deterministic weighted aggregate coverage — completed

- Shared Head/Torso/Arms/Hands/Legs/Feet weights total 100 and consume the
  union of functional equipped body-armor regions; overlap counts once.
- The active primary chest keeps ownership of construction, durability,
  proficiency, and authored slash/pierce/blunt values. Coverage ceiling-scales
  those values without creating a second armor skill or protection stack.
- Ordinary clothing, Broken items, held gear, and accessories contribute no
  coverage. Extremity armor contributes coverage but still cannot activate or
  train the chest-anchored proficiency.
- Current complete loadouts retain their existing effective profiles. A lone
  Breastplate resolves 3/3/1 as 2/2/1 until additional functional regions raise
  aggregate coverage.
- Browser Character Stats and agent world state show the weighted percentage
  and effective profile from synchronized inventory definitions. Combat remains
  server-authoritative and outcomes continue to publish actual mitigation.
- This phase does not add random hit locations, protocol fields, persistence
  state, construction-specific burden, or multi-layer occupancy.

### S. Coverage balance observability — completed

- The existing opt-in process-lifetime balance report records mitigation in
  fixed 0%, 1–49%, 50–74%, 75–99%, and 100% coverage bands.
- Each band contains only aggregate hit count plus raw, mitigated, and final
  damage totals. It stores no account, character, player, network, or
  per-character history.
- Coverage bands complement the existing physical-type and primary-construction
  breakdowns, allowing Breastplate, Hybrid, extended-chest, complete Plate, and
  complete Mail outcomes to be compared before weights or rounding change.
- Reporting stays disabled by default and requires the existing
  `SKILL_BALANCE_REPORT_SECS` opt-in. This phase changes no gameplay, protocol,
  persistence, client state, skill XP, or balance constant.

### T. Coverage-gap decision support — completed

- Browser Character Stats lists the canonical covered and missing regions
  beneath weighted percentage and effective slash/pierce/blunt protection.
- The agent world state carries the same ordered region sets so it can choose
  equipment for a real gap rather than infer coverage from an item name.
- Full coverage collapses to a clear `Full body coverage` state; partial and
  Broken profiles keep their exact gaps visible.
- Both clients derive this read-only presentation from synchronized inventory
  instances and generated item definitions. The server remains authoritative
  for coverage and mitigation during combat.
- This phase changes no gameplay, balance, protocol, persistence, item data,
  skill XP, or equipment occupancy.

### U. Item-level coverage contribution — completed

- Browser tooltips and agent equipment summaries show the weighted contribution
  of each body-armor item before it is equipped.
- Body armor uses `Armor Coverage: … (N% weight)`. Ordinary clothing uses
  `Garment Coverage: … (not defensive)` so a robe's anatomical reach cannot be
  mistaken for Guard, mitigation, or armor-skill eligibility.
- Shields, weapons, accessories, and other non-garments show no body-coverage
  line. The same shared/generated region definitions drive both clients.
- This phase changes no aggregate formula, combat result, balance, protocol,
  persistence, item data, skill XP, or equipment occupancy.

### V. Cross-client loadout contract — completed

The current progression loadouts are locked to one canonical matrix:

| Loadout            | Functional armor regions             | Coverage | Effective slash / pierce / blunt | Proficiency anchor                |
| ------------------ | ------------------------------------ | -------: | -------------------------------: | --------------------------------- |
| Traveler's Robe    | None; ordinary clothing only         |       0% |                 No armor profile | None                              |
| Padded Battle Robe | Torso, arms, legs                    |      75% |                        1 / 0 / 2 | Padded Battle Robe → Padded Armor |
| Leather set        | Head, torso, hands, legs, feet       |      85% |                        1 / 1 / 1 | Leather Armor → Leather Armor     |
| Mail set           | Head, torso, arms, hands, legs, feet |     100% |                        2 / 1 / 0 | Chain Mail → Mail Armor           |
| Plate set          | Head, torso, hands, legs, feet       |      85% |                        3 / 3 / 1 | Breastplate → Plate Armor         |
| Brigandine Coat    | Torso, arms                          |      55% |                        2 / 2 / 2 | Brigandine Coat → Hybrid Armor    |

- Server integration, browser-store, and agent-client tests now exercise this
  same matrix, including ordered covered/missing regions in both client
  projections.
- The contract makes the current mixed Mail set and Plate's uncovered arms
  explicit. A later item or balance change must update all three projections
  and this table together.
- This stabilization phase changes no runtime formula, item definition, combat
  result, balance, protocol, persistence, skill XP, or equipment occupancy.

Each phase needs an explicit owner approval and a completion gate. A later
phase may move earlier only as a complete vertical slice with its prerequisites;
the broad architecture is not permission to implement all subsystems at once.

## Required test gates for active phases

- Definition validation and generated JSON parity.
- Deterministic unit tests for classification and defense composition.
- Property/boundary tests: final damage never underflows, resistance caps hold,
  and increasing one protection rating cannot increase matching damage.
- Server integration tests for valid, rejected, duplicate, death, disconnect,
  equipment-change, and cooldown paths.
- Persistence migration and round-trip tests when item instances change.
- Protocol serialization/version tests for every wire change.
- Browser and agent-client state/outcome parity.
- A real smoke test for equip → combat/action → unequip → reconnect whenever a
  runtime phase lands.

## Open decisions

1. Does later wearable content justify an `outer` body layer, or should every
   robe continue using the current exclusive `primary/chest` rule?
2. Should the Mail progression loadout gain a dedicated leg item, or stay a
   mixed transitional set?
3. Do the 42-weight Mail loadout and 43-weight Plate set remain distinct enough
   once their Guard and typed mitigation are playtested together?
4. Does Hybrid's balanced mitigation and retained Guard create a clear enough
   tradeoff against lighter Leather and more deflective Plate in playtesting?
5. Do the provisional 10/40/15/5/20/10 region weights and ceiling rounding
   remain readable after playtesting, and does aggregate coverage remain
   sufficient without hit locations?
6. Do the 20% / 35% / 50% movement bands remain readable and fair across the
   full Strength range after live playtesting?
7. Are physical and magical armor enchantments item modifiers, crafted
   upgrades, affixes, or separate item definitions?
8. After comparing all five construction-specific progressions, should a future
   construction receive another dedicated skill or remain stat-only?

## Historical terminology references

The taxonomy uses broad historical construction distinctions as inspiration,
not as a simulation requirement. The Metropolitan Museum of Art describes
padded garments, mail, plate, and layered combinations in its overview of
[armor function](https://www.metmuseum.org/essays/the-function-of-armor-in-medieval-and-renaissance-europe),
and explains why “mail” is the precise construction term in its
[common-misconceptions guide](https://www.metmuseum.org/es/essays/arms-and-armor-common-misconceptions-and-frequently-asked-questions).
The Royal Armouries' [jack of plate](https://royalarmouries.org/objects-and-stories/up-close-online-exhibition/jack-of-plate)
is a useful example of why hybrid construction needs its own category.
