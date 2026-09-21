import React from 'react'
import { Copy, Eye, EyeOff, Layers, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { DetectionLineDirection } from '@/types'
import type { ExtendedRule } from './rulesStudioTypes'
import { calculatePolygonAreaPercent, getDirectionLabel } from './rulesStudioTypes'

export interface RulePropertiesPanelProps {
  rule: ExtendedRule
  onUpdateRule: (ruleId: string, partial: Partial<ExtendedRule>) => void
  onCloneRule: (ruleId: string) => void
  onDeleteRule: (ruleId: string) => void
  /** 当前已启用算法名称；几何规则由通道内全部算法共享判定 */
  activeAlgorithmNames: string[]
}

/**
 * 选中防区时的上下文属性面板。
 * 仅暴露真实规则字段（绊线方向与几何点）以及当前编辑会话的可见性，
 * 并显式说明几何规则对通道内全部算法的共享作用域，消除"算力与防区脱节"的困惑。
 */
export function RulePropertiesPanel({
  rule,
  onUpdateRule,
  onCloneRule,
  onDeleteRule,
  activeAlgorithmNames,
}: RulePropertiesPanelProps): React.ReactElement {
  const { t } = useTranslation('task')

  const isPolygon = rule.role !== 'line'

  return (
    <div className="space-y-3">
      {/* 规则名称是编辑态识别标签；当前后端 DetectionRule 契约不持久化名称 */}
      <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 shadow-2xs backdrop-blur-md">
        <span className="text-[11px] text-[var(--text-muted)]">
          {t('inspector.ruleName', { defaultValue: '规则名称' })}
        </span>
        <span className="max-w-[12rem] truncate text-xs font-semibold text-[var(--text-primary)]">
          {rule.name}
        </span>
      </div>

      {/* 几何参数（只读摘要） */}
      <div className={`grid ${isPolygon ? 'grid-cols-2' : 'grid-cols-1'} gap-2 text-[11px]`}>
        <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 shadow-2xs backdrop-blur-md">
          <span className="text-[var(--text-muted)]">
            {isPolygon
              ? t('inspector.vertexCount', { defaultValue: '顶点数' })
              : t('inspector.endpointCount', { defaultValue: '端点' })}
          </span>
          <span className="font-mono font-bold text-[var(--text-primary)]">
            {rule.points.length}
          </span>
        </div>
        {isPolygon && (
          <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 shadow-2xs backdrop-blur-md">
            <span className="text-[var(--text-muted)]">
              {t('inspector.coveragePercent', { defaultValue: '画幅占比' })}
            </span>
            <span className="font-mono font-bold text-[var(--accent)]">
              {calculatePolygonAreaPercent(rule.points).toFixed(1)}%
            </span>
          </div>
        )}
      </div>

      {/* 绊线方向（唯一持久化的规则属性） */}
      {rule.role === 'line' && (
        <div>
          <label className="mb-1.5 block font-mono text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
            {t('inspector.lineDirection', { defaultValue: '跨线判定方向' })}
          </label>
          <div className="grid grid-cols-3 gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-1 text-center text-xs backdrop-blur-md">
            {(['both', 'a_to_b', 'b_to_a'] as DetectionLineDirection[]).map((dir) => {
              const isActive = rule.lineDirection === dir
              return (
                <button
                  key={dir}
                  type="button"
                  onClick={() => onUpdateRule(rule.id, { lineDirection: dir })}
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

      {/* 可见性：仅影响当前编辑会话，不改变后端规则契约 */}
      <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 shadow-2xs backdrop-blur-md">
        <span className="text-[11px] font-medium text-[var(--text-secondary)]">
          {t('inspector.visibility', { defaultValue: '画面可见性' })}
        </span>
        <button
          type="button"
          aria-pressed={rule.visible}
          aria-label={t('inspector.visibility', { defaultValue: '画面可见性' })}
          onClick={() => onUpdateRule(rule.id, { visible: !rule.visible })}
          className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-[11px] font-semibold transition-all ${
            rule.visible
              ? 'border border-[var(--accent)]/30 bg-[var(--accent-soft)] text-[var(--accent)]'
              : 'border border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)]'
          }`}
        >
          {rule.visible ? <Eye className="h-3.5 w-3.5" /> : <EyeOff className="h-3.5 w-3.5" />}
          <span>
            {rule.visible
              ? t('inspector.visible', { defaultValue: '显示' })
              : t('inspector.hidden', { defaultValue: '隐藏' })}
          </span>
        </button>
      </div>

      {/* 算力作用域：说明几何规则与算法的真实关系 */}
      <div className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 shadow-2xs backdrop-blur-md">
        <div className="flex items-center gap-1.5 text-[11px] font-semibold text-[var(--text-primary)]">
          <Layers className="h-3.5 w-3.5 text-[var(--accent)]" />
          <span>{t('inspector.scopeTitle', { defaultValue: '算力作用域' })}</span>
        </div>
        {activeAlgorithmNames.length === 0 ? (
          <p className="text-[10px] text-[var(--status-warning)]">
            {t('inspector.noActiveAlgo', { defaultValue: '尚未启用任何算法，本规则暂不参与判定' })}
          </p>
        ) : (
          <>
            <div className="flex flex-wrap gap-1">
              {activeAlgorithmNames.map((name) => (
                <span
                  key={name}
                  className="rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-secondary)]"
                >
                  {name}
                </span>
              ))}
            </div>
            <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">
              {t('inspector.scopeHint', {
                defaultValue: '几何规则由该通道内全部已启用算法共享判定，命中后统一触发告警。',
              })}
            </p>
          </>
        )}
      </div>

      {/* 克隆与删除 */}
      <div className="grid grid-cols-2 gap-2">
        <button
          type="button"
          onClick={() => onCloneRule(rule.id)}
          className="flex items-center justify-center gap-1.5 rounded-xl border border-[var(--accent)]/30 bg-[var(--accent-soft)] py-2 text-xs font-semibold text-[var(--accent)] transition-all hover:bg-[var(--accent-soft)] active:scale-98"
        >
          <Copy className="h-3.5 w-3.5" />
          <span>{t('inspector.clone', { defaultValue: '克隆防区' })}</span>
        </button>
        <button
          type="button"
          onClick={() => onDeleteRule(rule.id)}
          className="flex items-center justify-center gap-1.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] py-2 text-xs font-semibold text-[var(--status-danger)] transition-all hover:bg-[var(--status-danger-soft)] active:scale-98"
        >
          <Trash2 className="h-3.5 w-3.5" />
          <span>{t('inspector.delete', { defaultValue: '删除防区' })}</span>
        </button>
      </div>
    </div>
  )
}
