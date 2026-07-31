/**
 * First-launch GPU probe. Runs a compute workload on a throwaway WebGPU
 * device and reports a throughput score, so the initial graphics preset can
 * be picked from measured speed instead of defaulting everyone to medium.
 *
 * Compute rather than a render pass: `queue.onSubmittedWorkDone()` gives a
 * real GPU completion signal, so the timing never has to fight vsync
 * clamping or compositor latency the way rAF-based fill-rate probes do.
 *
 * Must run before the renderer exists — `antialias` is fixed at
 * `new WebGPURenderer()` and can't be changed without a reload.
 *
 * Reference measurements, both Chrome 150:
 *   RTX 4090 (nvidia/lovelace, Windows)      ~1730, overhead ~3.0ms, ±8%
 *   M3 10-core (apple/metal-3, MacBook Air 15) ~92, overhead ~0.6ms, ±1.5%
 * The ~19x gap tracks their fp32 throughput ratio, so the probe is measuring
 * something real. Probe cost runs 110-300ms.
 */

const INVOCATIONS = 1 << 20
const WORKGROUP_SIZE = 64
/** Submit + queue-drain + promise-resolution overhead is ~3ms on a warm
 *  desktop queue and swamps small dispatches: measured on an RTX 4090, every
 *  workload up to 1024 iterations came back at a flat ~2.6-3.5ms. So start
 *  above that floor, subtract a measured baseline, and only trust a
 *  measurement once GPU work dominates it. */
const START_ITERATIONS = 1024
const TARGET_MS = 25
const MAX_DOUBLINGS = 10
/** Dispatches discarded before measuring. These must carry real GPU work, not
 *  a token amount: Apple Silicon ramps its clocks under sustained load, and an
 *  M3 measured 48 on its first loaded dispatch against a settled 88-93. A
 *  warmup of short, overhead-bound dispatches never triggers the ramp, so it
 *  reuses START_ITERATIONS rather than some smaller count. */
const WARMUP_DISPATCHES = 2
/** Hard ceiling on the whole probe, including adapter/device creation. */
const TIMEOUT_MS = 3000

const SHADER = /* wgsl */ `
struct Params { iterations: u32 };

@group(0) @binding(0) var<storage, read_write> data: array<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@compute @workgroup_size(${WORKGROUP_SIZE})
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  // Seed off the invocation id: buffers arrive zero-initialized, and an
  // all-zero accumulator would keep sin()/fma() on zero the whole way,
  // which some hardware retires on a denormal fast path.
  var acc = data[i] + f32(i & 1023u) * 0.001 + 0.5;
  // Mixed transcendental + FMA so the result can't be folded away and the
  // score reflects general shader throughput rather than one ALU port.
  for (var k = 0u; k < params.iterations; k = k + 1u) {
    acc = fma(acc, 1.0000001, sin(acc));
  }
  data[i] = acc;
}
`

interface GpuLike {
  requestAdapter(): Promise<GpuAdapterLike | null>
}
interface GpuAdapterLike {
  requestDevice(): Promise<GpuDeviceLike>
}
/** Structural subset of the WebGPU types we touch — @webgpu/types isn't a
 *  dependency, and pulling it in for one file isn't worth it. */
interface GpuDeviceLike {
  createShaderModule(d: { code: string }): unknown
  createBuffer(d: { size: number; usage: number }): { destroy(): void }
  createComputePipeline(d: unknown): {
    getBindGroupLayout(i: number): unknown
  }
  createBindGroup(d: unknown): unknown
  createCommandEncoder(): {
    beginComputePass(): {
      setPipeline(p: unknown): void
      setBindGroup(i: number, g: unknown): void
      dispatchWorkgroups(n: number): void
      end(): void
    }
    finish(): unknown
  }
  queue: {
    submit(b: unknown[]): void
    writeBuffer(b: unknown, off: number, data: ArrayBufferView): void
    onSubmittedWorkDone(): Promise<void>
  }
  destroy(): void
}

// GPUBufferUsage constants, inlined so the file needs no WebGPU type globals.
const USAGE_STORAGE = 0x0080
const USAGE_UNIFORM = 0x0040
const USAGE_COPY_DST = 0x0008

export interface GpuBenchmarkResult {
  /** Billions of loop iterations retired per second. Higher is faster. */
  score: number
  /** Wall-clock cost of the probe, for load-budget accounting. */
  elapsedMs: number
}

