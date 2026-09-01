<script lang="ts">
  import { fly } from 'svelte/transition'
  import { SvelteMap } from 'svelte/reactivity'
  import {
    INSTRUMENT_NOTES,
    type InstrumentNote,
  } from '../data/instrumentNotes'
  import { gameStore } from '../stores/gameStore'
  import {
    clearInstrumentPressedNotes,
    closeInstrumentPanel,
    instrumentPanelVisible,
    instrumentPressedNotes,
    setInstrumentNotePressed,
  } from '../stores/instrumentStore'
  import { mountOverlay } from '../stores/overlayStack'
  import { inputHandler } from '../managers/inputHandler'
  import {
    playInstrumentNote,
    stopInstrumentPerformer,
  } from '../managers/instrumentAudio'
  import {
    INSTRUMENT_NOTE_BY_CODE,
    InstrumentKeyLatch,
    InstrumentNoteBatcher,
    type InstrumentNoteEvent,
  } from '../managers/instrumentInput'
  import { networkManager } from '../network/socket'
  import { isTypingTarget } from '../utils/dom'
  import { emoteStopRequest } from '../stores/emoteStore'

  const KEY_ROWS = [
    ['KeyQ', 'KeyW', 'KeyE', 'KeyR', 'KeyT', 'KeyY', 'KeyU', 'KeyI'],
    ['KeyA', 'KeyS', 'KeyD', 'KeyF', 'KeyG', 'KeyH', 'KeyJ'],
    ['KeyZ', 'KeyX', 'KeyC', 'KeyV', 'KeyB', 'KeyN', 'KeyM'],
  ] as const

  const NOTES_BY_CODE = new Map(
    INSTRUMENT_NOTES.map((note) => [note.keyCode, note])
  )
  const NOTE_ROWS = KEY_ROWS.map((row) =>
    row
      .map((code) => NOTES_BY_CODE.get(code))
      .filter((note): note is InstrumentNote => !!note)
  )

  const ROW_META = [
    {
      register: 'HIGH',
      range: 'C5 — C6',
      clef: '𝄞',
      accent: '151, 231, 255',
    },
    {
      register: 'MIDDLE',
      range: 'C4 — B4',
      clef: '𝄢',
      accent: '246, 202, 121',
    },
    {
      register: 'LOW',
      range: 'C3 — B3',
      clef: '𝄢',
      accent: '91, 236, 201',
    },
  ] as const

  const SOLFEGE: Readonly<Record<string, string>> = {
    C: 'DO',
    D: 'RE',
    E: 'MI',
    F: 'FA',
    G: 'SO',
    A: 'LA',
    B: 'TI',
  }

  function solfege(note: InstrumentNote): string {
    return SOLFEGE[note.name[0]] ?? note.name
  }

  function scaleDegree(note: InstrumentNote): number {
    return (note.index % 7) + 1
  }

  function upperOctaveMark(note: InstrumentNote): string {
    if (note.name.endsWith('6')) return '••'
    if (note.name.endsWith('5')) return '•'
    return ''
  }

  function lowerOctaveMark(note: InstrumentNote): string {
    return note.name.endsWith('3') ? '•' : ''
  }

  let latch: InstrumentKeyLatch | null = null
  let batcher: InstrumentNoteBatcher | null = null
  let strikeBursts = $state<Record<number, number>>({})
  const strikeTimers = new SvelteMap<number, ReturnType<typeof setTimeout>>()

  function burstNote(note: number) {
    const token = (strikeBursts[note] ?? 0) + 1
    strikeBursts[note] = token
    const previous = strikeTimers.get(note)
    if (previous) clearTimeout(previous)
    strikeTimers.set(
      note,
      setTimeout(() => {
        if (strikeBursts[note] !== token) return
        delete strikeBursts[note]
        strikeTimers.delete(note)
      }, 760)
    )
  }

  function clearStrikeBursts() {
    for (const timer of strikeTimers.values()) clearTimeout(timer)
    strikeTimers.clear()
    strikeBursts = {}
  }

  function performerId(): number | string {
    return $gameStore.currentPlayer?.id ?? 'local-instrument'
  }

  function sendBatch(events: readonly InstrumentNoteEvent[]) {
    networkManager.sendInstrumentNotes(events)
  }

  function strike(code: string, repeated = false) {
    const note = latch?.press(code, repeated)
    if (note === null || note === undefined) return
    setInstrumentNotePressed(note, true)
    playInstrumentNote(note, performerId())
    burstNote(note)
    batcher?.add(note)
  }

  function release(code: string) {
    latch?.release(code)
    const note = INSTRUMENT_NOTE_BY_CODE.get(code)
    if (note !== undefined) setInstrumentNotePressed(note, false)
  }

  function stop() {
    if (!$instrumentPanelVisible) return
    batcher?.flush()
    stopInstrumentPerformer(performerId())
    inputHandler.clearTransientInput()
    emoteStopRequest.set(true)
    networkManager.sendStopInteraction()
    closeInstrumentPanel()
  }

  function claimGameplayKey(event: KeyboardEvent): boolean {
    if (event.ctrlKey || event.altKey || event.metaKey) return false
    if (isTypingTarget(event.target)) return false
    if (event.code !== 'Escape' && !INSTRUMENT_NOTE_BY_CODE.has(event.code)) {
      return false
    }
    event.preventDefault()
    event.stopImmediatePropagation()
    return true
  }

  $effect(() => {
    if (!$instrumentPanelVisible) return

    const sessionLatch = new InstrumentKeyLatch()
    const sessionBatcher = new InstrumentNoteBatcher(sendBatch)
    latch = sessionLatch
    batcher = sessionBatcher
    inputHandler.clearTransientInput()

    const onKeydown = (event: KeyboardEvent) => {
      if (!claimGameplayKey(event)) return
      if (event.code === 'Escape') {
        stop()
        return
      }
      strike(event.code, event.repeat)
    }
    const onKeyup = (event: KeyboardEvent) => {
      if (!claimGameplayKey(event)) return
      release(event.code)
    }
    const onBlur = () => {
      sessionLatch.clear()
      clearInstrumentPressedNotes()
      clearStrikeBursts()
      inputHandler.clearTransientInput()
    }

    window.addEventListener('keydown', onKeydown, true)
    window.addEventListener('keyup', onKeyup, true)
    window.addEventListener('blur', onBlur)
    const unmountOverlay = mountOverlay('instrument', stop)

    return () => {
      window.removeEventListener('keydown', onKeydown, true)
      window.removeEventListener('keyup', onKeyup, true)
      window.removeEventListener('blur', onBlur)
      unmountOverlay()
      sessionLatch.clear()
      sessionBatcher.dispose(false)
      if (latch === sessionLatch) latch = null
      if (batcher === sessionBatcher) batcher = null
      stopInstrumentPerformer(performerId())
      clearInstrumentPressedNotes()
      clearStrikeBursts()
      inputHandler.clearTransientInput()
    }
  })
