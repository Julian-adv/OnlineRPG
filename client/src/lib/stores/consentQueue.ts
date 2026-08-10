import type { Writable } from 'svelte/store'

/** Append to a capped consent queue unless the sender already has an entry:
 *  a flood can neither evict the head nor duplicate a toast. */
export function enqueueConsent<T>(
  queue: Writable<T[]>,
  cap: number,
  fromSameSender: (entry: T) => boolean,
  entry: T
) {
  queue.update((q) =>
    q.length >= cap || q.some(fromSameSender) ? q : [...q, entry]
  )
}
