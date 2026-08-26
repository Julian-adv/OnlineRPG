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
  <path class="point" d="M 24.5 10 L 14 4.4 L 14 15.6 Z"></path>
  <circle class="rim" cx="10" cy="10" r="8"></circle>
  <circle class="core" cx="10" cy="10" r="5.4"></circle>
  <circle class="shine" cx="8.2" cy="7.8" r="1.6"></circle>
</svg>

<style>
  .self-marker {
    position: absolute;
    z-index: 3;
    overflow: visible;
    pointer-events: none;
    will-change: transform;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.92))
      drop-shadow(0 0 3px rgba(218, 50, 27, 0.72));
  }

  .point {
    fill: #f2cf62;
    stroke: #35160c;
    stroke-width: 1.2;
    stroke-linejoin: round;
  }

  .rim {
    fill: #35160c;
    stroke: #f2cf62;
    stroke-width: 1.8;
  }

  .core {
    fill: #d92717;
    stroke: #720f0a;
    stroke-width: 0.8;
  }

  .shine {
    fill: #ffb37c;
    opacity: 0.88;
  }
</style>
