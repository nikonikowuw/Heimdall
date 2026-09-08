/**
 * 系统监控组件共享颜色工具
 *
 * 统一 CPU/NPU 使用率与温度的颜色编码逻辑，
 * 避免各组件重复实现。
 */

/** 使用率对应的强调色（阈值: 60/80/95） */
export function getUsageColor(percent: number): string {
  if (percent >= 95) return 'var(--destructive)'
  if (percent >= 80) return 'var(--accent-amber)'
  if (percent >= 60) return 'var(--accent)'
  return 'var(--accent-green)'
}

/** 使用率对应的半透明背景色 */
export function getUsageBgColor(percent: number): string {
  if (percent >= 95) return 'color-mix(in srgb, var(--destructive) 15%, transparent)'
  if (percent >= 80) return 'color-mix(in srgb, var(--accent-amber) 15%, transparent)'
  if (percent >= 60) return 'color-mix(in srgb, var(--accent) 15%, transparent)'
  return 'color-mix(in srgb, var(--accent-green) 15%, transparent)'
}

/** 温度对应的强调色（阈值: 50/70/85） */
export function getTemperatureColor(temp: number): string {
  if (temp >= 85) return 'var(--destructive)'
  if (temp >= 70) return 'var(--accent-amber)'
  if (temp >= 50) return 'var(--accent)'
  return 'var(--accent-green)'
}

/** 温度对应的半透明背景色 */
export function getTemperatureBgColor(temp: number): string {
  if (temp >= 85) return 'color-mix(in srgb, var(--destructive) 15%, transparent)'
  if (temp >= 70) return 'color-mix(in srgb, var(--accent-amber) 15%, transparent)'
  if (temp >= 50) return 'color-mix(in srgb, var(--accent) 15%, transparent)'
  return 'color-mix(in srgb, var(--accent-green) 15%, transparent)'
}
