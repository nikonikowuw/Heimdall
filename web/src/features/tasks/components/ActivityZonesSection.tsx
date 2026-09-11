import React from 'react'
import { Eye, EyeOff, Hexagon, PenTool, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { ExtendedRule } from './rulesStudioTypes'

export interface ActivityZonesSectionProps {
  rules: ExtendedRule[]
  selectedRuleId: string | null
  onSelectRule: (ruleId: string) => void
  onToggleRuleVisible: (ruleId: string) => void
  onDeleteRule: (ruleId: string) => void
  onEnterCalibration: () => void
  isCalibrating: boolean
}

export function ActivityZonesSection({
  rules,
  selectedRuleId,
  onSelectRule,
  onToggleRuleVisible,
  onDeleteRule,
  onEnterCalibration,
  isCalibrating,
}: ActivityZonesSectionProps): React.ReactElement {
  const { t } = useTranslation('task')

  return (
    <div className="space-y-2.5">
      {/* 栏目标题与快速行动 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-xs font-bold tracking-wide text-[var(--text-primary)]">
            {t('studio.zonesSectionTitle', { defaultValue: '空间活动防区 (Activity Zones)' })}
          </span>
          <span className="rounded-full border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-secondary)]">
            {rules.length} {t('studio.activeZonesCount', { defaultValue: '项防区' })}
          </span>
        </div>

        {/* 绘制/标定防区行动按钮 */}
        <button
          type="button"
          onClick={onEnterCalibration}
          className={`flex items-center gap-1.5 rounded-xl px-3 py-1.5 text-xs font-semibold shadow-xs transition-all ${
            isCalibrating
              ? 'border border-[var(--accent)] bg-[var(--accent)] text-white ring-2 ring-[var(--accent)]/30'
              : 'border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-primary)] hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]'
          }`}
        >
          <PenTool className="h-3.5 w-3.5" />
          <span>
            {isCalibrating
              ? t('studio.calibratingActive', { defaultValue: '正在标定中 (点击画面绘制)' })
              : t('studio.enterCalibration', { defaultValue: '绘制 / 编辑活动防区 ↗' })}
          </span>
        </button>
      </div>

      {/* 防区列表 / 空状态卡片 */}
      {rules.length === 0 ? (
        <div className="flex items-center justify-between rounded-2xl border border-dashed border-[var(--border)] bg-[var(--bg-surface)] px-4 py-3 text-xs text-[var(--text-muted)]">
          <div className="flex items-center gap-2">
            <Hexagon className="h-4 w-4 text-cyan-400 opacity-40" />
            <span>
              {t('studio.noZonesHint', {
                defaultValue:
                  '当前画面尚未划定局部防区，已启用的算法将在全画幅范围内生效并判定告警。',
              })}
            </span>
          </div>
          <button
            type="button"
            onClick={onEnterCalibration}
            className="text-[11px] font-semibold text-[var(--accent)] hover:underline"
          >
            {t('studio.startDrawingFirst', { defaultValue: '立即绘制首个防区' })}
          </button>
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {rules.map((rule) => {
            const isSelected = rule.id === selectedRuleId
            const isRoi = rule.role === 'roi'
            const isLine = rule.role === 'line'

            return (
              <div
                key={rule.id}
                onClick={() => onSelectRule(rule.id)}
                className={`group flex cursor-pointer items-center justify-between rounded-xl border p-2.5 text-xs transition-all ${
                  isSelected
                    ? 'border-[var(--accent)] bg-[var(--accent-soft)]/40 shadow-xs'
                    : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
                }`}
              >
                <div className="flex min-w-0 items-center gap-2">
                  {/* 类型图标徽标 */}
                  <span
                    className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-lg font-mono text-[10px] font-bold ${
                      isRoi
                        ? 'border border-cyan-500/30 bg-cyan-500/15 text-cyan-400'
                        : isLine
                          ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-400'
                          : 'border border-rose-500/30 bg-rose-500/15 text-rose-400'
                    }`}
                  >
                    {isRoi ? 'ROI' : isLine ? 'LINE' : 'MASK'}
                  </span>

                  <div className="min-w-0">
                    <span className="block truncate font-medium text-[var(--text-primary)]">
                      {rule.name}
                    </span>
                    <span className="font-mono text-[10px] text-[var(--text-muted)]">
                      {isLine
                        ? `${t('tools.line', { defaultValue: '绊线' })} · ${
                            rule.lineDirection === 'both'
                              ? t('inspector.dirBoth', { defaultValue: '双向 ⇄' })
                              : rule.lineDirection === 'a_to_b'
                                ? t('inspector.dirAtoB', { defaultValue: 'A→B →' })
                                : t('inspector.dirBtoA', { defaultValue: 'B→A ←' })
                          }`
                        : t('studio.polygonVertices', {
                            count: rule.points.length,
                            defaultValue: `${rule.points.length} 顶点多边形`,
                          })}
                    </span>
                  </div>
                </div>

                {/* 规则操作按钮 */}
                <div className="flex shrink-0 items-center gap-1 opacity-80 group-hover:opacity-100">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      onToggleRuleVisible(rule.id)
                    }}
                    title={
                      rule.visible
                        ? t('layers.hideRule', { defaultValue: '隐藏该规则' })
                        : t('layers.showRule', { defaultValue: '显示该规则' })
                    }
                    className="flex h-6 w-6 items-center justify-center rounded text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                  >
                    {rule.visible ? (
                      <Eye className="h-3.5 w-3.5" />
                    ) : (
                      <EyeOff className="h-3.5 w-3.5 opacity-50" />
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      onDeleteRule(rule.id)
                    }}
                    title={t('layers.deleteRule', { defaultValue: '删除该规则' })}
                    className="flex h-6 w-6 items-center justify-center rounded text-[var(--text-muted)] transition-colors hover:bg-rose-500/15 hover:text-rose-500"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
