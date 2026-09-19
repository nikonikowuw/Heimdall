import { useCallback, useSyncExternalStore } from 'react'
import { telemetryStore } from '@/lib/telemetryStore'
import type { CameraTelemetry } from '@/types'

export function subscribeCameraTelemetry(
  cameraId: string | undefined,
  onStoreChange: () => void,
): () => void {
  if (!cameraId) return () => {}
  return telemetryStore.subscribe(cameraId, onStoreChange)
}

export function getCameraTelemetrySnapshot(
  cameraId: string | undefined,
): CameraTelemetry | undefined {
  if (!cameraId) return undefined
  return telemetryStore.getTelemetry(cameraId)
}

/**
 * 针对单个摄像头的局部高效遥测订阅 Hook
 *
 * 遵循 Nuwa 前端规范：
 * 基于 useSyncExternalStore 实现无跳变粒度订阅，
 * 仅在目标摄像头的目标数/热度发生变化时触发当前卡片重新渲染，
 * 避免父级整个摄像头列表频繁重排重绘。
 */
export function useCameraTelemetry(cameraId: string | undefined): CameraTelemetry | undefined {
  const subscribe = useCallback(
    (onStoreChange: () => void) => subscribeCameraTelemetry(cameraId, onStoreChange),
    [cameraId],
  )

  const getSnapshot = useCallback(() => getCameraTelemetrySnapshot(cameraId), [cameraId])

  return useSyncExternalStore(subscribe, getSnapshot, () => undefined)
}
