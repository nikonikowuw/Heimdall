import React from 'react'
import { RefreshCw, Search, Upload } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export interface AlgoFilterBarProps {
  searchQuery: string
  onSearchChange: (val: string) => void
  typeFilter: string
  onTypeChange: (val: string) => void
  originFilter: 'all' | 'builtin' | 'custom'
  onOriginChange: (val: 'all' | 'builtin' | 'custom') => void
  onRefresh: () => void
  onOpenUpload: () => void
  isLoading?: boolean
}

export const AlgoFilterBar: React.FC<AlgoFilterBarProps> = ({
  searchQuery,
  onSearchChange,
  typeFilter,
  onTypeChange,
  originFilter,
  onOriginChange,
  onRefresh,
  onOpenUpload,
  isLoading,
}) => {
  const { t } = useTranslation('algo')

  return (
    <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-[var(--border)] p-3">
      {/* 搜索与过滤 */}
      <div className="flex flex-1 flex-wrap items-center gap-2.5">
        <div className="relative min-w-[240px] flex-1 sm:max-w-xs">
          <Search className="absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-[var(--text-muted)]" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => onSearchChange(e.target.value)}
            placeholder={t('filter.searchPlaceholder')}
            className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] py-2 pr-3 pl-9 text-xs text-[var(--text-primary)] transition-all placeholder:text-[var(--text-muted)] focus:border-[var(--accent)] focus:outline-hidden"
          />
        </div>

        {/* 算法类型筛选下拉框 */}
        <select
          value={typeFilter}
          onChange={(e) => onTypeChange(e.target.value)}
          className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-2 text-xs font-medium text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
        >
          <option value="all">{t('filter.typeAll')}</option>
          <option value="object_detection">{t('filter.typeDetection')}</option>
          <option value="face_recognition">{t('filter.typeFace')}</option>
          <option value="license_plate_recognition">{t('filter.typePlate')}</option>
        </select>

        {/* 来源切换分段按键 */}
        <div className="flex rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-0.5">
          {(['all', 'builtin', 'custom'] as const).map((opt) => (
            <button
              key={opt}
              type="button"
              onClick={() => onOriginChange(opt)}
              className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-all ${
                originFilter === opt
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              {opt === 'all'
                ? t('filter.originAll')
                : opt === 'builtin'
                  ? t('filter.originBuiltin')
                  : t('filter.originCustom')}
            </button>
          ))}
        </div>
      </div>

      {/* 操作按钮区 */}
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onRefresh}
          disabled={isLoading}
          className="flex h-9 w-9 items-center justify-center rounded-xl border border-[var(--border)] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
          title={t('filter.refresh')}
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
