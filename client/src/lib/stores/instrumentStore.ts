import { writable } from 'svelte/store'

export const instrumentPanelVisible = writable(false)
export const instrumentPressedNotes = writable<ReadonlySet<number>>(new Set())

export function openInstrumentPanel() {
  instrumentPressedNotes.set(new Set())
  instrumentPanelVisible.set(true)
}

export function closeInstrumentPanel() {
  instrumentPanelVisible.set(false)
  instrumentPressedNotes.set(new Set())
}

export function setInstrumentNotePressed(note: number, pressed: boolean) {
  instrumentPressedNotes.update((current) => {
    if (current.has(note) === pressed) return current
    const next = new Set(current)
    if (pressed) next.add(note)
    else next.delete(note)
    return next
  })
}

export function clearInstrumentPressedNotes() {
  instrumentPressedNotes.set(new Set())
}

export function resetInstrumentStore() {
  closeInstrumentPanel()
}
