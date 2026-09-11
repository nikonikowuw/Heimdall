import React, { useMemo, useState } from 'react'
import { Check, Cpu, RotateCcw, Sliders, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgoManifest } from '@/types'
import { getLocalizedClassName } from './rulesStudioTypes'

export interface AlgoParamDrawerProps {
  isOpen: boolean
  algo: AlgoManifest | null
  fps: number
  onFpsChange: (fps: number) => void
  params: Record<string, unknown>
  onSaveParams: (params: Record<string, unknown>) => void
  onClose: () => void
}

const IGNORED_KEYS = new Set([
  'confidence_threshold',
  'confidenceThreshold',
  'detection_confidence_threshold',
  'target_classes',
  'targetClasses',
  'classes',
  'allowed_classes',
])

export function AlgoParamDrawer({
  isOpen,
  algo,
  fps,
  onFpsChange,
  params,
  onSaveParams,
  onClose,
}: AlgoParamDrawerProps): React.ReactElement | null {
  const { t, i18n } = useTranslation('task')

  // 本地临时编辑状态
  const [localFps, setLocalFps] = useState<number>(fps)
  const [localParams, setLocalParams] = useState<Record<string, unknown>>(params)
  const [classSearch, setClassSearch] = useState<string>('')

  // 当弹窗打开时，同步外界属性
  React.useEffect(() => {
    if (isOpen) {
      setLocalFps(fps)
      setLocalParams({ ...params })
      setClassSearch('')
    }
  }, [isOpen, fps, params])

  const schemaObj = useMemo(() => {
    return (algo?.configSchema as Record<string, unknown>) || {}
  }, [algo])

  const propertiesObj = useMemo(() => {
    return (schemaObj.properties as Record<string, Record<string, unknown>>) || {}
  }, [schemaObj])

  // 提取置信度属性与标题
  const confProp = useMemo(() => {
    return (
      propertiesObj.confidence_threshold ||
      propertiesObj.confidenceThreshold ||
      propertiesObj.detection_confidence_threshold
    )
  }, [propertiesObj])

  const hasConfidence = !Object.keys(propertiesObj).length || Boolean(confProp)
  const confidenceTitle = confProp?.title
    ? String(confProp.title)
    : t('studio.confidenceThreshold', { defaultValue: '告警置信度阈值' })

  const rawConfidence =
    localParams.confidence_threshold ??
    localParams.confidenceThreshold ??
    localParams.detection_confidence_threshold
  const currentConfidence =
    typeof rawConfidence === 'number' ? rawConfidence : Number(confProp?.default ?? 0.45)

  // 目标类别列表
  const allClasses = useMemo(() => {
    return algo?.classes ?? []
  }, [algo])

  const currentClasses = useMemo(() => {
    const raw = localParams.target_classes ?? localParams.targetClasses
    if (Array.isArray(raw)) {
      return raw.filter((c: unknown): c is string => typeof c === 'string')
    }
    return allClasses
  }, [localParams, allClasses])

  const filteredClasses = useMemo(() => {
    if (!classSearch.trim()) return allClasses
    const q = classSearch.toLowerCase().trim()
    return allClasses.filter((c) => {
      const loc = getLocalizedClassName(c, i18n.language).toLowerCase()
      return c.toLowerCase().includes(q) || loc.includes(q)
    })
  }, [allClasses, classSearch, i18n.language])

  const extraProps = useMemo(() => {
    return Object.entries(propertiesObj).filter(([k]) => !IGNORED_KEYS.has(k))
  }, [propertiesObj])

  // 恢复官方推荐默认值
  const handleResetDefaults = () => {
    setLocalFps(10)
    const reset: Record<string, unknown> = {}
    for (const [key, prop] of Object.entries(propertiesObj)) {
      if (prop.default !== undefined) {
        reset[key] = prop.default
      } else if (prop.type === 'number' || prop.type === 'integer') {
        reset[key] = prop.minimum ?? 0
      } else if (prop.type === 'boolean') {
        reset[key] = false
      }
    }
    // 兼容置信度与类别
    if (confProp?.default !== undefined) {
      reset.confidence_threshold = confProp.default
      reset.confidenceThreshold = confProp.default
    } else {
      reset.confidence_threshold = 0.45
      reset.confidenceThreshold = 0.45
    }
    if (allClasses.length > 0) {
      reset.target_classes = [...allClasses]
      reset.targetClasses = [...allClasses]
    }
    setLocalParams(reset)
  }

  const handleApply = () => {
    onFpsChange(localFps)
    const finalParams = { ...localParams }
    if (hasConfidence) {
      finalParams.confidence_threshold = currentConfidence
      finalParams.confidenceThreshold = currentConfidence
      if ('detection_confidence_threshold' in propertiesObj) {
        finalParams.detection_confidence_threshold = currentConfidence
      }
    }
    if (allClasses.length > 0) {
      finalParams.target_classes = currentClasses
      finalParams.targetClasses = currentClasses
    }
    onSaveParams(finalParams)
    onClose()
  }

  if (!isOpen || !algo) return null

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 overflow-hidden">
        {/* 背景轻量微暗遮罩 */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: motionTokens.duration.fast }}
          onClick={onClose}
          className="fixed inset-0 bg-black/40 backdrop-blur-xs"
        />

        {/* 右侧滑出抽屉主体 */}
        <div className="fixed inset-y-0 right-0 flex max-w-full pl-10">
          <motion.div
            initial={{ x: '100%' }}
            animate={{ x: 0 }}
            exit={{ x: '100%' }}
            transition={{
              duration: motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            className="frosted-glass flex w-screen max-w-md flex-col border-l border-[var(--border)] bg-[var(--bg-surface-solid)] shadow-2xl"
          >
            {/* 抽屉头部 */}
            <div className="flex h-14 shrink-0 items-center justify-between border-b border-[var(--border)] px-5">
              <div className="flex items-center gap-2.5">
                <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--accent-soft)] text-[var(--accent)]">
                  <Sliders className="h-4 w-4" />
                </div>
                <div>
                  <h3 className="text-xs font-bold text-[var(--text-primary)]">{algo.name}</h3>
                  <div className="flex items-center gap-1.5 font-mono text-[10px] text-[var(--text-muted)]">
                    <span>v{algo.version}</span>
                    <span>·</span>
                    <span>{algo.algorithmId}</span>
                  </div>
                </div>
              </div>

              <button
                type="button"
                onClick={onClose}
                className="flex h-8 w-8 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* 抽屉正文表单（独立滚动视窗） */}
            <div className="flex-1 space-y-5 overflow-y-auto p-5 text-xs">
              {/* 1. 算力开销 FPS 调度 */}
              <div className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs">
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-[var(--text-primary)]">
                    {t('analysisFps', { defaultValue: '推理算力调度 (FPS)' })}
                  </span>
                  <span className="font-mono text-[11px] font-bold text-[var(--accent)]">
                    {localFps} FPS
                  </span>
                </div>
                <div className="grid grid-cols-4 gap-1.5 pt-1 font-mono text-[11px]">
                  {[5, 10, 15, 25].map((f) => (
                    <button
                      key={f}
                      type="button"
                      onClick={() => setLocalFps(f)}
                      className={`rounded-lg border py-1.5 font-semibold transition-all ${
                        localFps === f
                          ? 'border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs'
                          : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
                      }`}
                    >
                      {f} FPS
                    </button>
                  ))}
                </div>
                <p className="text-[10px] text-[var(--text-muted)]">
                  {t('fpsHint', {
                    defaultValue:
                      '较高帧率提供更及时的越界判断，较低帧率可有效节省边缘芯片 NPU 能耗。',
                  })}
                </p>
              </div>

              {/* 2. 置信度灵敏度调节 */}
              {hasConfidence && (
                <div className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs">
                  <div className="flex items-center justify-between">
                    <span className="font-semibold text-[var(--text-primary)]">
                      {confidenceTitle}
                    </span>
                    <span className="font-mono font-bold text-[var(--accent)]">
                      {(currentConfidence * 100).toFixed(0)}%
                    </span>
                  </div>
                  <input
                    type="range"
                    min={Number(confProp?.minimum ?? 0.1)}
                    max={Number(confProp?.maximum ?? 0.95)}
                    step="0.05"
                    value={currentConfidence}
                    onChange={(e) => {
                      const val = parseFloat(e.target.value)
                      setLocalParams((prev) => ({
                        ...prev,
                        confidence_threshold: val,
                        confidenceThreshold: val,
                        detection_confidence_threshold: val,
                      }))
                    }}
                    className="w-full cursor-pointer accent-[var(--accent)]"
                  />
                  <div className="flex justify-between font-mono text-[10px] text-[var(--text-muted)]">
                    <span>10% (敏锐/多报)</span>
                    <span>50% (标准推荐)</span>
                    <span>95% (严苛/防误报)</span>
                  </div>
                </div>
              )}

              {/* 3. 警戒目标类别多选（仅限多类别检测模型） */}
              {allClasses.length > 0 && (
                <div className="space-y-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
                      <span>{t('studio.targetClasses', { defaultValue: '警戒目标类别' })}</span>
                      <span className="font-mono text-[10px] text-[var(--text-muted)]">
                        ({currentClasses.length}/{allClasses.length})
                      </span>
                    </div>
                    <div className="flex items-center gap-2 text-[11px]">
                      <button
                        type="button"
                        onClick={() =>
                          setLocalParams((prev) => ({
                            ...prev,
                            target_classes: [...allClasses],
                            targetClasses: [...allClasses],
                          }))
                        }
                        className="font-medium text-[var(--accent)] hover:underline"
                      >
                        {t('studio.selectAll', { defaultValue: '全选' })}
                      </button>
                      <span className="text-[var(--text-muted)]">|</span>
                      <button
                        type="button"
                        onClick={() =>
                          setLocalParams((prev) => ({
                            ...prev,
                            target_classes: [],
                            targetClasses: [],
                          }))
                        }
                        className="text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                      >
                        {t('studio.clearAll', { defaultValue: '清空' })}
                      </button>
                    </div>
                  </div>

                  {/* 类别搜索过滤 */}
                  {allClasses.length > 6 && (
                    <input
                      type="text"
                      value={classSearch}
                      onChange={(e) => setClassSearch(e.target.value)}
                      placeholder={t('searchClassPlaceholder', {
                        defaultValue: '过滤类别 (如: 人, 车, dog)...',
                      })}
                      className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                    />
                  )}

                  {/* 类别多选芯片网格 */}
                  <div className="grid max-h-48 grid-cols-2 gap-1.5 overflow-y-auto pr-0.5">
                    {filteredClasses.map((cls) => {
                      const isChecked = currentClasses.includes(cls)
                      return (
                        <button
                          key={cls}
                          type="button"
                          onClick={() => {
                            const next = isChecked
                              ? currentClasses.filter((c) => c !== cls)
                              : [...currentClasses, cls]
                            setLocalParams((prev) => ({
                              ...prev,
                              target_classes: next,
                              targetClasses: next,
                            }))
                          }}
                          className={`flex items-center justify-between rounded-lg border px-2.5 py-1.5 text-left text-[11px] transition-all ${
                            isChecked
                              ? 'border-[var(--accent)] bg-[var(--accent-soft)] font-semibold text-[var(--accent)]'
                              : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
                          }`}
                        >
                          <span className="truncate">
                            {getLocalizedClassName(cls, i18n.language)}
                          </span>
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

              {/* 4. 模型自定义专属高级参数 */}
              {extraProps.length > 0 && (
                <div className="space-y-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs">
                  <div className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
                    <Cpu className="h-3.5 w-3.5 text-[var(--accent)]" />
                    <span>
                      {t('studio.advancedParams', { defaultValue: '模型自定义专属参数' })}
                    </span>
                  </div>

                  <div className="space-y-2.5 pt-1">
                    {extraProps.map(([key, prop]) => {
                      const val = localParams[key]
                      const title = String(prop.title || key)
                      const desc = prop.description ? String(prop.description) : undefined
                      const isNum = prop.type === 'number' || prop.type === 'integer'
                      const isBool = prop.type === 'boolean'

                      if (isBool) {
                        const checked = Boolean(val ?? prop.default ?? false)
                        return (
                          <div
                            key={key}
                            className="flex items-center justify-between rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1.5"
                          >
                            <span className="font-medium text-[var(--text-primary)]">{title}</span>
                            <input
                              type="checkbox"
                              checked={checked}
                              onChange={(e) =>
                                setLocalParams((p) => ({ ...p, [key]: e.target.checked }))
                              }
                              className="h-4 w-4 cursor-pointer rounded accent-[var(--accent)]"
                            />
                          </div>
                        )
                      }

                      if (isNum) {
                        return (
                          <div key={key} className="space-y-1">
                            <div className="flex items-center justify-between">
                              <span className="font-medium text-[var(--text-secondary)]">
                                {title}
                              </span>
                              <span className="font-mono text-[11px] font-bold text-[var(--accent)]">
                                {String(val ?? prop.default ?? 0)}
                              </span>
                            </div>
                            <input
                              type="number"
                              min={prop.minimum as number | undefined}
                              max={prop.maximum as number | undefined}
                              step={prop.type === 'integer' ? 1 : 0.05}
                              value={typeof val === 'number' ? val : Number(prop.default ?? 0)}
                              onChange={(e) => {
                                const n =
                                  prop.type === 'integer'
                                    ? parseInt(e.target.value, 10) || 0
                                    : parseFloat(e.target.value) || 0
                                setLocalParams((p) => ({ ...p, [key]: n }))
                              }}
                              className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                            />
                            {desc && <p className="text-[10px] text-[var(--text-muted)]">{desc}</p>}
                          </div>
                        )
                      }

                      return null
                    })}
                  </div>
                </div>
              )}
            </div>

            {/* 抽屉底部行动栏 */}
            <div className="flex shrink-0 items-center justify-between border-t border-[var(--border)] bg-[var(--bg-surface)] px-5 py-3.5">
              <button
                type="button"
                onClick={handleResetDefaults}
                className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                title={t('resetDefaults', { defaultValue: '恢复芯片推荐默认工况' })}
              >
                <RotateCcw className="h-3.5 w-3.5" />
                <span>{t('resetDefaults', { defaultValue: '恢复默认值' })}</span>
              </button>

              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={onClose}
                  className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:text-[var(--text-primary)]"
                >
                  {t('cancel', { defaultValue: '取消' })}
                </button>
                <button
                  type="button"
                  onClick={handleApply}
                  className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-1.5 text-xs font-semibold text-white shadow-xs transition-opacity hover:opacity-90 active:scale-95"
                >
                  <Check className="h-3.5 w-3.5" />
                  <span>{t('applyParams', { defaultValue: '应用参数' })}</span>
                </button>
              </div>
            </div>
          </motion.div>
        </div>
      </div>
    </AnimatePresence>
  )
}
