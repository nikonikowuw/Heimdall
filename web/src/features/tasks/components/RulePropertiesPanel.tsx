import React from 'react'
import { Copy, Eye, EyeOff, Layers, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { DetectionLineDirection } from '@/types'
import type { ExtendedRule } from './rulesStudioTypes'
import { getDirectionLabel } from './rulesStudioTypes'

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
    <div className="space-y-3.5">
      {/* 规则名称是编辑态识别标签；当前后端 DetectionRule 契约不持久化名称 */}
      <div className="flex items-center justify-between rounded-[7px] border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2">
        <span className="text-[11px] text-[var(--text-muted)]">
          {t('inspector.ruleName', { defaultValue: '规则名称' })}
        </span>
        <span className="max-w-[12rem] truncate text-xs font-semibold text-[var(--text-primary)]">
          {rule.name}
        </span>
      </div>

      {/* 几何参数（只读摘要） */}
      <div className="flex items-center justify-between rounded-[7px] border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-[11px]">
        <span className="text-[var(--text-muted)]">
          {isPolygon
            ? t('inspector.vertexCount', { defaultValue: '顶点数' })
            : t('inspector.endpointCount', { defaultValue: '端点' })}
        </span>
        <span className="font-mono font-semibold text-[var(--text-primary)]">
          {rule.points.length}
        </span>
      </div>

      {/* 绊线方向（唯一持久化的规则属性） */}
      {rule.role === 'line' && (
        <div>
          <label className="mb-1.5 block font-mono text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
            {t('inspector.lineDirection', { defaultValue: '跨线判定方向' })}
          </label>
          <div className="grid grid-cols-3 gap-1 rounded-[7px] border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-center text-xs">
            {(['both', 'a_to_b', 'b_to_a'] as DetectionLineDirection[]).map((dir) => (
              <button
                key={dir}
                type="button"
                onClick={() => onUpdateRule(rule.id, { lineDirection: dir })}
                className={`rounded-[5px] py-1.5 font-medium transition-all ${
                  rule.lineDirection === dir
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

      {/* 可见性：仅影响当前编辑会话，不改变后端规则契约 */}
      <div className="flex items-center justify-between rounded-[7px] border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2">
        <span className="text-[11px] font-medium text-[var(--text-secondary)]">
          {t('inspector.visibility', { defaultValue: '画面可见性' })}
        </span>
        <button
          type="button"
          aria-pressed={rule.visible}
          aria-label={t('inspector.visibility', { defaultValue: '画面可见性' })}
          onClick={() => onUpdateRule(rule.id, { visible: !rule.visible })}
          className={`flex items-center gap-1.5 rounded-[5px] px-2 py-1 text-[11px] font-semibold transition-colors ${
            rule.visible
              ? 'bg-[var(--accent-soft)] text-[var(--accent)]'
              : 'bg-[var(--bg-secondary)] text-[var(--text-muted)]'
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
      <div className="space-y-1.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-secondary)]/60 p-3">
        <div className="flex items-center gap-1.5 text-[11px] font-semibold text-[var(--text-primary)]">
          <Layers className="h-3.5 w-3.5 text-[var(--accent)]" />
          <span>{t('inspector.scopeTitle', { defaultValue: '算力作用域' })}</span>
        </div>
        {activeAlgorithmNames.length === 0 ? (
          <p className="text-[10px] text-amber-500">
            {t('inspector.noActiveAlgo', { defaultValue: '尚未启用任何算法，本规则暂不参与判定' })}
          </p>
        ) : (
          <>
            <div className="flex flex-wrap gap-1">
              {activeAlgorithmNames.map((name) => (
                <span
                  key={name}
                  className="rounded-[5px] border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)]"
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
          className="flex items-center justify-center gap-1.5 rounded-[7px] border border-[var(--border)] bg-[var(--bg-surface)] py-2 text-xs font-semibold text-[var(--accent)] transition-colors hover:bg-[var(--accent-soft)]"
        >
          <Copy className="h-3.5 w-3.5" />
          <span>{t('inspector.clone', { defaultValue: '克隆防区' })}</span>
        </button>
        <button
          type="button"
          onClick={() => onDeleteRule(rule.id)}
          className="flex items-center justify-center gap-1.5 rounded-[7px] border border-rose-500/30 bg-rose-500/10 py-2 text-xs font-semibold text-rose-500 transition-colors hover:bg-rose-500/20"
        >
          <Trash2 className="h-3.5 w-3.5" />
          <span>{t('inspector.delete', { defaultValue: '删除防区' })}</span>
        </button>
      </div>
    </div>
  )
}
