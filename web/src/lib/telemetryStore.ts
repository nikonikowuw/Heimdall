import type { CameraTelemetry } from '@/types'

/** 遥测快照过期时间，防止断流后 HUD 长时间显示旧热度。 */
export const CAMERA_TELEMETRY_TTL_MS = 1500

export type TelemetryListener = () => void

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function isCameraTelemetry(value: unknown): value is CameraTelemetry {
  if (!isRecord(value)) return false

  return (
    typeof value.cameraId === 'string' &&
    value.cameraId.length > 0 &&
    typeof value.timestamp === 'number' &&
    Number.isFinite(value.timestamp) &&
    typeof value.activeTracks === 'number' &&
    Number.isFinite(value.activeTracks) &&
    typeof value.personCount === 'number' &&
    Number.isFinite(value.personCount) &&
    typeof value.carCount === 'number' &&
    Number.isFinite(value.carCount) &&
    typeof value.motionScore === 'number' &&
    Number.isFinite(value.motionScore) &&
    value.motionScore >= 0 &&
    value.motionScore <= 1 &&
    typeof value.isMotionGated === 'boolean'
  )
}

class CameraTelemetryStore {
  private snapshots = new Map<string, CameraTelemetry>()
  private updatedAt = new Map<string, number>()
  private listeners = new Map<string, Set<TelemetryListener>>()
  private lastNotifiedAt = new Map<string, number>()
  private expiryTimers = new Map<string, ReturnType<typeof setTimeout>>()

  setTelemetry(telemetry: CameraTelemetry): void {
    const cameraId = telemetry.cameraId
    const now = Date.now()
    this.snapshots.set(cameraId, telemetry)
    this.updatedAt.set(cameraId, now)

    const oldTimer = this.expiryTimers.get(cameraId)
    if (oldTimer) clearTimeout(oldTimer)
    const expiryTimer = setTimeout(() => {
      if (this.updatedAt.get(cameraId) !== now) return
      this.snapshots.delete(cameraId)
      this.updatedAt.delete(cameraId)
      this.lastNotifiedAt.delete(cameraId)
      this.expiryTimers.delete(cameraId)
      this.listeners.get(cameraId)?.forEach((listener) => listener())
    }, CAMERA_TELEMETRY_TTL_MS)
    this.expiryTimers.set(cameraId, expiryTimer)

    const lastNotified = this.lastNotifiedAt.get(cameraId) ?? 0
    if (now - lastNotified < 100) return
    this.lastNotifiedAt.set(cameraId, now)

    this.listeners.get(cameraId)?.forEach((listener) => listener())
  }

  getTelemetry(cameraId: string): CameraTelemetry | undefined {
    const telemetry = this.snapshots.get(cameraId)
    const updatedAt = this.updatedAt.get(cameraId)
    if (!telemetry || updatedAt === undefined || Date.now() - updatedAt > CAMERA_TELEMETRY_TTL_MS) {
      return undefined
    }
    return telemetry
  }

  subscribe(cameraId: string, listener: TelemetryListener): () => void {
    if (!cameraId) return () => {}

    let listeners = this.listeners.get(cameraId)
    if (!listeners) {
      listeners = new Set()
      this.listeners.set(cameraId, listeners)
    }
    listeners.add(listener)

    return () => {
      const activeListeners = this.listeners.get(cameraId)
      if (!activeListeners) return
      activeListeners.delete(listener)
      if (activeListeners.size === 0) this.listeners.delete(cameraId)
    }
  }

  clear(cameraId?: string): void {
    if (cameraId) {
      this.snapshots.delete(cameraId)
      this.updatedAt.delete(cameraId)
      this.lastNotifiedAt.delete(cameraId)
      const expiryTimer = this.expiryTimers.get(cameraId)
      if (expiryTimer) clearTimeout(expiryTimer)
      this.expiryTimers.delete(cameraId)
      this.listeners.get(cameraId)?.forEach((listener) => listener())
      return
    }

    this.snapshots.clear()
    this.updatedAt.clear()
    this.lastNotifiedAt.clear()
    this.expiryTimers.forEach((timer) => clearTimeout(timer))
    this.expiryTimers.clear()
    this.listeners.forEach((listeners) => listeners.forEach((listener) => listener()))
  }
}

export const telemetryStore = new CameraTelemetryStore()
