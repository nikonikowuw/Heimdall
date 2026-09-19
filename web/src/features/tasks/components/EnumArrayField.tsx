import React from 'react'
import { Check, Search, X } from 'lucide-react'
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
 * 采用工业级芯片标签（Chip Tiles）排版与带图标的清空/搜索交互。
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

  function handleInvert(): void {
    const next = options.filter((item) => !value.includes(item))
    onChange(next)
  }

  return (
    <div className="space-y-2.5 rounded-xl border border-black/[0.06] bg-white/40 p-3.5 shadow-2xs backdrop-blur-md dark:border-white/[0.08] dark:bg-white/[0.02]">
      <div className="flex items-center justify-between">
        <span className="flex items-center gap-1.5 font-medium text-[var(--text-secondary)]">
          <span className="text-xs font-semibold text-[var(--text-primary)]">{title}</span>
          <span className="rounded-full border border-blue-500/20 bg-blue-500/10 px-2 py-0.5 font-mono text-[10px] font-bold text-[var(--accent)]">
            {value.length}/{options.length}
          </span>
        </span>
        <div className="flex items-center gap-1 font-mono text-[10px]">
          <button
            type="button"
            onClick={() => onChange([...options])}
            className="rounded-md px-1.5 py-0.5 font-semibold text-[var(--accent)] transition-colors hover:bg-[var(--accent)]/10"
          >
            {t('studio.selectAll', { defaultValue: '全选' })}
          </button>
          <span className="text-black/15 dark:text-white/15">·</span>
          <button
            type="button"
            onClick={handleInvert}
            className="rounded-md px-1.5 py-0.5 text-[var(--text-muted)] transition-colors hover:bg-black/5 hover:text-[var(--text-primary)] dark:hover:bg-white/5"
            title={t('studio.invertSelection', { defaultValue: '反选已选项目' })}
          >
            {t('studio.invert', { defaultValue: '反选' })}
          </button>
          <span className="text-black/15 dark:text-white/15">·</span>
          <button
            type="button"
            onClick={() => onChange([])}
            className="rounded-md px-1.5 py-0.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--destructive)]/10 hover:text-[var(--destructive)]"
          >
            {t('studio.clearAll', { defaultValue: '清空' })}
          </button>
        </div>
      </div>

      {options.length > SEARCH_THRESHOLD && (
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]" />
          <input
            type="text"
            value={search}
            onChange={(e) => onSearchChange(e.target.value)}
            placeholder={t('searchClassPlaceholder', {
              defaultValue: '过滤候选项 (如: 人, 车, dog)...',
            })}
            aria-label={title}
            className="w-full rounded-lg border border-black/10 bg-white/60 py-1.5 pr-7 pl-8 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] backdrop-blur-sm transition-all outline-none focus:border-[var(--accent)] focus:bg-white focus:ring-2 focus:ring-[var(--accent)]/20 dark:border-white/10 dark:bg-white/[0.04] dark:focus:bg-black/60"
          />
          {search && (
            <button
              type="button"
              onClick={() => onSearchChange('')}
              className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-0.5 text-[var(--text-muted)] hover:bg-black/5 hover:text-[var(--text-primary)] dark:hover:bg-white/10"
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </div>
      )}

      <div className="grid max-h-52 grid-cols-2 gap-1.5 overflow-y-auto pr-0.5">
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
              className={`group/opt flex items-center justify-between rounded-lg border px-2.5 py-1.5 text-left text-[11px] transition-all select-none ${
                isChecked
                  ? 'border-blue-500/40 bg-blue-500/10 font-semibold text-[var(--text-primary)] shadow-2xs ring-1 ring-blue-500/20 dark:bg-blue-500/15'
                  : 'border-black/5 bg-white/50 text-[var(--text-secondary)] hover:border-black/15 hover:bg-white/80 dark:border-white/[0.06] dark:bg-white/[0.02] dark:hover:border-white/15 dark:hover:bg-white/[0.05]'
              }`}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <span
                  className={`h-1.5 w-1.5 shrink-0 rounded-full transition-all ${
                    isChecked
                      ? 'bg-[var(--accent)] shadow-[0_0_6px_var(--accent)]'
                      : 'bg-transparent'
                  }`}
                />
                <span className="truncate">{getLocalizedClassName(option, language)}</span>
              </span>
              <div
                className={`flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border transition-colors ${
                  isChecked
                    ? 'border-[var(--accent)] bg-[var(--accent)] text-white shadow-xs'
                    : 'border-black/20 bg-transparent opacity-40 group-hover/opt:opacity-80 dark:border-white/20'
                }`}
              >
                {isChecked && <Check className="h-2.5 w-2.5 stroke-[3]" />}
              </div>
            </button>
          )
        })}
      </div>

      {description && (
        <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">{description}</p>
      )}
    </div>
  )
}
