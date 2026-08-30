import { writable } from 'svelte/store'

/** Object type the server laid the local player on when it respawned them
 *  (a sick-room bed). The server already holds the claim, so `PlayerControl`
 *  enters the pose without sending `InteractObject`, then clears this. */
export const respawnPoseRequest = writable<string | null>(null)
