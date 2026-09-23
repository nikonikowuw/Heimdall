export const RECOGNITION_STATUS_STYLES = {
  confirmed: {
    textColor: 'text-[var(--status-success)]',
    chipClass:
      'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)]',
    dotClass: 'bg-[var(--status-success)] shadow-[0_0_8px_var(--status-success-soft)]',
    ambientGlow: 'from-[var(--status-success-soft)]',
    cardBorder:
      'hover:border-[var(--status-success-border)] hover:shadow-[0_0_12px_var(--status-success-soft)]',
  },
  pending_review: {
    textColor: 'text-[var(--status-warning)]',
    chipClass:
      'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)]',
    // 状态点位于卡片毛玻璃栈内部：backdrop-filter 会因子树内任何持续动画而逐帧重采样，
    // 实测告警中心（24 张卡片）因此持续占用一个 CPU 核心的约 50%，冻结动画后回落至 0.4%。
    // 待复核语义由静态高亮 + 发光承担，不加动画。
    dotClass: 'bg-[var(--status-warning)] shadow-[0_0_8px_var(--status-warning-soft)]',
    ambientGlow: 'from-[var(--status-warning-soft)]',
    cardBorder:
      'border-[var(--status-warning-border)] hover:border-[var(--status-warning-border)] hover:shadow-[0_0_12px_var(--status-warning-soft)]',
  },
  rejected: {
    textColor: 'text-[var(--status-danger)]',
    chipClass:
      'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]',
    dotClass: 'bg-[var(--status-danger)]/80',
    ambientGlow: 'from-[var(--status-danger-soft)]',
    cardBorder:
      'border-[var(--status-danger-border)] opacity-85 hover:opacity-100 hover:border-[var(--status-danger-border)]',
  },
} as const

export const SIMILARITY_STYLES = {
  high: 'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)] shadow-[0_0_12px_var(--status-success-soft)]',
  medium:
    'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)] shadow-[0_0_12px_var(--status-warning-soft)]',
  low: 'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)] shadow-[0_0_12px_var(--status-danger-soft)]',
} as const
