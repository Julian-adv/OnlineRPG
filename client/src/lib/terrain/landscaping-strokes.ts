import type { LandscapingStroke } from './landscaping'
import { shortestWrappedDeltaX } from './world-wrap'

export class LandscapingStrokes {
  private queued: LandscapingStroke[] = []
  private anchor: [number, number] | null = null
  private current: LandscapingStroke | null = null

  begin(stroke: LandscapingStroke) {
    this.finish()
    this.queued.push(stroke)
    this.anchor = stroke.start
    this.current = stroke
  }

  move(stroke: LandscapingStroke) {
    if (this.anchor) this.current = stroke
  }

  finish() {
    if (this.current && this.anchor && this.distance() > 0.01) {
      this.queued.push(this.segment())
    }
    this.anchor = null
    this.current = null
  }

  addRoad(stroke: LandscapingStroke) {
    this.queued.push(stroke)
  }

  take(): LandscapingStroke | null {
    const queued = this.queued.shift()
    if (queued) return queued
    if (!this.current || !this.anchor) return null
    const stroke = this.segment()
    this.anchor = this.current.start
    return stroke
  }

  clear() {
    this.queued = []
    this.anchor = null
    this.current = null
  }

  private distance() {
    return Math.hypot(
      shortestWrappedDeltaX(this.anchor![0], this.current!.start[0]),
      this.current!.start[1] - this.anchor![1]
    )
  }

  private segment(): LandscapingStroke {
    const current = this.current!
    const distance = this.distance()
    if (distance <= 0.01 || distance > 362.1) return current
    return { ...current, start: this.anchor!, end: current.start }
  }
}
