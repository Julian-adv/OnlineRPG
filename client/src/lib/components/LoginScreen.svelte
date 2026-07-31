<script lang="ts">
  import { onMount } from 'svelte'
  import { getDefaultServerUrl } from '../utils/networkUtils'
  import AnnouncementsPanel from './AnnouncementsPanel.svelte'

  const GSI_SRC = 'https://accounts.google.com/gsi/client'

  interface Props {
    onLogin: (
      serverUrl: string,
      googleIdToken: string
    ) => Promise<{ ok: boolean; message?: string }>
    kickedMessage?: string
  }

  let { onLogin, kickedMessage }: Props = $props()

  let isConnecting = $state(false)
  let errorMessage = $state('')
  let buttonContainer = $state<HTMLDivElement | null>(null)

  function loadGsiScript(): Promise<void> {
    if (window.google?.accounts?.id) return Promise.resolve()
    return new Promise((resolve, reject) => {
      const existing = document.querySelector(`script[src="${GSI_SRC}"]`)
      if (existing) {
        existing.addEventListener('load', () => resolve())
        existing.addEventListener('error', () =>
          reject(new Error('Failed to load Google Sign-In'))
        )
        return
      }
      const script = document.createElement('script')
      script.src = GSI_SRC
      script.async = true
      script.onload = () => resolve()
      script.onerror = () => reject(new Error('Failed to load Google Sign-In'))
      document.head.appendChild(script)
    })
  }

  async function handleCredential(response: GoogleCredentialResponse) {
    errorMessage = ''
    isConnecting = true

    try {
      const result = await onLogin(getDefaultServerUrl(), response.credential)
      if (!result.ok) {
        errorMessage = result.message ?? 'Authentication failed'
      }
    } catch (e) {
      // onLogin can reject (e.g. WASM init failure) — without this the button
      // stays disabled with no message.
      errorMessage = e instanceof Error ? e.message : 'Authentication failed'
    } finally {
      isConnecting = false
    }
  }

  onMount(async () => {
    const clientId = import.meta.env.VITE_GOOGLE_CLIENT_ID as string | undefined
    // Container builds bake a placeholder that the entrypoint substitutes at
    // startup; unsubstituted it would reach Google as a bogus ID.
    if (!clientId || clientId === '__GOOGLE_CLIENT_ID__') {
      errorMessage = 'VITE_GOOGLE_CLIENT_ID is not configured'
      return
    }

    try {
      await loadGsiScript()
    } catch (e) {
      errorMessage = e instanceof Error ? e.message : String(e)
      return
    }

    const googleId = window.google?.accounts?.id
    if (!googleId || !buttonContainer) {
      errorMessage = 'Google Sign-In failed to initialize'
      return
    }

    googleId.initialize({
      client_id: clientId,
      callback: (response) => void handleCredential(response),
      auto_select: true,
      itp_support: true,
      use_fedcm_for_prompt: true,
    })
    // Auto sign-in for returning users; button below is the fallback.
    googleId.prompt()
    googleId.renderButton(buttonContainer, {
      theme: 'filled_blue',
      size: 'large',
      text: 'signin_with',
      shape: 'pill',
      width: 280,
    })
  })
</script>

