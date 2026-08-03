# Fishing

Cast a rod at water, wait for the bite, hook in time, land the fish. The first
gathering profession, and the first consumer of the trained-skill system
(`shared/src/skills.rs`). Server-authoritative end to end: every timer, roll
and outcome lives in `server/src/game_state/fishing.rs`; clients render
broadcasts and answer with `FishingRespond`.

## The loop

```
FishingCast ─► Casting (1 s) ─► Waiting (4–12 s, skill-shortened)
                                    │ bite rolls the fish (species/size/trophy)
                                    ▼
                              Bite (2.5 s + 0.5 s latency grace)
                              │ Hook in time      │ too late / never
                              ▼                   ▼
                            Fight              Escaped
              (continuous reel/give-line tension sim)
                              │ exhaust + reel in  │ line snaps / hook thrown
                              ▼                    ▼
                           Caught               Escaped
```

**Getting a rod:** buy a Fishing Rod from a general merchant (Rica stocks it
for 3 silver — a starter tool between a torch and a potion) and equip it in
the main hand.
Rods are excluded from dungeon treasure chests — they are bought tools, not
endgame combat loot (`server/src/item_defs.rs::equipment_ids_with_min_price`).

- **Cast** (`FishingCast { position }`): needs a fishing rod in the main hand
  (`category == "fishing_rod"`), the overworld floor, a target within 8 m, and
  **water**. Water is `waterSurfaceY − terrainBed > 0.1 m` at the target,
  sampled server-side from the baked **unified water field** (WFD1, sea +
  rivers) via `terrain::WaterSampler` alongside the terrain `HeightSampler`.
  This is true over the **ocean** (surface at sea level, bed below) AND over
  **rivers** (the carved channel surface sits above its bed even high in the
  hills — a river bed bottoms out at sea level and climbs, so the older
  "terrain height < 0" test wrongly rejected every inland river). On land the
  water surface collapses below the terrain, so `depth ≤ 0` and the cast is
  refused with a direct `FishingError`. Sea-only tiles have no baked water
  file; they sample as flat sea level, matching the client's synthesis.
- **Wait**: uniform 4–12 s, shortened 2% per fishing level (floored at half
  the minimum). The fish — species, size, trophy — is rolled *at the bite*,
  not at resolution, but only revealed on a catch.
- **Bite** (`FishingBite` broadcast): the bobber dips. `Hook` must arrive
  within 2.5 s plus 0.5 s latency grace — judged against the server's own
  clock, so a laggy-but-in-time click is never punished and a hacked client
  can't stretch the window. Hooking *early* (before the bite) scares the fish
  off. The reaper tick allows one extra grace period before declaring an
  unanswered bite escaped, so a response racing the deadline is judged by the
  handler, not the tick.
- **End** (`FishingEnded { outcome }` broadcast): `Caught { item_def_id,
  size_cm, trophy }`, `Escaped`, or `Aborted`. A caught fish arrives through
  the normal `InventoryUpdated` (its own slot), or spills as a ground item when the
  bag can't take the weight — never silently lost. Moving, attacking,
  disconnecting, dying, stowing the rod (unequipping it, or swapping a
  weapon into the main hand), or `FishingStop` aborts the session; gear
  changes that leave the rod in hand — a hat, an off-hand torch — don't
  break concentration.

Timers advance on a 250 ms server tick (`run_ticks` in `main.rs`) using
`tokio::time::Instant`, so the whole state machine is tested with paused time
(`server/src/game_state/tests.rs`, `fishing_tests`).

## The catch table

Anything with a `catchWeight` in `data-src/items.csv` can end up on the
hook — fish (`category: "fish"`), junk flotsam, and coin catches alike.
The catch columns:

| column | meaning |
|---|---|
| `rarityTier` | fish: 1 (common) … 5 (legendary); junk/coins: 0 — drives XP and skill weighting |
| `catchWeight` | relative weight in the catch table at fishing level 0 |
| `minFishingLevel` | fishing level a species is locked behind (blank = 0) |
| `sizeDice` | rolled length in cm (e.g. `6d8`) |
| `trophyCm` | fish only — length at or above this is a trophy |

Species pick: weighted draw over two pools. A fish's weight grows
`RARITY_SKILL_BONUS_PCT` (3%) per level per rarity tier — multiplicative, so
skill closes the gap on rare fish but can never invert the table's order.
Flotsam holds a flat `FLOTSAM_SHARE_PCT` (20%) of the draw at every level,
so junk never thins out as the fish pool grows. `minFishingLevel` locks a
species until the angler earns it: salmon at 10, golden sturgeon at 20.
Size: `sizeDice`,
plus a d20 quality roll; a natural 20 doubles the size and — for fish —
is always a trophy. Trophies are a fish concept: a nat-20 Old Boot is
just a very large boot, no celebration.

