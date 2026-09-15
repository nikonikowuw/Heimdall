import React from 'react'
import { Check } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { getLocalizedClassName } from './rulesStudioTypes'

export interface EnumArrayFieldProps {
  title: string
  description?: string
  /** schema `items.enum` 声明的全部候选项 */
  options: string[]
  value: string[]
  search: string
  onSearchChange: (value: string) => void
  onChange: (value: string[]) => void
  language: string
}

/** 候选项超过此数量时提供过滤输入框，避免长列表难以定位 */
const SEARCH_THRESHOLD = 6

/**
 * schema 驱动的枚举多选控件。
 *
 * 候选项完全来自参数自身的 `items.enum`，不依赖任何按键名硬编码的类别表，
 * 因此对「检测目标类别」「可上报车牌颜色」这类同构参数一视同仁。
 */
export function EnumArrayField({
  title,
  description,
  options,
  value,
  search,
  onSearchChange,
  onChange,
  language,
}: EnumArrayFieldProps): React.ReactElement {
  const { t } = useTranslation('task')

  const query = search.toLowerCase().trim()
  const visible = query
    ? options.filter(
        (option) =>
          option.toLowerCase().includes(query) ||
          getLocalizedClassName(option, language).toLowerCase().includes(query),
      )
    : options

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="flex items-center gap-1.5 font-medium text-[var(--text-secondary)]">
          <span>{title}</span>
          <span className="font-mono text-[10px] text-[var(--text-muted)]">
            ({value.length}/{options.length})
          </span>
        </span>
        <div className="flex items-center gap-2 text-[11px]">
          <button
            type="button"
            onClick={() => onChange([...options])}
            className="font-medium text-[var(--accent)] hover:underline"
          >
            {t('studio.selectAll', { defaultValue: '全选' })}
          </button>
          <span className="text-[var(--text-muted)]">|</span>
          <button
            type="button"
            onClick={() => onChange([])}
            className="text-[var(--text-muted)] transition-colors hover:text-[var(--text-primary)]"
          >
            {t('studio.clearAll', { defaultValue: '清空' })}
          </button>
        </div>
      </div>

      {options.length > SEARCH_THRESHOLD && (
        <input
          type="text"
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder={t('searchClassPlaceholder', {
            defaultValue: '过滤候选项 (如: 人, 车, dog)...',
          })}
          aria-label={title}
          className="w-full rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1 text-xs text-[var(--text-primary)] transition-colors outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
        />
      )}

      <div className="grid max-h-48 grid-cols-2 gap-1.5 overflow-y-auto pr-0.5">
        {visible.map((option) => {
          const isChecked = value.includes(option)
          return (
            <button
              key={option}
              type="button"
              aria-pressed={isChecked}
              onClick={() =>
                onChange(isChecked ? value.filter((item) => item !== option) : [...value, option])
              }
              className={`flex items-center justify-between rounded-lg border px-2.5 py-1.5 text-left text-[11px] transition-all ${
                isChecked
                  ? 'border-[var(--border-strong)] bg-[var(--accent-soft)] font-semibold text-[var(--accent)]'
                  : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
              }`}
            >
              <span className="truncate">{getLocalizedClassName(option, language)}</span>
              <div
                className={`flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border transition-colors ${
                  isChecked
                    ? 'border-[var(--accent)] bg-[var(--accent)] text-white'
                    : 'border-[var(--border-strong)] bg-transparent'
                }`}
              >
                {isChecked && <Check className="h-2.5 w-2.5 stroke-[3]" />}
              </div>
            </button>
          )
        })}
      </div>

      {description && <p className="text-[10px] text-[var(--text-muted)]">{description}</p>}
    </div>
  )
}
