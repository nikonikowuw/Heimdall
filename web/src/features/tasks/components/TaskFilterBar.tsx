import React from 'react'
import { Layers, RotateCcw, ShieldCheck } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { SearchInput } from '@/components/ui/SearchInput'
import { SelectField } from '@/components/ui/SelectField'
import {
  ALGORITHM_FILTER_ALL,
  ARM_STATUS_FILTERS,
  hasActiveTaskFilters,
  type ArmStatusCounts,
  type ArmStatusFilter,
} from '../taskFilter'

export interface TaskFilterBarProps {
  query: string
  onQueryChange: (value: string) => void
  onQueryClear: () => void

  armStatus: ArmStatusFilter
  onArmStatusChange: (value: ArmStatusFilter) => void
  /** 各档位计数（**全量口径**，不随当前筛选变化），与页面 KPI 同源 */
  armStatusCounts: ArmStatusCounts

  algorithmId: string
  onAlgorithmChange: (value: string) => void
  /** 已派生的算法选项（不含「全部算法」档），由 TasksPage 从任务集派生 */
  algorithmOptions: Array<{ value: string; label: string }>

  onClearFilters: () => void
  /** 当前筛选命中数，与 PageHeader 的全量 KPI 并置展示 */
  matchedCount: number
}

/**
 * 药丸档位的选中 / 悬浮语义色。
 *
 * 抽成函数而非内联嵌套三元：「档位 × 选中态」在模板串里会叠成多层条件；
 * 「已布防」的悬浮态用成功色呼应页面 KPI 的绿点，其余档位用中性色。
 */
function armOptionClassName(option: ArmStatusFilter, isActive: boolean): string {
  if (isActive) return 'bg-[var(--accent)] text-white shadow-xs'
  if (option === 'armed') return 'hover:text-status-success text-[var(--text-secondary)]'
  return 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
}

/**
 * AI 任务的检索与筛选工作台。
 *
 * 三个维度即三个显式控件：身份检索（任务名 / 摄像头名 / 通道 ID）、算法枚举、
 * 布防状态。维度间为 AND，维度内为 OR。**KPI 与命中数的口径在此分离**：
 * 页面标题栏的「任务总数 / 已布防」始终是全量舰队口径，本栏内的药丸计数亦然；
 * 只有「筛选命中 N」随筛选变化，避免用户把缩水后的数字误读为舰队规模。
 *
 * 「是否存在生效筛选」由本栏从三个受控值自行推导（而非调用方传入）：该判定与
 * 过滤逻辑共用 [hasActiveTaskFilters]，避免同一个条件在页面与组件里各写一遍。
 *
 * 药丸点击语义与 [CamerasPage](../../cameras/CamerasPage.tsx) 一致：点击已选中档位
 * 保持选中（不回退「全部」），复位统一走「清除筛选」按钮。
 *
 * 算法下拉的选项标签直接使用传入的 `label`（源自 `AlgorithmItem.name` 或回落的
 * 原始 `algorithmId`）：算法名是服务端资产清单里的数据，不是可翻译的界面文案，
 * 不能喂给 `t()`。
 */
export function TaskFilterBar({
  query,
  onQueryChange,
  onQueryClear,
  armStatus,
  onArmStatusChange,
  armStatusCounts,
  algorithmId,
  onAlgorithmChange,
  algorithmOptions,
  onClearFilters,
  matchedCount,
}: TaskFilterBarProps): React.ReactElement {
  const { t } = useTranslation('task')
  // 清空按钮的无障碍名称复用 common 的共享键（三语已就绪），不为此新增 task 域键
  const { t: tc } = useTranslation('common')

  const armLabels: Record<ArmStatusFilter, string> = {
    all: t('filter.armAll'),
    armed: t('filter.armed'),
    disarmed: t('filter.disarmed'),
  }
  const hasActiveFilters = hasActiveTaskFilters({ query, armStatus, algorithmId })

  return (
    <div className="frosted-glass relative flex min-h-[52px] items-center justify-between gap-3 rounded-2xl border border-[var(--border)] p-2.5 shadow-xs">
      <div className="flex flex-1 [scrollbar-width:none] items-center gap-2 overflow-x-auto text-xs [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden">
        <SearchInput
          showKbdHint
          value={query}
          // 不能直接传 `onQueryChange`：SearchInput 的签名是 `(value, event)`，
          // 而本组件的回调契约只接收单参，直接透传会把 DOM 事件一并泄漏给父层。
          onChange={(value) => onQueryChange(value)}
          onClear={onQueryClear}
          placeholder={t('filter.searchPlaceholder')}
          aria-label={t('filter.searchPlaceholder')}
          clearAriaLabel={tc('actions.clearSearch')}
          containerClassName="min-w-[220px] flex-1 sm:max-w-xs"
        />

        <div className="hidden h-4 w-px bg-[var(--border)]/60 sm:block" />

        <SelectField<string>
          label={t('filter.algorithmLabel')}
          sizeVariant="compact"
          icon={Layers}
          value={algorithmId}
          emphasis={algorithmId !== ALGORITHM_FILTER_ALL}
          // 同上：`onChange` 是 `(value, event)`，需先丢弃 event 再上报单参
          onChange={(value) => onAlgorithmChange(value)}
          allOption={{ value: ALGORITHM_FILTER_ALL, label: t('filter.algorithmAll') }}
          options={algorithmOptions}
        />

        <div
          role="group"
          aria-label={t('filter.armStatusLabel')}
          className="flex shrink-0 items-center gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1"
        >
          {ARM_STATUS_FILTERS.map((option) => (
            <button
              key={option}
              type="button"
              aria-pressed={armStatus === option}
              onClick={() => onArmStatusChange(option)}
              className={`flex items-center gap-1 rounded-lg px-2.5 py-1 font-medium whitespace-nowrap transition-colors ${armOptionClassName(option, armStatus === option)}`}
            >
              {option === 'armed' && <ShieldCheck className="h-3.5 w-3.5" />}
              <span>{armLabels[option]}</span>
              <span className="font-mono tabular-nums opacity-80">({armStatusCounts[option]})</span>
            </button>
          ))}
        </div>

        {hasActiveFilters && (
          <button
            type="button"
            onClick={onClearFilters}
            className="flex shrink-0 items-center gap-1 rounded-xl px-2 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:text-[var(--accent)]"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            <span>{t('filter.clearAll')}</span>
          </button>
        )}
      </div>

      {/* 命中数：仅在筛选真正收敛了视图时展示，否则与药丸的「全部」计数重复 */}
      {matchedCount !== armStatusCounts.all && (
        <span className="text-status-success shrink-0 font-mono text-xs font-semibold whitespace-nowrap">
          {t('filter.matched', { count: matchedCount })}
        </span>
      )}
    </div>
  )
}
