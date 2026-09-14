import type { DateTimeRangeValue } from './components/DateTimeRangePicker'

const PRESET_DURATIONS_MS: Record<string, number> = {
  '5m': 5 * 60_000,
  '15m': 15 * 60_000,
  '30m': 30 * 60_000,
  '1h': 3_600_000,
  '24h': 86_400_000,
  '7d': 7 * 86_400_000,
}

/**
 * 实时解析时间范围值中的实际起止 UTC 毫秒时间戳
 * - 对于 'all'：无起止时间限制（均为 undefined）
 * - 对于 'custom'：严格遵循指定的 startTime 与 endTime
 * - 对于 'today'：起始为当天 00:00:00，截止为 undefined（无上限截断，确保新告警可被查出）
 * - 对于相对时间预设（'5m', '15m', '30m', '1h', '24h', '7d'）：基于当前时刻 now 动态回溯滑动窗口
 */
export function resolveEffectiveTimeRange(
  range: DateTimeRangeValue,
  now = Date.now(),
): {
  startTime?: number
  endTime?: number
} {
  if (range.quickPreset === 'all') {
    return { startTime: undefined, endTime: undefined }
  }

  if (range.quickPreset === 'custom') {
    return { startTime: range.startTime, endTime: range.endTime }
  }

  if (range.quickPreset === 'today') {
    const todayStart = new Date(now)
    todayStart.setHours(0, 0, 0, 0)
    return {
      startTime: todayStart.getTime(),
      endTime: undefined,
    }
  }

  const duration = PRESET_DURATIONS_MS[range.quickPreset]
  if (duration) {
    return {
      startTime: now - duration,
      endTime: now,
    }
  }

  return { startTime: range.startTime, endTime: range.endTime }
}

/**
 * 格式化展示 Unix 毫秒时间戳或 ISO 日期时间字符串
 */
export function formatTimestamp(val?: number | string | null): string {
  if (!val) return '-'
  if (typeof val === 'number') {
    return new Date(val).toLocaleString()
  }
  const parsed = Date.parse(val)
  if (!Number.isNaN(parsed)) {
    return new Date(parsed).toLocaleString()
  }
  return String(val)
}

export interface ParsedBBoxCoords {
  x1: number
  y1: number
  x2: number
  y2: number
}

/**
 * 解析归一化对角两点式 BBox 坐标
 * 支持 [x1, y1, x2, y2] 数组或具名对象 { x1, y1, x2, y2 }
 */
export function parseBBoxCoords(raw?: string): ParsedBBoxCoords | null {
  if (!raw) return null
  try {
    const parsed = JSON.parse(raw)
    if (Array.isArray(parsed) && parsed.length >= 4) {
      const [x1, y1, x2, y2] = parsed.map(Number)
      return { x1, y1, x2, y2 }
    }
    if (parsed && typeof parsed === 'object' && 'x1' in parsed && 'y1' in parsed) {
      return {
        x1: Number(parsed.x1),
        y1: Number(parsed.y1),
        x2: Number(parsed.x2),
        y2: Number(parsed.y2),
      }
    }
    return null
  } catch {
    return null
  }
}

/**
 * 映射布防规则展示标签
 */
export function getRuleTypeLabel(ruleType: string | undefined, t: (key: string) => string): string {
  return ruleType === 'line' ? t('types.lineCrossing') : t('types.regionIntrusion')
}

export function preloadImage(src: string): void {
  if (!src || typeof Image === 'undefined') return
  const image = new Image()
  image.src = src
}

export interface FittedImageRect {
  x: number
  y: number
  width: number
  height: number
}

/**
 * 根据容器尺寸和图片原始尺寸，计算 object-contain 模式下的实际渲染几何位置
 */
export function calculateFittedImageRect(
  containerWidth: number,
  containerHeight: number,
  naturalWidth: number,
  naturalHeight: number,
): FittedImageRect {
  const cW = containerWidth
  const cH = containerHeight
  const nW = naturalWidth || 1
  const nH = naturalHeight || 1

  const cRatio = cW / cH
  const iRatio = nW / nH

  if (iRatio > cRatio) {
    const width = cW
    const height = cW / iRatio
    return {
      width,
      height,
      x: 0,
      y: (cH - height) / 2,
    }
  }

  const height = cH
  const width = cH * iRatio
  return {
    width,
    height,
    x: (cW - width) / 2,
    y: 0,
  }
}

/**
 * 推导图片下载保存文件名：优先级自定义文件名 > URL 文件名 > 标题语义名称 > 时间戳保底
 */
export function deriveDownloadFilename(
  src: string,
  customFilename?: string,
  title?: string,
): string {
  if (customFilename?.trim()) {
    return customFilename.trim()
  }

  // 1. 尝试从 src 路径提取最后的文件名
  try {
    const base = typeof window !== 'undefined' ? window.location.href : 'http://localhost'
    const pathname = new URL(src, base).pathname
    const last = pathname.split('/').filter(Boolean).pop()
    if (last && /\.(jpg|jpeg|png|webp|bmp|gif|svg)$/i.test(last)) {
      return decodeURIComponent(last)
    }
  } catch {
    // ignore URL parsing error
  }

  // 2. 尝试从 title 生成业务可读的文件名
  const safeTitle = title?.replace(/[\\/:*?"<>|\s]+/g, '_').trim()
  if (safeTitle) {
    return `${safeTitle}.jpg`
  }

  // 3. 安全回退：带时间戳的文件名，杜绝重名覆盖
  return `image_${Date.now()}.jpg`
}