Fish are sellable (`basePrice`, ordinary merchant flow) and edible —
`category "fish"` maps to the same `Heal(dice)` use-effect as potions. Every
catch takes its own bag slot (`stackable false`, like the equipment rows), so
the bag reads as a catch log. Size is deliberately **not stored on the item**;
it lives only in the catch announcement.

Prices are anchored to the game's *income* economy, not just the catalog:
monster kills drop unsellable worn weapons by design, so the repeatable gold
faucets are coin piles (1–10c) and gated dungeon chests — and an NPC's
salary is 50s/day. Fish: minnow 10c, perch 25c, trout 60c, salmon 2s,
golden sturgeon 15s (the jackpot, a goblin-sword's worth — 1.7% of draws even
at the level-30 cap). With the flotsam rows in the table, the expected *sell*
value of one catch runs ~8c at level 0 to ~24c at the level-30 cap — a couple of coin
piles, so an hour of active fishing earns roughly half a guard's daily salary.
Steady pocket money, not a money printer. Final tuning is explicitly the
maintainer's call. That band is a **contract test**
(`item_defs::tests::expected_catch_value_stays_in_the_coin_pile_economy_at_every_level`):
it sweeps every fishing level, and fails if the per-catch EV leaves 5–25c, if
skill ever makes an angler poorer, or if the cap earns more than 4x level 0 —
mastery should pay a better wage, not open a different economy.

## Flotsam (junk & coin catches)

Not everything that bites is a fish. Four flotsam rows share the catch
table (a flat 20% of draws at every level): an **Old Boot** and a **Clump of Kelp**
(worthless bag junk — the classic fishing gag), a **Message in a Bottle**
(sells for a token 15c), and a **Sunken Coin Pouch**
(`category: "coin_catch"` — it lands in the bag sealed like any other
catch; opening it via `use_item` (double-click in the bag) rolls its
`dice` column, `3d8`, pays the copper to the wallet through the same
path as ground coin piles, and the combat log reports the amount). All are `rarityTier 0`: **no fishing XP** (the
`10·rarity²` formula grants nothing naturally), no trophy, and in the
fight they pull and tire like a common fish (`rarity.max(1)` clamps pull and
stamina). An *escaped* junk catch
still pays the flat 2 XP consolation — the species is never revealed on
an escape, and a varying consolation would leak the hidden roll. Junk
keeps the bite/struggle stakes honest without inflating income — the EV
guardrail above counts flotsam in its average.

## Skill

Catches grant fishing XP: `10 × rarity²` (10 for a minnow, 250 for a golden
sturgeon); a hooked fish that escapes consoles with 2. Fishing grants **no
character XP** — combat balance is untouched. Level effects today: shorter
waits, better rare weights, and drag control in the fight (`1%` less pull
tension and `1%` faster reeling per level, pull relief capped at 30%).

## Client

- Click water with a rod equipped → `cast_fishing` intent
  (`managers/inputHandler.ts`; water = the baked `WaterFieldManager.surfaceAt`
  sits >0.1 m above the clicked terrain, so both ocean and rivers cast while
  dry ground still walks) → stop, face the water, send (`PlayerControl.svelte`).
  The server re-validates, so the client check only decides cast-vs-walk.
- `components/FishingBobber.svelte`: every nearby angler's bobber (broadcasts
  are radius-gated), gentle idle bob, hard dip on bite. The float stays
  hidden through the cast swing + flight and splashes down on the same
  schedule as the splash sound. A sagging white line connects the angler's
  rod tip to the float. During the fight it chases the fish's broadcast
  position (client-side smoothing — the 4 Hz beats are never snapped to) and
  wears a splash of droplets whose intensity follows the fish's remaining
  stamina — bystanders read the whole fight from the water.
- `components/FishingPrompt.svelte`: SPACE, any click, or a wheel flick to
  hook (clicks are captured before the canvas, so a hasty click can't walk
  the angler and abort the session), then the fight HUD — fish-state line,
  tension gauge, and two hold-to-act stance buttons. REEL: hold the button,
  hold SPACE, or wheel down (winding toward you); GIVE LINE: hold the
  button, hold S, or wheel up (wheel inputs are short bursts). ESC reels in.
  Combat-log lines narrate cast/bite/outcome.
