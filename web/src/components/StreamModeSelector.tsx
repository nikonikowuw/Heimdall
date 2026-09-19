import React from 'react'
import { Check } from 'lucide-react'
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
  labelClassName = 'text-xs font-semibold text-[var(--text-secondary)]',
}: StreamModeSelectorProps): React.ReactElement {
  const { t } = useTranslation('camera')

  const options: { id: StreamMode; label: string; tag: string }[] = [
    {
      id: 'auto',
      label: t('manage.streamModeAuto', { defaultValue: '自动适配' }),
      tag: t('manage.streamModeAutoTag', { defaultValue: '推荐' }),
    },
    {
      id: 'main',
      label: t('manage.streamModeMain', { defaultValue: '主码流' }),
      tag: t('manage.streamModeMainTag', { defaultValue: '4K/1080P' }),
    },
    {
      id: 'sub',
      label: t('manage.streamModeSub', { defaultValue: '子码流' }),
      tag: t('manage.streamModeSubTag', { defaultValue: '低能耗' }),
    },
  ]

  return (
    <div>
      <div className="flex items-center justify-between">
        <label className={labelClassName}>
          {t('manage.streamMode', { defaultValue: 'AI 分析码流选择' })}
        </label>
      </div>

      <div className="mt-2 grid grid-cols-3 gap-2">
        {options.map((opt) => {
          const isSelected = value === opt.id
          return (
            <button
              key={opt.id}
              type="button"
              disabled={disabled}
              onClick={() => onChange(opt.id)}
              className={`relative flex flex-col items-center justify-center rounded-xl border p-2.5 text-center transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                isSelected
                  ? 'border-emerald-500/60 bg-emerald-500/10 font-semibold text-emerald-700 shadow-xs dark:border-emerald-500/50 dark:bg-emerald-500/15 dark:text-emerald-300'
                  : 'border-[var(--border)]/80 bg-white text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)] dark:bg-[var(--bg-surface-solid)]'
              } ${disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}`}
            >
              {isSelected && (
                <div className="absolute top-1.5 right-1.5 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-emerald-500 text-white">
                  <Check className="h-2.5 w-2.5 stroke-[3]" />
                </div>
              )}
              <span className="text-xs">{opt.label}</span>
              <span
                className={`py-0.2 mt-0.5 rounded-sm px-1 font-mono text-[10px] ${
                  isSelected
                    ? 'bg-emerald-500/20 text-emerald-800 dark:text-emerald-200'
                    : 'bg-[var(--bg-secondary)] text-[var(--text-muted)]'
                }`}
              >
                {opt.tag}
              </span>
            </button>
          )
        })}
      </div>

      <p className="mt-2 text-[11px] leading-relaxed text-[var(--text-muted)]">
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
