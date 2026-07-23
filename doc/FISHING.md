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
                           Caught              Escaped
```

- **Cast** (`FishingCast { position }`): needs a fishing rod in the main hand
  (`category == "fishing_rod"`), the overworld floor, a target within 8 m, and
  **water** — terrain height below 0 at the target, sampled server-side via the
  `terrain` crate's `HeightSampler`. Sea level is fixed at Y=0
  (`doc/WATER_SYSTEM.md`); this is the server's first gameplay use of that
  fact. Failures answer with a direct `FishingError`.
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
  the normal `InventoryUpdated` (stacked), or spills as a ground item when the
  bag can't take the weight — never silently lost. Moving, attacking,
  disconnecting, dying, or `FishingStop` aborts the session.

Timers advance on a 250 ms server tick (`run_ticks` in `main.rs`) using
`tokio::time::Instant`, so the whole state machine is tested with paused time
(`server/src/game_state/tests.rs`, `fishing_tests`).

## The catch table

Fish are ordinary item defs (`data-src/items.csv`) with `category: "fish"`
plus four fish-only columns:

| column | meaning |
|---|---|
| `rarityTier` | 1 (common) … 5 (legendary); drives XP and skill weighting |
| `catchWeight` | relative weight in the catch table at fishing level 0 |
| `sizeDice` | rolled length in cm (e.g. `6d8`) |
| `trophyCm` | length at or above this is a trophy |

Species pick: weighted draw where each fish weighs
`catchWeight + fishing_level × rarityTier` — skill shifts the table toward
rare fish without ever emptying the commons. Size: `sizeDice`, plus a d20
quality roll; a natural 20 doubles the size and is always a trophy. (Trophy
celebration UI/notice lands with the struggle minigame; the flag already
rides `FishingEnded` so the shape won't change.)

Fish are stackable, sellable (`basePrice`, ordinary merchant flow), and
edible — `category "fish"` maps to the same `Heal(dice)` use-effect as
potions. Size is deliberately **not stored on the item** so fish stay
stackable commodities; it lives only in the catch announcement.

## Skill

Catches grant fishing XP: `10 × rarity²` (10 for a minnow, 250 for a golden
carp); a hooked fish that escapes consoles with 2. Fishing grants **no
character XP** — combat balance is untouched. Level effects today: shorter
waits, better rare weights. The struggle minigame will add wider response
windows.

## Client

- Click water with a rod equipped → `cast_fishing` intent
  (`managers/inputHandler.ts`; underwater terrain = hit point below −0.05 so
  shoreline clicks still walk) → stop, face the water, send
  (`PlayerControl.svelte`).
- `components/FishingBobber.svelte`: every nearby angler's bobber (broadcasts
  are radius-gated), gentle idle bob, hard dip on bite.
- `components/FishingPrompt.svelte`: status line + SPACE to hook, ESC to reel
  in. Combat-log lines narrate cast/bite/outcome.
- State in `stores/fishingStore.ts`; server messages handled in
  `network/messageHandlers.ts`.

## Agent parity

Agents speak the same protocol: `FishingBite` carries everything needed to
respond, and the windows (2.5 s + grace) are sized for an agent-client's
network round trip as much as for human reflexes — no mechanic requiring
reactions only software can deliver, none too fast for software either. The
agent-client's auto-hook reflex and `fish()` MCP tool land in a follow-up
(the pattern is its existing local A* layer).

## Deliberate limits (v1)

- Single-round hook — the ArcheAge-style tension struggle is the next PR
  (purely additive protocol).
- No bait, no rod tiers, no designated fishing spots (any water works).
- Cast/idle animations reuse existing clips; dedicated Mixamo clips and
  splash/reel SFX come with the polish pass.
