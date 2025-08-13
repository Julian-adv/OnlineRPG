<script lang="ts">
  let fps = $state(0)
  let frameCount = $state(0)
  let lastFpsTime = $state(0)

  function updateFPS() {
    frameCount++
    const currentTime = performance.now()
    
    if (currentTime - lastFpsTime >= 1000) { // Update FPS every second
      fps = Math.round(frameCount * 1000 / (currentTime - lastFpsTime))
      frameCount = 0
      lastFpsTime = currentTime
    }
    
    requestAnimationFrame(updateFPS)
  }

  // Start FPS monitoring
  lastFpsTime = performance.now()
  requestAnimationFrame(updateFPS)
</script>

<div class="fps-counter">
  FPS: {fps}
</div>

<style>
  .fps-counter {
    position: fixed;
    top: 10px;
    left: 10px;
    background: rgba(0, 0, 0, 0.8);
    color: #00ff00;
    padding: 8px 12px;
    border-radius: 6px;
    font-family: 'Courier New', monospace;
    font-size: 14px;
    font-weight: bold;
    z-index: 1000;
    pointer-events: none;
    border: 1px solid rgba(0, 255, 0, 0.3);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
  }
</style>