/** 'no-webgpu' means the renderer will fall back to WebGL (often software
 *  rendering) — `qualityForOutcome` maps it to the slowest tier, unlike a
 *  plain inconclusive null. */
export type GpuBenchmarkOutcome = GpuBenchmarkResult | 'no-webgpu' | null

async function probe(): Promise<GpuBenchmarkOutcome> {
  const gpu = (navigator as Navigator & { gpu?: GpuLike }).gpu
  if (!gpu) return 'no-webgpu'

  const startedAt = performance.now()
  const adapter = await gpu.requestAdapter()
  if (!adapter) return 'no-webgpu'
  const device = await adapter.requestDevice()

  try {
    const module = device.createShaderModule({ code: SHADER })
    const dataBuffer = device.createBuffer({
      size: INVOCATIONS * 4,
      usage: USAGE_STORAGE | USAGE_COPY_DST,
    })
    const paramsBuffer = device.createBuffer({
      size: 16,
      usage: USAGE_UNIFORM | USAGE_COPY_DST,
    })
    const pipeline = device.createComputePipeline({
      layout: 'auto',
      compute: { module, entryPoint: 'main' },
    })
    const bindGroup = device.createBindGroup({
      layout: pipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: dataBuffer } },
        { binding: 1, resource: { buffer: paramsBuffer } },
      ],
    })

    const dispatch = async (iterations: number): Promise<number> => {
      device.queue.writeBuffer(
        paramsBuffer,
        0,
        new Uint32Array([iterations, 0, 0, 0])
      )
      const encoder = device.createCommandEncoder()
      const pass = encoder.beginComputePass()
      pass.setPipeline(pipeline)
      pass.setBindGroup(0, bindGroup)
      pass.dispatchWorkgroups(INVOCATIONS / WORKGROUP_SIZE)
      pass.end()
      const t0 = performance.now()
      device.queue.submit([encoder.finish()])
      await device.queue.onSubmittedWorkDone()
      return performance.now() - t0
    }

    // A single-iteration dispatch does no meaningful GPU work, so what it
    // measures is the fixed per-submit cost. Median of three to shrug off a
    // stray scheduling hiccup. Taken before the warmup so the short dispatches
    // can't undo the clock ramp the warmup is there to establish.
    const baselines = [await dispatch(1), await dispatch(1), await dispatch(1)]
    baselines.sort((a, b) => a - b)
    const overheadMs = baselines[1]

    // Discard loaded dispatches: pipeline creation, driver first-use warmup,
    // a cold queue, and the GPU's clock ramp all land here.
    for (let i = 0; i < WARMUP_DISPATCHES; i++) {
      await dispatch(START_ITERATIONS)
    }

    let iterations = START_ITERATIONS
    let gpuMs = (await dispatch(iterations)) - overheadMs
    for (let i = 0; i < MAX_DOUBLINGS && gpuMs < TARGET_MS; i++) {
      iterations *= 2
      gpuMs = (await dispatch(iterations)) - overheadMs
    }

    // Re-measure the settled workload and take the median. Windows/NVIDIA
    // queue latency is noisy enough that a lone sample swung ±25% run to run,
    // and that noise rides straight through the overhead subtraction.
    const samples = [
      gpuMs,
      (await dispatch(iterations)) - overheadMs,
      (await dispatch(iterations)) - overheadMs,
    ]
    samples.sort((a, b) => a - b)
    gpuMs = samples[1]

    dataBuffer.destroy()
    paramsBuffer.destroy()

    // Guard against a device so fast the workload never escapes the noise
    // floor; a nonsense score is worse than no score.
    if (gpuMs <= 1) return null

    const opsPerSecond = (INVOCATIONS * iterations) / (gpuMs / 1000)
    return {
      score: opsPerSecond / 1e9,
      elapsedMs: performance.now() - startedAt,
    }
  } finally {
    device.destroy()
  }
}

/** Run the probe. Resolves 'no-webgpu' when no adapter exists, or null if the
 *  probe throws or overruns its time budget (callers keep the default preset). */
export async function runGpuBenchmark(): Promise<GpuBenchmarkOutcome> {
  const timeout = new Promise<null>((resolve) =>
    setTimeout(() => resolve(null), TIMEOUT_MS)
  )
  try {
    return await Promise.race([probe(), timeout])
  } catch {
    return null
  }
}
