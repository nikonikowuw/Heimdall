import React, { useMemo, useState } from 'react'
import { Check, Cpu, RotateCcw, Sliders, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgoManifest } from '@/types'
import {
  denormalizeCosineSimilarity,
  isCosineThresholdKey,
  normalizeCosineSimilarity,
} from '@/lib/similarity'
import { getEnumOptions, resolveEnumSelection, stripLegacyInjectedParams } from '../algoMetadata'
import {
  clampNumericParam,
  formatNumericDraft,
  getNumericParamConfig,
  isFiniteNumber,
  parseNumericDraft,
} from '../numericParam'
import { EnumArrayField } from './EnumArrayField'

export interface AlgoParamDrawerProps {
  isOpen: boolean
  algo: AlgoManifest | null
  fps: number
  onFpsChange: (fps: number) => void
  params: Record<string, unknown>
  onSaveParams: (params: Record<string, unknown>) => void
  onClose: () => void
}

/** 算力调度档位（宿主级设置，不属于算法包 schema） */
const FPS_PRESETS = [5, 10, 15, 25] as const

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
  const reduceMotion = useReducedMotion()

  // 本地临时编辑状态。数值参数以字符串草稿承载，才能保留空串、小数点、负号等
  // 键盘中间态；校验与取整推迟到失焦或应用时执行。
  const [localFps, setLocalFps] = useState<number>(fps)
  const [localParams, setLocalParams] = useState<Record<string, unknown>>(params)
  const [numberDrafts, setNumberDrafts] = useState<Record<string, string>>({})
  const [arraySearches, setArraySearches] = useState<Record<string, string>>({})

  // 当弹窗打开时，同步外界属性
  React.useEffect(() => {
    if (isOpen) {
      setLocalFps(fps)
      setLocalParams({ ...params })
      setNumberDrafts({})
      setArraySearches({})
    }
  }, [isOpen, fps, params])

  const schemaObj = useMemo(() => {
    return (algo?.configSchema as Record<string, unknown>) || {}
  }, [algo])

  const propertiesObj = useMemo(() => {
    return (schemaObj.properties as Record<string, Record<string, unknown>>) || {}
  }, [schemaObj])

  /**
   * schema 是参数的唯一事实来源：声明了什么就渲染什么。
   *
   * 历史上这里曾用一份按键名硬编码的表把「置信度」「目标类别」从列表中挖出去、
   * 提升为首屏专属控件，导致同一包内语义平行的参数分居两处、控件形态不同；array 与
   * string 型参数又因缺少渲染分支而完全不可见。现在一律收归列表，由参数自身的
   * type 决定控件。
   */
  const schemaParams = useMemo(() => Object.entries(propertiesObj), [propertiesObj])

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
    setNumberDrafts({})
    setArraySearches({})
    setLocalParams(reset)
  }

  const getDisplayNumericValue = (key: string, rawValue: number): number =>
    isCosineThresholdKey(key) ? normalizeCosineSimilarity(rawValue) * 100 : rawValue

  const getRawNumericValue = (key: string, displayValue: number): number =>
    isCosineThresholdKey(key) ? denormalizeCosineSimilarity(displayValue / 100) : displayValue

  const commitNumericDraft = (
    key: string,
    prop: Record<string, unknown>,
    draft: string,
    fallbackRawValue: number,
  ) => {
    const config = getNumericParamConfig(prop)
    if (!config) return

    const parsedDisplayValue = parseNumericDraft(draft, config.type)
    const rawValue =
      parsedDisplayValue === null ? fallbackRawValue : getRawNumericValue(key, parsedDisplayValue)
    const committedValue = clampNumericParam(rawValue, config.minimum, config.maximum)

    setLocalParams((prev) => ({ ...prev, [key]: committedValue }))
    setNumberDrafts((prev) => ({
      ...prev,
      [key]: formatNumericDraft(getDisplayNumericValue(key, committedValue), config.type),
    }))
  }

  const getFinalParams = (): Record<string, unknown> => {
    // 先剥掉 schema 未声明的历史注入键，再按 schema 收敛数值参数
    const finalParams = stripLegacyInjectedParams(localParams, propertiesObj)

    for (const [key, prop] of Object.entries(propertiesObj)) {
      const config = getNumericParamConfig(prop)
      if (!config || (numberDrafts[key] === undefined && !(key in finalParams))) continue

      const currentRawValue = clampNumericParam(
        isFiniteNumber(finalParams[key]) ? finalParams[key] : config.defaultValue,
        config.minimum,
        config.maximum,
      )
      const draft = numberDrafts[key]
      const parsedDisplayValue =
        draft === undefined
          ? getDisplayNumericValue(key, currentRawValue)
          : parseNumericDraft(draft, config.type)
      const rawValue =
        parsedDisplayValue === null ? currentRawValue : getRawNumericValue(key, parsedDisplayValue)

      finalParams[key] = clampNumericParam(rawValue, config.minimum, config.maximum)
    }

    return finalParams
  }

  const handleApply = () => {
    onFpsChange(localFps)
    onSaveParams(getFinalParams())
    onClose()
  }

  useDismissStack(isOpen, onClose)

  return (
    <AnimatePresence mode="wait">
      {isOpen && algo && (
        <motion.div
          key="algo-param-drawer-root"
          role="dialog"
          aria-modal="true"
          aria-labelledby="algo-param-drawer-title"
          className="fixed inset-0 z-50 overflow-hidden"
        >
          {/* 背景轻量微暗遮罩 */}
          <motion.div
            key="algo-param-drawer-backdrop"
            initial={{ opacity: reduceMotion ? 1 : 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduceMotion ? 0 : motionTokens.duration.fast }}
            onClick={onClose}
            className="fixed inset-0 bg-[var(--overlay-scrim)] backdrop-blur-xs"
          />

          {/* 右侧滑出抽屉主体 */}
          <div className="fixed inset-y-0 right-0 flex max-w-full pl-10">
            <motion.div
              key="algo-param-drawer-panel"
              initial={{ x: reduceMotion ? 0 : '100%', opacity: reduceMotion ? 1 : 0 }}
              animate={{ x: 0, opacity: 1 }}
              exit={{ x: reduceMotion ? 0 : '100%', opacity: 0 }}
              transition={{
                duration: reduceMotion ? 0 : motionTokens.duration.normal,
                ease: motionTokens.easing.smooth,
              }}
              className="lens-glass flex w-screen max-w-md flex-col border-l border-[var(--border)] bg-[var(--bg-surface-solid)] shadow-2xl"
            >
              {/* 抽屉头部 */}
              <div className="flex h-14 shrink-0 items-center justify-between border-b border-[var(--border)] px-5">
                <div className="flex items-center gap-2.5">
                  <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--accent-soft)] text-[var(--accent)]">
                    <Sliders className="h-4 w-4" />
                  </div>
                  <div>
                    <h3
                      id="algo-param-drawer-title"
                      className="text-xs font-bold text-[var(--text-primary)]"
                    >
                      {algo.name}
                    </h3>
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
                  aria-label={t('studio.closeDrawerHint', { defaultValue: '关闭参数面板' })}
                  title={t('studio.closeDrawerHint', { defaultValue: '关闭参数面板' })}
                  className="flex h-8 w-8 items-center justify-center rounded-[6px] text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>

              {/* 抽屉正文表单（独立滚动视窗） */}
              <div className="flex-1 space-y-4 overflow-y-auto bg-[var(--bg-primary)]/30 p-4 text-xs sm:p-5">
                {/* 1. 算力开销 FPS 调度 */}
                <div className="space-y-2 rounded-[8px] border border-[var(--border)] bg-[var(--bg-surface)] p-3">
                  <div className="flex items-center justify-between">
                    <span className="font-semibold text-[var(--text-primary)]">
                      {t('analysisFps', { defaultValue: '推理算力调度 (FPS)' })}
                    </span>
                    <span className="font-mono text-[11px] font-bold text-[var(--accent)]">
                      {localFps} FPS
                    </span>
                  </div>
                  <div className="grid grid-cols-4 gap-1.5 pt-1 font-mono text-[11px]">
                    {FPS_PRESETS.map((f) => (
                      <button
                        key={f}
                        type="button"
                        onClick={() => setLocalFps(f)}
                        className={`rounded-lg border py-1.5 font-semibold transition-all ${
                          localFps === f
                            ? 'border-[var(--border-strong)] bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs'
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

                {/* 2. 模型自定义专属参数 */}
                {schemaParams.length > 0 && (
                  <div className="space-y-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-surface)] p-3">
                    <div className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
                      <Cpu className="h-3.5 w-3.5 text-[var(--accent)]" />
                      <span>
                        {t('studio.advancedParams', { defaultValue: '模型自定义专属参数' })}
                      </span>
                    </div>

                    <div className="space-y-2.5 pt-1">
                      {schemaParams.map(([key, prop]) => {
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
                              <span className="font-medium text-[var(--text-primary)]">
                                {title}
                              </span>
                              <input
                                type="checkbox"
                                aria-label={title}
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
                          const config = getNumericParamConfig(prop)
                          if (!config) return null

                          const isCosineThreshold = isCosineThresholdKey(key)
                          const rawValue = isFiniteNumber(val) ? val : config.defaultValue
                          const boundedRawValue = clampNumericParam(
                            rawValue,
                            config.minimum,
                            config.maximum,
                          )
                          const inputMin =
                            config.minimum === undefined
                              ? undefined
                              : isCosineThreshold
                                ? normalizeCosineSimilarity(config.minimum) * 100
                                : config.minimum
                          const inputMax =
                            config.maximum === undefined
                              ? undefined
                              : isCosineThreshold
                                ? normalizeCosineSimilarity(config.maximum) * 100
                                : config.maximum
                          const displayValue = getDisplayNumericValue(key, boundedRawValue)
                          const inputValue =
                            numberDrafts[key] ?? formatNumericDraft(displayValue, config.type)

                          return (
                            <div key={key} className="space-y-1">
                              <div className="flex items-center justify-between">
                                <span className="font-medium text-[var(--text-secondary)]">
                                  {title}
                                </span>
                                <span className="font-mono text-[11px] font-bold text-[var(--accent)]">
                                  {isCosineThreshold
                                    ? `${displayValue.toFixed(1)}%`
                                    : String(displayValue)}
                                </span>
                              </div>
                              <input
                                type="number"
                                inputMode={config.type === 'integer' ? 'numeric' : 'decimal'}
                                aria-label={title}
                                min={inputMin}
                                max={inputMax}
                                step={
                                  isCosineThreshold ? 0.5 : config.type === 'integer' ? 1 : 0.05
                                }
                                value={inputValue}
                                onChange={(e) => {
                                  const draft = e.target.value
                                  setNumberDrafts((prev) => ({ ...prev, [key]: draft }))

                                  const parsedDisplayValue = parseNumericDraft(draft, config.type)
                                  if (parsedDisplayValue === null) return

                                  const nextRawValue = getRawNumericValue(key, parsedDisplayValue)
                                  setLocalParams((prev) => ({ ...prev, [key]: nextRawValue }))
                                }}
                                onBlur={(e) =>
                                  commitNumericDraft(
                                    key,
                                    prop,
                                    e.currentTarget.value,
                                    boundedRawValue,
                                  )
                                }
                                className="w-full rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1 text-xs text-[var(--text-primary)] transition-colors outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
                              />
                              {desc && (
                                <p className="text-[10px] text-[var(--text-muted)]">{desc}</p>
                              )}
                            </div>
                          )
                        }

                        const enumOptions = getEnumOptions(prop)
                        if (prop.type === 'array' && enumOptions.length > 0) {
                          return (
                            <EnumArrayField
                              key={key}
                              title={title}
                              description={desc}
                              options={enumOptions}
                              value={resolveEnumSelection(prop, val)}
                              search={arraySearches[key] ?? ''}
                              onSearchChange={(next) =>
                                setArraySearches((prev) => ({ ...prev, [key]: next }))
                              }
                              onChange={(next) =>
                                setLocalParams((prev) => ({ ...prev, [key]: next }))
                              }
                              language={i18n.language}
                            />
                          )
                        }

                        if (prop.type === 'string') {
                          const textValue =
                            typeof val === 'string' ? val : String(prop.default ?? '')
                          return (
                            <div key={key} className="space-y-1">
                              <span className="font-medium text-[var(--text-secondary)]">
                                {title}
                              </span>
                              <input
                                type="text"
                                aria-label={title}
                                value={textValue}
                                onChange={(e) =>
                                  setLocalParams((prev) => ({ ...prev, [key]: e.target.value }))
                                }
                                className="w-full rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1 text-xs text-[var(--text-primary)] transition-colors outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
                              />
                              {desc && (
                                <p className="text-[10px] text-[var(--text-muted)]">{desc}</p>
                              )}
                            </div>
                          )
                        }

                        // array 型但未声明 items.enum：无候选项可渲染，交由算法包默认值处理
                        return null
                      })}
                    </div>
                  </div>
                )}
              </div>

              {/* 抽屉底部行动栏 */}
              <div className="flex shrink-0 items-center justify-between border-t border-[var(--border)] bg-[var(--bg-surface-solid)] px-4 py-3.5 sm:px-5">
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
                    className="rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] px-3.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                  >
                    {t('cancel', { defaultValue: '取消' })}
                  </button>
                  <button
                    type="button"
                    onClick={handleApply}
                    className="flex items-center gap-1.5 rounded-[6px] bg-[var(--accent)] px-4 py-1.5 text-xs font-semibold text-white shadow-xs transition-opacity hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95"
                  >
                    <Check className="h-3.5 w-3.5" />
                    <span>{t('applyParams', { defaultValue: '应用参数' })}</span>
                  </button>
                </div>
              </div>
            </motion.div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
