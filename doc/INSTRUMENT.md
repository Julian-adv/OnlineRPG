# Live Instrument Performance

`/play_instrument` or **Social → Play Instrument** starts the interactive instrument mode. The server accepts it only for a living, world-ready player who owns an item in the `instrument` category. The existing `guitar_playing` animation and project-owned mandolin model provide the performance pose.

## Keyboard

| Register | Keys              | Notes                   |
| -------- | ----------------- | ----------------------- |
| High     | `Q W E R T Y U I` | C5 D5 E5 F5 G5 A5 B5 C6 |
| Middle   | `A S D F G H J`   | C4 D4 E4 F4 G4 A4 B4    |
| Low      | `Z X C V B N M`   | C3 D3 E3 F3 G3 A3 B3    |

A key sounds once per press and rearms on release. Mouse and touch use the same latch. Up to four voices sound at once; a fifth replaces the oldest, and retriggering the same performer's note replaces its previous voice. Escape stops the session.

## Audio

The 22 notes use twelve-tone equal temperament at A4 = 440 Hz. Their measured one-shot durations live in `client/src/lib/data/instrumentNotes.ts`. Audio is synthesized locally with a damped plucked-string model and procedural room impulse, so the feature adds no third-party sound assets.

Local notes sound immediately. The client groups audience events for 250 ms and sends note indexes with offsets from the first event. The server validates the batch (at most 16 notes), snapshots the authoritative performer position and relays it only to players on the same floor within 30 m — skipping listeners who blocked the performer. Receivers replay the relative offsets and apply the distance curve in `instrumentAudio.ts`.

## Session rules

Live instrument state is separate from `/play_music`, although both reuse the same visual pose. Starting either mode replaces the other. Movement, attacking, a landed hit, death, equipment changes, an opened trade, Escape and disconnect end the live performance. The playable panel also locks movement, scene clicks and camera controls while it is open.

The playlist BGM yields to free play, like it does to a bard NPC's earshot: it fades out while the local panel is open, and while notes from a nearby performer are heard (released 10 s after the last one, so a dramatic pause does not hand the speakers back mid-tune). Battle music and `/play_music` keep their higher rank.
