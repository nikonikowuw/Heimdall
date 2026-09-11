import React from 'react'
import { useTranslation } from 'react-i18next'
import type { StreamMode } from '@/types'

export interface StreamModeSelectorProps {
  value: StreamMode
  onChange: (mode: StreamMode) => void
  disabled?: boolean
  labelClassName?: string
}

export function StreamModeSelector({
  value,
  onChange,
  disabled = false,
  labelClassName = 'text-xs font-medium text-[var(--text-secondary)]',
}: StreamModeSelectorProps): React.ReactElement {
  const { t } = useTranslation('camera')

  return (
    <div>
      <label className={labelClassName}>
        {t('manage.streamMode', { defaultValue: 'AI 分析码流选择' })}
      </label>
      <div className="mt-1.5 grid grid-cols-3 gap-2">
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange('main')}
          className={`flex flex-col items-center justify-center rounded-xl border p-2 text-center transition-all ${
            value === 'main'
              ? 'border-cyan-500/50 bg-cyan-500/15 font-semibold text-cyan-400 shadow-xs'
              : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--text-primary)]'
          } ${disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}`}
        >
          <span className="text-xs">
            {t('manage.streamModeMain', { defaultValue: '主码流 (高清分析·推荐)' })}
          </span>
        </button>
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange('sub')}
          className={`flex flex-col items-center justify-center rounded-xl border p-2 text-center transition-all ${
            value === 'sub'
              ? 'border-cyan-500/50 bg-cyan-500/15 font-semibold text-cyan-400 shadow-xs'
              : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--text-primary)]'
          } ${disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}`}
        >
          <span className="text-xs">
            {t('manage.streamModeSub', { defaultValue: '子码流 (低能耗)' })}
          </span>
        </button>
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange('auto')}
          className={`flex flex-col items-center justify-center rounded-xl border p-2 text-center transition-all ${
            value === 'auto'
              ? 'border-cyan-500/50 bg-cyan-500/15 font-semibold text-cyan-400 shadow-xs'
              : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--text-primary)]'
          } ${disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}`}
        >
          <span className="text-xs">
            {t('manage.streamModeAuto', { defaultValue: '自动适配 (自适应)' })}
          </span>
        </button>
      </div>
      <p className="mt-1.5 text-[11px] text-[var(--text-muted)]">
        {value === 'main'
          ? t('manage.streamModeMainDesc', {
              defaultValue: '全高清原图硬件下采样，小目标与远距离识别最清晰，快照零延迟',
            })
          : value === 'sub'
            ? t('manage.streamModeSubDesc', {
                defaultValue: '低码率子流推理，节约 VPU 算力，适合超多路密集布防',
              })
            : t('manage.streamModeAutoDesc', {
                defaultValue: '自动探活子码流，若无子流或不可用则自适应降级主码流',
              })}
      </p>
    </div>
  )
}
