# Music Assets

앰비언트 트랙 목록은 `data-src/bgm.csv`가 단일 진실 소스다 (제목·파일명·재생
시간). 웹 클라이언트가 플레이리스트와 `/play_music` 재생에 쓰고, 서버가 곡
제목을 해석하며, 오디오가 없는 agent-client는 재생 시간으로 연주가 끝나는
시점을 안다. 전투 BGM은 여기에 없다 — 클라이언트(`bgmManager.ts`) 전용이라
`/play_music`으로 부를 수 없다.

## BGM

suno.com에서 다음의 프롬프트로 생성

### Field BGM

- Winds of the Open Plain

  ```
  game bgm something like "Stones" in Ultima series. Medieval instruments(lute, recorder, harp, strings, percussion, brass), melodic, easy melody to memory. Something like that makes you to want to go adventures, vast open plains, endless oceans, rugged mountain ranges. hopeful and grand.
  ```

- Dawn Over the Kingdom, Beyond the Horizon

  ```
  A sweeping medieval fantasy RPG overworld theme inspired by "Stones" from the Ultima series. Begin with a gentle, memorable melody on solo lute or recorder over soft harp arpeggios — evoking a peaceful village at dawn. Gradually layer in strings and light percussion as the melody builds, conveying the feeling of stepping out into vast open plains and endless oceans. The music swells with noble brass and full orchestral strings as the adventurer crosses rugged mountain ranges in search of treasure. The mood is hopeful and grand — a sense of the heart expanding, chest opening wide with wonder and possibility. Melodic, singable theme that lingers in memory. Warm, nostalgic, yet epic. No vocals.
  ```

- Quiet Longing, Twilight Fields

  ```
  game bgm something like "Stones" in Ultima series. Medieval instruments(lute, recorder, harp, strings, percussion), melodic, easy melody to memory. lonely, little bit sad, peaceful, calm
  ```

- Wanderer of the Old Fields (1), Lonely Roads of Eldoria

  ```
  Medieval fantasy field BGM, Ultima-style, minor key, melancholic and bittersweet,
fingerpicked lute, haunting recorder melody, harp shimmer, strings swelling with quiet grandeur,
soft percussion, lonely wanderer, vast open world, seamless loop
  ```

- Lonely Steppe of Ages, Lonely Steppe of Ages (1)

  ```
  Medieval fantasy field BGM, Ultima-style, minor key, melancholic and bittersweet,
haunting memorable melody, quiet grandeur, lonely wanderer crossing a vast open world,
seamless loop, game BGM
  ```

- Hearthside Respite, Hearthside Respite (1)

  ```
  Warm and relaxed medieval tavern music. Gentle lute fingerpicking sets a cozy mood. Soft fiddle plays a slow, sweet melody. Harp adds delicate touches. Feels like resting by the fireplace with a mug of ale after a long journey. Calm and inviting. No vocals.
  ```

- Triumphal Procession, Triumphal Procession (1)

  ```
  Epic orchestral score with bold brass fanfare, sweeping strings, and thunderous percussion, Opens with brilliant trumpets and French horns over timpani swells, then locks into a ceremonial march guided by snare, Middle section softens into harp arpeggios and warm woodwinds, then surges back into a fuller, more heroic reprise that climbs into a massive, shining crescendo to close, harp, soft, soaring, orchestral
  ```

- Crescendo of Remembering

  ```
  Cinematic orchestral build: intimate felt-piano and warm synth pads bloom into nostalgic strings, then widen with harp arpeggios and airy choir aahs before cresting in a full-ensemble, brass-driven climax with heroic choir and cymbal swells, ending in a tender, dissolving echo, nostalgic, orchestral, gentle, soaring, soft, emotional, harp
  ```

- Sky of Burning Wings, Sky of Burning Wings (1)

  ```
  Blazing 80s-style anime score at high tempo: gated drums and a piercing synth lead open in unison, then electric guitar and string stabs carry a heroic melody, Arrangement swells with counter-melodies, modulated key changes, and tom fills that kick into half-time hits before racing back, Final act erupts into full orchestra with soaring brass fanfares, cymbal swells, and a massive choir carrying the main theme in stacked harmonies, ending on a held, radiant chord, orchestral, bright, anime
  ```