- State in `stores/fishingStore.ts`; server messages handled in
  `network/messageHandlers.ts`.

## Agent parity

Agents speak the same protocol: `FishingBite` carries everything needed to
respond, and the windows (2.5 s + grace) are sized for an agent-client's
network round trip as much as for human reflexes — no mechanic requiring
reactions only software can deliver, none too fast for software either.

The agent-client implements this as a reflex layer (`src/state.rs`): it
auto-hooks its own bites and plays each `FishingFight` beat through the
shared `auto_stance` policy (resending only on change) — mechanically, like
its A* movement layer, while the LLM makes the decisions via two actions:
`{"type": "fish", "x": …, "z": …}` (coordinates optional — omitted means
"just ahead") and `{"type": "stop_fishing"}`. Outcomes come back to the
model as `[Fishing]` events; in-flight messages are classified as noise so
they cost no LLM calls. Reflex speed confers no real advantage: the
simulation advances 4 times a second regardless, and one beat of stance lag
is well inside what the tuning absorbs.

## The fight

Hooking is only the start: the fight is a continuous tug-of-war simulated on
the server's 250 ms tick (constants in `shared/src/fishing.rs`, pure step in
`server/src/game_state/fishing.rs::step_fight`). The fish alternates
**Running** bursts (2–3.5 s, longer for rarer fish) and shorter **Resting**
breathers (0.8–2 s) — most of the fight is spent under pressure;
the angler holds one of three stances, changed any time via
`FishingRespond`: **reel**, **give line**, or **hold**.

- **Tension** (the gauge; snaps at 100, `Escaped`): a Running fish pulls
  `(20 + 2·rarity)/s`, scaled up to 1.3× by how much line is out and down by
  skill; reeling adds 14/s (more than the 8/s rest decay — the reel can
  never simply be held); giving line sheds 44/s (always more than the
  strongest pull). The hook-set itself opens the fight at 30 — deliberately
  hot: an unanswered run leaves the safe range in about a second and snaps
  the line in a few.
- **Distance** (shown, not numbered: the bobber *is* the fish): runs take
  ~1.1–1.5 m/s of line, reeling takes it back (1.6 m/s vs a Resting fish,
  0.6 against a run, 2.5 when Exhausted). The fish wanders but stays within
  6 m of the cast point, and can never be reeled past the session's **line
  floor**: the cast handler walks the player→cast ray (0.5 m steps, the same
  tiles the cast validation touched — the tick stays IO-free) to find where
  fishable water starts, and the fight clamps to that plus 0.4 m, at least
  the rod's 2 m reach. The exhausted reel-in also steers the fish back onto
  the cast ray, so it comes home along the line whose waterline was actually
  measured — the float stays on the water instead of climbing the shore.
- **Stamina** (hidden from the gauge; read it from the splash): only drag
  burns it — Running under ≥20 tension costs `2 + 12·(tension/100)²` per
  second; the square means timid mid-band play barely tires the fish and
  real progress comes from riding the gauge near the top, while a slack
  line lets a Resting fish *recover*. Pools are `38 + 12·rarity`.
- **Endgame**: at 0 stamina the fish goes **Exhausted** — reel it down to
  the line floor (within 0.3 m) and it lands (`Caught`). A lively fish
  dragged within 1 m of the floor panics into a fresh run instead, so only
  a spent fish can ever be landed. A fight
  that outlives 60 s throws the hook (`Escaped`): slack-line stalling is not
  a strategy, and neither is walking away (unmanaged tension snaps within
  seconds).

Every beat is broadcast as `FishingFight { bobber, fish_state, tension_pct,
stamina_pct }` — public information by design, which is what keeps humans
(reading gauge and splash) and agent-clients (running the shared
`auto_stance` policy) on equal footing. Trophy catches are celebrated to
everyone in delivery radius via the `FishingEnded` broadcast they already
receive.

## Deliberate limits

- No bait, no rod tiers, no designated fishing spots (any water — ocean or
  river — works).
- Animations are in: a Mixamo cast plays once on `FishingCasted`, then a
  fishing idle loops until the line comes in (`fishing.glb` pack, local
  player only — remote anglers still read through the bobber). SFX are in:
  the line whirs out on the swing, the splash lands with the bobber a second
  later, plop on bite, reel click when the reel stance engages, line snap on escape,
  jingle on catch (CC0 packs except the contributor-original cast whir —
  see `assets/sfx.md`; self-only, matching the combat sound precedent).
