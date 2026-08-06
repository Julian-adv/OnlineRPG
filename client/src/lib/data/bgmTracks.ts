import bgmJson from '../../../../data/bgm.json'

/** Ambient tracks by title. Battle music is not in here — it lives in
 *  `bgmManager`, where a bard cannot summon it. Title queries are resolved by
 *  the server (`bgm_defs.rs`), the one resolver every client shares; this map
 *  only turns resolved titles into files. */
const tracks = bgmJson as Record<string, { file: string }>

/** Every ambient track title — the playlist pool. */
export const BGM_TRACKS = Object.keys(tracks)

export const bgmFileFor = (track: string) => tracks[track]?.file
