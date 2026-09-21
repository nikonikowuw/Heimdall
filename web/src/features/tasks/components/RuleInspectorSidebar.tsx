import React from 'react'
import {
  Copy,
  Crop,
  Eye,
  EyeOff,
  Layers,
  ShieldAlert,
  Slash,
  SlidersHorizontal,
  Trash2,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { AlgoManifest, DetectionLineDirection } from '@/types'
import {
  ExtendedRule,
  getDirectionLabel,
  getLocalizedClassName,
  getRuleTheme,
  ROI_COLOR_PALETTES,
} from './rulesStudioTypes'

export interface RuleInspectorSidebarProps {
  rules: ExtendedRule[]
  selectedRuleId: string | null
  selectedRule?: ExtendedRule
  activeAlgo: AlgoManifest
  globalTargetClasses: string[]
  onSelectRule: (ruleId: string) => void
  onToggleRuleVisibility: (ruleId: string) => void
  onDeleteRule: (ruleId: string) => void
  onCloneRule: (ruleId: string) => void
  onUpdateRule: (ruleId: string, partial: Partial<ExtendedRule>) => void
}

export function RuleInspectorSidebar({
  rules,
  selectedRuleId,
  selectedRule,
  activeAlgo,
  globalTargetClasses,
  onSelectRule,
  onToggleRuleVisibility,
  onDeleteRule,
  onCloneRule,
  onUpdateRule,
}: RuleInspectorSidebarProps): React.ReactElement {
  const { t, i18n } = useTranslation('task')

  const hasClasses = Boolean(activeAlgo.classes && activeAlgo.classes.length > 0)
  const targetClasses = selectedRule?.targetClasses || globalTargetClasses
  const isAllTargetsSelected = hasClasses && targetClasses.length === activeAlgo.classes.length

  const handleToggleAllTargets = () => {
    if (!selectedRule || !hasClasses) return
    const nextTargets = isAllTargetsSelected ? [] : [...activeAlgo.classes]
    onUpdateRule(selectedRule.id, { targetClasses: nextTargets })
  }

  const handleToggleSingleTarget = (cls: string) => {
    if (!selectedRule || !hasClasses) return
    const current = selectedRule.targetClasses || globalTargetClasses
    const isChecked = current.includes(cls)
    const next = isChecked ? current.filter((c) => c !== cls) : [...current, cls]
    onUpdateRule(selectedRule.id, { targetClasses: next })
  }

  return (
    <aside className="flex w-72 max-w-72 min-w-72 flex-col overflow-hidden border-l border-[var(--border)] bg-[var(--bg-surface-solid)] text-sm shadow-[var(--shadow-lg)] backdrop-blur-2xl">
      <div className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--bg-surface)] p-3.5 backdrop-blur-xl">
        <span className="flex items-center gap-1.5 font-bold tracking-tight text-[var(--text-primary)]">
          <Layers className="h-4 w-4 text-[var(--accent)]" />
          <span>{t('layers.title')}</span>
        </span>
        <span className="rounded-full border border-[var(--accent)]/20 bg-[var(--accent-soft)] px-2.5 py-0.5 font-mono text-xs font-bold text-[var(--accent)]">
          {rules.length} {t('footer.items')}
        </span>
      </div>

      {/* 图层列表 */}
      <div className="max-h-56 space-y-1.5 overflow-y-auto border-b border-[var(--border)] p-2.5">
        {rules.length === 0 ? (
          <div className="py-6 text-center text-[var(--text-muted)]">
            <p>{t('layers.empty')}</p>
            <p className="text-xs opacity-75">{t('layers.emptyTip')}</p>
          </div>
        ) : (
          rules.map((rule, ruleIdx) => {
            const isSelected = rule.id === selectedRuleId
            const theme = getRuleTheme(rule, ruleIdx)
            return (
              <div
                key={rule.id}
                onClick={() => onSelectRule(rule.id)}
                className={`flex cursor-pointer items-center justify-between rounded-xl border p-2.5 backdrop-blur-md transition-all select-none ${
                  isSelected
                    ? 'border-[var(--accent)]/40 bg-[var(--accent-soft)] font-semibold text-[var(--text-primary)] shadow-2xs ring-1 ring-[var(--accent)]/20'
                    : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface-solid)]'
                }`}
              >
                <div className="flex items-center gap-2.5">
                  {rule.role === 'roi' && (
                    <span
                      className="h-2.5 w-2.5 shrink-0 rounded-full shadow-xs"
                      style={{ backgroundColor: theme.stroke }}
                    />
                  )}
                  {rule.role === 'line' && <Slash className="h-4 w-4 text-emerald-500" />}
                  {rule.role === 'mask' && <ShieldAlert className="h-4 w-4 text-slate-400" />}
                  {rule.role === 'precrop' && <Crop className="h-4 w-4 text-lime-400" />}
                  <span className="max-w-[140px] truncate text-xs font-semibold">{rule.name}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      onToggleRuleVisibility(rule.id)
                    }}
                    className="flex h-6 w-6 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                  >
                    {rule.visible ? (
                      <Eye className="h-3.5 w-3.5" />
                    ) : (
                      <EyeOff className="h-3.5 w-3.5 opacity-40" />
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      onDeleteRule(rule.id)
                    }}
                    className="flex h-6 w-6 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--destructive)]/10 hover:text-[var(--destructive)]"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            )
          })
        )}
      </div>

      {/* 规则属性检查器 */}
      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 font-bold tracking-tight text-[var(--text-primary)]">
            <SlidersHorizontal className="h-4 w-4 text-[var(--accent)]" />
            <span>{t('inspector.title')}</span>
          </span>
          {selectedRule && (
            <span
              className={`rounded-lg border px-2 py-0.5 font-mono text-[10px] font-bold uppercase ${
                selectedRule.role === 'precrop'
                  ? 'border-lime-500/30 bg-lime-500/10 text-lime-400'
                  : 'border-[var(--accent)]/30 bg-[var(--accent-soft)] text-[var(--accent)]'
              }`}
            >
              {selectedRule.role === 'precrop'
                ? t('tools.precrop', { defaultValue: '特写取景' })
                : selectedRule.role}
            </span>
          )}
        </div>

        {!selectedRule ? (
          <div className="py-8 text-center text-xs text-[var(--text-muted)]">
            {t('inspector.emptyTip')}
          </div>
        ) : (
          <div className="space-y-3.5">
            <div>
              <label className="mb-1.5 block font-mono text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
                {t('inspector.ruleName')}
              </label>
              <input
                type="text"
                value={selectedRule.name}
                onChange={(e) => onUpdateRule(selectedRule.id, { name: e.target.value })}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs text-[var(--text-primary)] transition-all outline-none focus:border-[var(--accent)] focus:bg-[var(--bg-surface-solid)] focus:ring-2 focus:ring-[var(--accent)]/20"
              />
            </div>

            {/* 特写取景专属说明卡片 */}
            {selectedRule.role === 'precrop' && (
              <div className="rounded-xl border border-lime-500/30 bg-lime-500/10 p-3 text-xs text-lime-400 backdrop-blur-md">
                <p className="font-semibold">
                  {t('inspector.precropTitle', { defaultValue: '局部特写取景' })}
                </p>
                <p className="mt-1 text-[11px] leading-relaxed opacity-90">
                  {t('inspector.precropScopeHint', {
                    defaultValue:
                      '特写取景框定摄像机送入算法模型的物理画幅，全局唯一生效，不参与报警判定。',
                  })}
                </p>
              </div>
            )}

            {/* 规则生效目标筛选（仅当算法具有多类别且非特写取景时展示） */}
            {hasClasses && selectedRule.role !== 'precrop' && (
              <div>
                <div className="mb-1.5 flex items-center justify-between">
                  <label className="font-mono text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
                    {t('inspector.ruleTargets', { defaultValue: '本规则生效目标' })}
                  </label>
                  <button
                    type="button"
                    onClick={handleToggleAllTargets}
                    className="text-[10px] text-[var(--accent)] hover:underline"
                  >
                    {isAllTargetsSelected
                      ? t('studio.clearAll', { defaultValue: '清空' })
                      : t('studio.selectAll', { defaultValue: '全选' })}
                  </button>
                </div>
                <div className="flex max-h-32 flex-wrap gap-1 overflow-y-auto rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-2 backdrop-blur-md">
                  {activeAlgo.classes.map((cls) => {
                    const isChecked = targetClasses.includes(cls)
                    return (
                      <button
                        key={cls}
                        type="button"
                        onClick={() => handleToggleSingleTarget(cls)}
                        className={`rounded-lg px-2 py-1 text-xs font-medium transition-all ${
                          isChecked
                            ? 'border border-[var(--accent)] bg-[var(--accent)] font-semibold text-white shadow-xs'
                            : 'border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {getLocalizedClassName(cls, i18n.language)}
                      </button>
                    )
                  })}
                </div>
              </div>
            )}

            {/* 防区色彩主题 (ROI 多配置区分) */}
            {selectedRule.role === 'roi' && (
              <div>
                <label className="mb-1.5 block font-mono text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
                  {t('inspector.colorTheme')}
                </label>
                <div className="flex items-center gap-2">
                  {ROI_COLOR_PALETTES.map((palette) => {
                    const isCurrent =
                      (selectedRule.color || ROI_COLOR_PALETTES[0].stroke) === palette.stroke
                    return (
                      <button
                        key={palette.stroke}
                        type="button"
                        onClick={() => onUpdateRule(selectedRule.id, { color: palette.stroke })}
                        className={`h-5 w-5 rounded-full transition-transform hover:scale-115 ${
                          isCurrent
                            ? 'scale-115 ring-2 ring-[var(--accent)] ring-offset-2 ring-offset-[var(--bg-surface)]'
                            : 'opacity-65 hover:opacity-100'
                        }`}
                        style={{ backgroundColor: palette.stroke }}
                        title={palette.stroke}
                      />
                    )
                  })}
                </div>
              </div>
            )}

            {/* 绊线方向 */}
            {selectedRule.role === 'line' && (
              <div>
                <label className="mb-1.5 block font-mono text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
                  {t('inspector.lineDirection')}
                </label>
                <div className="grid grid-cols-3 gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-1 text-center text-xs backdrop-blur-md">
                  {(['both', 'a_to_b', 'b_to_a'] as DetectionLineDirection[]).map((dir) => {
                    const isActive = selectedRule.lineDirection === dir
                    return (
                      <button
                        key={dir}
                        type="button"
                        onClick={() => onUpdateRule(selectedRule.id, { lineDirection: dir })}
                        className={`rounded-lg py-1.5 font-medium transition-all ${
                          isActive
                            ? 'border border-[var(--border)] bg-[var(--bg-surface-solid)] font-bold text-[var(--text-primary)] shadow-sm'
                            : 'text-[var(--text-muted)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-secondary)]'
                        }`}
                      >
                        {getDirectionLabel(dir, t)}
                      </button>
                    )
                  })}
                </div>
              </div>
            )}

            {/* 克隆与删除 */}
            <div className="grid grid-cols-2 gap-2 pt-2">
              <button
                type="button"
                disabled={selectedRule.role === 'precrop'}
                onClick={() => onCloneRule(selectedRule.id)}
                className={`flex items-center justify-center gap-1.5 rounded-xl border py-2 text-xs font-semibold transition-all ${
                  selectedRule.role === 'precrop'
                    ? 'cursor-not-allowed border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] opacity-50'
                    : 'border-[var(--accent)]/30 bg-[var(--accent-soft)] text-[var(--accent)] hover:bg-[var(--accent-soft)] active:scale-98'
                }`}
                title={
                  selectedRule.role === 'precrop'
                    ? t('inspector.precropNoClone', {
                        defaultValue: '特写取景区域全局唯一，不支持克隆',
                      })
                    : t('inspector.clone')
                }
              >
                <Copy className="h-3.5 w-3.5" />
                <span>{t('inspector.clone')}</span>
              </button>
              <button
                type="button"
                onClick={() => onDeleteRule(selectedRule.id)}
                className="flex items-center justify-center gap-1.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] py-2 text-xs font-semibold text-[var(--status-danger)] transition-all hover:bg-[var(--status-danger-soft)] active:scale-98"
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>{t('inspector.delete')}</span>
              </button>
            </div>
          </div>
        )}
      </div>
    </aside>
  )
}
