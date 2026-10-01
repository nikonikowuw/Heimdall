import { recordingApi } from '@/lib/api'
import type { RecordingDetail } from '@/types'

/**
 * fMP4（moof/mdat 分片）→ 标准非分片 MP4。
 *
 * 浏览器可直接播放 fMP4，但部分外部播放器与剪辑软件只认带完整样本表的普通 MP4；
 * 导出时在客户端重排容器结构，编码数据按原样搬运、不重新编码，因此无质量损失，
 * 后端不参与任何处理（宿主零负载）。
 *
 * mediabunny 体积不小而导出是低频操作，因此按需加载，不拖累监控页首屏。
 */
export async function remuxFmp4ToMp4(fmp4: ArrayBuffer): Promise<ArrayBuffer> {
  const { ALL_FORMATS, BufferSource, BufferTarget, Conversion, Input, Mp4OutputFormat, Output } =
    await import('mediabunny')

  const input = new Input({ source: new BufferSource(fmp4), formats: ALL_FORMATS })
  try {
    const output = new Output({
      format: new Mp4OutputFormat(),
      target: new BufferTarget(),
    })
    const conversion = await Conversion.init({ input, output })
    if (!conversion.isValid) {
      const reasons = conversion.discardedTracks.map((item) => item.reason).join(', ')
      throw new Error(`remux rejected: ${reasons}`)
    }
    await conversion.execute()

    const buffer = output.target.buffer
    if (!buffer) {
      throw new Error('remux produced no output')
    }
    return buffer
  } finally {
    input.dispose()
  }
}

/** 导出文件名：通道 + 录像起始本地时间，便于与后端落盘文件对应 */
export function buildExportFileName(recording: RecordingDetail): string {
  const at = new Date(recording.startTime)
  const pad = (value: number) => String(value).padStart(2, '0')
  const stamp = `${at.getFullYear()}${pad(at.getMonth() + 1)}${pad(at.getDate())}_${pad(at.getHours())}${pad(at.getMinutes())}${pad(at.getSeconds())}`
  return `recording_${recording.cameraId}_${stamp}.mp4`
}

function triggerDownload(data: ArrayBuffer, fileName: string): void {
  const url = URL.createObjectURL(new Blob([data], { type: 'video/mp4' }))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = fileName
  anchor.rel = 'noopener'
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  // 立即释放会让部分浏览器来不及读取 blob，延后一拍回收
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}

/**
 * 拉取录像并导出为标准 MP4。失败时抛错，由调用方呈现本地化文案。
 */
export async function exportRecordingAsMp4(recording: RecordingDetail): Promise<void> {
  const response = await fetch(recordingApi.getFileUrl(recording.recordingId))
  if (!response.ok) {
    throw new Error(`fetch recording failed: HTTP ${response.status}`)
  }
  const fmp4 = await response.arrayBuffer()
  triggerDownload(await remuxFmp4ToMp4(fmp4), buildExportFileName(recording))
}
