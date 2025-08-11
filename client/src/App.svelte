<script lang="ts">
  import { Canvas } from '@threlte/core';
  import GameScene from './lib/components/GameScene.svelte';
  import ChatPanel from './lib/components/ChatPanel.svelte';
  import FPSCounter from './lib/components/FPSCounter.svelte';
</script>

<main>
  <Canvas>
    <GameScene />
  </Canvas>
  <ChatPanel />
  <FPSCounter />
</main>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    overflow: hidden;
    background: #1a1a1a;
  }

  main {
    width: 100vw;
    height: 100vh;
    position: relative;
  }
</style>
