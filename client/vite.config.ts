import fs from 'node:fs'
import { Agent } from 'node:https'
import { execSync } from 'node:child_process'
import { defineConfig, loadEnv } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import wasm from 'vite-plugin-wasm'
// @ts-expect-error no type declarations for .mjs
import { monsterCsvPlugin } from '../tools/vitePlugin.mjs'

// https://vite.dev/config/
function gitShortHash(): string {
  try {
    return execSync('git rev-parse --short HEAD', {
      stdio: ['ignore', 'pipe', 'ignore'],
    })
      .toString()
      .trim()
  } catch {
    return 'unknown'
  }
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '')

  // Default to IPv4 explicitly: Node 18+ resolves 'localhost' to ::1 first,
  // which fails because the Rust server only listens on 127.0.0.1, causing
  // the proxy to reset every /ws and /api request.
  const backendHost = env.VITE_BACKEND_HOST ?? '127.0.0.1'
  const apiTarget = env.VITE_API_TARGET ?? `http://${backendHost}:10007`
  const wsTarget = env.VITE_WS_TARGET ?? `ws://${backendHost}:10006`
  const proxyAgent = env.VITE_PROXY_CA
    ? new Agent({
        ca: fs.readFileSync(env.VITE_PROXY_CA),
        ...(env.VITE_PROXY_SERVERNAME
          ? { servername: env.VITE_PROXY_SERVERNAME }
          : {}),
      })
    : undefined

  const httpsKey = env.VITE_HTTPS_KEY
  const httpsCert = env.VITE_HTTPS_CERT
  const httpsCa = env.VITE_HTTPS_CA
  const https =
    httpsKey && httpsCert
      ? {
          key: fs.readFileSync(httpsKey),
          cert: fs.readFileSync(httpsCert),
          ...(httpsCa ? { ca: fs.readFileSync(httpsCa) } : {}),
        }
      : undefined

  const hmrHost = env.VITE_HMR_HOST
  const hmrProtocol = env.VITE_HMR_PROTOCOL
  const hmr =
    hmrHost || hmrProtocol
      ? {
          ...(hmrHost ? { host: hmrHost } : {}),
          ...(hmrProtocol ? { protocol: hmrProtocol as 'ws' | 'wss' } : {}),
        }
      : undefined

  // The build's commit rides on the handshake version so the server log can
  // tell which bundle a session runs.
  const appVersion = `${
    JSON.parse(
      fs.readFileSync(new URL('./package.json', import.meta.url), 'utf8')
    ).version
  }+${gitShortHash()}`

  return {
    plugins: [monsterCsvPlugin(), wasm(), svelte()],
    define: { __APP_VERSION__: JSON.stringify(appVersion) },
    server: {
      host: true,
      port: 10004,
      strictPort: true,
      https,
      hmr,
      // No global Cache-Control here: it only ever applied to transformed
      // source modules (proxied /api responses bypass server.headers), where
      // an hour of max-age serves stale modules after HMR/scp churn — wasm
      // export errors, split store singletons. Vite's default ETag/304
      // revalidation is the right policy for dev.
      proxy: {
        // All REST endpoints share one backend, so a single prefix covers them.
        '/api': { target: apiTarget, changeOrigin: true, agent: proxyAgent },
        '/ws': {
          target: wsTarget,
          ws: true,
          changeOrigin: true,
          agent: proxyAgent,
        },
      },
    },
    build: { target: 'esnext' },
    optimizeDeps: { esbuildOptions: { target: 'esnext' } },
  }
})
