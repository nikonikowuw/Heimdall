import { useCallback, useEffect, useState } from 'react'
import { recordingApi } from '@/lib/api'
import type { EventRecordingType, RecordingDetail } from '@/types'

export interface UseEventRecordingResult {
  /** 关联录像详情；加载中、无关联或查询失败时均为 null */
  recording: RecordingDetail | null
  isLoading: boolean
  /** 查询失败原因；与「该事件确实没有关联录像」是两种语义，调用方需区分渲染 */
  error: string | null
  /** 失败后重试；无失败时调用无副作用 */
  retry: () => void
}

interface SettledRecording {
  query: string
  recording: RecordingDetail | null
  error: string | null
}

/**
 * 按事件反查关联录像（告警 / 抓拍详情回放入口）。
 *
 * 事件类型白名单由服务端校验；无关联时服务端返回 `data: null` 而非 404，
 * 因此「查不到」与「查询失败」必须以独立状态暴露，避免把网络故障伪装成无录像。
 */
export function useEventRecording(
  eventType: EventRecordingType,
  eventId: string,
): UseEventRecordingResult {
  const [refreshVersion, setRefreshVersion] = useState(0)
  // 查询签名：事件标识与重试代次共同决定一次查询的归属；
  // 用签名派生 isLoading 而不是在 effect 内 setIsLoading，
  // 重新查询时旧结果立即失效，无需额外一轮渲染清零。
  const query = eventId ? `${eventType}:${eventId}:${refreshVersion}` : ''

  const [settled, setSettled] = useState<SettledRecording | null>(null)

  const retry = useCallback(() => {
    setRefreshVersion((version) => version + 1)
  }, [])

  useEffect(() => {
    if (!eventId) return

    // 中止过期请求：详情可在告警/抓拍之间快速切换，迟到响应不得覆盖新结果
    const controller = new AbortController()
    recordingApi
      .findByEvent(eventType, eventId, controller.signal)
      .then((recording) => {
        setSettled({ query, recording, error: null })
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return
        setSettled({
          query,
          recording: null,
          error: error instanceof Error ? error.message : String(error),
        })
      })

    return () => controller.abort()
  }, [eventType, eventId, query])

  const current = settled !== null && settled.query === query ? settled : null

  return {
    recording: current?.recording ?? null,
    isLoading: Boolean(query) && current === null,
    error: current?.error ?? null,
    retry,
  }
}
