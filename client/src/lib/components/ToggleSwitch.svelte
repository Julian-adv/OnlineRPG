<script lang="ts">
  import type { Writable } from 'svelte/store'

  interface Props {
    checked: Writable<boolean>
    label: string
  }

  let { checked, label }: Props = $props()
</script>

<button
  class="switch"
  class:on={$checked}
  role="switch"
  aria-checked={$checked}
  aria-label={label}
  onclick={() => checked.update((v) => !v)}
>
  <span class="knob"></span>
</button>

<style>
  .switch {
    width: 40px;
    height: 22px;
    flex-shrink: 0;
    padding: 2px;
    border: 1px solid #4a5568;
    border-radius: 11px;
    background: #2d3748;
    cursor: pointer;
    display: flex;
    justify-content: flex-start;
    align-items: center;
    transition:
      background 150ms ease,
      border-color 150ms ease;
  }

  .switch.on {
    background: #4299e1;
    border-color: #4299e1;
    justify-content: flex-end;
  }

  .knob {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #a0aec0;
    transition: background 150ms ease;
  }

  .switch.on .knob {
    background: #fff;
  }
</style>
