# Skill Balance Review

Phase 2 established measurement for the One-Handed Sword vertical slice before
changing its provisional rules. Phases 3 and 4 reuse the same no-identity
metrics for Dagger and Spear. Phase 5 extends the same process-lifetime report
with aggregate Shield defense outcomes. Phase 6 adds Bandage treatment and
actual-HP-restored measurements. The first armor slice adds Leather Armor to
the same per-defense-skill breakdown. The Padded, Leather, Mail, Plate, and
Hybrid physical-mitigation profiles also record aggregate raw, mitigated, and
final damage by physical type and primary construction, without player identity.

## Baseline

The current cumulative XP thresholds and the Sword/Dagger 1.533-second attack
window imply these uninterrupted-combat bounds:

| Target | XP | All misses, 5 XP | All hits, 10 XP | All kills, 20 XP |
|---|---:|---:|---:|---:|
| Level 5 | 5,500 | 1,100 / 28.1m | 550 / 14.1m | 275 / 7.0m |
| Level 15 | 124,000 | 24,800 / 10.6h | 12,400 / 5.3h | 6,200 / 2.6h |
| Level 25 | 552,500 | 110,500 / 47.1h | 55,250 / 23.5h | 27,625 / 11.8h |
| Level 30 | 945,500 | 189,100 / 80.5h | 94,550 / 40.3h | 47,275 / 20.1h |

Travel, target acquisition, death, and downtime make real progression slower.
The table is a bound, not an approved progression target.

Spear uses the same XP awards with a 2.467-second window, so its corresponding
uninterrupted bounds are about 1.61× longer. Review Spear-only and mixed-weapon
sessions separately: the report pairs per-skill outcomes with the aggregate
observed cadence distribution.

Shield uses the same cumulative curve. A monster miss awards 10 XP and a hit
awards 5 XP, so level 5 takes 550 avoids or 1,100 hits taken; level 15 takes
12,400 or 24,800; level 25 takes 55,250 or 110,500; and level 30 takes 94,550
or 189,100. Wall-clock projections depend on each monster's attack cooldown and
real engagement time, so the report records defense counts and XP instead of
pretending there is one defensive cadence.

Leather Armor awards 5 XP only on landed hits while the mapped leather chest
is equipped. Its uninterrupted hit-count bounds therefore match Shield's
all-hit column: 1,100 hits for level 5, 24,800 for level 15, 110,500 for level
25, and 189,100 for level 30. Misses award nothing, and the trained Guard bonus
itself lowers later hit frequency, so these are lower bounds. When Shield and
Leather Armor are both active, the aggregate `defense` count represents two
qualified skill events for one swing; use `defense_skills` for per-skill review.

Healing earns one XP per HP actually restored by applying a Bandage. A Bandage
rolls `2d4` (2–8, average 5) before the modest +0/+1/+2/+3 trained modifier,
but max-HP clamping can reduce any use to as little as 1 XP. At the level-0
average and with enough missing HP, level 5 is about 1,100 Bandages, level 15
about 24,800, level 25 about 110,500, and level 30 about 189,100. These are
economic/resource bounds rather than time projections; actual progress depends
on wounds and Bandage supply. Healing Potions retain their independent `6d4`
effect and never contribute Healing XP.

## Aggregate server report

Enable periodic reports with either:

```text
--skill-balance-report-secs 300
```

or:

```text
SKILL_BALANCE_REPORT_SECS=300
```

Zero, the default, disables reporting. A final report is emitted during a
graceful shutdown when reporting is enabled.

The `skill_balance` log line contains:

- total attack requests, resolved attacks, and rejection reasons;
- cooldown rejections for packet-spam and cadence review;
- supported weapon-skill attacks, hits, kills, awarded XP, and observed hit rate;
- a per-skill breakdown for One-Handed Sword, Dagger, and Spear;
- weapon-skill results by skill band: 0–4, 5–14, 15–24, and 25–30;
- observed hit rate for each character-level and skill-band pair;
- results against targets at least five levels weaker, within four levels, or
  at least five levels stronger;
- browser, CLI-agent, and other-client aggregate counts;
- aggregate results by configured monster type;
- accepted attack cadence samples;
- `SkillXpGained` message count and newly created weapon-skill rows;
- Shield and Leather Armor qualified defenses, hits taken, avoids, awarded XP,
  per-skill/skill-band results, `SkillXpGained` messages, and newly created
  defensive-skill rows;
- mapped Bandage uses, actual HP restored, awarded XP, skill-band
  results, `SkillXpGained` messages, and newly created Healing rows;
- physical hits plus raw, mitigated, and final damage totals, broken down by
  damage type and primary armor construction;
- successful periodic, logout, and shutdown skill-save batches and rows;
- observed attacks and time projected to levels 5, 15, 25, and 30.

Reports contain no account name, character name, player ID, network address,
or per-character history. Metrics live only for the server process lifetime.

## Review procedure

Collect a session with both browser and agent clients, multiple character and
skill bands, and weak, peer, and strong monsters. Keep the raw report lines
with the balance review. Before tuning, review at least:

1. observed hit rate in each skill band;
2. weak-target share of attacks and XP;
3. cooldown rejections relative to all requests;
4. browser and agent attack cadence;
5. projected progression time from the observed XP mix;
6. XP-message rate and successful skill-save rows per hour;
7. Shield avoid/hit mix, XP per accepted defense, and whether one controllable
   monster can produce trivial low-risk progression;
8. Leather Armor hit frequency, progression with its rising Guard bonus,
   Shield overlap, and whether primary-chest anchoring remains understandable;
9. Healing XP per Bandage, capped-heal frequency, Bandage expenditure, and
   whether repeated tiny wounds dominate legitimate recovery.
10. Padded raw-to-final damage by slash, pierce, blunt, and untyped; compare its
    migrated Guard budget with unarmored and future construction candidates.
11. Leather's balanced typed mitigation, reduced chest Guard, hit frequency,
    skill progression, and Shield overlap compared with Padded and no armor.
12. Mail's slash-heavy mitigation, retained Guard, extreme carried weight, and
    lack of skill progression compared with Leather and Plate.
13. Plate's broad mitigation, reduced chest Guard, Blunt weakness, and lack of
    skill progression compared with Mail and Hybrid.
14. Hybrid's balanced mitigation, migrated Brigandine Guard, and lack of skill
    progression compared with lighter Leather and more deflective Plate.
15. Equipment-burden time-to-target by Strength, complete loadout, and movement
    tier; especially Mail's weight 67 versus Plate's weight 43 and the effect of
    adding a weapon and shield. Bag-only weight must remain a zero-speed-effect
    control.

XP constants, accuracy thresholds, target scaling, diminishing returns, and
cooldown behavior remain unchanged until the data supports a specific change
and the progression target is approved.
