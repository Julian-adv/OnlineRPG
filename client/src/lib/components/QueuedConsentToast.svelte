<script lang="ts" generics="T extends { offeredAt: number }">
  import type { Snippet } from 'svelte'
  import type { Writable } from 'svelte/store'
  import ConsentToast from './ConsentToast.svelte'

  /** Queue discipline shared by the consent toasts: the head entry shows,
   *  oldest first — a flood can neither swap the name under the user's click
   *  nor bury an earlier legitimate request — and the head dismisses itself
   *  when its mirrored server-side TTL runs out. */
  let {
    queue,
    ttlMs,
    label,
    top,
    accent,
    acceptLabel,
    declineLabel,
    respond,
    dismissOnAccept = true,
    children,
  }: {
    queue: Writable<T[]>
    ttlMs: number
    label: string
    top: string
    accent: string
    acceptLabel: string
    declineLabel: string
    /** Sends the answer; dismissal is handled here. */
    respond: (entry: T, accept: boolean) => void
    dismissOnAccept?: boolean
    children: Snippet<[T]>
  } = $props()

  const entry = $derived($queue[0] ?? null)
  const queued = $derived(Math.max(0, $queue.length - 1))

  function dismiss(entry: T) {
    queue.update((q) => q.filter((e) => e !== entry))
  }

  function answer(entry: T, accept: boolean) {
    respond(entry, accept)
    if (!accept || dismissOnAccept) dismiss(entry)
  }

  $effect(() => {
    if (!entry) return
    const timer = setTimeout(
      () => dismiss(entry),
      Math.max(0, entry.offeredAt + ttlMs - Date.now())
    )
    return () => clearTimeout(timer)
  })
</script>

{#if entry}
  <ConsentToast
    {label}
    {top}
    {accent}
    {acceptLabel}
    {declineLabel}
    onaccept={() => answer(entry, true)}
    ondecline={() => answer(entry, false)}
    {queued}
    gaugeDurationMs={ttlMs}
    gaugeStartAt={entry.offeredAt}
  >
    {@render children(entry)}
  </ConsentToast>
{/if}
