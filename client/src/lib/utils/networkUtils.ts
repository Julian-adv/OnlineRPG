export function getDefaultServerUrl(): string {
  if (typeof window === 'undefined') return 'ws://localhost:5002'
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const hostname = window.location.hostname
  const port = window.location.port
  const host = port ? `${hostname}:${port}` : hostname
  return `${protocol}//${host}/ws`
}

export function getTerrainApiUrl(): string {
  if (typeof window === 'undefined') return 'http://localhost:5003'
  // In dev, Vite proxies /api/terrain → http://localhost:5003
  // Use same origin so the request goes through the proxy
  return window.location.origin
}

/** Where an uploaded cape print is served from. Content-addressed and
 *  immutable, so the browser caches it forever; no file extension, because
 *  nginx routes `*.png` to the static bundle before it reaches `/api/`. */
export function capeTextureUrl(hash: string | null | undefined): string | null {
  return hash ? `/api/cape-texture/${hash}` : null
}

let apiAuthToken: string | null = null

/** Google ID token for REST writes (server checks the admin allowlist).
 *  Single owner of the credential; socket.ts sets/clears it on auth. */
export function setApiAuthToken(token: string | null): void {
  apiAuthToken = token
}

export function getApiAuthToken(): string | null {
  return apiAuthToken
}

let capeUploadToken: string | null = null

/** Credential for the player's own REST calls, handed out over the socket at
 *  login. Kept apart from the Google id token: that one expires inside an
 *  hour and only ever authorises admin writes. */
export function setCapeUploadToken(token: string | null): void {
  capeUploadToken = token
}

export function getCapeUploadToken(): string | null {
  return capeUploadToken
}

/** fetch with a bearer credential attached; use for all /api write requests.
 *  Defaults to the admin-checked Google token; `token` overrides it for the
 *  player's own writes, which carry the session credential instead. */
export function apiFetch(
  url: string,
  init: RequestInit & { headers?: Record<string, string>; token?: string } = {}
): Promise<Response> {
  const { token, ...rest } = init
  const headers: Record<string, string> = { ...init.headers }
  const bearer = token ?? apiAuthToken
  if (bearer) headers.Authorization = `Bearer ${bearer}`
  return fetch(url, { ...rest, headers })
}
