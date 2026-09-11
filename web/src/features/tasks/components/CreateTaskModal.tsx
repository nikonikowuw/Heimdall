import React, { useEffect, useState } from 'react'
import { AlertCircle, Check, Loader2, Plus, Sliders, Video, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { taskApi, algorithmApi } from '@/lib/api'
import { StreamModeSelector } from '@/components/StreamModeSelector'
import type { AlgorithmItem, Camera, TaskConfigDto, StreamMode } from '@/types'

export interface CreateTaskModalProps {
  isOpen: boolean
  cameras: Camera[]
  existingCameraIdsWithTasks: Set<string>
  preselectedCameraId?: string | null
  onClose: () => void
  onSuccess: (camera: Camera, task: TaskConfigDto) => void
  onGoToCameras: () => void
}

export function CreateTaskModal({
  isOpen,
  cameras,
  existingCameraIdsWithTasks,
  preselectedCameraId,
  onClose,
  onSuccess,
  onGoToCameras,
}: CreateTaskModalProps): React.ReactElement | null {
  const { t } = useTranslation('task')
  const { t: tc } = useTranslation('common')

  const [selectedCameraId, setSelectedCameraId] = useState<string>('')
  const [taskName, setTaskName] = useState<string>('')
  const [streamMode, setStreamMode] = useState<StreamMode>('auto')
  const [desiredEnabled, setDesiredEnabled] = useState<boolean>(true)
  const [availableAlgorithms, setAvailableAlgorithms] = useState<AlgorithmItem[]>([])
  const [selectedAlgorithmId, setSelectedAlgorithmId] = useState<string>('')
  const [analysisFps, setAnalysisFps] = useState<number>(10)
  const [algoParams, setAlgoParams] = useState<Record<string, unknown>>({})
  const [isSubmitting, setIsSubmitting] = useState<boolean>(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  const selectedAlgo = availableAlgorithms.find((item) => item.algorithmId === selectedAlgorithmId)
  const activeVersion =
    selectedAlgo?.versions.find((v) => v.isActive) ||
    selectedAlgo?.versions.find((v) => v.version === selectedAlgo.activeVersion) ||
    selectedAlgo?.versions[0]
  const schemaObj = (activeVersion?.configSchema as Record<string, unknown>) || {}
  const propertiesObj = (schemaObj.properties as Record<string, Record<string, unknown>>) || {}

  // 当选定算法变化时，根据其 configSchema 初始化 algoParams
  useEffect(() => {
    if (!activeVersion) {
      setAlgoParams({})
      return
    }
    const schemaProps =
      (activeVersion.configSchema?.properties as Record<string, Record<string, unknown>>) || {}
    const initial: Record<string, unknown> = {}
    for (const [key, prop] of Object.entries(schemaProps)) {
      if (prop.default !== undefined) {
        initial[key] = prop.default
      } else if (prop.type === 'number' || prop.type === 'integer') {
        initial[key] = prop.minimum ?? 0
      } else if (prop.type === 'boolean') {
        initial[key] = false
      } else if (prop.type === 'string') {
        initial[key] = (prop.enum as string[])?.[0] ?? ''
      } else if (prop.type === 'array') {
        initial[key] = []
      }
    }
    setAlgoParams(initial)
  }, [selectedAlgorithmId, activeVersion])

  // 打开弹窗时加载当前可用算法，并默认选择已激活版本的算法。
  useEffect(() => {
    if (!isOpen) return
    let active = true
    algorithmApi
      .list({ page: 1, pageSize: 100 })
      .then((result) => {
        if (!active) return
        setAvailableAlgorithms(result.items)
        const defaultAlgorithm =
          result.items.find((item) => item.activeVersion.trim() !== '') ?? result.items[0]
        setSelectedAlgorithmId(defaultAlgorithm?.algorithmId ?? '')
      })
      .catch(() => {
        if (active) {
          setAvailableAlgorithms([])
          setSelectedAlgorithmId('')
        }
      })
    return () => {
      active = false
    }
  }, [isOpen])

  // 当弹窗打开时，默认选第一个未绑定任务的摄像头
  useEffect(() => {
    if (isOpen) {
      setErrorMsg(null)
      const preselected = preselectedCameraId
        ? cameras.find((c) => c.cameraId === preselectedCameraId)
        : undefined
      const defaultCam =
        preselected ??
        cameras.find((c) => !existingCameraIdsWithTasks.has(c.cameraId)) ??
        cameras[0]

      if (defaultCam) {
        setSelectedCameraId(defaultCam.cameraId)
        setTaskName(`Task-${defaultCam.name || defaultCam.cameraId}`)
        setStreamMode(defaultCam.streamMode || 'auto')
      } else {
        setSelectedCameraId('')
        setTaskName('')
        setStreamMode('auto')
      }
      setDesiredEnabled(true)
      setAnalysisFps(10)
    }
  }, [isOpen, cameras, existingCameraIdsWithTasks, preselectedCameraId])

  // ESC 快捷键关闭
  useEffect(() => {
    if (!isOpen) return
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isSubmitting) {
        onClose()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [isOpen, isSubmitting, onClose])

  if (!isOpen) return null

  const selectedCam = cameras.find((c) => c.cameraId === selectedCameraId)

  const handleCameraChange = (camId: string) => {
    setSelectedCameraId(camId)
    const cam = cameras.find((c) => c.cameraId === camId)
    if (cam) {
      setTaskName(`Task-${cam.name || cam.cameraId}`)
      setStreamMode(cam.streamMode || 'auto')
    }
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!selectedCameraId) {
      setErrorMsg(t('selectChannelPlaceholder', { defaultValue: '请选择要分配任务的摄像头通道' }))
      return
    }

    const trimmedName = taskName.trim()
    if (!trimmedName) {
      setErrorMsg(t('validation.nameRequired', { defaultValue: '任务名称不能为空' }))
      return
    }

    const cam = cameras.find((c) => c.cameraId === selectedCameraId)
    if (!cam) {
      setErrorMsg(t('validation.cameraNotFound', { defaultValue: '所选摄像头不存在' }))
      return
    }

    setIsSubmitting(true)
    setErrorMsg(null)

    try {
      const payload: TaskConfigDto = {
        cameraId: selectedCameraId,
        name: trimmedName,
        desiredEnabled,
        streamMode,
        rules: [],
        motionGate: {
          enabled: true,
          threshold: 25,
          contourArea: 100,
          keepaliveIntervalMs: 2000,
        },
        algorithmInstances: selectedAlgorithmId
          ? [
              {
                algorithmId: selectedAlgorithmId,
                analysisFps,
                algoParams,
                enabled: desiredEnabled,
              },
            ]
          : undefined,
      }

      const created = await taskApi.updateTask(selectedCameraId, payload)
      onSuccess({ ...cam, streamMode }, created)
      onClose()
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('errors.createFailed', { defaultValue: '创建任务失败，请稍后重试' })
      setErrorMsg(msg)
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <div
      onClick={(e) => {
        if (e.target === e.currentTarget && !isSubmitting) {
          onClose()
        }
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs"
    >
      <div className="frosted-glass relative max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-2xl border border-[var(--border)] p-6 shadow-2xl transition-all">
        {/* 右上角关闭 */}
        <button
          type="button"
          onClick={onClose}
          disabled={isSubmitting}
          className="absolute top-5 right-5 rounded-lg p-1 text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
        >
          <X className="h-4 w-4" />
        </button>

        {/* 头部标题与图标 */}
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs">
            <Sliders className="h-5 w-5" />
          </div>
          <div>
            <h3 className="text-base font-bold text-[var(--text-primary)]">
              {t('createTaskTitle', { defaultValue: '创建 AI 分析与布防任务' })}
            </h3>
            <p className="text-xs text-[var(--text-muted)]">
              {t('createTaskDesc', {
                defaultValue: '为已接入的摄像头通道建立计算任务，配置空间几何规则并开启 NPU 推理。',
              })}
            </p>
          </div>
        </div>

        {cameras.length === 0 ? (
          <div className="mt-6 flex flex-col items-center justify-center py-6 text-center text-xs">
            <Video className="mb-2 h-8 w-8 text-[var(--text-muted)] opacity-50" />
            <p className="font-semibold text-[var(--text-primary)]">
              {t('noCamerasAvailable', {
                defaultValue: '系统中暂无任何摄像头设备，请先接入摄像机',
              })}
            </p>
            <p className="mt-1 text-[var(--text-muted)]">
              {t('noCamerasHint', { defaultValue: 'AI 任务需要绑定在有效的视频流通道上运行。' })}
            </p>
            <button
              type="button"
              onClick={() => {
                onClose()
                onGoToCameras()
              }}
              className="mt-4 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90"
            >
              <Plus className="h-3.5 w-3.5" />
              <span>{t('goToCameras', { defaultValue: '前往设备管理' })}</span>
            </button>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="mt-5 space-y-4 text-xs">
            {/* 1. 选择通道 */}
            <div>
              <label className="mb-1.5 block font-semibold text-[var(--text-primary)]">
                {t('selectChannel', { defaultValue: '选择摄像头通道' })}
                <span className="ml-1 text-rose-500">*</span>
              </label>
              <select
                value={selectedCameraId}
                onChange={(e) => handleCameraChange(e.target.value)}
                disabled={isSubmitting}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
              >
                {cameras.map((cam) => {
                  const alreadyHas = existingCameraIdsWithTasks.has(cam.cameraId)
                  return (
                    <option key={cam.cameraId} value={cam.cameraId}>
                      {cam.name || cam.cameraId} ({cam.cameraId}){' '}
                      {alreadyHas
                        ? t('alreadyConfiguredTag', { defaultValue: '· [已配置任务]' })
                        : ''}
                    </option>
                  )
                })}
              </select>

              {selectedCam && (
                <div className="mt-1.5 flex items-center gap-2 font-mono text-[11px] text-[var(--text-muted)]">
                  <span>
                    {selectedCam.lastWidth && selectedCam.lastHeight
                      ? `${selectedCam.lastWidth}x${selectedCam.lastHeight}`
                      : '1080P'}
                  </span>
                  <span>·</span>
                  <span className="text-[var(--accent)]">
                    {selectedCam.lastCodec?.toUpperCase() || 'H.264'}
                  </span>
                  <span>·</span>
                  <span className="text-emerald-500">
                    {selectedCam.lastFps ? selectedCam.lastFps.toFixed(1) : '25.0'} fps
                  </span>
                  {existingCameraIdsWithTasks.has(selectedCam.cameraId) && (
                    <span className="font-sans text-amber-500">
                      {t('alreadyHasTask', {
                        defaultValue: '(该通道已有任务，保存将覆盖更新)',
                      })}
                    </span>
                  )}
                </div>
              )}
            </div>

            {/* AI 分析码流来源偏好 */}
            <StreamModeSelector
              value={streamMode}
              onChange={setStreamMode}
              labelClassName="mb-1.5 block font-semibold text-[var(--text-primary)]"
            />

            {/* 2. 任务名称 */}
            <div>
              <label className="mb-1.5 block font-semibold text-[var(--text-primary)]">
                {t('taskName', { defaultValue: '任务名称' })}
                <span className="ml-1 text-rose-500">*</span>
              </label>
              <input
                type="text"
                value={taskName}
                onChange={(e) => setTaskName(e.target.value)}
                placeholder={t('taskNamePlaceholder', {
                  defaultValue: '例如：周界入侵防护 - 库房正门',
                })}
                disabled={isSubmitting}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
              />
            </div>

            {/* 3. 算法与抽帧配置 */}
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_8rem]">
              <div>
                <label className="mb-1.5 block font-semibold text-[var(--text-primary)]">
                  {t('algorithm', { defaultValue: '分析算法' })}
                </label>
                <select
                  value={selectedAlgorithmId}
                  onChange={(e) => setSelectedAlgorithmId(e.target.value)}
                  disabled={isSubmitting || availableAlgorithms.length === 0}
                  className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)] disabled:opacity-60"
                >
                  {availableAlgorithms.length === 0 ? (
                    <option value="">
                      {t('algorithmFallback', { defaultValue: '使用系统默认算法' })}
                    </option>
                  ) : (
                    availableAlgorithms.map((algorithm) => (
                      <option key={algorithm.algorithmId} value={algorithm.algorithmId}>
                        {algorithm.name} ({algorithm.algorithmId})
                      </option>
                    ))
                  )}
                </select>
              </div>
              <div>
                <label className="mb-1.5 block font-semibold text-[var(--text-primary)]">
                  {t('analysisFps', { defaultValue: '分析 FPS' })}
                </label>
                <input
                  type="number"
                  min={0}
                  max={60}
                  step={1}
                  value={analysisFps}
                  onChange={(e) => setAnalysisFps(Number(e.target.value))}
                  disabled={isSubmitting}
                  className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
                />
              </div>
            </div>

            {/* 3.1 动态算法参数配置区 */}
            {Object.keys(propertiesObj).length > 0 && (
              <div className="space-y-3 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 p-3">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
                    <Sliders className="h-3.5 w-3.5 text-[var(--accent)]" />
                    <span>{t('studio.algoPanelTitle', { defaultValue: '算法运行参数' })}</span>
                  </div>
                  <span className="font-mono text-[10px] text-[var(--text-muted)]">
                    {selectedAlgo?.name} v{activeVersion?.version}
                  </span>
                </div>

                <div className="max-h-48 space-y-2.5 overflow-y-auto pr-1">
                  {Object.entries(propertiesObj).map(([key, prop]) => {
                    const val = algoParams[key]
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
                            <span className="font-medium text-[var(--text-secondary)]">
                              {title}
                            </span>
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
                            onChange={(e) =>
                              setAlgoParams((p) => ({
                                ...p,
                                [key]: parseFloat(e.target.value),
                              }))
                            }
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
                            <span className="font-medium text-[var(--text-secondary)]">
                              {title}
                            </span>
                            {desc && (
                              <p className="truncate text-[10px] text-[var(--text-muted)]">
                                {desc}
                              </p>
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
                            onChange={(e) =>
                              setAlgoParams((p) => ({
                                ...p,
                                [key]: Number(e.target.value),
                              }))
                            }
                            className="w-24 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                          />
                        </div>
                      )
                    }

                    if (prop.type === 'boolean') {
                      return (
                        <div key={key} className="flex items-center justify-between gap-2">
                          <div>
                            <span className="font-medium text-[var(--text-secondary)]">
                              {title}
                            </span>
                            {desc && <p className="text-[10px] text-[var(--text-muted)]">{desc}</p>}
                          </div>
                          <input
                            type="checkbox"
                            checked={Boolean(val)}
                            onChange={(e) =>
                              setAlgoParams((p) => ({ ...p, [key]: e.target.checked }))
                            }
                            className="h-3.5 w-3.5 cursor-pointer rounded accent-[var(--accent)]"
                          />
                        </div>
                      )
                    }

                    if (prop.type === 'array' && (prop.items as Record<string, unknown>)?.enum) {
                      const enumItems = (prop.items as Record<string, unknown>).enum as string[]
                      const currentArr = Array.isArray(val) ? (val as string[]) : []
                      return (
                        <div key={key} className="space-y-1.5">
                          <div className="flex items-center justify-between">
                            <span className="font-medium text-[var(--text-secondary)]">
                              {title}
                            </span>
                            <div className="flex gap-1 text-[10px]">
                              <button
                                type="button"
                                onClick={() =>
                                  setAlgoParams((p) => ({ ...p, [key]: [...enumItems] }))
                                }
                                className="text-[var(--accent)] hover:underline"
                              >
                                {t('studio.selectAll', { defaultValue: '全选' })}
                              </button>
                              <span className="text-[var(--text-muted)]">|</span>
                              <button
                                type="button"
                                onClick={() => setAlgoParams((p) => ({ ...p, [key]: [] }))}
                                className="text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                              >
                                {t('studio.clearAll', { defaultValue: '清空' })}
                              </button>
                            </div>
                          </div>
                          <div className="flex max-h-24 flex-wrap gap-1 overflow-y-auto">
                            {enumItems.map((item) => {
                              const isSelected = currentArr.includes(item)
                              return (
                                <button
                                  key={item}
                                  type="button"
                                  onClick={() =>
                                    setAlgoParams((p) => {
                                      const next = isSelected
                                        ? currentArr.filter((c) => c !== item)
                                        : [...currentArr, item]
                                      return { ...p, [key]: next }
                                    })
                                  }
                                  className={`rounded border px-2 py-0.5 font-mono text-[11px] transition-all ${
                                    isSelected
                                      ? 'border-[var(--accent)] bg-[var(--accent)] text-white'
                                      : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
                                  }`}
                                >
                                  {item}
                                </button>
                              )
                            })}
                          </div>
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
                            onChange={(e) =>
                              setAlgoParams((p) => ({ ...p, [key]: e.target.value }))
                            }
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
                            onChange={(e) =>
                              setAlgoParams((p) => ({ ...p, [key]: e.target.value }))
                            }
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

            {/* 4. 初始布防状态 */}
            <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
              <div>
                <span className="font-semibold text-[var(--text-primary)]">
                  {t('enableArmImmediately', { defaultValue: '创建后立即启动布防' })}
                </span>
                <p className="mt-0.5 text-[11px] text-[var(--text-muted)]">
                  {t('enableArmImmediatelyDesc', {
                    defaultValue:
                      '开启后系统将启动该路摄像头的分析码流解码并在后台调度 NPU 规则判定。',
                  })}
                </p>
              </div>
              <button
                type="button"
                onClick={() => setDesiredEnabled(!desiredEnabled)}
                disabled={isSubmitting}
                className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                  desiredEnabled ? 'bg-[var(--accent)]' : 'bg-slate-400'
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-lg ring-0 transition duration-200 ease-in-out ${
                    desiredEnabled ? 'translate-x-4' : 'translate-x-0'
                  }`}
                />
              </button>
            </div>

            {/* 错误提示 */}
            {errorMsg && (
              <div className="flex items-center gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-500">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{errorMsg}</span>
              </div>
            )}

            {/* 底部按钮 */}
            <div className="flex items-center justify-end gap-3 pt-2">
              <button
                type="button"
                onClick={onClose}
                disabled={isSubmitting}
                className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-semibold text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
              >
                {tc('actions.cancel', { defaultValue: '取消' })}
              </button>
              <button
                type="submit"
                disabled={isSubmitting}
                className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
              >
                {isSubmitting ? (
                  <>
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    <span>{t('creating', { defaultValue: '创建中...' })}</span>
                  </>
                ) : (
                  <>
                    <Check className="h-3.5 w-3.5" />
                    <span>{t('confirmAndDrawRules', { defaultValue: '创建并进入标定画板' })}</span>
                  </>
                )}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  )
}
