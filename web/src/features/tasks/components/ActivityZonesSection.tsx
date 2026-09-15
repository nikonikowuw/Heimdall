import React from 'react'
import { Eye, EyeOff, Hexagon, Slash, ShieldAlert, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ExtendedRule, ToolMode } from './rulesStudioTypes'

export interface ActivityZonesSectionProps {
  rules: ExtendedRule[]
  selectedRuleId: string | null
  onSelectRule: (ruleId: string) => void
  onToggleRuleVisible: (ruleId: string) => void
  onDeleteRule: (ruleId: string) => void
  /** 切换画板工具并直接进入绘制（工具岛与侧栏共享同一状态） */
  onStartDrawing: (tool: ToolMode) => void
  activeTool: ToolMode
  activeAlgorithmNames: string[]
}

const DRAW_ACTIONS: Array<{ tool: ToolMode; icon: React.ReactNode; labelKey: string }> = [
  { tool: 'roi', icon: <Hexagon className="h-3.5 w-3.5" />, labelKey: 'tools.roi' },
  { tool: 'line', icon: <Slash className="h-3.5 w-3.5" />, labelKey: 'tools.line' },
  { tool: 'mask', icon: <ShieldAlert className="h-3.5 w-3.5" />, labelKey: 'tools.mask' },
]

export function ActivityZonesSection({
  rules,
  selectedRuleId,
  onSelectRule,
  onToggleRuleVisible,
  onDeleteRule,
  onStartDrawing,
  activeTool,
  activeAlgorithmNames,
}: ActivityZonesSectionProps): React.ReactElement {
  const { t } = useTranslation('task')

  return (
    <div className="space-y-2.5">
      {/* 栏目标题与绘制入口 */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-xs font-bold tracking-wide text-[var(--text-primary)]">
            {t('studio.zonesSectionTitle', { defaultValue: '空间活动防区' })}
          </span>
          <span className="rounded-full border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-secondary)]">
            {rules.length}
          </span>
        </div>

        {/* 一键下笔：直接把画板工具切到对应类型 */}
        <div
          className="flex shrink-0 items-center gap-2 rounded-[7px] border border-[var(--border)] bg-[var(--bg-surface)] p-1"
          role="group"
          aria-label={t('studio.zoneTypeTools', { defaultValue: '防区类型' })}
        >
          {DRAW_ACTIONS.map((action) => {
            const label = t(action.labelKey, { defaultValue: action.tool })
            const isActive = activeTool === action.tool
            return (
              <button
                key={action.tool}
                type="button"
                onClick={() => onStartDrawing(action.tool)}
                aria-pressed={isActive}
                aria-label={label}
                title={`${label} - ${t('studio.startDrawingHint', { defaultValue: '在画面上单击开始绘制' })}`}
                className={`flex h-9 min-w-[4.5rem] items-center justify-center gap-1.5 rounded-[5px] px-2 text-[10px] font-semibold whitespace-nowrap transition-colors sm:text-[11px] ${
                  isActive
                    ? 'bg-[var(--accent)] text-white shadow-sm'
                    : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]'
                }`}
              >
                {action.icon}
                <span>{label}</span>
              </button>
            )
          })}
        </div>
      </div>

      {/* 算力作用域提示：几何规则与算法的真实关系 */}
      <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">
        {activeAlgorithmNames.length === 0
          ? t('studio.zonesNoAlgoHint', {
              defaultValue: '尚未启用算法；先在上方启用检测引擎，防区才会参与判定。',
            })
          : t('studio.zonesScopeHint', {
              count: activeAlgorithmNames.length,
              defaultValue: '这 {{count}} 个算法共享同一套几何防区，命中任一防区即触发告警。',
            })}
      </p>

      {/* 防区列表 / 空状态 */}
      {rules.length === 0 ? (
        <div className="rounded-[8px] border border-dashed border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-3 text-[11px] text-[var(--text-muted)]">
          <div className="flex items-start gap-2">
            <Hexagon className="mt-0.5 h-4 w-4 shrink-0 text-cyan-400 opacity-40" />
            <span>
              {t('studio.noZonesHint', {
                defaultValue: '尚未划定局部防区，已启用的算法将在全画幅范围内生效并判定告警。',
              })}
            </span>
          </div>
          <button
            type="button"
            onClick={() => onStartDrawing('roi')}
            className="mt-2 text-[11px] font-semibold text-[var(--accent)] hover:underline"
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
                className={`group flex items-center justify-between rounded-[7px] border p-2.5 text-xs transition-all ${
                  isSelected
                    ? 'border-[var(--accent)] bg-[var(--accent-soft)]/40 shadow-xs'
                    : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
                }`}
              >
                <button
                  type="button"
                  onClick={() => onSelectRule(rule.id)}
                  aria-pressed={isSelected}
                  aria-label={`${rule.name} - ${
                    isRoi
                      ? t('tools.roi', { defaultValue: '多边形防区' })
                      : isLine
                        ? t('tools.line', { defaultValue: '越界绊线' })
                        : t('tools.mask', { defaultValue: '屏蔽遮罩' })
                  }`}
                  className="flex min-w-0 flex-1 items-center gap-2 rounded-lg text-left focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:outline-none"
                >
                  <span
                    className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-[5px] font-mono text-[10px] font-bold ${
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
                              ? t('inspector.dirBoth', { defaultValue: '双向' })
                              : rule.lineDirection === 'a_to_b'
                                ? t('inspector.dirAtoB', { defaultValue: 'A→B' })
                                : t('inspector.dirBtoA', { defaultValue: 'B→A' })
                          }`
                        : t('studio.polygonVertices', {
                            count: rule.points.length,
                            defaultValue: `${rule.points.length} 顶点`,
                          })}
                    </span>
                  </div>
                </button>

                <div className="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      onToggleRuleVisible(rule.id)
                    }}
                    aria-label={
                      rule.visible
                        ? t('layers.hideRule', { defaultValue: '隐藏该规则' })
                        : t('layers.showRule', { defaultValue: '显示该规则' })
                    }
                    className="flex h-6 w-6 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
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
                    aria-label={t('layers.deleteRule', { defaultValue: '删除该规则' })}
                    className="flex h-6 w-6 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-rose-500/15 hover:text-rose-500"
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
