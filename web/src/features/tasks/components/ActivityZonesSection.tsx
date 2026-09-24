import React from 'react'
import { Crop, Eye, EyeOff, Hexagon, Slash, ShieldAlert, Trash2 } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { DetectionRule } from '@/types'
import {
  calculatePolygonAreaPercent,
  ExtendedRule,
  getDirectionLabel,
  ToolMode,
} from './rulesStudioTypes'

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
  { tool: 'precrop', icon: <Crop className="h-3.5 w-3.5" />, labelKey: 'tools.precrop' },
]

const ROLE_BADGES: Record<
  DetectionRule['role'],
  { label: string; badgeClass: string; labelKey: string }
> = {
  roi: {
    label: 'ROI',
    labelKey: 'tools.roi',
    badgeClass: 'border border-blue-500/25 bg-blue-500/15 text-[var(--accent)]',
  },
  line: {
    label: 'LINE',
    labelKey: 'tools.line',
    badgeClass: 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500',
  },
  mask: {
    label: 'MASK',
    labelKey: 'tools.mask',
    badgeClass:
      'border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]',
  },
  precrop: {
    label: 'CROP',
    labelKey: 'tools.precrop',
    badgeClass: 'border border-lime-400/30 bg-lime-500/15 text-lime-400',
  },
}

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
  const reduceMotion = useReducedMotion()
  // 画板是否已处于下笔状态：进入工作台时空防区任务会自动切到 roi 工具，
  // 此处据此给出"已就绪"或"先选类型"的差异化引导，避免与上方类型按钮重复出 CTA。
  const isDrawingArmed = activeTool !== 'select'

  return (
    <div className="space-y-2.5">
      {/* 栏目标题与快速说明 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-xs font-bold tracking-wide text-[var(--text-primary)]">
            {t('studio.zonesSectionTitle', { defaultValue: '空间活动防区' })}
          </span>
          <span className="rounded-full border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-secondary)]">
            {rules.length}
          </span>
        </div>
        <span className="font-mono text-[10px] text-[var(--text-muted)]">
          {t('studio.pickTypeToDraw', { defaultValue: '选择类型并在画面上绘制' })}
        </span>
      </div>

      {/* 一键下笔：4 类绘制工具并列 (ROI / 绊线 / 遮罩 / 特写取景) */}
      <div
        className="grid grid-cols-4 gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-1 backdrop-blur-md"
        role="group"
        aria-label={t('studio.zoneTypeTools', { defaultValue: '防区类型' })}
      >
        {DRAW_ACTIONS.map((action) => {
          const label = t(action.labelKey, { defaultValue: action.tool })
          const isActive = activeTool === action.tool
          const isPrecrop = action.tool === 'precrop'
          const hasPrecropRule = isPrecrop && rules.some((r) => r.role === 'precrop')

          return (
            <motion.button
              key={action.tool}
              type="button"
              onClick={() => onStartDrawing(action.tool)}
              whileHover={reduceMotion ? undefined : { scale: 1.02 }}
              whileTap={reduceMotion ? undefined : { scale: 0.96 }}
              aria-pressed={isActive}
              aria-label={label}
              title={`${label} - ${
                isPrecrop && hasPrecropRule
                  ? t('studio.precropReplaceHint', {
                      defaultValue: '重新框定取景（将自动替换现有取景框）',
                    })
                  : t('studio.startDrawingHint', { defaultValue: '在画面上单击开始绘制' })
              }`}
              className={`group/action relative flex h-8 min-w-0 items-center justify-center gap-1 rounded-lg px-1 text-[11px] font-semibold whitespace-nowrap transition-all ${
                isActive
                  ? 'border border-[var(--border)] bg-[var(--bg-surface-solid)] font-bold text-[var(--text-primary)] shadow-sm'
                  : 'text-[var(--text-muted)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-secondary)]'
              }`}
            >
              {action.icon}
              <span className="truncate">{label}</span>
              {hasPrecropRule && !isActive && (
                <span
                  className="absolute top-1.5 right-1.5 h-1.5 w-1.5 rounded-full bg-lime-400 shadow-[0_0_6px_#a3e635]"
                  title={t('studio.precropConfigured', { defaultValue: '已配置物理取景框' })}
                />
              )}
            </motion.button>
          )
        })}
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
        <div className="rounded-xl border border-dashed border-[var(--border)] bg-[var(--bg-surface)] px-4 py-3.5 text-[11px] text-[var(--text-muted)] backdrop-blur-md">
          <div className="flex items-start gap-2.5">
            <Hexagon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--accent)] opacity-50" />
            <div className="space-y-1">
              <span className="block">
                {t('studio.noZonesHint', {
                  defaultValue: '尚未划定局部防区，已启用的算法将在全画幅范围内生效并判定告警。',
                })}
              </span>
              <span className="block">
                {isDrawingArmed
                  ? t('studio.zonesArmedHint', {
                      defaultValue: '画板已就绪，直接在画面上单击落点即可开始绘制。',
                    })
                  : t('studio.zonesPickTypeHint', {
                      defaultValue: '请先在上方选择防区类型，再在画面上绘制。',
                    })}
              </span>
            </div>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          <AnimatePresence mode="popLayout" initial={false}>
            {rules.map((rule) => {
              const isSelected = rule.id === selectedRuleId
              const isLine = rule.role === 'line'
              const roleInfo = ROLE_BADGES[rule.role]
              const roleName = t(roleInfo.labelKey, { defaultValue: roleInfo.label })

              return (
                <motion.div
                  key={rule.id}
                  layout={!reduceMotion}
                  initial={{ opacity: 0, y: reduceMotion ? 0 : 8, scale: 0.98 }}
                  animate={{ opacity: 1, y: 0, scale: 1 }}
                  exit={{
                    opacity: 0,
                    scale: 0.95,
                    transition: { duration: reduceMotion ? 0 : motionTokens.duration.fast },
                  }}
                  transition={{
                    duration: reduceMotion ? 0 : motionTokens.duration.fast,
                    ease: motionTokens.easing.smooth,
                  }}
                  className={`group flex items-center justify-between rounded-xl border p-2.5 text-xs backdrop-blur-md transition-all duration-200 ${
                    isSelected
                      ? 'border-[var(--accent)]/40 bg-[var(--accent-soft)] shadow-sm ring-1 ring-[var(--accent)]/20'
                      : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface-solid)]'
                  }`}
                >
                  <button
                    type="button"
                    onClick={() => onSelectRule(rule.id)}
                    aria-pressed={isSelected}
                    aria-label={`${rule.name} - ${roleName}`}
                    className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg text-left focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:outline-none"
                  >
                    <span
                      className={`flex h-6 w-7 shrink-0 items-center justify-center rounded-lg font-mono text-[9px] font-bold ${roleInfo.badgeClass}`}
                    >
                      {roleInfo.label}
                    </span>

                    <div className="min-w-0">
                      <span className="block truncate font-semibold text-[var(--text-primary)]">
                        {rule.name}
                      </span>
                      <span className="font-mono text-[10px] text-[var(--text-muted)]">
                        {isLine
                          ? `${t('tools.line', { defaultValue: '绊线' })} · ${getDirectionLabel(rule.lineDirection || 'both', t)}`
                          : `${t('studio.polygonVertices', {
                              count: rule.points.length,
                              defaultValue: `${rule.points.length} 顶点`,
                            })}${
                              rule.points.length >= 3
                                ? ` · ${calculatePolygonAreaPercent(rule.points).toFixed(1)}%`
                                : ''
                            }`}
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
                      className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
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
                      className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--status-danger)]/10 hover:text-[var(--status-danger)]"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </motion.div>
              )
            })}
          </AnimatePresence>
        </div>
      )}
    </div>
  )
}
