# Skill System Roadmap

This roadmap guides later work; it does not authorize future implementation.
Every phase requires implementation, automated validation, save/protocol
compatibility review, human/agent parity review, gameplay testing, balance
review, and explicit approval before the next phase.

## Phase 1 — One-Handed Sword Foundation

Deliver one trained combat skill, explicit item-to-skill metadata, use-based
XP, a modest accuracy advantage, persistence, existing Skills-tab integration,
server authority, browser/agent compatibility, tests, and documentation.

Completion gate: existing combat and Fishing do not regress; progression
survives reconnect/restart; unmapped weapons remain unchanged; rejected
requests cannot award XP.

## Phase 2 — Stabilization and Balance

Do not add another skill immediately. Measure attacks and time required for
levels 5, 15, 25, and 30; actual hit-rate improvement; character-level and
skill-level interaction; attack cadence; low-level farming; agent behavior;
skill-message frequency; save frequency; and database growth.

Possible adjustments include XP constants, accuracy thresholds, target
difficulty, weak-target eligibility, repeated-target diminishing returns,
debug metrics, cooldown tuning, and capped-skill UX. Difficulty scaling should
wait for observed Phase 1 behavior.

Completion gate: real gameplay data has reviewed the provisional formulas, no
obvious packet-spam or trivial-target exploit remains, and progression speed
has an approved target.

Current step: opt-in aggregate measurement is implemented and documented in
`doc/SKILL_BALANCE.md`. Balance constants remain unchanged until a representative
browser/agent gameplay session is reviewed. Phase 2 is not complete until the
completion gate above is met.

## Phase 3 — Validate with One Additional Weapon Skill

Add exactly one second weapon skill—Dagger or Spear—based on which has enough
current content. Reuse `weaponSkill`, generic persistence/messages, the Skills
tab, and combat-profile resolution. Add family-specific behavior only where
gameplay requires it; do not add every weapon category.

Completion gate: two skills use the same architecture without duplicated
persistence or UI, and sword-specific branches express only sword balance.

Completed step: Dagger was selected because it already has an item, merchant and
economy presence, a usable sword-model/`slash1` presentation, and compatible
melee behavior. It now uses the generic weapon-skill combat, persistence,
messages, UI, agent, metrics, and test paths. Phase 3 began with explicit
project-owner approval to override the still-open Phase 2 representative-data
gate; that gate still applies before balance tuning.

## Phase 4 — Expand Core Weapon Families Gradually

After the two-skill design is stable, consider one family at a time in an order
driven by available items, animations, ranges, and content. A possible order is
Dagger, Spear, Axe, Mace, Two-Handed Sword, Bow/Archery, then Unarmed Combat.

For each family, separately approve eligible items, range, hand usage,
accuracy, any later damage progression, animation needs, agent behavior,
tooltip behavior, and tests. Do not register skills for content that is not
usable yet.

Completion gate per family: at least one real item, correct animation/range,
authoritative calculation, persistence/UI, human/agent parity, and balance
tests.

Current step: Spear is the Phase 4 family. Its pinned model and `slash3` clip
support a measured 3-meter reach, 2.467-second cadence, and 1.060-second impact.
The item, authoritative server profile, browser presentation/chase, agent
behavior, persistence, metrics, and tests are wired through the generic skill
architecture. The still-open Phase 2 representative-data gate continues to
block balance tuning, not this explicitly approved vertical slice.

## Phase 5 — Defensive Combat Skills

After offensive skills stabilize, consider Shield, Parry, Dodge, Light Armor,
or Heavy Armor. Define the training action and XP timing, whether failed
defense trains, DEX/Guard/armor interactions, equipment requirements, event
ownership, and packet-abuse prevention. Do not count the same defensive value
through both Guard and skill.

Armor proficiency is gated by [ARMOR_SYSTEM.md](ARMOR_SYSTEM.md). Clothing,
padded, leather, mail, plate, hybrid construction, layers, garment form, set
identity, and equipment burden must not be collapsed into Light/Heavy merely to
create two skills. At least one real armor mechanic must produce a
server-resolved training event before an armor skill can be approved.

Completion gate: calculations remain server-authoritative, Guard remains
understandable, and outcomes remain readable.

First slice: Shield maps Wooden Shield and Raven Shield explicitly through
`defenseSkill`. Accepted server-resolved monster
misses award 10 XP and hits award 5 XP; all ownership, life, floor, range, and
cooldown gates run before the award. Its +0/+1/+2/+3 trained modifier is added
once after base Guard and equipment Guard, and the authoritative total is
pushed on join, equipment changes, and bonus thresholds. Persistence,
browser/agent state, metrics, tests, and protocol v17 reuse the generic skill
architecture.

