<script lang="ts">
  import { T } from "@threlte/core";
  import { OrbitControls, Grid } from "@threlte/extras";
  import { onMount } from "svelte";
  import { gameStore, type Player } from "../stores/gameStore";
  import { networkManager } from "../network/socket";
  import PlayerModel from "./PlayerModel.svelte";

  let currentPlayer = $state<Player | null>(null);
  let otherPlayers = $state(new Map());
  let isConnected = $state(false);

  gameStore.subscribe((state) => {
    currentPlayer = state.currentPlayer;
    otherPlayers = state.otherPlayers;
    isConnected = state.isConnected;
  });

  onMount(() => {
    networkManager.connect();

    // Join the game with a default player name
    setTimeout(() => {
      networkManager.joinGame("Player");
    }, 1000);

    return () => {
      networkManager.disconnect();
    };
  });

  function handlePlayerMove(detail: { x: number; y: number; z: number }) {
    const { x, y, z } = detail;
    networkManager.sendPlayerMove({ x, y, z });
  }
</script>

<T.PerspectiveCamera makeDefault position={[0, 15, 10]} fov={75}>
  <OrbitControls
    enableRotate={false}
    enablePan={false}
    enableZoom={true}
    target={currentPlayer ? [currentPlayer.position.x, currentPlayer.position.y, currentPlayer.position.z] : [0, 0, 0]}
    minDistance={5}
    maxDistance={50}
  />
</T.PerspectiveCamera>

<T.DirectionalLight position={[10, 10, 10]} intensity={1.5} castShadow />
<T.AmbientLight intensity={0.4} />

<Grid
  infiniteGrid
  gridSize={100}
  sectionColor="#4a5568"
  sectionThickness={1.2}
  fadeDistance={100}
/>

<T.Mesh position={[0, -0.5, 0]} receiveShadow>
  <T.PlaneGeometry args={[100, 100]} />
  <T.MeshLambertMaterial color="#2d3748" />
</T.Mesh>

{#if currentPlayer}
  <PlayerModel
    position={currentPlayer.position}
    name={currentPlayer.name}
    isCurrentPlayer={true}
    onmove={handlePlayerMove}
  />
{/if}

{#each [...otherPlayers.values()] as player (player.id)}
  <PlayerModel
    position={player.position}
    name={player.name}
    isCurrentPlayer={false}
  />
{/each}
