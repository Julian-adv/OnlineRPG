<script lang="ts">
  import { onMount } from 'svelte'
  import { getTerrainApiUrl } from '../utils/networkUtils'

  interface Translation {
    title: string
    body: string
  }
  interface Announcement {
    id: string
    date: string
    category?: string
    translations: Record<string, Translation>
  }

  const DEFAULT_LANG = 'ko'
  const LANG_LABELS: Record<string, string> = { ko: '한국어', en: 'English' }
  // Preferred display order; anything else is appended alphabetically.
  const LANG_ORDER = ['ko', 'en']

  let announcements = $state<Announcement[]>([])
  let lang = $state(DEFAULT_LANG)
  let expandedId = $state<string | null>(null)

  // Every locale present across all announcements, in display order.
  const languages = $derived.by(() => {
    const present = new Set(
      announcements.flatMap((a) => Object.keys(a.translations))
    )
    return [...present].sort((a, b) => {
      const ia = LANG_ORDER.indexOf(a)
      const ib = LANG_ORDER.indexOf(b)
      return (ia === -1 ? 99 : ia) - (ib === -1 ? 99 : ib) || a.localeCompare(b)
    })
  })

  function labelFor(code: string): string {
    return LANG_LABELS[code] ?? code.toUpperCase()
  }

  function pick(item: Announcement): Translation {
    return (
      item.translations[lang] ??
      item.translations[DEFAULT_LANG] ??
      Object.values(item.translations)[0] ?? { title: item.date, body: '' }
    )
  }

  onMount(async () => {
    try {
      const res = await fetch(`${getTerrainApiUrl()}/api/announcements`)
      if (!res.ok) return
      const data = (await res.json()) as Announcement[]
      announcements = Array.isArray(data) ? data : []
      expandedId = announcements[0]?.id ?? null

      const browser = navigator.language?.slice(0, 2).toLowerCase()
      if (browser && languages.includes(browser)) lang = browser
      else if (languages.includes(DEFAULT_LANG)) lang = DEFAULT_LANG
      else lang = languages[0] ?? DEFAULT_LANG
    } catch {
      // Announcements are non-critical; failing here must not block login.
    }
  })

  function toggle(id: string) {
    expandedId = expandedId === id ? null : id
  }
</script>

{#if announcements.length > 0}
  <section class="announcements">
    <div class="bar">
      <h2 class="heading">Announcements</h2>
      {#if languages.length > 1}
        <div class="langs">
          {#each languages as code (code)}
            <button
              class="lang"
              class:active={lang === code}
              onclick={() => (lang = code)}
            >
              {labelFor(code)}
            </button>
          {/each}
        </div>
      {/if}
    </div>
    <ul class="list">
      {#each announcements as item (item.id)}
        {@const open = expandedId === item.id}
        {@const t = pick(item)}
        <li class="entry" class:open>
          <button class="entry-head" onclick={() => toggle(item.id)}>
            <span class="date">{item.date}</span>
            {#if item.category}
              <span class="tag">{item.category}</span>
            {/if}
            <span class="title">{t.title}</span>
            <span class="chevron" aria-hidden="true">{open ? '−' : '+'}</span>
          </button>
          {#if open}
            <div class="body">{t.body}</div>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .announcements {
    box-sizing: border-box;
    width: min(480px, 100%);
    min-width: 0;
    margin-top: 20px;
    padding: 16px 18px;
    background: rgba(0, 0, 0, 0.8);
    border: 1px solid #4a5568;
    border-radius: 12px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 12px;
  }

  .heading {
    margin: 0;
    color: #a0aec0;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 2px;
    text-transform: uppercase;
  }

  .langs {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  .lang {
    padding: 2px 9px;
    background: none;
    border: 1px solid #4a5568;
    border-radius: 999px;
    color: #718096;
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    transition:
      color 0.15s,
      border-color 0.15s,
      background 0.15s;
  }

  .lang:hover {
    color: #cbd5e0;
    border-color: #718096;
  }

  .lang.active {
    background: rgba(66, 153, 225, 0.18);
    border-color: rgba(66, 153, 225, 0.6);
    color: #90cdf4;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 260px;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: rgba(113, 128, 150, 0.5) transparent;
  }

  .list::-webkit-scrollbar {
    width: 8px;
  }

  .list::-webkit-scrollbar-track {
    background: transparent;
  }

  .list::-webkit-scrollbar-thumb {
    background: rgba(113, 128, 150, 0.5);
    border-radius: 999px;
  }

  .list::-webkit-scrollbar-thumb:hover {
    background: rgba(160, 174, 192, 0.7);
  }

  .entry {
    border-top: 1px solid #2d3748;
  }

  .entry:first-child {
    border-top: none;
  }

  .entry-head {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 10px 6px;
    background: none;
    border: none;
    border-radius: 6px;
    color: #cbd5e0;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .entry-head:hover {
    color: #fff;
  }

  /* Inset outline: a negative offset keeps the focus ring inside the button so
     the scrolling list (overflow clips both axes) can't crop its edges. Uses
     :focus, not :focus-visible, so a mouse click also gets the un-clipped ring. */
  .entry-head:focus {
    outline: 2px solid rgba(144, 205, 244, 0.9);
    outline-offset: -2px;
  }

  .date {
    flex-shrink: 0;
    color: #718096;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .tag {
    flex-shrink: 0;
    padding: 1px 7px;
    background: rgba(66, 153, 225, 0.18);
    border: 1px solid rgba(66, 153, 225, 0.5);
    border-radius: 999px;
    color: #90cdf4;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .title {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 14px;
  }

  .chevron {
    flex-shrink: 0;
    color: #718096;
    font-size: 16px;
    line-height: 1;
  }

  .entry.open .title {
    white-space: normal;
    color: #fff;
  }

  .body {
    padding: 4px 6px 14px;
    color: #a0aec0;
    font-size: 13px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