<div class="login-container">
  <a
    class="github-link"
    href="https://github.com/Julian-adv/OpenMMO"
    target="_blank"
    rel="noopener noreferrer"
    aria-label="GitHub repository"
  >
    <svg viewBox="0 0 16 16" width="28" height="28" fill="currentColor">
      <path
        d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z"
      />
    </svg>
  </a>
  <div class="login-wrapper">
    <svg
      class="arch-title"
      viewBox="-160 -80 1320 500"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <path id="archPath" d="M -20,360 Q 500,90 1020,360" fill="none" />
        <pattern
          id="flowerPattern"
          patternUnits="userSpaceOnUse"
          width="256"
          height="256"
        >
          <image href="/textures/flowerx4.png" width="256" height="256" />
        </pattern>
      </defs>
      <text stroke="white" stroke-width="3" paint-order="stroke">
        <textPath
          href="#archPath"
          startOffset="50%"
          text-anchor="middle"
          dominant-baseline="auto"
          fill="url(#flowerPattern)"
          font-family="'Black Han Sans', sans-serif"
          font-size="200">Open<tspan dx="40">MMO</tspan></textPath
        >
      </text>
    </svg>
    <h1 class="title">OpenMMO</h1>

    <div class="login-panel">
      {#if kickedMessage}
        <div class="kicked-message">{kickedMessage}</div>
      {/if}

      {#if errorMessage}
        <div class="error-message">{errorMessage}</div>
      {/if}

      <div class="google-signin" class:connecting={isConnecting}>
        <div bind:this={buttonContainer}></div>
        {#if isConnecting}
          <div class="connecting-label">Connecting...</div>
        {/if}
      </div>
    </div>

    <AnnouncementsPanel />
  </div>
</div>

<style>
  .login-container {
    position: fixed;
    inset: 0;
    box-sizing: border-box;
    width: 100%;
    max-width: 100vw;
    height: 100vh;
    height: 100dvh;
    padding: max(14px, env(safe-area-inset-top))
      max(14px, env(safe-area-inset-right))
      max(58px, calc(env(safe-area-inset-bottom) + 58px))
      max(14px, env(safe-area-inset-left));
    overflow-x: hidden;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    -webkit-overflow-scrolling: touch;
  }

  .github-link {
    position: absolute;
    top: max(14px, env(safe-area-inset-top));
    right: max(14px, env(safe-area-inset-right));
    display: flex;
    padding: 8px;
    color: #a0aec0;
    transition: color 0.15s;
  }

  .github-link:hover {
    color: #fff;
  }

  .login-wrapper {
    width: min(800px, 100%);
    min-width: 0;
    max-height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
  }

  .arch-title {
    width: min(800px, 100%);
    height: auto;
    aspect-ratio: 1320 / 500;
    margin-bottom: -20px;
    filter: drop-shadow(0 4px 16px rgba(0, 0, 0, 0.6));
  }

  .title {
    margin: 0 0 20px 0;
    color: #a0aec0;
    font-size: 18px;
    font-weight: 400;
    text-align: center;
    letter-spacing: 6px;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .login-panel {
    box-sizing: border-box;
    width: min(480px, 100%);
    min-width: 0;
    padding: 40px;
    background: rgba(0, 0, 0, 0.8);
    border: 1px solid #4a5568;
    border-radius: 12px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }

  .google-signin {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    min-height: 44px;
  }

  .google-signin.connecting {
    pointer-events: none;
    opacity: 0.6;
  }

  .connecting-label {
    color: #a0aec0;
    font-size: 13px;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .kicked-message {
    margin-bottom: 20px;
    padding: 12px 14px;
    background: rgba(236, 201, 75, 0.15);
    border: 1px solid #ecc94b;
    border-radius: 6px;
    color: #ecc94b;
    font-size: 13px;
    text-align: center;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .error-message {
    margin-bottom: 16px;
    padding: 10px 14px;
    background: rgba(245, 101, 101, 0.2);
    border: 1px solid #fc8181;
    border-radius: 6px;
    color: #fc8181;
    font-size: 13px;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  @media (max-width: 600px), (max-height: 700px) {
    .login-container {
      padding: max(10px, env(safe-area-inset-top))
        max(10px, env(safe-area-inset-right))
        max(52px, calc(env(safe-area-inset-bottom) + 52px))
        max(10px, env(safe-area-inset-left));
    }

    .arch-title {
      width: min(340px, 100%);
      margin-bottom: -12px;
    }

    .title {
      margin-bottom: 12px;
      font-size: 13px;
      letter-spacing: 4px;
    }

    .login-panel {
      width: min(320px, 100%);
      padding: 18px;
      border-radius: 8px;
    }

    .kicked-message {
      margin-bottom: 14px;
      padding: 10px 12px;
    }
  }

  @media (max-height: 560px) {
    .login-container {
      justify-content: flex-start;
    }

    .arch-title {
      width: min(320px, 100%);
      margin-bottom: -10px;
    }

    .login-panel {
      padding: 16px;
    }
  }
</style>
