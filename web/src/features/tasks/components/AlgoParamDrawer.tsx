import React, { useMemo, useState } from 'react'
import { Check, Cpu, RotateCcw, Sliders, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgoManifest } from '@/types'
import { isCosineThresholdKey, percentToScore, scoreToPercent } from '@/lib/similarity'
import { getEnumOptions, resolveEnumSelection, stripLegacyInjectedParams } from '../algoMetadata'
import {
  clampNumericParam,
  formatNumericDraft,
  getNumericParamConfig,
  isFiniteNumber,
  parseNumericDraft,
} from '../numericParam'
import { EnumArrayField } from './EnumArrayField'
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

/** 算力调度档位（宿主级设置，不属于算法包 schema） */
const FPS_PRESETS: Array<{ fps: number; hint: string }> = [
  { fps: 5, hint: '极低能耗' },
  { fps: 10, hint: '平衡推荐' },
  { fps: 15, hint: '高速捕获' },
  { fps: 25, hint: '满血实时' },
]

function getNumericParamBounds(
  isCosineThreshold: boolean,
  config: { minimum?: number; maximum?: number; type: 'integer' | 'number' },
): { inputMin?: number; inputMax?: number; stepVal: number } {
  if (isCosineThreshold) {
    return {
      inputMin: config.minimum === undefined ? undefined : scoreToPercent(config.minimum),
      inputMax: config.maximum === undefined ? undefined : scoreToPercent(config.maximum),
      stepVal: 0.5,
    }
  }
  return {
    inputMin: config.minimum,
    inputMax: config.maximum,
    stepVal: config.type === 'integer' ? 1 : 0.05,
  }
}

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
  function handleResetDefaults(): void {
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

  function getDisplayNumericValue(key: string, rawValue: number): number {
    return isCosineThresholdKey(key) ? scoreToPercent(rawValue) : rawValue
  }

  function getRawNumericValue(key: string, displayValue: number): number {
    return isCosineThresholdKey(key) ? percentToScore(displayValue) : displayValue
  }

  function commitNumericDraft(
    key: string,
    prop: Record<string, unknown>,
    draft: string,
    fallbackRawValue: number,
  ): void {
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

  function getFinalParams(): Record<string, unknown> {
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

  function handleApply(): void {
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

          {/* 右侧滑出抽屉主体：现代 SaaS 半透明玻璃磨砂风格 */}
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
              className="flex w-screen max-w-md flex-col border-l border-[var(--border)] bg-[var(--bg-surface-solid)] shadow-[var(--shadow-lg)] backdrop-blur-2xl"
            >
              {/* 抽屉头部：毛玻璃微光 */}
              <div className="flex h-16 shrink-0 items-center justify-between border-b border-[var(--border)] bg-[var(--bg-surface)] px-5 backdrop-blur-xl">
                <div className="flex items-center gap-3">
                  <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-[var(--accent)]/25 bg-[var(--accent-soft)] text-[var(--accent)] shadow-sm">
                    <Sliders className="h-4 w-4" />
                  </div>
                  <div>
                    <div className="flex items-center gap-2">
                      <h3
                        id="algo-param-drawer-title"
                        className="text-sm font-bold tracking-tight text-[var(--text-primary)]"
                      >
                        {algo.name}
                      </h3>
                      <span className="rounded-full border border-[var(--accent)]/20 bg-[var(--accent-soft)] px-2 py-0.5 font-mono text-[9px] font-bold text-[var(--accent)]">
                        {algo.category?.toUpperCase() || 'ALGO'}
                      </span>
                    </div>
                    <div className="mt-0.5 flex items-center gap-1.5 font-mono text-[10px] text-[var(--text-muted)]">
                      <span>v{algo.version}</span>
                      <span>·</span>
                      <span className="max-w-[12rem] truncate">{algo.algorithmId}</span>
                    </div>
                  </div>
                </div>

                <button
                  type="button"
                  onClick={onClose}
                  aria-label={t('studio.closeDrawerHint', { defaultValue: '关闭参数面板' })}
                  title={t('studio.closeDrawerHint', { defaultValue: '关闭参数面板' })}
                  className="flex h-8 w-8 items-center justify-center rounded-full border border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>

              {/* 抽屉正文表单（半透明独立滚动视窗） */}
              <div className="flex-1 space-y-4 overflow-y-auto p-4 text-xs sm:p-5">
                {/* 1. 算力开销 FPS 调度 */}
                <div className="space-y-3 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 shadow-sm backdrop-blur-xl transition-all">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-bold tracking-tight text-[var(--text-primary)]">
                      {t('analysisFps', { defaultValue: '推理算力调度 (FPS)' })}
                    </span>
                    <span className="rounded-lg border border-[var(--accent)]/30 bg-[var(--accent)]/10 px-2 py-0.5 font-mono text-xs font-bold text-[var(--accent)] shadow-2xs">
                      {localFps} FPS
                    </span>
                  </div>

                  {/* 分段跑道式调度选择器 */}
                  <div className="grid grid-cols-4 gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-1 font-mono text-[11px]">
                    {FPS_PRESETS.map((item) => {
                      const isActive = localFps === item.fps
                      return (
                        <button
                          key={item.fps}
                          type="button"
                          onClick={() => setLocalFps(item.fps)}
                          className={`flex flex-col items-center justify-center rounded-lg py-2 transition-all ${
                            isActive
                              ? 'border border-[var(--border)] bg-[var(--bg-surface-solid)] font-bold text-[var(--text-primary)] shadow-sm'
                              : 'text-[var(--text-muted)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-secondary)]'
                          }`}
                        >
                          <span className="text-xs">{item.fps} FPS</span>
                          <span className="mt-0.5 font-sans text-[9px] opacity-75">
                            {item.hint}
                          </span>
                        </button>
                      )
                    })}
                  </div>

                  <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">
                    {t('fpsHint', {
                      defaultValue:
                        '较高帧率提供更及时的越界判断，较低帧率可有效节省边缘芯片 NPU 能耗。',
                    })}
                  </p>
                </div>

                {/* 2. 模型自定义专属参数 */}
                {schemaParams.length > 0 && (
                  <div className="space-y-3 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 shadow-sm backdrop-blur-xl transition-all">
                    <div className="flex items-center gap-1.5 font-bold text-[var(--text-primary)]">
                      <Cpu className="h-3.5 w-3.5 text-[var(--accent)]" />
                      <span className="text-xs">
                        {t('studio.advancedParams', { defaultValue: '模型自定义专属参数' })}
                      </span>
                    </div>

                    <div className="space-y-3 pt-1">
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
                              className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 shadow-2xs backdrop-blur-md"
                            >
                              <div className="pr-2">
                                <span className="block text-xs font-semibold text-[var(--text-primary)]">
                                  {title}
                                </span>
                                {desc && (
                                  <p className="mt-0.5 text-[10px] text-[var(--text-muted)]">
                                    {desc}
                                  </p>
                                )}
                              </div>
                              <button
                                type="button"
                                role="switch"
                                aria-checked={checked}
                                aria-label={title}
                                onClick={() => setLocalParams((p) => ({ ...p, [key]: !checked }))}
                                className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                                  checked
                                    ? 'bg-[var(--status-success)] shadow-[0_0_10px_var(--status-success-soft)]'
                                    : 'border border-[var(--border)] bg-[var(--bg-secondary)]'
                                }`}
                              >
                                <span
                                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-[var(--bg-surface-solid)] shadow-md transition-transform ${
                                    checked ? 'translate-x-4' : 'translate-x-0.5'
                                  }`}
                                />
                              </button>
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
                          const { inputMin, inputMax, stepVal } = getNumericParamBounds(
                            isCosineThreshold,
                            config,
                          )
                          const displayValue = getDisplayNumericValue(key, boundedRawValue)
                          const inputValue =
                            numberDrafts[key] ?? formatNumericDraft(displayValue, config.type)
                          const hasRangeBounds = inputMin !== undefined && inputMax !== undefined

                          return (
                            <div
                              key={key}
                              className="space-y-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs backdrop-blur-md"
                            >
                              <div className="flex items-center justify-between">
                                <span className="text-xs font-semibold text-[var(--text-primary)]">
                                  {title}
                                </span>
                                <span className="rounded-md border border-[var(--accent)]/30 bg-[var(--accent)]/10 px-2 py-0.5 font-mono text-xs font-bold text-[var(--accent)] shadow-2xs">
                                  {isCosineThreshold
                                    ? `${displayValue.toFixed(1)}%`
                                    : String(displayValue)}
                                </span>
                              </div>

                              {/* 滑块与数值输入双模联动 */}
                              <div className="space-y-2">
                                {hasRangeBounds && (
                                  <div className="space-y-1">
                                    <input
                                      type="range"
                                      min={inputMin}
                                      max={inputMax}
                                      step={stepVal}
                                      value={displayValue}
                                      aria-label={title}
                                      onChange={(e) => {
                                        const parsedDisplayValue = Number(e.target.value)
                                        const nextRawValue = getRawNumericValue(
                                          key,
                                          parsedDisplayValue,
                                        )
                                        setLocalParams((prev) => ({ ...prev, [key]: nextRawValue }))
                                        setNumberDrafts((prev) => ({
                                          ...prev,
                                          [key]: formatNumericDraft(
                                            parsedDisplayValue,
                                            config.type,
                                          ),
                                        }))
                                      }}
                                      className="h-1.5 w-full cursor-pointer appearance-none rounded-lg bg-[var(--bg-secondary)] accent-[var(--accent)]"
                                    />
                                    <div className="flex items-center justify-between font-mono text-[9px] text-[var(--text-muted)]">
                                      <span>
                                        {isCosineThreshold ? `${inputMin?.toFixed(0)}%` : inputMin}
                                      </span>
                                      <span>
                                        {isCosineThreshold ? `${inputMax?.toFixed(0)}%` : inputMax}
                                      </span>
                                    </div>
                                  </div>
                                )}

                                <div className="flex items-center gap-2">
                                  <input
                                    type="number"
                                    inputMode={config.type === 'integer' ? 'numeric' : 'decimal'}
                                    aria-label={title}
                                    min={inputMin}
                                    max={inputMax}
                                    step={stepVal}
                                    value={inputValue}
                                    onChange={(e) => {
                                      const draft = e.target.value
                                      setNumberDrafts((prev) => ({ ...prev, [key]: draft }))

                                      const parsedDisplayValue = parseNumericDraft(
                                        draft,
                                        config.type,
                                      )
                                      if (parsedDisplayValue === null) return

                                      const nextRawValue = getRawNumericValue(
                                        key,
                                        parsedDisplayValue,
                                      )
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
                                    className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 font-mono text-xs text-[var(--text-primary)] transition-all outline-none focus:border-[var(--accent)] focus:bg-[var(--bg-surface-solid)] focus:ring-2 focus:ring-[var(--accent)]/20"
                                  />
                                </div>
                              </div>

                              {desc && (
                                <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">
                                  {desc}
                                </p>
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

                        const stringEnumOptions = Array.isArray(prop.enum)
                          ? (prop.enum as unknown[]).filter(
                              (x): x is string => typeof x === 'string',
                            )
                          : undefined

                        if (prop.type === 'string') {
                          const textValue =
                            typeof val === 'string' ? val : String(prop.default ?? '')

                          if (stringEnumOptions && stringEnumOptions.length > 0) {
                            if (stringEnumOptions.length <= 4) {
                              return (
                                <div
                                  key={key}
                                  className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs backdrop-blur-md"
                                >
                                  <span className="text-xs font-semibold text-[var(--text-primary)]">
                                    {title}
                                  </span>
                                  <div className="grid grid-cols-2 gap-1 sm:grid-cols-4">
                                    {stringEnumOptions.map((opt) => (
                                      <button
                                        key={opt}
                                        type="button"
                                        onClick={() =>
                                          setLocalParams((prev) => ({ ...prev, [key]: opt }))
                                        }
                                        className={`rounded-lg border px-2 py-1.5 text-center text-xs font-semibold transition-all ${
                                          textValue === opt
                                            ? 'border-[var(--accent)] bg-[var(--accent)] text-white shadow-sm'
                                            : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
                                        }`}
                                      >
                                        {getLocalizedClassName(opt, i18n.language)}
                                      </button>
                                    ))}
                                  </div>
                                  {desc && (
                                    <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">
                                      {desc}
                                    </p>
                                  )}
                                </div>
                              )
                            }

                            return (
                              <div
                                key={key}
                                className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs backdrop-blur-md"
                              >
                                <span className="text-xs font-semibold text-[var(--text-primary)]">
                                  {title}
                                </span>
                                <select
                                  aria-label={title}
                                  value={textValue}
                                  onChange={(e) =>
                                    setLocalParams((prev) => ({ ...prev, [key]: e.target.value }))
                                  }
                                  className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs text-[var(--text-primary)] transition-all outline-none focus:border-[var(--accent)] focus:bg-[var(--bg-surface-solid)] focus:ring-2 focus:ring-[var(--accent)]/20"
                                >
                                  {stringEnumOptions.map((opt) => (
                                    <option key={opt} value={opt}>
                                      {getLocalizedClassName(opt, i18n.language)}
                                    </option>
                                  ))}
                                </select>
                                {desc && (
                                  <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">
                                    {desc}
                                  </p>
                                )}
                              </div>
                            )
                          }

                          return (
                            <div
                              key={key}
                              className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3 shadow-2xs backdrop-blur-md"
                            >
                              <span className="text-xs font-semibold text-[var(--text-primary)]">
                                {title}
                              </span>
                              <input
                                type="text"
                                aria-label={title}
                                value={textValue}
                                onChange={(e) =>
                                  setLocalParams((prev) => ({ ...prev, [key]: e.target.value }))
                                }
                                className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs text-[var(--text-primary)] transition-all outline-none focus:border-[var(--accent)] focus:bg-[var(--bg-surface-solid)] focus:ring-2 focus:ring-[var(--accent)]/20"
                              />
                              {desc && (
                                <p className="text-[10px] leading-relaxed text-[var(--text-muted)]">
                                  {desc}
                                </p>
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

              {/* 抽屉底部行动栏：磨砂高光 */}
              <div className="flex shrink-0 items-center justify-between border-t border-[var(--border)] bg-[var(--bg-surface)] px-5 py-4 backdrop-blur-2xl">
                <button
                  type="button"
                  onClick={handleResetDefaults}
                  className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                  title={t('resetDefaults', { defaultValue: '恢复芯片推荐默认工况' })}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  <span>{t('resetDefaults', { defaultValue: '恢复默认值' })}</span>
                </button>

                <div className="flex items-center gap-2.5">
                  <button
                    type="button"
                    onClick={onClose}
                    className="rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-3.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                  >
                    {t('cancel', { defaultValue: '取消' })}
                  </button>
                  <button
                    type="button"
                    onClick={handleApply}
                    className="flex items-center gap-1.5 rounded-lg bg-[var(--accent)] px-4 py-1.5 text-xs font-semibold text-white shadow-[var(--shadow-md)] transition-all hover:opacity-95 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95"
                  >
                    <Check className="h-3.5 w-3.5 stroke-[2.5]" />
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
