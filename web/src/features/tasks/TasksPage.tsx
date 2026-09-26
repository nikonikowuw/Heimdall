import React, { useCallback, useEffect, useMemo, useState } from 'react'
import { Plus, ShieldAlert, Sliders, Video } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { RefreshButton } from '@/components/RefreshButton'
import { motionTokens } from '@/lib/motionTokens'
import { cameraApi, taskApi } from '@/lib/api'
import type { Camera, TaskConfigDto, StreamMode } from '@/types'
import { CreateTaskModal } from './components/CreateTaskModal'
import { DeleteTaskModal } from './components/DeleteTaskModal'
import { LiveRulesStudio } from './components/LiveRulesStudio'
import { TaskCameraCard } from './components/TaskCameraCard'
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
  const [selectedCameraForConfig, setSelectedCameraForConfig] = useState<Camera | null>(null)
  const [isLoading, setIsLoading] = useState(true)

  // 任务创建与删除模态框状态
  const [isCreateTaskModalOpen, setIsCreateTaskModalOpen] = useState(false)
  const [taskToDelete, setTaskToDelete] = useState<{ cameraId: string; name: string } | null>(null)

  // 取数不碰 loading：挂载时由 useState(true) 承担，刷新时在事件处理器内置位。
  // 用 `.then/.catch/.finally` 链，理由同 CamerasPage（async + try/finally 会被
  // set-state-in-effect 保守判为同步 setState）。
  const loadData = useCallback((): Promise<void> => {
    return Promise.all([cameraApi.list(), taskApi.list()])
      .then(([cams, tasks]) => {
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

  // 真正绑定了 AI 任务的摄像头通道
  const camerasWithTasks = cameras.filter((c) => taskConfigs[c.cameraId] !== undefined)
  const totalArmed = Object.values(taskConfigs).filter((cfg) => cfg.desiredEnabled).length
  // 引用必须稳定：创建向导依赖它推导候选通道，每次渲染新建 Set 会把向导状态重置
  const existingCameraIdsWithTasks = useMemo(() => new Set(Object.keys(taskConfigs)), [taskConfigs])

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
                    {camerasWithTasks.length}
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
            ) : camerasWithTasks.length === 0 ? (
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
                {camerasWithTasks.map((camera) => (
                  <TaskCameraCard
                    key={camera.id}
                    camera={camera}
                    config={taskConfigs[camera.cameraId]}
                    onToggleArm={() => handleToggleArm(camera)}
                    onConfigure={() => setSelectedCameraForConfig(camera)}
                    onStreamModeChange={(nextMode) => handleStreamModeChange(camera, nextMode)}
                    onDelete={() =>
                      setTaskToDelete({
                        cameraId: camera.cameraId,
                        name: taskConfigs[camera.cameraId]?.name || camera.name || camera.cameraId,
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
