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

- sword-leather.ogg, sword-miss3.ogg — predate this file; provenance not
  recorded here. Mapped in `data/material-impact-sounds.json`.
- sword-flesh4.ogg — sword cutting into a fleshy monster (`metal` → `flesh`:
  ogre, troll, scp939), generated with [ElevenLabs Sound
  Effects](https://elevenlabs.io/sound-effects) via the API on 2026-08-22
  (Starter tier, own generation; original
  `sword-flesh4.ogg_take5_2026-08-22.mp3` kept in `~/assets_original/sfx/` on
  pc5090; prompt: "A sword blade cutting deep into a huge fleshy monster: a
  heavy wet meaty thud with a thick tearing of flesh. Impact only, no voice,
  no music."). Take 5 of a 0.7 s request, kept full length with a 100 ms tail
  fade, peak-normalized to −3 dB, 44.1 kHz mono.
  2026-08-24 오거 타격음이 너무 커서 게인 ×0.9 (−0.9 dB, peak −2.5 dB) — flesh 재질 전부에 걸린다.
- sword-stone.ogg — sword striking a stone golem (`metal` → `stone`),
  generated with [ElevenLabs Sound Effects](https://elevenlabs.io/sound-effects)
  via the API on 2026-08-22 (Starter tier, own generation; original
  `sword-stone.ogg_take9_2026-08-22.mp3` kept in `~/assets_original/sfx/` on
  pc5090; prompt: "A steel sword blade striking a stone golem: a hard metallic
  clang ringing off solid rock with a short spray of stone chips and grit.
  Impact only, no voice, no music."). Take 9 of a 0.7 s request, kept full
  length with a 100 ms tail fade, peak-normalized to −3 dB, 44.1 kHz mono.

- sword-flesh.ogg, sword-flesh2.ogg, sword-miss.ogg, sword-miss2.ogg —
  **[미사용]** earlier takes, removed 2026-08-19.
- sword-flesh3.ogg — **[미사용]** replaced by sword-flesh4.ogg on 2026-08-22.

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

The eleven cries below were generated with [ElevenLabs Sound
Effects](https://elevenlabs.io/sound-effects) via the API on 2026-08-22
(Starter tier, own generations). Each original mp3 is kept on pc5090 as
`~/assets_original/sfx/<ogg name>_take<N>_2026-08-22.mp3`; each ogg is cut
to the length given with a 100 ms tail fade, peak-normalized to −3 dB, 44.1
kHz mono.

- orc-death.ogg — orc. Take 9, 1.0 s request trimmed to 0.55 s. "The short,
  guttural death roar of an orc warrior cut down by a sword in battle"
- orc-female-death.ogg — female orc. **2026-08-24 재생성** — 첫 버전(take 1, "The short
  death cry of a female orc warrior struck down by a sword in battle")이 남자
  오크와 거의 같은 음높이였다. 오크 묘사를 넣으면 계속 남성 톤이 나와서 프롬프트를
  "A woman's very short, sharp, high-pitched death scream, a single shrill
  feminine cry less than half a second long, cut off instantly as a sword
  strikes her down. Clearly a female human-like voice, high soprano pitch, no
  growl, no male voice, no music."로 바꾸고 0.6 s·prompt_influence 0.6으로
  5테이크 뽑아 take 5 채택 (스펙트럼 중심 3.6 kHz, 이전 1.8 kHz). 0.5 s에서
  100 ms 테일 페이드. 원본 `orc-female-death_take5hi2_2026-08-24.mp3`
- hobgoblin-death.ogg — hobgoblin. **2026-08-24 재생성** — 첫 버전("harsh,
  barking death cry")이 개 짖는 소리처럼 들려서 "The harsh, guttural death cry
  of a hobgoblin soldier felled by a sword: a rough snarling humanoid war-cry
  choking off as he falls. Voice only, no dog, no barking, no animal, no
  music."(0.8 s·influence 0.6)으로 4테이크 뽑아 take 2 채택, 0.7 s에서 100 ms
  테일 페이드. 원본 `hobgoblin-death_take2_2026-08-24.mp3`
- gnoll-death.ogg — gnoll. Take 1, 1.0 s request trimmed to 0.85 s. "The
  yelping, hyena-like death howl of a gnoll cut down in battle"
- bugbear-death.ogg — bugbear. Take 2, 1.0 s request trimmed to 0.70 s. "A big
  shaggy bear-like goblin monster's death cry: a snarling guttural growl
  breaking into a pained yelp as it is cut down. Animal voice only, no drums,
  no percussion, no music."
- ogre-death.ogg — ogre. Take 4, 1.0 s request trimmed to 0.90 s. "A huge
  brutish ogre monster's deep guttural death groan, a hoarse animal voice
  choking off as it collapses from a sword wound. Voice only, no horns, no
  music." 2026-08-24 너무 커서 게인 ×0.9 (−0.9 dB, peak −3.4 dB).
- troll-death.ogg — troll. Take 6, 0.6 s request trimmed to 0.50 s. "The
  drawn-out, rasping death roar of a troll dying from a deep sword wound"
- stone-golem-death.ogg — stone golem. Take 1, 1.0 s request trimmed to 0.65
  s. "A stone golem crumbling apart, grinding rock and falling rubble as it
  dies"
- orc-boss-death.ogg — orc chieftain (orc_boss). Take 2, 1.0 s request trimmed
  to 0.70 s. "The furious, booming death roar of a massive orc chieftain
  falling in battle"
- ogre-boss-death.ogg — ogre warlord (ogre_boss). Take 9, 0.6 s request
  trimmed to 0.55 s. "A giant ogre warlord monster's deep roaring death cry, a
  massive hoarse beast voice breaking into a groan as it falls. Voice only, no
  horns, no music."
- scp939-death.ogg — scp939. Take 4, 0.6 s request trimmed to 0.45 s. "The
  wet, distorted death shriek of a fleshy eyeless monster, unnatural and
  wrong"

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

- player-death-female.ogg — the cry a female character lets out as she is
  struck down; every player nearby hears it. Take 5, 0.7 s request trimmed to
  0.68 s. "A young female warrior's short agonized death scream as a sword
  strikes her down, a single piercing cry cut off abruptly as she falls. Human
  voice only, no music, no reverb."
- player-death-male.ogg — male counterpart. Take 4, 1.2 s request trimmed to
  1.00 s. "A young male warrior's short agonized death scream as a sword
  strikes him down, a single hoarse cry cut off abruptly as he falls. Human
  voice only, no music, no reverb."

Both were generated and processed exactly like the monster cries above.

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
