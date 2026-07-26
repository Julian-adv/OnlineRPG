<script lang="ts">
  import { useTask, useThrelte } from '@threlte/core'
  import { createRenderCadence } from '../utils/renderCadence'
  import {
    getEffectivePreset,
    graphicsQuality,
  } from '../stores/graphicsSettings'

  const { invalidate } = useThrelte()
  const cadence = $derived(
    createRenderCadence(getEffectivePreset($graphicsQuality).maxRenderFps)
  )

  useTask(
    (delta) => {
      if (cadence.shouldRender(delta * 1000)) invalidate()
    },
    { autoInvalidate: false }
  )
</script>
