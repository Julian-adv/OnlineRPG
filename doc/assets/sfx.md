# Sound Effect Assets

Short one-shot effects in `client/public/sounds/` (plain `.ogg`, not LFS).
All effects route through `sfxManager.ts` so the Settings SFX slider/mute
applies — never construct `Audio` elements elsewhere.

## Fishing

- fishing-cast.ogg — original synthesis by the contributor (band-passed noise
  sweep, no sampled material); owned outright, contributed under the CLA
- fishing-splash.ogg — `splash_03` from [40 CC0 water splash & slime SFX](https://opengameart.org/content/40-cc0-water-splash-slime-sfx) by rubberduck (CC0)
- fishing-plop.ogg — `bubble_02` from the same pack (CC0)
- fishing-reel.ogg — ratchet built from 4× `click_004` layered at 55 ms, [Kenney Interface Sounds](https://kenney.nl/assets/interface-sounds) (CC0)
- fishing-snap.ogg — `pluck_001` from [Kenney Interface Sounds](https://kenney.nl/assets/interface-sounds) (CC0)
- fishing-catch.ogg — `jingles_PIZZI06` from [Kenney Music Jingles](https://kenney.nl/assets/music-jingles) (CC0)

Everything below the cast sound is CC0 as credited. All six are trimmed,
peak-normalized to ≈ −3 dB, with a short tail fade.

## Combat

- sword-leather.ogg, sword-flesh3.ogg, sword-miss3.ogg — predate this file;
  provenance not recorded here. Mapped in `data/material-impact-sounds.json`.
- sword-flesh.ogg, sword-flesh2.ogg, sword-miss.ogg, sword-miss2.ogg —
  **[미사용]** earlier takes, removed 2026-08-19.

## Monsters

- kobold-death.ogg — kobold death groan, generated with
  [ElevenLabs Sound Effects](https://elevenlabs.io/sound-effects) on
  2026-08-22 (free tier, own generation; original
  `The_short_groan_a_ko_#4-1787326141343.mp3` kept in `~/assets_original/sfx/`
  on pc5090; prompt: "The short groan a kobold makes when it dies"). The
  original holds two takes; only the first (0.60–1.05 s) is kept, with a
  150 ms tail fade, peak-normalized to −3 dB, resampled 48→44.1 kHz.

- goblin-death.ogg — goblin death cry (used by both goblin and goblin_boss),
  generated with [ElevenLabs Sound Effects](https://elevenlabs.io/sound-effects)
  on 2026-08-22 (free tier, own generation; original
  `The_sound_of_a_gobli_#2-1787399405795.mp3` kept in `~/assets_original/sfx/`
  on pc5090; prompt: "The sound of a goblin dying from an opponent's sword
  during battle"). Trimmed to 0.62 s (trailing silence) with a 90 ms tail fade,
  peak-normalized to ≈ −3 dB, resampled 48→44.1 kHz.

## Players

- player-hurt-female.ogg — female character's cry when a hit lands on her (any
  class), generated with
  [ElevenLabs Sound Effects](https://elevenlabs.io/sound-effects) on
  2026-08-22 (free tier, own generation; original
  `A_female_warrior_let_#1-1787398743608.mp3` kept in `~/assets_original/sfx/`
  on pc5090; prompt: "A female warrior lets out a short groan after being
  struck by an opponent's sword during battle"). Trimmed to 0.24 s (trailing
  silence) with a 70 ms tail fade, peak-normalized to −3 dB, resampled
  48→44.1 kHz.

- player-hurt-male.ogg — male counterpart, same generator and date (original
  `A_male_warrior_lets__#3-1787399204599.mp3` kept alongside; same prompt with
  "female" → "male"). Trimmed to 0.50 s (trailing silence) with an 80 ms tail
  fade, peak-normalized to ≈ −3 dB, resampled 48→44.1 kHz.

## Props

- crate-break.ogg — sword hitting a wooden crate, generated with
  [Verse8](https://create.verse8.io/?chat=jksong3%2F3d-sword-box-interaction)
  on 2026-08-19 ([original ogg](https://agent8-games.verse8.io/0x11e0427b8e50fcb8deda5fde7395c208018a7b89/mcp-uploads/static-assets/audio-548d2c36-7909-4673-ac67-e6d509b3ab33.ogg),
  own generation, paid credits). The original has two hits; only the second
  (1.70–2.60 s) is kept, with a 150 ms tail fade and −3 dB gain to match
  sword-leather's impact level.
- chest-open.ogg — wooden chest lid opening, generated with
  [ElevenLabs Sound Effects](https://elevenlabs.io/sound-effects) on
  2026-08-19 (free tier, own generation; original
  `The_sound_of_an_old__#3-1787144916360.mp3` kept in `~/assets_original/sfx/`
  on pc5090; prompt: "The sound of an old treasure chest slowly creaking as it
  opens"). Trimmed to 0.95 s (trailing silence) with a 150 ms tail fade,
  −3 dB gain, resampled 48→44.1 kHz. Replaces a Verse8 take from the same day.
- coin-spill.ogg — gold coins pouring out of the chest, generated with
  [ElevenLabs Sound Effects](https://elevenlabs.io/sound-effects) on
  2026-08-19 (free tier, own generation; original
  `The_sound_of_hundred_#2-1787144560968.mp3` kept in `~/assets_original/sfx/`
  on pc5090; prompt: "The sound of hundreds of gold coins slithering out of a
  treasure chest"). Trimmed to 1.8 s (trailing silence) with a 150 ms tail fade,
  −1.5 dB gain, resampled 48→44.1 kHz.

## World

- dungeon-roar.ogg — the roar that wakes far below when sunset resets the
  dungeons, generated with
  [ElevenLabs Sound Effects](https://elevenlabs.io/sound-effects) on
  2026-08-21 (free tier, own generation; original
  `The_sound_of_monster_#3-1787310281519.mp3` kept in `~/assets_original/sfx/`
  on pc5090; prompt: "The sound of monsters waking up and howling deep within
  the dungeon"). Trimmed to 4.64 s with a 150 ms tail fade, −3 dB gain,
  resampled 48→44.1 kHz.
