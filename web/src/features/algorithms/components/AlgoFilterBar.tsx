import React from 'react'
import {
  ArrowDownWideNarrow,
  ArrowUpDown,
  ListFilter,
  RefreshCw,
  RotateCcw,
  Search,
  Upload,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { AlgoListQuery, AlgoOriginFilter, AlgoSortKey } from '../algoFilters'

export interface AlgoFilterBarProps {
  query: AlgoListQuery
  onQueryChange: (patch: Partial<AlgoListQuery>) => void
  /** 从当前清单派生，避免硬编码类型表在新增算法类型时漏项 */
  typeOptions: string[]
  platformOptions: string[]
  /** 是否存在任一非默认筛选条件 */
  hasActiveFilters: boolean
  onClearFilters: () => void
  onRefresh: () => void
  onOpenUpload: () => void
  isLoading?: boolean
}

const ORIGIN_OPTIONS: AlgoOriginFilter[] = ['all', 'builtin', 'custom']
const SORT_OPTIONS: AlgoSortKey[] = ['default', 'name', 'updated', 'versions']

const SELECT_CLASS =
  'rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-2 text-xs font-medium text-[var(--text-primary)] transition-colors hover:border-[var(--border-strong)] focus:border-[var(--accent)] focus:outline-hidden'

/**
 * 搜索与筛选工具栏。
 *
 * 类型、来源、排序、平台四档都是显式控件而不是隐式行为：运维排查
 * 「某算法为何不可用」时，需要能一眼看出当前视图正在按哪个维度收敛。
 */
export function AlgoFilterBar({
  query,
  onQueryChange,
  typeOptions,
  platformOptions,
  hasActiveFilters,
  onClearFilters,
  onRefresh,
  onOpenUpload,
  isLoading,
}: AlgoFilterBarProps): React.ReactElement {
  const { t } = useTranslation('algo')

  return (
    <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-[var(--border)] p-3">
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2.5">
        <div className="group/search relative min-w-[220px] flex-1 sm:max-w-xs">
          <Search className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)] transition-colors group-focus-within/search:text-[var(--accent)]" />
          <input
            type="text"
            data-search-input="true"
            value={query.keyword}
            onChange={(event) => onQueryChange({ keyword: event.target.value })}
            placeholder={t('filter.searchPlaceholder')}
            aria-label={t('filter.searchPlaceholder')}
            className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] py-2 pr-9 pl-9 text-xs text-[var(--text-primary)] backdrop-blur-md transition-all placeholder:text-[var(--text-muted)] hover:border-[var(--border-strong)] focus:border-[var(--accent)] focus:bg-[var(--bg-surface)] focus:ring-2 focus:ring-[var(--accent)]/15 focus:outline-hidden"
          />
          {query.keyword ? (
            <button
              type="button"
              onClick={() => onQueryChange({ keyword: '' })}
              aria-label={t('filter.clearSearch')}
              className="absolute top-1/2 right-2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-md text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          ) : (
            <kbd className="pointer-events-none absolute top-1/2 right-2.5 hidden -translate-y-1/2 rounded border border-[var(--border)]/70 bg-[var(--bg-surface)]/70 px-1 font-mono text-[10px] text-[var(--text-muted)] shadow-2xs sm:inline-block">
              /
            </kbd>
          )}
        </div>

        <div className="relative">
          <ListFilter className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]" />
          <select
            value={query.algorithmType}
            onChange={(event) => onQueryChange({ algorithmType: event.target.value })}
            aria-label={t('filter.typeLabel')}
            className={`${SELECT_CLASS} pl-8`}
          >
            <option value="all">{t('filter.typeAll')}</option>
            {typeOptions.map((type) => (
              <option key={type} value={type}>
                {t(`filter.types.${type}`, { defaultValue: type })}
              </option>
            ))}
          </select>
        </div>

        <div className="relative">
          <ArrowUpDown className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]" />
          <select
            value={query.platform}
            onChange={(event) => onQueryChange({ platform: event.target.value })}
            aria-label={t('filter.platformLabel')}
            className={`${SELECT_CLASS} pl-8 font-mono`}
          >
            <option value="all">{t('filter.platformAll')}</option>
            {platformOptions.map((platform) => (
              <option key={platform} value={platform}>
                {platform}
              </option>
            ))}
          </select>
        </div>

        <div
          role="group"
          aria-label={t('filter.originLabel')}
          className="flex rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-0.5"
        >
          {ORIGIN_OPTIONS.map((option) => (
            <button
              key={option}
              type="button"
              aria-pressed={query.origin === option}
              onClick={() => onQueryChange({ origin: option })}
              className={`rounded-lg px-2.5 py-1.5 text-xs font-medium transition-all ${
                query.origin === option
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              {option === 'all'
                ? t('filter.originAll')
                : option === 'builtin'
                  ? t('filter.originBuiltin')
                  : t('filter.originCustom')}
            </button>
          ))}
        </div>

        <div className="relative">
          <ArrowDownWideNarrow className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]" />
          <select
            value={query.sortKey}
            onChange={(event) => onQueryChange({ sortKey: event.target.value as AlgoSortKey })}
            aria-label={t('filter.sortLabel')}
            className={`${SELECT_CLASS} pl-8`}
          >
            {SORT_OPTIONS.map((option) => (
              <option key={option} value={option}>
                {t(`filter.sort.${option}`)}
              </option>
            ))}
          </select>
        </div>

        {hasActiveFilters && (
          <button
            type="button"
            onClick={onClearFilters}
            className="flex items-center gap-1 rounded-xl px-2 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:text-[var(--accent)]"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            <span>{t('filter.clearAll')}</span>
          </button>
        )}
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onRefresh}
          disabled={isLoading}
          aria-label={t('filter.refresh')}
          title={t('filter.refresh')}
          className="flex h-9 w-9 items-center justify-center rounded-xl border border-[var(--border)] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden disabled:opacity-50"
        >
          <RefreshCw className={`h-4 w-4 ${isLoading ? 'animate-spin' : ''}`} />
        </button>

        <button
          type="button"
          onClick={onOpenUpload}
          className="flex h-9 items-center gap-1.5 rounded-xl bg-[var(--accent)] px-3.5 text-xs font-semibold text-white shadow-md transition-all hover:opacity-90 active:scale-95"
        >
          <Upload className="h-4 w-4" />
          <span>{t('actions.uploadPackage')}</span>
        </button>
      </div>
    </div>
  )
}