- Festival in the Lantern Square

  ```
  dance, Bright medieval jig for game BGM; nimble lute carries the main melody with sprightly recorder answering in call-and-response phrases, Fiddle adds soaring embellishments, harp fills arpeggiated warmth under a steady hand-drum pulse and accented tambourine shakes, Arrangement cycles through two main themes with a lifted middle section; energy builds as more instruments join, then eases into a cozy reprise for looping in a lively town hub, warm, joyful, theme, medieval, harp
  ```

- Ruins Beneath the Black Moon

  ```
  Slow-burning orchestral score in a minor key: low strings and dark cello lead with a narrow, tense motif, joined by hushed, clustered choir harmonies that swell and recede, Subtle metallic percussion taps and distant hits mark the creeping advance, with eerie aleatoric whispers in the upper strings, Arrangement gradually thickens, then drops to a hushed, torchlit pulse before a final, ominous ascent, slow, theme, low, medieval, cello
  ```

- Dawn Watch at the Gate

  ```
  Bright orchestral march in a major key with bold trumpet fanfare, proud fife carrying the main melody, and warm arpeggiated lute underneath, Steady, moderate snare and field drum cadence keeps a heroic march feel, with occasional cymbal swells for sunrise moments, Secondary brass and horn countermelodies swell in the B section, then fall back to a simple, earnest reprise, Perfect loop point where the final cadence lands softly on strings and lute before the fanfare returns, bright, theme, warm, medieval, uplifting, simple
  ```

- Shadow Over the Village

  ```
  Dark orchestral score built around heavy war drums and thunderous timpani, dissonant low brass clusters, and tremolo strings rising in waves, Slow, stalking tempo that ramps aggressively, with chromatic descents in the orchestra under ominous bell motifs, Guttural low choirs blend into growling basses as ground-shaking footsteps enter, torches suggested by flickering high strings and eerie metallic hits, Final section swells into a crushing wall of sound that cuts abruptly, leaving a lingering low drone, theme, orchestral, medieval, deep, low
  ```

- Sunlit Shopfront Waltz

  ```
  folk, Warm acoustic lute-led fantasy waltz with gentle harp arpeggios, soft breathy recorder, and light pizzicato strings; tempo sways in a tender 3/4, Verses feel like shy, halting steps, with small pauses and rubato; main theme blossoms into a hopeful, singable hook that returns with subtle variations, Recorder carries the crush-melody in the upper register while lute and harp trade countermelodies; strings add playful, bouncing heartbeats and a nostalgic swell in the final refrain, Ideal for a cozy RPG town scene focused on innocent first love, wistful yet quietly joyful, medieval, playful, romantic, soft, acoustic, theme, nostalgic, light, beautiful, gentle, harp, heartfelt, warm
  ```

### Battle BGM

- Blood and Bronze, Blood and Bronze (1), Drums of Valor

  ```
  High-intensity medieval battle score, 160+ BPM, Explosive percussive stabs into rolling war drums, aggressive lute ostinatos, piercing recorder lead, and bright harp arpeggios weaving countermelodies, Constant layered tension, tight looping eight-bar structure with evolving drum accents and heroic yet desperate harmonic shifts; mix is upfront and dry for maximum impact in combat, piercing, medieval, harp
  ```

### Google Flow Music BGM (.m4a)

아래 .m4a 파일들은 [Google Flow Music](https://www.flowmusic.app/)으로 생성했다.

**Field BGM**

- Echoes of the Horizon
- Echoes of the Old Realm
- First Light Waltz
- Path of the Eternal Quest
- Shadows of the Citadel
- The Gallant Expedition
- The Gilded Tankard Jig
- The Hearth's Welcome
- The Silent Frontier
- Visions of the Realm
- Visions of the Realm (Epic Orchestral Version)
- Wanderer's Solitude

**Battle BGM**

- Clash of the Iron Realm
- Radiant Vanguard Overdrive
- The Abyssal March
- Triumph of the Vanguard
