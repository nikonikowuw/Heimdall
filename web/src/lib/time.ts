/**
 * 时间格式化工具 — 集中管理所有时间显示逻辑
 *
 * 规则：
 * - DTO 时间字段是 13 位 UTC 毫秒整数 (number)
 * - 显示时通过 Intl.DateTimeFormat 转换，不使用 date.toLocaleString()
 * - 不使用 Date 对象做格式化以外的计算
 */

/** UTC 毫秒 → 本地化日期时间字符串 */
export function formatTimestamp(ms: number, locale = 'zh-CN'): string {
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(new Date(ms))
}

/** UTC 毫秒 → 仅日期时间（无秒） */
export function formatTimestampShort(ms: number, locale = 'zh-CN'): string {
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(new Date(ms))
}

/** UTC 毫秒 → 相对时间（"3 分钟前" / "just now"） */
export function formatRelativeTime(ms: number, locale = 'zh-CN'): string {
  const diffMs = Date.now() - ms
  const absDiff = Math.abs(diffMs)
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' })

  if (absDiff < 60_000) {
    return rtf.format(-0, 'second')
  }
  if (absDiff < 3_600_000) {
    return rtf.format(-Math.floor(absDiff / 60_000), 'minute')
  }
  if (absDiff < 86_400_000) {
    return rtf.format(-Math.floor(absDiff / 3_600_000), 'hour')
  }
  return formatTimestamp(ms, locale)
}
