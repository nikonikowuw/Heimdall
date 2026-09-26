import React from 'react'
import { ArrowDownWideNarrow, ArrowUpDown, ListFilter, RotateCcw, Upload } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { RefreshButton } from '@/components/RefreshButton'
import { SearchInput } from '@/components/ui/SearchInput'
import { SelectField } from '@/components/ui/SelectField'
import {
  DEFAULT_ALGO_QUERY,
  type AlgoListQuery,
  type AlgoOriginFilter,
  type AlgoSortKey,
} from '../algoFilters'

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
        <SearchInput
          showKbdHint
          value={query.keyword}
          onChange={(val) => onQueryChange({ keyword: val })}
          onClear={() => onQueryChange({ keyword: '' })}
          placeholder={t('filter.searchPlaceholder')}
          aria-label={t('filter.searchPlaceholder')}
          clearAriaLabel={t('filter.clearSearch')}
          containerClassName="min-w-[220px] flex-1 sm:max-w-xs"
        />

        <SelectField<string>
          label={t('filter.typeLabel')}
          sizeVariant="compact"
          icon={ListFilter}
          value={query.algorithmType}
          emphasis={query.algorithmType !== DEFAULT_ALGO_QUERY.algorithmType}
          onChange={(algorithmType) => onQueryChange({ algorithmType })}
          options={[
            { value: DEFAULT_ALGO_QUERY.algorithmType, label: t('filter.typeAll') },
            ...typeOptions.map((type) => ({
              value: type,
              label: t(`filter.types.${type}`, { defaultValue: type }),
            })),
          ]}
        />

        <SelectField<string>
          label={t('filter.platformLabel')}
          sizeVariant="compact"
          icon={ArrowUpDown}
          value={query.platform}
          emphasis={query.platform !== DEFAULT_ALGO_QUERY.platform}
          onChange={(platform) => onQueryChange({ platform })}
          options={[
            { value: DEFAULT_ALGO_QUERY.platform, label: t('filter.platformAll') },
            ...platformOptions.map((platform) => ({ value: platform, label: platform })),
          ]}
        />

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

        <SelectField<AlgoSortKey>
          label={t('filter.sortLabel')}
          sizeVariant="compact"
          icon={ArrowDownWideNarrow}
          value={query.sortKey}
          emphasis={query.sortKey !== DEFAULT_ALGO_QUERY.sortKey}
          onChange={(sortKey) => onQueryChange({ sortKey })}
          options={SORT_OPTIONS.map((option) => ({
            value: option,
            label: t(`filter.sort.${option}`),
          }))}
        />

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
        <RefreshButton onClick={onRefresh} loading={isLoading} label={t('filter.refresh')} />

        <button
          type="button"
          onClick={onOpenUpload}
          className="page-action-btn page-action-btn--primary"
        >
          <Upload className="h-4 w-4" />
          <span>{t('actions.uploadPackage')}</span>
        </button>
      </div>
    </div>
  )
}
