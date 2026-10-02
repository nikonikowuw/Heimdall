import React, { useCallback, useEffect, useMemo, useState } from 'react'
import { Plus, RotateCcw, Search, ShieldAlert, Sliders, Video } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { RefreshButton } from '@/components/RefreshButton'
import { motionTokens } from '@/lib/motionTokens'
import { algorithmApi, cameraApi, taskApi } from '@/lib/api'
import type { Camera, TaskConfigDto, StreamMode } from '@/types'
import { CreateTaskModal } from './components/CreateTaskModal'
import { DeleteTaskModal } from './components/DeleteTaskModal'
import { LiveRulesStudio } from './components/LiveRulesStudio'
import { TaskCameraCard } from './components/TaskCameraCard'
import { TaskFilterBar } from './components/TaskFilterBar'
import {
  ALGORITHM_FILTER_ALL,
  countByArmStatus,
  deriveAlgorithmOptions,
  filterTasks,
  type ArmStatusFilter,
  type TaskFilterEntry,
} from './taskFilter'
import { PageHeader } from '@/components/ui/PageHeader'

export interface TasksPageProps {
  onNavigateToCameras?: () => void
  onNavigateToAlgorithms?: () => void
  initialConfigCameraId?: string | null
}

export function TasksPage({
  onNavigateToCameras,
  onNavigateToAlgorithms,
  initialConfigCameraId,
}: TasksPageProps): React.ReactElement {
  const { t } = useTranslation('task')
  const { t: tc } = useTranslation('common')
  const reduceMotion = useReducedMotion()

  const [cameras, setCameras] = useState<Camera[]>([])
  const [taskConfigs, setTaskConfigs] = useState<Record<string, TaskConfigDto>>({})
  const [algoNames, setAlgoNames] = useState<ReadonlyMap<string, string>>(new Map())
  const [selectedCameraForConfig, setSelectedCameraForConfig] = useState<Camera | null>(null)
  const [isLoading, setIsLoading] = useState(true)

  // 筛选状态：三个维度独立持有，互不干扰；清除筛选一次性复位
  const [query, setQuery] = useState('')
  const [armStatus, setArmStatus] = useState<ArmStatusFilter>('all')
  const [algorithmId, setAlgorithmId] = useState(ALGORITHM_FILTER_ALL)

  // 任务创建与删除模态框状态
  const [isCreateTaskModalOpen, setIsCreateTaskModalOpen] = useState(false)
  const [taskToDelete, setTaskToDelete] = useState<{ cameraId: string; name: string } | null>(null)

  // 取数不碰 loading：挂载时由 useState(true) 承担，刷新时在事件处理器内置位。
  // 用 `.then/.catch/.finally` 链，理由同 CamerasPage（async + try/finally 会被
  // set-state-in-effect 保守判为同步 setState）。
  //
  // 算法清单走 `allSettled` 而非 `all`：算法名仅影响筛选下拉的标签，其取数失败
  // 不得让整个任务列表报错。不传 `pageSize`，对齐 LiveRulesStudio 的既有调用形态；
  // 算法数超出首页返回量时标签回落原始 ID，功能不降级，不为此新增分页循环。
  const loadData = useCallback((): Promise<void> => {
    return Promise.allSettled([cameraApi.list(), taskApi.list(), algorithmApi.list()])
      .then(([camsResult, tasksResult, algosResult]) => {
        if (camsResult.status === 'rejected' || tasksResult.status === 'rejected') {
          throw new Error('task list load failed')
        }

        const cams = camsResult.value
        const tasks = tasksResult.value
        setCameras(cams)

        const configs: Record<string, TaskConfigDto> = {}
        for (const item of tasks) {
          configs[item.cameraId] = {
            cameraId: item.cameraId,
            name: item.name,
            desiredEnabled: item.desiredEnabled,
            algorithmId: item.algorithmId,
            analysisFps: item.analysisFps,
            algoParams: item.algoParams,
            actualStatus: item.actualStatus,
            statusMessage: item.statusMessage,
            algorithmInstances: item.algorithmInstances,
            rules: item.rules || [],
            motionGate: item.motionGate,
            configRevision: item.configRevision,
          }
        }
        setTaskConfigs(configs)

        setAlgoNames(
          algosResult.status === 'fulfilled'
            ? new Map(algosResult.value.items.map((item) => [item.algorithmId, item.name]))
            : new Map(),
        )
      })
      .catch(() => {
        // 优雅降级处理
      })
      .finally(() => {
        setIsLoading(false)
      })
  }, [])

  useEffect(() => {
    void loadData()
  }, [loadData])

  // 联动初始要配置的摄像头 ID：外部传入时打开对应配置。
  //
  // 用渲染期状态调整 + 「已处理标记」而非 effect：effect 会在弹窗打开后再多渲染一轮，
  // 且需额外维护依赖数组。标记保证同一个 id 只联动一次，不会因 cameras/taskConfigs
  // 刷新而反复抢焦点。
  const [handledConfigCameraId, setHandledConfigCameraId] = useState<string | null>(null)
  if (
    initialConfigCameraId &&
    initialConfigCameraId !== handledConfigCameraId &&
    cameras.length > 0
  ) {
    const targetCam = cameras.find((c) => c.cameraId === initialConfigCameraId)
    if (targetCam) {
      setHandledConfigCameraId(initialConfigCameraId)
      if (taskConfigs[targetCam.cameraId]) {
        setSelectedCameraForConfig(targetCam)
      } else {
        setIsCreateTaskModalOpen(true)
      }
    }
  }

  async function handleToggleArm(camera: Camera): Promise<void> {
    const currentCfg = taskConfigs[camera.cameraId]
    const nextDesired = !(currentCfg?.desiredEnabled ?? false)

    try {
      // 状态动词：只提交期望的布防状态，服务端从已持久化配置读取防区、门控与实例参数，
      // 不再有「省略哪些字段才不会被清空」的载荷约定。
      const updated = await taskApi.setEnabled(camera.cameraId, nextDesired)
      setTaskConfigs((prev) => ({ ...prev, [camera.cameraId]: updated }))
    } catch {
      // 开关失败保持列表原样，并重新拉取服务端真实状态避免状态残留
      await loadData()
    }
  }

  function handleTaskCreated(camera: Camera, task: TaskConfigDto): void {
    setTaskConfigs((prev) => ({ ...prev, [task.cameraId]: task }))
    // 创建后直接进入动态矢量标定画板
    setSelectedCameraForConfig(camera)
  }

  function handleTaskDeleted(deletedCameraId: string): void {
    setTaskConfigs((prev) => {
      const copy = { ...prev }
      delete copy[deletedCameraId]
      return copy
    })
  }

  async function handleStreamModeChange(camera: Camera, nextMode: StreamMode): Promise<void> {
    try {
      const updated = await cameraApi.update(camera.cameraId, { streamMode: nextMode })
      setCameras((prev) =>
        prev.map((c) =>
          c.cameraId === camera.cameraId ? { ...c, streamMode: updated.streamMode } : c,
        ),
      )
    } catch {
      // 容错处理
    }
  }

  // 真正绑定了 AI 任务的摄像头通道（页面 KPI 与空态共用）
  const totalArmed = Object.values(taskConfigs).filter((cfg) => cfg.desiredEnabled).length
  // 引用必须稳定：创建向导依赖它推导候选通道，每次渲染新建 Set 会把向导状态重置
  const existingCameraIdsWithTasks = useMemo(() => new Set(Object.keys(taskConfigs)), [taskConfigs])

  /* ── 筛选编排 ──
   *
   * 检索单元按 `cameras` 数组顺序构建，**不重排**：该顺序由 `CameraRepo::list_all`
   * 决定，与 CamerasPage / LivePage 共享同一口径（三页均消费 `cameraApi.list()`），
   * 在任务页单独排序会让三页视觉口径分裂。`filterTasks` 为顺序保持过滤，
   * 因此筛选后卡片的相对先后与筛选前一致，只有被滤除的项消失。
   *
   * 全量口径（KPI 与药丸计数）与命中数在此分离：标题栏的「任务总数 / 已布防」
   * 始终描述整个任务集，只有「筛选命中 N」随筛选变化。`entries.length` 即「任务
   * 总数」，不另建一份摄像头数组 —— 下游只需要它的长度。
   */
  const entries = useMemo(() => {
    const list: TaskFilterEntry[] = []
    for (const camera of cameras) {
      const config = taskConfigs[camera.cameraId]
      if (config) list.push({ camera, config })
    }
    return list
  }, [cameras, taskConfigs])
  const armStatusCounts = useMemo(
    () => countByArmStatus(entries.map((entry) => entry.config)),
    [entries],
  )
  const algorithmOptions = useMemo(
    () =>
      deriveAlgorithmOptions(
        entries.map((entry) => entry.config),
        algoNames,
      ),
    [entries, algoNames],
  )
  const visibleEntries = useMemo(
    () => filterTasks(entries, { query, armStatus, algorithmId }),
    [entries, query, armStatus, algorithmId],
  )

  function clearFilters(): void {
    setQuery('')
    setArmStatus('all')
    setAlgorithmId(ALGORITHM_FILTER_ALL)
  }

  return (
    <AnimatePresence mode="wait">
      {selectedCameraForConfig ? (
        <motion.div
          key="studio-view"
          initial={{ opacity: 0, scale: 0.99 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.99 }}
          transition={{ duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }}
          className="h-full w-full"
        >
          <LiveRulesStudio
            camera={selectedCameraForConfig}
            onBack={() => {
              setSelectedCameraForConfig(null)
              loadData()
            }}
            onNavigateToAlgorithms={onNavigateToAlgorithms}
          />
        </motion.div>
      ) : (
        <motion.div
          key="task-cards-view"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }}
          className="flex h-full min-h-0 flex-col gap-3 text-[var(--text-primary)] select-none"
        >
          {/* 顶部状态与操作栏：现代 SaaS 磨砂中枢 */}
          <PageHeader
            icon={Sliders}
            title={t('title', { defaultValue: 'AI 任务与空间布防' })}
            subtitle={
              <div className="flex items-center gap-3 font-mono text-xs">
                <span className="flex items-center gap-1 text-[var(--text-muted)]">
                  <span>{t('channelCount', { defaultValue: '任务总数' })}:</span>
                  <strong className="font-semibold text-[var(--text-primary)]">
                    {entries.length}
                  </strong>
                </span>
                <span className="text-[var(--border-strong)]">/</span>
                <span className="text-status-success flex items-center gap-1">
                  <span className="bg-status-success h-1.5 w-1.5 animate-pulse rounded-full" />
                  <span>{t('armedCount', { defaultValue: '已布防' })}:</span>
                  <strong className="font-semibold">{totalArmed}</strong>
                </span>
              </div>
            }
            actions={
              <>
                <RefreshButton
                  onClick={loadData}
                  loading={isLoading}
                  label={tc('actions.refresh')}
                />
                <button
                  type="button"
                  onClick={() => setIsCreateTaskModalOpen(true)}
                  className="page-action-btn page-action-btn--primary"
                >
                  <Plus className="h-4 w-4" />
                  <span>{t('createTask', { defaultValue: '新建布防任务' })}</span>
                </button>
              </>
            }
          />

          {/* 检索与筛选工作台：仅在系统存在通道时渲染（对齐 CamerasPage 的 `cameras.length > 0` 门控） */}
          {cameras.length > 0 && (
            <TaskFilterBar
              query={query}
              onQueryChange={setQuery}
              onQueryClear={() => setQuery('')}
              armStatus={armStatus}
              onArmStatusChange={setArmStatus}
              armStatusCounts={armStatusCounts}
              algorithmId={algorithmId}
              onAlgorithmChange={setAlgorithmId}
              algorithmOptions={algorithmOptions}
              onClearFilters={clearFilters}
              matchedCount={visibleEntries.length}
            />
          )}

          {/* AI 任务卡片矩阵 */}
          <div className="min-h-0 flex-1 overflow-auto pt-1">
            {cameras.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-24 text-center text-[var(--text-muted)]">
                <Video className="mb-2 h-8 w-8 opacity-40" />
                <p className="font-medium text-[var(--text-secondary)]">
                  {t('noCamerasAvailable', {
                    defaultValue: '系统中暂无任何摄像头设备，请先接入摄像机',
                  })}
                </p>
                <p className="mt-1 text-xs opacity-75">
                  {t('createTaskDesc', {
                    defaultValue:
                      '为已接入的摄像头通道建立计算任务，配置空间几何规则并开启 NPU 推理。',
                  })}
                </p>
                {onNavigateToCameras && (
                  <button
                    type="button"
                    onClick={onNavigateToCameras}
                    className="page-action-btn page-action-btn--primary mt-4"
                  >
                    <Plus className="h-4 w-4" />
                    <span>{t('goToCameras', { defaultValue: '前往设备管理' })}</span>
                  </button>
                )}
              </div>
            ) : entries.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-24 text-center text-[var(--text-muted)]">
                <ShieldAlert className="mb-2 h-8 w-8 text-[var(--accent)] opacity-60" />
                <p className="font-medium text-[var(--text-secondary)]">
                  {t('emptyTasks', { defaultValue: '暂无运行中的 AI 任务' })}
                </p>
                <p className="mt-1 max-w-sm text-xs opacity-75">
                  {t('emptyTasksHint', {
                    defaultValue:
                      '为接入的摄像头配置算法与空间规则（ROI/绊线/遮罩），实现智能检测与告警。',
                  })}
                </p>
                <button
                  type="button"
                  onClick={() => setIsCreateTaskModalOpen(true)}
                  className="page-action-btn page-action-btn--primary mt-4"
                >
                  <Plus className="h-4 w-4" />
                  <span>{t('createFirstTask', { defaultValue: '创建首个布防任务' })}</span>
                </button>
              </div>
            ) : visibleEntries.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-24 text-center text-[var(--text-muted)]">
                <Search className="mb-2 h-8 w-8 opacity-40" />
                <p className="font-medium text-[var(--text-secondary)]">
                  {t('filter.noMatchTitle')}
                </p>
                <p className="mt-1 max-w-sm text-xs opacity-75">{t('filter.noMatchHint')}</p>
                <button type="button" onClick={clearFilters} className="page-action-btn mt-4">
                  <RotateCcw className="h-4 w-4" />
                  <span>{t('filter.clearAll')}</span>
                </button>
              </div>
            ) : (
              <motion.div
                variants={{
                  hidden: {},
                  visible: {
                    transition: {
                      staggerChildren: reduceMotion ? 0 : 0.05,
                    },
                  },
                }}
                initial="hidden"
                animate="visible"
                className="grid grid-cols-1 gap-5 md:grid-cols-2 lg:grid-cols-3"
              >
                {visibleEntries.map(({ camera, config }) => (
                  <TaskCameraCard
                    key={camera.id}
                    camera={camera}
                    config={config}
                    onToggleArm={() => handleToggleArm(camera)}
                    onConfigure={() => setSelectedCameraForConfig(camera)}
                    onStreamModeChange={(nextMode) => handleStreamModeChange(camera, nextMode)}
                    onDelete={() =>
                      setTaskToDelete({
                        cameraId: camera.cameraId,
                        name: config.name || camera.name || camera.cameraId,
                      })
                    }
                    t={t}
                  />
                ))}
              </motion.div>
            )}
          </div>

          {/* 新建布防任务模态框 */}
          <CreateTaskModal
            isOpen={isCreateTaskModalOpen}
            cameras={cameras}
            existingCameraIdsWithTasks={existingCameraIdsWithTasks}
            preselectedCameraId={initialConfigCameraId}
            onClose={() => setIsCreateTaskModalOpen(false)}
            onSuccess={handleTaskCreated}
            onGoToCameras={onNavigateToCameras}
            onGoToAlgorithms={onNavigateToAlgorithms}
          />

          {/* 删除布防任务确认模态框 */}
          <DeleteTaskModal
            isOpen={Boolean(taskToDelete)}
            cameraId={taskToDelete?.cameraId || null}
            taskName={taskToDelete?.name || null}
            onClose={() => setTaskToDelete(null)}
            onSuccess={handleTaskDeleted}
          />
        </motion.div>
      )}
    </AnimatePresence>
  )
}
