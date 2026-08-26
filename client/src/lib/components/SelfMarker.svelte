<script lang="ts">
  let {
    left,
    top,
    angle,
    size = 20,
  }: { left: number; top: number; angle: number; size?: number } = $props()
</script>

<svg
  class="self-marker"
  style="left: {left}px; top: {top}px; width: {size}px; height: {size}px; transform: translate(-50%, -50%) rotate({angle}rad);"
  viewBox="0 0 20 20"
  aria-hidden="true"
>
  <path class="arrow" d="M 19 10 L 3.5 16.5 L 6.75 10 L 3.5 3.5 Z"></path>
</svg>

<style>
  .self-marker {
    position: absolute;
    z-index: 3;
    overflow: visible;
    pointer-events: none;
    will-change: transform;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.8));
  }

  .arrow {
    fill: #ff3333;
    stroke: #ffffff;
    stroke-width: 1.5;
    stroke-linejoin: round;
  }
</style>
