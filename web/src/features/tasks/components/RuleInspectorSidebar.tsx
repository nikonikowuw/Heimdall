import React from 'react'
import {
  Copy,
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

  const targetClasses = selectedRule?.targetClasses || globalTargetClasses
  const isAllTargetsSelected = targetClasses.length === activeAlgo.classes.length

  const handleToggleAllTargets = () => {
    if (!selectedRule) return
    const nextTargets = isAllTargetsSelected ? [] : [...activeAlgo.classes]
    onUpdateRule(selectedRule.id, { targetClasses: nextTargets })
  }

  const handleToggleSingleTarget = (cls: string) => {
    if (!selectedRule) return
    const current = selectedRule.targetClasses || globalTargetClasses
    const isChecked = current.includes(cls)
    const next = isChecked ? current.filter((c) => c !== cls) : [...current, cls]
    onUpdateRule(selectedRule.id, { targetClasses: next })
  }

  return (
    <aside className="frosted-glass flex w-72 max-w-72 min-w-72 flex-col overflow-hidden border-l border-[var(--border)] text-sm">
      <div className="flex items-center justify-between border-b border-[var(--border)] p-3.5">
        <span className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
          <Layers className="h-4 w-4 text-[var(--accent)]" />
          <span>{t('layers.title')}</span>
        </span>
        <span className="rounded-full border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-0.5 font-mono text-xs font-bold text-[var(--accent)]">
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
                className={`flex cursor-pointer items-center justify-between rounded-xl p-2.5 transition-all ${
                  isSelected
                    ? 'border border-[var(--accent)]/40 bg-[var(--accent-soft)] font-semibold text-[var(--text-primary)] shadow-2xs'
                    : 'border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)]'
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
                  <span className="max-w-[140px] truncate text-sm font-medium">{rule.name}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      onToggleRuleVisibility(rule.id)
                    }}
                    className="text-[var(--text-muted)] hover:text-[var(--text-primary)]"
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
                    className="text-[var(--text-muted)] hover:text-rose-500"
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
          <span className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
            <SlidersHorizontal className="h-4 w-4 text-[var(--accent)]" />
            <span>{t('inspector.title')}</span>
          </span>
          {selectedRule && (
            <span className="rounded-md border border-[var(--border)] bg-[var(--accent-soft)] px-2.5 py-0.5 font-mono text-xs font-bold text-[var(--accent)] uppercase">
              {selectedRule.role}
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
              <label className="mb-1.5 block font-mono text-xs tracking-wider text-[var(--text-muted)] uppercase">
                {t('inspector.ruleName')}
              </label>
              <input
                type="text"
                value={selectedRule.name}
                onChange={(e) => onUpdateRule(selectedRule.id, { name: e.target.value })}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-sm text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
              />
            </div>

            {/* 规则生效目标筛选 */}
            <div>
              <div className="mb-1.5 flex items-center justify-between">
                <label className="font-mono text-xs tracking-wider text-[var(--text-muted)] uppercase">
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
              <div className="flex max-h-32 flex-wrap gap-1 overflow-y-auto rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1.5">
                {activeAlgo.classes.map((cls) => {
                  const isChecked = targetClasses.includes(cls)
                  return (
                    <button
                      key={cls}
                      type="button"
                      onClick={() => handleToggleSingleTarget(cls)}
                      className={`rounded-md px-2 py-0.5 text-xs font-medium transition-all ${
                        isChecked
                          ? 'bg-[var(--accent)] text-white shadow-2xs'
                          : 'bg-[var(--bg-secondary)] text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                      }`}
                    >
                      {getLocalizedClassName(cls, i18n.language)}
                    </button>
                  )
                })}
              </div>
            </div>

            {/* 防区色彩主题 (ROI 多配置区分) */}
            {selectedRule.role === 'roi' && (
              <div>
                <label className="mb-1.5 block font-mono text-xs tracking-wider text-[var(--text-muted)] uppercase">
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
                <label className="mb-1.5 block font-mono text-xs tracking-wider text-[var(--text-muted)] uppercase">
                  {t('inspector.lineDirection')}
                </label>
                <div className="grid grid-cols-3 gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-center text-xs">
                  {(['both', 'a_to_b', 'b_to_a'] as DetectionLineDirection[]).map((dir) => (
                    <button
                      key={dir}
                      type="button"
                      onClick={() => onUpdateRule(selectedRule.id, { lineDirection: dir })}
                      className={`rounded-lg py-1.5 font-medium transition-all ${
                        selectedRule.lineDirection === dir
                          ? 'bg-[var(--accent)] font-semibold text-white shadow-xs'
                          : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                      }`}
                    >
                      {getDirectionLabel(dir, t)}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {/* 克隆与删除 */}
            <div className="grid grid-cols-2 gap-2 pt-2">
              <button
                type="button"
                onClick={() => onCloneRule(selectedRule.id)}
                className="flex items-center justify-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] py-2 text-xs font-semibold text-[var(--accent)] transition-colors hover:bg-[var(--accent-soft)]"
              >
                <Copy className="h-3.5 w-3.5" />
                <span>{t('inspector.clone')}</span>
              </button>
              <button
                type="button"
                onClick={() => onDeleteRule(selectedRule.id)}
                className="flex items-center justify-center gap-1.5 rounded-xl border border-rose-500/30 bg-rose-500/10 py-2 text-xs font-semibold text-rose-500 transition-colors hover:bg-rose-500/20"
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