Current armor slice: Leather Armor is construction-specific rather than a
Light/Heavy bucket. Runtime construction now covers padded, leather, mail,
plate, and hybrid content, but only the mapped leather chest activates the
skill. Kind, primary-layer occupancy, and garment form are also explicit.
Accepted landed monster hits award 5 XP; misses award none. Its +0/+1/+2/+3
trained Guard term is added once beside item Guard and any Shield term.
Persistence, browser/agent state, generated-data parity, metrics, tests, and
protocol v19 reuse the generic architecture. Protocol v20 later adds typed
physical damage plus Padded, Leather, Mail, Plate, and Hybrid mitigation as
combat rules, without registering Padded, Mail, Plate, or Hybrid skills or
changing Leather Armor training. The upstream tier baselines keep Leather Armor
at Guard 2, Chain Mail at Guard 5, and Breastplate at Guard 7 while construction
adds typed protection. Brigandine Coat uses Guard 2 with balanced protection
against all three typed physical
channels. Mail, Plate, Hybrid, Parry, and Dodge remain unapproved skills.

Protocol v21 later makes movement the first equipment-burden consumer. It uses
equipped weight against Strength-derived carry capacity and does not register
or train a skill. Bag weight, armor mitigation, and Leather Armor XP remain
independent channels.

Protocol v23 adds the first durability/repair economy slice without adding a
skill. Only functional primary chest body armor contributes Guard, mitigation,
or Leather Armor activation. Accepted landed monster hits wear the resolved
instance. Cloth, Leather, Metal, and Hybrid kits restore only matching armor
families, and finished-kit use grants no skill XP. Condition persists through
legacy migration, equipment, ground items, trades, buyback, and reconnect.
Smithing, Tailoring, repair quality, and durability skills remain unapproved.

## Phase 6 — Noncombat Skills

Reuse generic identity, persistence, messages, UI, and level cap for one
noncombat vertical slice before adding several professions. Candidates include
Mining, Lumberjacking, Cooking, Smithing, Tailoring, Alchemy, Healing,
animal-related skills, and trading/appraisal.

Action-specific rules stay in their systems: Mining XP comes from valid mining
actions, not the combat handler. Skills need not share one XP or effect formula.

Current step: Healing is the single Phase 6 slice and represents applying a
Bandage, not drinking a finished product. Only Bandages map through `useSkill`;
Healing Potions and fish keep their existing healing behavior without training.
A valid self-treatment adds +0/+1/+2/+3 HP by skill band and awards XP equal to
actual HP restored. Atomic item consumption prevents duplicate packets from
healing or training twice. Protocol v18, persistence, browser/agent state,
metrics, tests, and documentation reuse the generic skill architecture.
Magical healing, Alchemy, Mining, crafting, trading, and every other noncombat
family remain unapproved.

## Phase 7 — Optional Skill Governance

Only consider total caps, per-skill increase/lock/decrease controls, decay,
respecialization, trainers, or transfers after many skills create a demonstrated
build/economy problem. Ultima-inspired governance remains optional.

Before implementation, decide whether all skills can be mastered, whether
specialization helps, how agents manage controls, how decreases avoid surprise,
how existing characters migrate, and what happens above a new cap.

Completion gate: separate approved design, migration plan, UI and agent design,
and persistence-compatibility tests.

## Phase 8 — Mastery, Abilities, and Content

Only after core skills stabilize, consider techniques, passive mastery,
stances, combos, skill-gated equipment, trainer/mastery quests, recipes, rare
resource access, titles, achievements, or visual effects. Established skills
may unlock these rewards; advanced rewards must not define the foundation or
make level, attributes, equipment, and player decisions irrelevant.

## Phase 9 — Long-Term Operations and Maintenance

Add only justified operational support: aggregate balance metrics,
progression-time reports, admin inspection and safe correction tools, database
migration tests, protocol notes, balance-version docs, regression coverage,
abuse monitoring, and character-restore procedures. Do not collect unnecessary
personal player data.

## Development rule

```text
Design one vertical slice
→ implement it
→ test it
→ play it
→ review balance
→ document lessons
→ approve the next slice
```

Do not pre-fill enums, database fields, UI rows, or generic frameworks for
unproven future content. Grow the system from demonstrated gameplay needs.
