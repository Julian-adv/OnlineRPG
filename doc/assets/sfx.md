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

- sword-*.ogg — predate this file; provenance not recorded here.
