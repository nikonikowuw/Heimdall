import { AlertCircle, AlertTriangle, Info, type LucideIcon } from 'lucide-react'
import type { OperationalLogLevel } from '@/types'

/**
 * 日志语义色调词汇表。
 *
 * 状态码、事件级别、HTTP 方法与耗时共用同一套色调，避免 2xx/4xx/5xx 与
 * info/warn/error 的着色级联在表格、统计卡与详情抽屉里各写一遍。
 * 颜色只取自 globals.css 暴露的 CSS 变量 token，不写具名 Tailwind 色值。
 */
export type LogTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'

export interface LogToneClasses {
  /** 描边徽章：表格单元格与抽屉标题栏 */
  badge: string
  /** 图标底托与浅色块 */
  chip: string
  /** 状态圆点 */
  dot: string
  /** 实心分段器选中态 */
  solid: string
  /** 纯文本着色 */
  text: string
}

const TONE_CLASSES: Record<LogTone, LogToneClasses> = {
  neutral: {
    badge: 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)]',
    chip: 'bg-[var(--bg-secondary)] text-[var(--text-muted)]',
    dot: 'bg-[var(--text-muted)]',
    solid: 'bg-[var(--text-secondary)] text-white',
    text: 'text-[var(--text-secondary)]',
  },
  accent: {
    badge: 'border-[var(--accent)]/30 bg-[var(--accent-soft)] text-[var(--accent)]',
    chip: 'bg-[var(--accent-soft)] text-[var(--accent)]',
    dot: 'bg-[var(--accent)]',
    solid: 'bg-[var(--accent)] text-white',
    text: 'text-[var(--accent)]',
  },
  success: {
    badge: 'border-[var(--accent-green)]/30 bg-[var(--accent-green)]/10 text-[var(--accent-green)]',
    chip: 'bg-[var(--accent-green)]/10 text-[var(--accent-green)]',
    dot: 'bg-[var(--accent-green)]',
    solid: 'bg-[var(--accent-green)] text-white',
    text: 'text-[var(--accent-green)]',
  },
  warning: {
    badge: 'border-[var(--accent-amber)]/30 bg-[var(--accent-amber)]/10 text-[var(--accent-amber)]',
    chip: 'bg-[var(--accent-amber)]/10 text-[var(--accent-amber)]',
    dot: 'bg-[var(--accent-amber)]',
    solid: 'bg-[var(--accent-amber)] text-white',
    text: 'text-[var(--accent-amber)]',
  },
  danger: {
    badge: 'border-[var(--destructive)]/30 bg-[var(--destructive)]/10 text-[var(--destructive)]',
    chip: 'bg-[var(--destructive)]/10 text-[var(--destructive)]',
    dot: 'bg-[var(--destructive)]',
    solid: 'bg-[var(--destructive)] text-white',
    text: 'text-[var(--destructive)]',
  },
}

export function getToneClasses(tone: LogTone): LogToneClasses {
  return TONE_CLASSES[tone]
}

/**
 * HTTP 状态码语义分类，键名与 oplog namespace 的 `status.*` 翻译一一对应。
 */
export type HttpStatusClass = 'success' | 'clientError' | 'serverError' | 'other'

export function classifyHttpStatus(statusCode: number): HttpStatusClass {
  if (statusCode >= 200 && statusCode < 300) return 'success'
  if (statusCode >= 400 && statusCode < 500) return 'clientError'
  if (statusCode >= 500) return 'serverError'
  return 'other'
}

const STATUS_CLASS_TONES: Record<HttpStatusClass, LogTone> = {
  success: 'success',
  clientError: 'warning',
  serverError: 'danger',
  other: 'neutral',
}

/** 耗时分级，键名与 oplog namespace 的 `stats.{fast,normal,slow}` 翻译一一对应 */
export type LatencyTier = 'fast' | 'normal' | 'slow'

/** 100ms 以内视为本地接口正常水位，500ms 以上进入需关注区间 */
export function classifyLatency(durationMs: number): LatencyTier {
  if (durationMs < 100) return 'fast'
  if (durationMs < 500) return 'normal'
  return 'slow'
}

const LATENCY_TIER_TONES: Record<LatencyTier, LogTone> = {
  fast: 'success',
  normal: 'warning',
  slow: 'danger',
}

/** 运维事件级别 → 色调，级别契约见 crates/types/src/oplog.rs 的 OpEvent::level */
export function levelTone(level: OperationalLogLevel): LogTone {
  switch (level) {
    case 'error':
      return 'danger'
    case 'warn':
      return 'warning'
    case 'info':
      return 'accent'
  }
}

/** 运维事件级别 → 图标 */
export const LEVEL_ICONS: Record<OperationalLogLevel, LucideIcon> = {
  error: AlertCircle,
  warn: AlertTriangle,
  info: Info,
}

/** HTTP 方法 → 色调，用于审计表格的方法徽章 */
export function methodTone(method: string): LogTone {
  switch (method.toUpperCase()) {
    case 'GET':
      return 'success'
    case 'POST':
      return 'accent'
    case 'PUT':
    case 'PATCH':
      return 'warning'
    case 'DELETE':
      return 'danger'
    default:
      return 'neutral'
  }
}

export function statusTone(statusCode: number): LogTone {
  return STATUS_CLASS_TONES[classifyHttpStatus(statusCode)]
}

export function latencyTone(durationMs: number): LogTone {
  return LATENCY_TIER_TONES[classifyLatency(durationMs)]
}
