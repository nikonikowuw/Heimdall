import React from 'react'
import { Check, ChevronRight, Cpu } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { AlgoManifest } from '@/types'
import { getLocalizedClassName } from './rulesStudioTypes'

export interface AlgoSettingsSidebarProps {
  availableAlgos: AlgoManifest[]
  selectedAlgoId: string
  activeAlgo: AlgoManifest
  onAlgoChange: (newAlgoId: string) => void
  onOpenAlgoDrawer: () => void
  globalTargetClasses: string[]
  onToggleTargetClass: (cls: string) => void
  onSelectAllClasses: () => void
  onClearAllClasses: () => void
  confidenceThreshold: number
  onConfidenceThresholdChange: (val: number) => void
  motionGateEnabled: boolean
  onMotionGateEnabledChange: (enabled: boolean) => void
  customAlgoParams?: Record<string, unknown>
  onCustomAlgoParamChange?: (key: string, val: unknown) => void
}

export function AlgoSettingsSidebar({
  availableAlgos,
  selectedAlgoId,
  activeAlgo,
  onAlgoChange,
  onOpenAlgoDrawer,
  globalTargetClasses,
  onToggleTargetClass,
  onSelectAllClasses,
  onClearAllClasses,
  confidenceThreshold,
  onConfidenceThresholdChange,
  motionGateEnabled,
  onMotionGateEnabledChange,
  customAlgoParams,
  onCustomAlgoParamChange,
}: AlgoSettingsSidebarProps): React.ReactElement {
  const { t, i18n } = useTranslation('task')

  const propertiesObj =
    (activeAlgo.configSchema?.properties as Record<string, Record<string, unknown>>) || {}
  const ignoredKeys = new Set([
    'confidence_threshold',
    'confidenceThreshold',
    'detection_confidence_threshold',
    'target_classes',
    'targetClasses',
    'classes',
    'allowed_classes',
  ])
  const extraProps = Object.entries(propertiesObj).filter(([k]) => !ignoredKeys.has(k))

  const hasSchema = Object.keys(propertiesObj).length > 0
  const confProp =
    propertiesObj.confidence_threshold ||
    propertiesObj.confidenceThreshold ||
    propertiesObj.detection_confidence_threshold
  const hasConfidenceParam = !hasSchema || Boolean(confProp)
  const confidenceTitle = confProp?.title
    ? String(confProp.title)
    : t('studio.confidenceThreshold', { defaultValue: '告警置信度阈值' })

  const hasClasses = Boolean(activeAlgo.classes && activeAlgo.classes.length > 0)

  return (
    <aside className="frosted-glass flex w-72 max-w-72 min-w-72 flex-col overflow-hidden border-r border-[var(--border)] text-xs">
      <div className="flex h-12 shrink-0 items-center justify-between border-b border-[var(--border)] px-3.5">
        <div className="flex items-center gap-2 font-semibold text-[var(--text-primary)]">
          <Cpu className="h-4 w-4 text-[var(--accent)]" />
          <span>{t('studio.algoPanelTitle', { defaultValue: '算法与目标感知' })}</span>
        </div>
        <span className="rounded-full border border-emerald-500/20 bg-emerald-500/10 px-2 py-0.5 font-mono text-[10px] font-semibold text-emerald-500">
          READY
        </span>
      </div>

      <div className="flex-1 space-y-4 overflow-y-auto p-3">
        {/* 1. 算法模型切换 */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="font-semibold text-[var(--text-primary)]">
              {t('studio.selectAlgo', { defaultValue: '当前运行算法模型' })}
            </span>
            <span className="font-mono text-[10px] text-[var(--text-muted)]">
              {availableAlgos.length} 个就绪
            </span>
          </div>

          <select
            value={selectedAlgoId}
            onChange={(e) => onAlgoChange(e.target.value)}
            className="w-full cursor-pointer rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-2 font-medium text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
          >
            {availableAlgos.map((algo) => (
              <option key={algo.algorithmId} value={algo.algorithmId}>
                {algo.name} (v{algo.version})
              </option>
            ))}
          </select>

          {/* 当前算法信息卡片 */}
          <div className="space-y-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-2.5 shadow-2xs">
            <div className="flex items-center justify-between">
              <span className="max-w-[140px] truncate font-mono font-bold text-[var(--accent)]">
                {activeAlgo.algorithmId}
              </span>
              <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-emerald-500">
                v{activeAlgo.version}
              </span>
            </div>
            <p className="line-clamp-2 text-[11px] leading-relaxed text-[var(--text-muted)]">
              {activeAlgo.description}
            </p>
            <div className="flex flex-wrap items-center gap-1 pt-0.5">
              {activeAlgo.supportedPlatforms.map((p) => (
                <span
                  key={p}
                  className="rounded border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[9px] text-[var(--text-secondary)] uppercase"
                >
                  {p}
                </span>
              ))}
            </div>
            <button
              type="button"
              onClick={onOpenAlgoDrawer}
              className="mt-1 flex w-full items-center justify-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)]/70 py-1 text-[11px] font-medium text-[var(--accent)] transition-colors hover:bg-[var(--accent-soft)]"
            >
              <span>{t('studio.viewSandbox', { defaultValue: '沙箱详情与参数' })}</span>
              <ChevronRight className="h-3 w-3" />
            </button>
          </div>
        </div>

        {/* 2. 警戒目标类别多选（仅当算法定义了目标类别时展示） */}
        {hasClasses && (
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-1">
                <span className="font-semibold text-[var(--text-primary)]">
                  {t('studio.targetClasses', { defaultValue: '警戒目标类别' })}
                </span>
                <span className="font-mono text-[10px] text-[var(--text-muted)]">
                  ({globalTargetClasses.length}/{activeAlgo.classes.length})
                </span>
              </div>
              <div className="flex items-center gap-1.5">
                <button
                  type="button"
                  onClick={onSelectAllClasses}
                  className="text-[10px] font-medium text-[var(--accent)] hover:underline"
                >
                  {t('studio.selectAll', { defaultValue: '全选' })}
                </button>
                <span className="text-[var(--text-muted)]">|</span>
                <button
                  type="button"
                  onClick={onClearAllClasses}
                  className="text-[10px] text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                >
                  {t('studio.clearAll', { defaultValue: '清空' })}
                </button>
              </div>
            </div>

            <div className="grid max-h-44 grid-cols-1 gap-1.5 overflow-y-auto pr-0.5">
              {activeAlgo.classes.map((cls) => {
                const isChecked = globalTargetClasses.includes(cls)
                return (
                  <button
                    key={cls}
                    type="button"
                    onClick={() => onToggleTargetClass(cls)}
                    className={`flex items-center justify-between rounded-lg border px-2.5 py-1.5 text-left transition-all ${
                      isChecked
                        ? 'border-[var(--accent)] bg-[var(--accent-soft)] font-semibold text-[var(--accent)]'
                        : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
                    }`}
                  >
                    <span className="truncate">{getLocalizedClassName(cls, i18n.language)}</span>
                    <div
                      className={`flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border transition-colors ${
                        isChecked
                          ? 'border-[var(--accent)] bg-[var(--accent)] text-white'
                          : 'border-[var(--border-strong)] bg-transparent'
                      }`}
                    >
                      {isChecked && <Check className="h-2.5 w-2.5 stroke-[3]" />}
                    </div>
                  </button>
                )
              })}
            </div>
          </div>
        )}

        {/* 3. 感知灵敏度与置信度（仅当算法支持置信度调节时展示） */}
        {hasConfidenceParam && (
          <div className="space-y-2 border-t border-[var(--border)] pt-3">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-[var(--text-primary)]">{confidenceTitle}</span>
              <span className="font-mono font-bold text-[var(--accent)]">
                {(confidenceThreshold * 100).toFixed(0)}%
              </span>
            </div>
            <input
              type="range"
              min={Number(confProp?.minimum ?? 0.1)}
              max={Number(confProp?.maximum ?? 0.95)}
              step="0.05"
              value={confidenceThreshold}
              onChange={(e) => onConfidenceThresholdChange(parseFloat(e.target.value))}
              className="w-full cursor-pointer accent-[var(--accent)]"
            />
            <div className="flex justify-between font-mono text-[10px] text-[var(--text-muted)]">
              <span>10% (敏锐)</span>
              <span>50% (标准)</span>
              <span>95% (严苛)</span>
            </div>
          </div>
        )}

        {/* 4. 运动门控过滤 */}
        <div className="space-y-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-2.5">
          <div className="flex items-center justify-between">
            <span className="font-semibold text-[var(--text-primary)]">
              {t('studio.motionGate', { defaultValue: '运动门控过滤' })}
            </span>
            <input
              type="checkbox"
              checked={motionGateEnabled}
              onChange={(e) => onMotionGateEnabledChange(e.target.checked)}
              className="h-3.5 w-3.5 cursor-pointer rounded accent-[var(--accent)]"
            />
          </div>
          <p className="text-[10px] leading-normal text-[var(--text-muted)]">
            {t('studio.motionGateDesc', {
              defaultValue: '无像素变动时跳过 NPU 推理，降低能耗',
            })}
          </p>
        </div>

        {/* 4.1 模型扩展自定义参数 */}
        {extraProps.length > 0 && (
          <div className="space-y-2 border-t border-[var(--border)] pt-3">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-[var(--text-primary)]">
                {t('studio.advancedParams', { defaultValue: '模型自定义参数' })}
              </span>
              <span className="font-mono text-[10px] text-[var(--text-muted)]">
                {extraProps.length} 项
              </span>
            </div>

            <div className="space-y-2.5">
              {extraProps.map(([key, prop]) => {
                const val = customAlgoParams?.[key]
                const title = String(prop.title || key)
                const desc = prop.description ? String(prop.description) : undefined
                const isNum = prop.type === 'number' || prop.type === 'integer'
                const hasMinMax =
                  isNum &&
                  prop.minimum !== undefined &&
                  prop.maximum !== undefined &&
                  (prop.maximum as number) <= 1

                if (hasMinMax) {
                  const numVal = typeof val === 'number' ? val : Number(prop.default ?? 0.5)
                  return (
                    <div key={key} className="space-y-1">
                      <div className="flex items-center justify-between">
                        <span className="font-medium text-[var(--text-secondary)]">{title}</span>
                        <span className="font-mono font-semibold text-[var(--accent)]">
                          {(numVal * 100).toFixed(0)}%
                        </span>
                      </div>
                      <input
                        type="range"
                        min={Number(prop.minimum ?? 0)}
                        max={Number(prop.maximum ?? 1)}
                        step={prop.type === 'integer' ? 1 : 0.05}
                        value={numVal}
                        onChange={(e) => onCustomAlgoParamChange?.(key, parseFloat(e.target.value))}
                        className="w-full cursor-pointer accent-[var(--accent)]"
                      />
                      {desc && <p className="text-[10px] text-[var(--text-muted)]">{desc}</p>}
                    </div>
                  )
                }

                if (isNum) {
                  return (
                    <div key={key} className="flex items-center justify-between gap-2">
                      <div className="min-w-0">
                        <span className="font-medium text-[var(--text-secondary)]">{title}</span>
                        {desc && (
                          <p className="truncate text-[10px] text-[var(--text-muted)]">{desc}</p>
                        )}
                      </div>
                      <input
                        type="number"
                        min={prop.minimum as number | undefined}
                        max={prop.maximum as number | undefined}
                        step={prop.type === 'integer' ? 1 : 0.1}
                        value={
                          typeof val === 'number'
                            ? val
                            : ((prop.default as number | undefined) ?? 0)
                        }
                        onChange={(e) => onCustomAlgoParamChange?.(key, Number(e.target.value))}
                        className="w-20 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                      />
                    </div>
                  )
                }

                if (prop.type === 'boolean') {
                  return (
                    <div key={key} className="flex items-center justify-between gap-2">
                      <div>
                        <span className="font-medium text-[var(--text-secondary)]">{title}</span>
                        {desc && <p className="text-[10px] text-[var(--text-muted)]">{desc}</p>}
                      </div>
                      <input
                        type="checkbox"
                        checked={Boolean(val)}
                        onChange={(e) => onCustomAlgoParamChange?.(key, e.target.checked)}
                        className="h-3.5 w-3.5 cursor-pointer rounded accent-[var(--accent)]"
                      />
                    </div>
                  )
                }

                if (prop.type === 'string' && prop.enum) {
                  const enumItems = prop.enum as string[]
                  return (
                    <div key={key} className="flex items-center justify-between gap-2">
                      <span className="font-medium text-[var(--text-secondary)]">{title}</span>
                      <select
                        value={String(val ?? '')}
                        onChange={(e) => onCustomAlgoParamChange?.(key, e.target.value)}
                        className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                      >
                        {enumItems.map((opt) => (
                          <option key={opt} value={opt}>
                            {opt}
                          </option>
                        ))}
                      </select>
                    </div>
                  )
                }

                if (prop.type === 'string') {
                  return (
                    <div key={key} className="space-y-1">
                      <span className="font-medium text-[var(--text-secondary)]">{title}</span>
                      <input
                        type="text"
                        value={String(val ?? '')}
                        onChange={(e) => onCustomAlgoParamChange?.(key, e.target.value)}
                        placeholder={desc}
                        className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                      />
                    </div>
                  )
                }

                return null
              })}
            </div>
          </div>
        )}
      </div>
    </aside>
  )
}
