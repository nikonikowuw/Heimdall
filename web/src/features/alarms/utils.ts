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
