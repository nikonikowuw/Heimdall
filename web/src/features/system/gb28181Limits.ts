/**
 * GB28181 页面数值字段允许区间。
 *
 * 端口字段 wire 类型为 u16，上下界即类型边界；心跳上界 86400（一天）是操作意义的
 * 软约束、非后端硬约束（wire 为 u32），仅用于防误输入，与输入框区间同源。
 */
export const GB28181_LIMITS = {
  sipPort: { min: 1, max: 65535 },
  rtpPort: { min: 1, max: 65535 },
  heartbeatTimeoutSec: { min: 1, max: 86400 },
} as const
