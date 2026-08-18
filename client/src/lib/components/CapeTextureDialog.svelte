<script lang="ts">
  import { capeTexturePreview } from '../stores/capeTextureStore'
  import { mountOverlay } from '../stores/overlayStack'
  import { PRINT_ASPECT } from '../effects/cape-rig'
  import { apiFetch, getCapeUploadToken } from '../utils/networkUtils'

  interface Props {
    onConfirm: (textureHash: string) => void
    onCancel: () => void
  }

  let { onConfirm, onCancel }: Props = $props()

  /** What the server accepts; the picture is fitted to this before upload so
   *  the round trip never carries a camera-sized file. */
  /** Tallest a print is stored; its width follows `PRINT_ASPECT`. */
  const HEIGHT = 512

  let fileInput = $state<HTMLInputElement | null>(null)
  /** The picture waiting to be printed: blob and preview URL together, so the
   *  two can never disagree about whether one was chosen. */
  let chosen = $state<{ blob: Blob; url: string } | null>(null)
  let busy = $state(false)
  let error = $state<string | null>(null)

  $effect(() => mountOverlay('capeTexture', onCancel))

  // The wearer sees the picture on their own cape while they decide. One
  // effect owns the object URL end to end: leaving the dialog by any route, or
  // picking a different file, puts the worn cape back and frees the old URL.
  $effect(() => {
    const url = chosen?.url ?? null
    capeTexturePreview.set(url)
    return () => {
      capeTexturePreview.set(null)
      if (url) URL.revokeObjectURL(url)
    }
  })

  /** The picture, scaled to the square's height and centred; the canvas clips
   *  anything wider. Where it sits on the cape and how much of its width the
   *  cloth crops are the cloth's business (`cape-rig.ts`), so tuning that
   *  re-places prints already uploaded instead of needing them sent again.
   *  Small pictures are not blown up — the sampler would do that for free, and
   *  stored pixels cost every viewer, which is also why the file is only as
   *  wide as the cloth can show. */
  async function toCapePng(file: File): Promise<Blob> {
    const bitmap = await createImageBitmap(file)
    const height = Math.min(HEIGHT, bitmap.height)
    const width = Math.round(height * PRINT_ASPECT)
    const w = Math.round((bitmap.width * height) / bitmap.height)

    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('no canvas')
    ctx.drawImage(bitmap, (width - w) / 2, 0, w, height)
    bitmap.close()

    const blob = await new Promise<Blob | null>((resolve) =>
      canvas.toBlob(resolve, 'image/png')
    )
    if (!blob) throw new Error('could not encode the picture')
    return blob
  }

  async function pick(e: Event) {
    const input = e.currentTarget as HTMLInputElement
    const file = input.files?.[0]
    // Cleared before anything can fail: a file input fires `change` only when
    // its value differs, so keeping the last pick would make choosing the same
    // file again do nothing at all.
    input.value = ''
    if (!file) return
    error = null
    try {
      const blob = await toCapePng(file)
      chosen = { blob, url: URL.createObjectURL(blob) }
    } catch {
      error = 'That file could not be read as a picture.'
    }
  }

  async function upload() {
    const token = getCapeUploadToken()
    if (!chosen || !token || busy) return
    busy = true
    error = null
    try {
      const response = await apiFetch('/api/cape-texture', {
        method: 'POST',
        token,
        headers: { 'Content-Type': 'image/png' },
        body: chosen.blob,
      })
      if (!response.ok) {
        error = (await response.text()) || 'The upload was refused.'
        return
      }
      const { hash } = (await response.json()) as { hash: string }
      onConfirm(hash)
    } catch {
      error = 'The upload could not be sent.'
    } finally {
      busy = false
    }
  }
</script>

<div
  class="print-dialog"
  role="dialog"
  aria-label="Print on cape"
  tabindex="-1"
>
  <h2>Print your cape</h2>
  <p>
    Pick a picture. Transparent parts keep the colour your cape is dyed, so a
    crest on a clear background sits on the cloth as it is.
  </p>

  <input
    bind:this={fileInput}
    type="file"
    accept="image/*"
    class="hidden-input"
    onchange={pick}
  />

  <button class="picker" onclick={() => fileInput?.click()}>
    {chosen ? 'Choose another picture' : 'Choose a picture'}
  </button>

  {#if chosen}
    <img class="preview" src={chosen.url} alt="Chosen print" />
  {/if}

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <div class="print-actions">
    <button class="primary" disabled={!chosen || busy} onclick={upload}>
      {busy ? 'Printing…' : 'Print'}
    </button>
    <button class="secondary" onclick={onCancel}>Cancel</button>
  </div>
</div>

<style>
  /* Left-hand side with no backdrop, like the dye picker: the point is
     watching the print land on your own cape, and a centred dialog would
     cover the one thing it is there to show. */
  .print-dialog {
    position: fixed;
    left: 16px;
    top: 45%;
    transform: translateY(-50%);
    z-index: 40;
    width: min(300px, calc(100vw - 32px));
    padding: 20px;
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    background: rgba(16, 16, 16, 0.92);
    backdrop-filter: blur(4px);
    color: #f4f4f4;
    text-align: center;
  }

  .print-dialog h2 {
    margin: 0 0 8px 0;
    font-size: 20px;
  }

  .print-dialog p {
    margin: 0 0 14px 0;
    color: #d4d4d4;
    font-size: 13px;
  }

  .hidden-input {
    display: none;
  }

  .picker {
    width: 100%;
    padding: 8px 12px;
    margin-bottom: 12px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    background: rgba(255, 255, 255, 0.08);
    color: #f4f4f4;
    cursor: pointer;
  }

  .preview {
    width: 128px;
    height: 128px;
    object-fit: contain;
    margin-bottom: 12px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    /* Checkerboard, so a transparent background reads as transparent rather
       than as whatever the dialog happens to sit on. */
    background-image:
      linear-gradient(45deg, #444 25%, transparent 25%),
      linear-gradient(-45deg, #444 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #444 75%),
      linear-gradient(-45deg, transparent 75%, #444 75%);
    background-size: 16px 16px;
    background-position:
      0 0,
      0 8px,
      8px -8px,
      -8px 0;
  }

  .error {
    margin: 0 0 12px 0;
    color: #f0a0a0;
    font-size: 12px;
  }

  .print-actions {
    display: flex;
    gap: 10px;
    justify-content: center;
  }

  .print-actions button {
    flex: 1;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    cursor: pointer;
  }

  .primary {
    background: #3c6e3c;
    color: #fff;
  }

  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .secondary {
    background: rgba(255, 255, 255, 0.08);
    color: #f4f4f4;
  }
</style>