</script>

{#if $instrumentPanelVisible}
  <section
    class="instrument-panel"
    aria-label="Mandolin keyboard"
    transition:fly={{ y: 18, duration: 220 }}
  >
    <img
      class="instrument-ornament"
      src="/textures/ui/instrument/mandolin-ornament.webp"
      alt=""
      aria-hidden="true"
    />

    <header class="panel-head">
      <div class="instrument-title">
        <span class="eyebrow">FREE PLAY</span>
        <div class="title-line">
          <h2>Mandolin</h2>
          <span class="subtitle">THREE OCTAVES · 22 NOTES</span>
        </div>
      </div>

      <div class="head-actions">
        <button class="close" type="button" title="Stop playing" onclick={stop}>
          <span aria-hidden="true">×</span>
          <span class="sr-only">Stop playing</span>
        </button>
      </div>
    </header>

    <div class="performance-bed">
      {#each NOTE_ROWS as row, rowIndex (row[0].index)}
        <div
          class="instrument-row"
          style={`--accent:${ROW_META[rowIndex].accent}`}
        >
          <div class="register-label">
            <span class="clef" aria-hidden="true"
              >{ROW_META[rowIndex].clef}</span
            >
            <span class="register-copy">
              <strong>{ROW_META[rowIndex].register}</strong>
              <small>{ROW_META[rowIndex].range}</small>
            </span>
          </div>

          <div class="note-grid">
            {#each row as note (note.index)}
              <div
                class="note-cell"
                class:active={$instrumentPressedNotes.has(note.index) ||
                  !!strikeBursts[note.index]}
              >
                <button
                  class="note-orb"
                  class:pressed={$instrumentPressedNotes.has(note.index)}
                  type="button"
                  aria-label={`${note.name}, ${solfege(note)}, keyboard ${note.key}`}
                  aria-pressed={$instrumentPressedNotes.has(note.index)}
                  onpointerdown={(event) => {
                    event.preventDefault()
                    event.stopPropagation()
                    event.currentTarget.setPointerCapture(event.pointerId)
                    strike(note.keyCode)
                  }}
                  onpointerup={(event) => {
                    event.preventDefault()
                    release(note.keyCode)
                  }}
                  onpointercancel={() => release(note.keyCode)}
                  onlostpointercapture={() => release(note.keyCode)}
                  onkeydown={(event) => {
                    if (
                      !event.repeat &&
                      (event.key === 'Enter' || event.key === ' ')
                    ) {
                      event.preventDefault()
                      strike(note.keyCode)
                    }
                  }}
                  onkeyup={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault()
                      release(note.keyCode)
                    }
                  }}
                >
                  {#if strikeBursts[note.index]}
                    {#key strikeBursts[note.index]}
                      <span class="cross-flare" aria-hidden="true"></span>
                      <span class="sound-ring" aria-hidden="true"></span>
                      <span class="rising-note" aria-hidden="true">♪</span>
                    {/key}
                  {/if}
                  <span class="upper-mark">{upperOctaveMark(note)}</span>
                  <span class="degree">{scaleDegree(note)}</span>
                  <span class="lower-mark">{lowerOctaveMark(note)}</span>
                  <span class="solfege">{solfege(note)}</span>
                  <span class="note-name">{note.name}</span>
                </button>
                <kbd>{note.key}</kbd>
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>

    <footer class="panel-foot">
      <span class="hint">
        <kbd>Q</kbd>—<kbd>I</kbd> · <kbd>A</kbd>—<kbd>J</kbd> ·
        <kbd>Z</kbd>—<kbd>M</kbd>
      </span>
      <span class="hint-center">CLICK NOTES · HOLD CHORDS</span>
      <span class="exit-hint">Press <kbd>ESC</kbd> to end performance</span>
    </footer>
  </section>
{/if}

<style>
  .instrument-panel {
    position: fixed;
    left: 50%;
    bottom: clamp(18px, 5vh, 52px);
    z-index: 45;
    width: min(1040px, calc(100vw - 24px));
    box-sizing: border-box;
    padding: 46px 26px 14px;
    border-top: 1px solid rgba(226, 190, 118, 0.38);
    border-radius: 42px 42px 18px 18px;
    background:
      linear-gradient(
        180deg,
        rgba(8, 13, 15, 0.15) 0%,
        rgba(7, 13, 15, 0.72) 22%,
        rgba(5, 10, 12, 0.9) 100%
      ),
      radial-gradient(
        ellipse at 50% 0%,
        rgba(224, 174, 92, 0.17),
        transparent 54%
      );
    box-shadow:
      0 -18px 54px rgba(0, 0, 0, 0.18),
      0 22px 52px rgba(0, 0, 0, 0.48),
      inset 0 1px rgba(255, 244, 207, 0.08);
    color: #f8edd1;
    font-family: 'Trebuchet MS', 'Segoe UI', sans-serif;
    transform: translateX(-50%);
    user-select: none;
    touch-action: none;
    pointer-events: auto;
    isolation: isolate;
  }

  .instrument-panel::before,
  .instrument-panel::after {
    position: absolute;
    z-index: -1;
    pointer-events: none;
    content: '';
  }

  .instrument-panel::before {
    inset: 5px 9px 7px;
    border: 1px solid rgba(224, 186, 112, 0.13);
    border-radius: 37px 37px 14px 14px;
  }

  .instrument-panel::after {
    right: 13%;
    bottom: 0;
    left: 13%;
    height: 1px;
    background: linear-gradient(
      90deg,
      transparent,
      rgba(126, 255, 219, 0.35),
      transparent
    );
    box-shadow: 0 0 18px rgba(94, 255, 215, 0.16);
  }

  .instrument-ornament {
    position: absolute;
    top: -39px;
    left: 50%;
    z-index: -1;
    width: min(840px, 86%);
    height: 136px;
    object-fit: contain;
    opacity: 0.53;
    filter: drop-shadow(0 2px 1px rgba(0, 0, 0, 0.8))
      drop-shadow(0 0 11px rgba(227, 177, 94, 0.15));
    transform: translateX(-50%);
    pointer-events: none;
  }

  .panel-head,
  .panel-foot {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .panel-head {
    min-height: 40px;
    padding: 0 8px 6px;
  }

  .instrument-title {
    min-width: 190px;
  }

  .eyebrow {
    display: block;
    margin-bottom: 1px;
    color: rgba(116, 246, 212, 0.8);
    font-family: 'Courier New', monospace;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.27em;
  }

  .title-line {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  .title-line h2 {
    margin: 0;
    color: #fff2cf;
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 23px;
    font-weight: 500;
    letter-spacing: 0.035em;
    text-shadow: 0 2px 10px rgba(255, 207, 118, 0.22);
  }

  .subtitle {
    color: rgba(242, 227, 193, 0.64);
    font-family: 'Courier New', monospace;
    font-size: 10px;
    letter-spacing: 0.11em;
  }

  .head-actions {
    display: flex;
    align-items: center;
  }

  .close {
    display: grid;
    width: 31px;
    height: 31px;
    padding: 0;
    place-items: center;
    border: 1px solid rgba(231, 203, 143, 0.28);
    border-radius: 50%;
    background: rgba(10, 13, 14, 0.5);
    color: rgba(244, 227, 190, 0.74);
    font: inherit;
    font-size: 19px;
    cursor: pointer;
    transition:
      border-color 120ms ease,
      background 120ms ease,
      color 120ms ease,
      transform 120ms ease;
  }

  .close:hover {
    border-color: rgba(111, 255, 218, 0.72);
    background: rgba(55, 133, 113, 0.3);
    color: #fff;
    transform: rotate(5deg) scale(1.05);
  }

  .performance-bed {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .instrument-row {
    --accent: 91, 236, 201;
    position: relative;
    display: grid;
    min-height: 67px;
    grid-template-columns: 83px minmax(0, 1fr);
    align-items: center;
  }

  .instrument-row::before {
    position: absolute;
    top: 31px;
    right: 0;
    left: 5px;
    height: 1px;
    background: linear-gradient(
      90deg,
      transparent,
      rgba(var(--accent), 0.16) 10%,
      rgba(232, 215, 174, 0.19) 52%,
      rgba(var(--accent), 0.12) 91%,
      transparent
    );
    box-shadow:
      0 -8px rgba(229, 214, 176, 0.04),
      0 8px rgba(229, 214, 176, 0.04);
    content: '';
    pointer-events: none;
  }

  .register-label {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 4px;
    color: rgba(var(--accent), 0.9);
  }

  .clef {
    width: 29px;
    color: rgba(var(--accent), 0.72);
    font-family: 'Segoe UI Symbol', serif;
    font-size: 35px;
    line-height: 1;
    text-align: center;
    text-shadow: 0 0 12px rgba(var(--accent), 0.22);
  }

  .register-copy {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .register-copy strong,
  .register-copy small {
    font-family: 'Courier New', monospace;
  }

  .register-copy strong {
    font-size: 11px;
    letter-spacing: 0.13em;
  }

  .register-copy small {
    color: rgba(244, 230, 197, 0.6);
    font-size: 10px;
    letter-spacing: 0.03em;
  }

  .note-grid {
    position: relative;
    z-index: 2;
    display: grid;
    min-width: 0;
    grid-template-columns: repeat(8, minmax(43px, 1fr));
    align-items: start;
  }

  .note-cell {
    position: relative;
    display: flex;
    min-width: 0;
    flex-direction: column;
    align-items: center;
    gap: 1px;
  }

  .note-cell.active {
    z-index: 6;
  }

  .note-orb {
    position: relative;
    display: grid;
    width: clamp(49px, 5.6vw, 61px);
    height: clamp(49px, 5.6vw, 61px);
    padding: 0;
    place-items: center;
    overflow: visible;
    border: 1px solid rgba(240, 222, 180, 0.27);
    border-radius: 50%;
    background:
      radial-gradient(
        circle at 34% 27%,
        rgba(255, 245, 213, 0.08),
        transparent 31%
      ),
      radial-gradient(
        circle,
        rgba(18, 27, 28, 0.87) 0 59%,
        rgba(5, 9, 10, 0.94) 76%
      );
    box-shadow:
      0 6px 12px rgba(0, 0, 0, 0.43),
      inset 0 0 0 3px rgba(var(--accent), 0.035),
      inset 0 1px rgba(255, 246, 217, 0.1);
    color: #fff3d4;
    font: inherit;
    cursor: pointer;
    transform: translateY(0) scale(1);
    transition:
      transform 85ms ease,
      border-color 120ms ease,
      background 120ms ease,
      box-shadow 140ms ease;
  }

  .note-orb::before,
  .note-orb::after {
    position: absolute;
    border-radius: 50%;
    pointer-events: none;
    content: '';
  }

  .note-orb::before {
    inset: 4px;
    border: 1px solid rgba(var(--accent), 0.13);
  }

  .note-orb::after {
    inset: 8px;
    border: 1px dashed rgba(245, 221, 166, 0.08);
  }

  .note-orb:hover {
    border-color: rgba(var(--accent), 0.62);
    background:
      radial-gradient(
        circle at 50% 38%,
        rgba(var(--accent), 0.16),
        transparent 43%
      ),
      radial-gradient(
        circle,
        rgba(18, 32, 31, 0.92) 0 62%,
        rgba(5, 10, 10, 0.97) 78%
      );
    box-shadow:
      0 7px 16px rgba(0, 0, 0, 0.48),
      0 0 13px rgba(var(--accent), 0.12),
      inset 0 0 0 3px rgba(var(--accent), 0.06);
  }

  .note-orb.pressed {
    border-color: rgba(var(--accent), 1);
    background: radial-gradient(
      circle,
      rgba(236, 255, 246, 0.92) 0 7%,
      rgba(var(--accent), 0.61) 19%,
      rgba(var(--accent), 0.18) 54%,
      rgba(4, 12, 11, 0.94) 75%
    );
    box-shadow:
      0 0 8px rgba(230, 255, 246, 0.95),
      0 0 25px rgba(var(--accent), 0.88),
      0 0 50px rgba(var(--accent), 0.31),
      inset 0 0 12px rgba(255, 255, 255, 0.62);
    color: #fff;
    transform: translateY(1px) scale(1.08);
  }

  .degree,
  .upper-mark,
  .lower-mark,
  .solfege,
  .note-name {
    position: absolute;
    z-index: 3;
    line-height: 1;
    pointer-events: none;
  }

  .degree {
    top: 13px;
    font-family: Georgia, 'Times New Roman', serif;
    font-size: clamp(16px, 1.9vw, 21px);
    font-weight: 600;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.8);
  }

  .upper-mark,
  .lower-mark {
    color: rgba(var(--accent), 0.96);
    font-size: 9px;
    letter-spacing: -1px;
  }

  .upper-mark {
    top: 5px;
  }

  .lower-mark {
    top: 34px;
  }

  .solfege {
    bottom: 8px;
    color: rgba(250, 236, 204, 0.87);
    font-family: 'Courier New', monospace;
    font-size: clamp(9px, 0.9vw, 11px);
    font-weight: 700;
    letter-spacing: 0.07em;
  }

  .note-name {
    right: 1px;
    bottom: -1px;
    color: rgba(var(--accent), 0.9);
    font-family: 'Courier New', monospace;
    font-size: 9px;
  }

  .pressed .degree,
  .pressed .solfege {
    color: #fff;
    text-shadow: 0 0 8px rgba(255, 255, 255, 0.96);
  }

  .cross-flare,
  .sound-ring,
  .rising-note {
    position: absolute;
    z-index: 0;
    opacity: 0;
    pointer-events: none;
  }

  .cross-flare {
    top: 50%;
    left: 50%;
    width: 152%;
    height: 170%;
    background:
      linear-gradient(
        90deg,
        transparent 0 10%,
        rgba(var(--accent), 0.62) 49%,
        rgba(255, 255, 255, 0.96) 50%,
        rgba(var(--accent), 0.62) 51%,
        transparent 90%
      ),
      linear-gradient(
        0deg,
        transparent 0 17%,
        rgba(var(--accent), 0.62) 49%,
        rgba(255, 255, 255, 0.82) 50%,
        rgba(var(--accent), 0.62) 51%,
        transparent 83%
      );
    filter: blur(3px);
    transform: translate(-50%, -50%);
  }

  .sound-ring {
    inset: -6px;
    border: 1px solid rgba(var(--accent), 0.84);
    border-radius: 50%;
    box-shadow: 0 0 12px rgba(var(--accent), 0.5);
  }

  .rising-note {
    top: -5px;
    right: -3px;
    color: rgba(var(--accent), 0.95);
    font-family: Georgia, serif;
    font-size: 15px;
    text-shadow: 0 0 8px rgba(var(--accent), 0.8);
  }

  .cross-flare {
    animation: flare-breathe 460ms ease-out both;
  }

  .sound-ring {
    animation: sound-ring 660ms ease-out both;
  }

  .rising-note {
    animation: note-rise 720ms ease-out both;
  }

  .note-cell > kbd {
    position: relative;
    z-index: 4;
    min-width: 22px;
    height: 17px;
    box-sizing: border-box;
    padding: 1px 5px 0;
    border: 1px solid rgba(244, 228, 191, 0.38);
    border-bottom-color: rgba(244, 228, 191, 0.58);
    border-radius: 4px;
    background: rgba(7, 11, 12, 0.82);
    box-shadow:
      0 2px 0 rgba(0, 0, 0, 0.46),
      inset 0 1px rgba(255, 255, 255, 0.05);
    color: rgba(255, 240, 207, 0.95);
    font-family: 'Courier New', monospace;
    font-size: 11px;
    line-height: 13px;
    text-align: center;
    transition: 90ms ease;
  }

  .note-cell.active > kbd {
    border-color: rgba(var(--accent), 0.83);
    background: rgba(var(--accent), 0.22);
    box-shadow:
      0 0 9px rgba(var(--accent), 0.55),
      inset 0 0 5px rgba(var(--accent), 0.3);
    color: #fff;
    transform: translateY(1px);
  }

  .panel-foot {
    min-height: 21px;
    padding: 5px 8px 0;
    color: rgba(245, 230, 196, 0.64);
    font-family: 'Courier New', monospace;
    font-size: 10px;
    letter-spacing: 0.06em;
  }

  .panel-foot kbd {
    padding: 1px 3px;
    border: 1px solid rgba(239, 222, 183, 0.38);
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.24);
    color: rgba(255, 241, 207, 0.88);
    font-family: inherit;
    font-size: inherit;
  }

  .hint-center {
    color: rgba(124, 248, 215, 0.62);
    letter-spacing: 0.12em;
  }

  .exit-hint {
    color: rgba(250, 233, 197, 0.76);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  @keyframes flare-breathe {
    from {
      opacity: 0.55;
      transform: translate(-50%, -50%) scale(0.8);
    }
    to {
      opacity: 1;
      transform: translate(-50%, -50%) scale(1.08);
    }
  }

  @keyframes sound-ring {
    from {
      opacity: 0.9;
      transform: scale(0.8);
    }
    to {
      opacity: 0;
      transform: scale(1.55);
    }
  }

  @keyframes note-rise {
    from {
      opacity: 0;
      transform: translate(0, 7px) rotate(-8deg) scale(0.72);
    }
    24% {
      opacity: 0.95;
    }
    to {
      opacity: 0;
      transform: translate(7px, -18px) rotate(9deg) scale(1.08);
    }
  }

  @media (max-width: 760px) {
    .instrument-panel {
      bottom: 12px;
      width: calc(100vw - 12px);
      padding: 38px 10px 10px;
      border-radius: 28px 28px 12px 12px;
    }

    .instrument-ornament {
      top: -30px;
      height: 112px;
    }

    .panel-head {
      min-height: 34px;
      padding-inline: 4px;
    }

    .subtitle,
    .hint,
    .hint-center {
      display: none;
    }

    .title-line h2 {
      font-size: 19px;
    }

    .instrument-row {
      min-height: 58px;
      grid-template-columns: 43px minmax(0, 1fr);
    }

    .instrument-row::before {
      top: 27px;
    }

    .register-copy {
      display: none;
    }

    .clef {
      width: 36px;
      font-size: 29px;
    }

    .note-grid {
      grid-template-columns: repeat(8, minmax(38px, 1fr));
    }

    .note-orb {
      width: clamp(42px, 10.5vw, 51px);
      height: clamp(42px, 10.5vw, 51px);
    }

    .degree {
      top: 11px;
      font-size: 16px;
    }

    .upper-mark {
      top: 4px;
    }

    .lower-mark {
      top: 29px;
    }

    .solfege {
      bottom: 7px;
      font-size: 9px;
    }

    .note-cell > kbd {
      height: 14px;
      line-height: 10px;
    }
  }

  @media (max-width: 430px) {
    .instrument-panel {
      padding-inline: 5px;
    }

    .instrument-title {
      min-width: 120px;
    }

    .instrument-row {
      min-height: 53px;
      grid-template-columns: 29px minmax(0, 1fr);
    }

    .instrument-row::before {
      top: 24px;
    }

    .clef {
      width: 26px;
      font-size: 23px;
    }

    .note-grid {
      grid-template-columns: repeat(8, minmax(35px, 1fr));
    }

    .note-orb {
      width: 40px;
      height: 40px;
    }

    .note-name {
      display: none;
    }

    .degree {
      top: 9px;
      font-size: 15px;
    }

    .upper-mark {
      top: 3px;
    }

    .lower-mark {
      top: 26px;
    }

    .solfege {
      bottom: 6px;
    }

    .panel-foot {
      justify-content: center;
    }
  }

  @media (max-width: 360px) {
    .instrument-panel {
      bottom: 0;
      width: 100vw;
      padding-inline: 0;
      border-radius: 24px 24px 0 0;
    }

    .instrument-row {
      grid-template-columns: 0 minmax(0, 1fr);
    }

    .register-label {
      display: none;
    }

    .note-grid {
      grid-template-columns: repeat(8, minmax(0, 1fr));
    }
  }

  @media (max-height: 510px) and (min-width: 761px) {
    .instrument-panel {
      bottom: 10px;
      padding-top: 35px;
    }

    .instrument-ornament {
      top: -32px;
      height: 116px;
    }

    .panel-head {
      min-height: 33px;
    }

    .instrument-row {
      min-height: 58px;
    }

    .instrument-row::before {
      top: 27px;
    }

    .note-orb {
      width: 49px;
      height: 49px;
    }

    .degree {
      top: 10px;
      font-size: 17px;
    }

    .upper-mark {
      top: 4px;
    }

    .lower-mark {
      top: 28px;
    }

    .solfege {
      bottom: 7px;
      font-size: 9px;
    }

    .panel-foot {
      min-height: 17px;
      padding-top: 2px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .note-orb,
    .note-cell > kbd,
    .close {
      transition: none;
    }

    .cross-flare,
    .sound-ring,
    .rising-note {
      animation: none;
    }

    .cross-flare {
      opacity: 0.75;
    }
  }
</style>
