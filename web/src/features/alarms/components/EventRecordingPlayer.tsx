import React from 'react'
import { useTranslation } from 'react-i18next'
import { recordingApi } from '@/lib/api'
import type { RecordingDetail } from '@/types'

export interface EventRecordingPlayerProps {
  recording: RecordingDetail
  /** 事件在文件内的偏移（毫秒）；元数据就绪后自动定位到触发时刻 */
  initialOffsetMs: number
  /** 覆盖默认尺寸类；调用方浮层容器不同时需要自行约束宽高 */
  className?: string
}

/**
 * 事件录像播放器。
 *
 * 录像为 fMP4 容器，浏览器可原生播放并按 Range 拖动；打开后直接定位到事件时刻，
 * 而不是从头播放。组件卸载即销毁播放器与在途请求，不残留媒体会话。
 */
export function EventRecordingPlayer({
  recording,
  initialOffsetMs,
  className,
}: EventRecordingPlayerProps): React.ReactElement {
  const { t } = useTranslation('recording')

  const handleLoadedMetadata = (video: HTMLVideoElement) => {
    const target = initialOffsetMs / 1000
    if (!Number.isFinite(target) || target <= 0) return

    const { duration } = video
    // 事件可能落在最后一帧附近，留出余量避免 seek 被浏览器夹回并触发额外请求
    video.currentTime =
      Number.isFinite(duration) && duration > 0
        ? Math.min(target, Math.max(0, duration - 0.5))
        : target
  }

  return (
    <video
      src={recordingApi.getFileUrl(recording.recordingId)}
      controls
      playsInline
      preload="metadata"
      aria-label={t('playback.playerLabel', { defaultValue: '事件录像播放器' })}
      onLoadedMetadata={(event) => handleLoadedMetadata(event.currentTarget)}
      className={className ?? 'max-h-full max-w-full rounded-xl bg-black shadow-2xl'}
    />
  )
}
