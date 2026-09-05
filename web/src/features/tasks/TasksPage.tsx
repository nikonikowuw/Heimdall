import React, { useCallback, useEffect, useState } from 'react'
import { Plus, RefreshCw, ShieldAlert, Sliders, Video } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import { cameraApi, taskApi } from '../../lib/api'
import type { Camera, TaskConfigDto } from '../../types'
import { CreateTaskModal } from './components/CreateTaskModal'
import { DeleteTaskModal } from './components/DeleteTaskModal'
import { LiveRulesStudio } from './components/LiveRulesStudio'
import { TaskCameraCard } from './components/TaskCameraCard'

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

  const [cameras, setCameras] = useState<Camera[]>([])
  const [taskConfigs, setTaskConfigs] = useState<Record<string, TaskConfigDto>>({})
  const [selectedCameraForConfig, setSelectedCameraForConfig] = useState<Camera | null>(null)
  const [isLoading, setIsLoading] = useState(false)

  // 任务创建与删除模态框状态
  const [isCreateTaskModalOpen, setIsCreateTaskModalOpen] = useState(false)
  const [taskToDelete, setTaskToDelete] = useState<{ cameraId: string; name: string } | null>(null)

  const loadData = useCallback(async (): Promise<void> => {
    setIsLoading(true)
    try {
      const [cams, tasks] = await Promise.all([cameraApi.list(), taskApi.list()])
      setCameras(cams)

      const configs: Record<string, TaskConfigDto> = {}
      for (const item of tasks) {
        configs[item.cameraId] = {
          cameraId: item.cameraId,
          name: item.name,
          desiredEnabled: item.desiredEnabled,
          rules: item.rules || [],
          motionGate: item.motionGate,
        }
      }
      setTaskConfigs(configs)
    } catch {
      // 优雅降级处理
    } finally {
      setIsLoading(false)
    }
  }, [])

  useEffect(() => {
    loadData()
  }, [loadData])

  // 联动初始要配置的摄像头 ID
  useEffect(() => {
    if (!initialConfigCameraId || cameras.length === 0) return
    const targetCam = cameras.find((c) => c.cameraId === initialConfigCameraId)
    if (targetCam) {
      if (taskConfigs[targetCam.cameraId]) {
        setSelectedCameraForConfig(targetCam)
      } else {
        setIsCreateTaskModalOpen(true)
      }
    }
  }, [initialConfigCameraId, cameras, taskConfigs])

  const handleToggleArm = async (camera: Camera): Promise<void> => {
    const currentCfg = taskConfigs[camera.cameraId]
    const nextDesired = !(currentCfg?.desiredEnabled ?? false)

    try {
      const payload: TaskConfigDto = {
        cameraId: camera.cameraId,
        name: currentCfg?.name || camera.name || `Task-${camera.cameraId}`,
        desiredEnabled: nextDesired,
        rules: currentCfg?.rules || [],
        motionGate: currentCfg?.motionGate || { enabled: true },
      }
      const updated = await taskApi.updateTask(camera.cameraId, payload)
      setTaskConfigs((prev) => ({ ...prev, [camera.cameraId]: updated }))
    } catch {
      // ignore
    }
  }

  const handleTaskCreated = (camera: Camera, task: TaskConfigDto) => {
    setTaskConfigs((prev) => ({ ...prev, [task.cameraId]: task }))
    // 创建后直接进入动态矢量标定画板
    setSelectedCameraForConfig(camera)
  }

  const handleTaskDeleted = (deletedCameraId: string) => {
    setTaskConfigs((prev) => {
      const copy = { ...prev }
      delete copy[deletedCameraId]
      return copy
    })
  }

  // 真正绑定了 AI 任务的摄像头通道
  const camerasWithTasks = cameras.filter((c) => taskConfigs[c.cameraId] !== undefined)
  const totalArmed = Object.values(taskConfigs).filter((cfg) => cfg.desiredEnabled).length
  const existingCameraIdsWithTasks = new Set(Object.keys(taskConfigs))

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
          className="flex h-full flex-col gap-4 bg-[var(--bg-primary)] p-4 text-[var(--text-primary)] select-none"
        >
          {/* 顶部状态与操作栏 */}
          <div className="frosted-glass flex items-center justify-between rounded-2xl p-3.5 shadow-xs">
            <div className="flex items-center gap-3.5">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs">
                <Sliders className="h-5 w-5" />
              </div>
              <div>
                <h2 className="text-base font-bold text-[var(--text-primary)]">
                  {t('title', { defaultValue: 'AI 任务与空间布防' })}
                </h2>
                <div className="mt-0.5 flex items-center gap-3 text-xs">
                  <span className="flex items-center gap-1 font-mono text-[var(--text-muted)]">
                    <span>{t('channelCount', { defaultValue: '任务总数' })}:</span>
                    <strong className="font-semibold text-[var(--text-primary)]">
                      {camerasWithTasks.length}
                    </strong>
                  </span>
                  <span className="text-[var(--border-strong)]">/</span>
                  <span className="flex items-center gap-1 font-mono text-emerald-500">
                    <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />
                    <span>{t('armedCount', { defaultValue: '已布防' })}:</span>
                    <strong className="font-semibold">{totalArmed}</strong>
                  </span>
                </div>
              </div>
            </div>

            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={loadData}
                disabled={isLoading}
                className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-50"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
                <span>{tc('actions.refresh')}</span>
              </button>
              <button
                type="button"
                onClick={() => setIsCreateTaskModalOpen(true)}
                className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-3.5 py-1.5 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95"
              >
                <Plus className="h-3.5 w-3.5" />
                <span>{t('createTask', { defaultValue: '新建布防任务' })}</span>
              </button>
            </div>
          </div>

          {/* AI 任务卡片矩阵 */}
          <div className="frosted-glass flex-1 overflow-auto rounded-2xl p-4 shadow-xs">
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
                    className="mt-4 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95"
                  >
                    <Plus className="h-4 w-4" />
                    <span>{t('goToCameras', { defaultValue: '前往设备管理' })}</span>
                  </button>
                )}
              </div>
            ) : camerasWithTasks.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-24 text-center text-[var(--text-muted)]">
                <ShieldAlert className="mb-2 h-8 w-8 text-cyan-500 opacity-60" />
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
                  className="mt-4 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95"
                >
                  <Plus className="h-4 w-4" />
                  <span>{t('createFirstTask', { defaultValue: '创建首个布防任务' })}</span>
                </button>
              </div>
            ) : (
              <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
                {camerasWithTasks.map((camera) => (
                  <TaskCameraCard
                    key={camera.id}
                    camera={camera}
                    config={taskConfigs[camera.cameraId]}
                    onToggleArm={() => handleToggleArm(camera)}
                    onConfigure={() => setSelectedCameraForConfig(camera)}
                    onDelete={() =>
                      setTaskToDelete({
                        cameraId: camera.cameraId,
                        name: taskConfigs[camera.cameraId]?.name || camera.name || camera.cameraId,
                      })
                    }
                    t={t}
                  />
                ))}
              </div>
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
            onGoToCameras={() => {
              if (onNavigateToCameras) {
                onNavigateToCameras()
              }
            }}
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
