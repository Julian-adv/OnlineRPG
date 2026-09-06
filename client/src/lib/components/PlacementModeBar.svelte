<script lang="ts">
  let {
    title,
    message,
    hint,
    actionLabel = 'Done',
    onaction,
  }: {
    title: string
    message: string
    hint: string
    actionLabel?: string
    onaction: () => void
  } = $props()
</script>

<div class="placement-bar" role="status">
  <strong>{title}</strong>
  <span>{message}</span>
  <small>{hint}</small>
  <button onclick={onaction}>{actionLabel}</button>
</div>

<style>
  .placement-bar {
    position: fixed;
    bottom: 100px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 40;
    display: grid;
    gap: 6px;
    width: max-content;
    max-width: calc(100vw - 32px);
    padding: 14px 18px;
    border: 1px solid #b49860;
    border-radius: 8px;
    background: #211c16ed;
    color: #f3e8d2;
    text-align: center;
    pointer-events: auto;
  }
  small {
    color: #c7baa4;
  }
  button {
    justify-self: center;
    padding: 4px 18px;
    cursor: pointer;
  }
</style>
