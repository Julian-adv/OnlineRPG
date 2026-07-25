<script lang="ts">
  import { useTask, useThrelte } from '@threlte/core'
  import {
    TARGET_RENDER_FPS,
    createRenderCadence,
  } from '../utils/renderCadence'

  const { invalidate } = useThrelte()
  const cadence = createRenderCadence(TARGET_RENDER_FPS)

  useTask(
    (delta) => {
      if (cadence.shouldRender(delta)) invalidate()
    },
    { autoInvalidate: false }
  )
</script>
