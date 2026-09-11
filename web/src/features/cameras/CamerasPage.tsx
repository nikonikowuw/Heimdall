import React, { useEffect, useMemo, useState } from 'react'
import {
  AlertCircle,
  Check,
  CheckCircle2,
  Copy,
  Pencil,
  Plus,
  Radio,
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  Video,
  X,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import { cameraApi, taskApi } from '../../lib/api'
import type { Camera, TaskSummaryDto } from '../../types'
import { getProbeBadge, normalizeProbeStatus } from './cameraStatus'
import { CameraModal } from './components/CameraModal'
import { DeleteCameraModal } from './components/DeleteCameraModal'
import { copyToClipboard } from '../../lib/utils'

export interface CamerasPageProps {
  onNavigateToTasks?: (camera: Camera) => void
}

export function CamerasPage({ onNavigateToTasks }: CamerasPageProps): React.ReactElement {
  const { t } = useTranslation('camera')
  const { t: tc } = useTranslation('common')

  const [cameras, setCameras] = useState<Camera[]>([])
  const [tasks, setTasks] = useState<TaskSummaryDto[]>([])
  const [isLoading, setIsLoading] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [probingCameraId, setProbingCameraId] = useState<string | null>(null)
  const [copiedCameraId, setCopiedCameraId] = useState<string | null>(null)

  // 模态框状态
  const [isCameraModalOpen, setIsCameraModalOpen] = useState(false)
  const [cameraToEdit, setCameraToEdit] = useState<Camera | null>(null)
  const [cameraToDelete, setCameraToDelete] = useState<Camera | null>(null)
  const [probeFeedback, setProbeFeedback] = useState<Record<string, 'success' | 'failed'>>({})
  const [toast, setToast] = useState<{
    id: number
    type: 'success' | 'error'
    title: string
    message: string
  } | null>(null)

  const showToast = (type: 'success' | 'error', title: string, message: string) => {
    const id = Date.now()
    setToast({ id, type, title, message })
    setTimeout(() => {
      setToast((curr) => (curr?.id === id ? null : curr))
    }, 4000)
  }

  const loadData = async (): Promise<void> => {
    setIsLoading(true)
    try {
      const [cams, taskList] = await Promise.all([cameraApi.list(), taskApi.list()])
      setCameras(cams)
      setTasks(taskList)
    } catch {
      // 优雅降级
    } finally {
      setIsLoading(false)
    }
  }

  useEffect(() => {
    loadData()
  }, [])

  const handleManualProbe = async (camera: Camera) => {
    if (probingCameraId) return
    setProbingCameraId(camera.cameraId)
    try {
      const res = await cameraApi.probe(camera.cameraId)
      setCameras((prev) =>
        prev.map((c) =>
          c.cameraId === camera.cameraId
            ? {
                ...c,
                lastProbeStatus: 'healthy',
                lastCodec: res.codec || c.lastCodec,
                lastWidth: res.width || c.lastWidth,
                lastHeight: res.height || c.lastHeight,
                lastFps: res.fps || c.lastFps,
              }
            : c,
        ),
      )
      setProbeFeedback((prev) => ({ ...prev, [camera.cameraId]: 'success' }))
      setTimeout(() => {
        setProbeFeedback((prev) => {
          const next = { ...prev }
          delete next[camera.cameraId]
          return next
        })
      }, 3000)

      const details = [
        res.codec ? res.codec.toUpperCase() : null,
        res.width && res.height ? `${res.width}x${res.height}` : null,
        res.fps && res.fps > 0 ? `${res.fps} FPS` : null,
      ]
        .filter(Boolean)
        .join(' · ')

      showToast(
        'success',
        t('manage.probeSuccess', { defaultValue: '探活成功' }),
        details
          ? `${camera.name} (${details})`
          : `${camera.name} ${t('status.healthy', { defaultValue: '正常在线' })}`,
      )
    } catch {
      setCameras((prev) =>
        prev.map((c) => (c.cameraId === camera.cameraId ? { ...c, lastProbeStatus: 'failed' } : c)),
      )
      setProbeFeedback((prev) => ({ ...prev, [camera.cameraId]: 'failed' }))
      setTimeout(() => {
        setProbeFeedback((prev) => {
          const next = { ...prev }
          delete next[camera.cameraId]
          return next
        })
      }, 3000)

      showToast(
        'error',
        t('manage.probeFailed', { defaultValue: '探活失败' }),
        `${camera.name}: ${t('manage.probeFailedDesc', { defaultValue: 'RTSP 连接超时或鉴权失败，请检查网络与流地址' })}`,
      )
    } finally {
      setProbingCameraId(null)
    }
  }

  const handleCopyRtsp = async (cameraId: string, url: string) => {
    if (!url) return
    const success = await copyToClipboard(url)
    if (success) {
      setCopiedCameraId(cameraId)
      setTimeout(() => setCopiedCameraId(null), 2000)
    }
  }

  const handleCameraSaved = (saved: Camera) => {
    setCameras((prev) => {
      const idx = prev.findIndex((c) => c.cameraId === saved.cameraId)
      if (idx >= 0) {
        const next = [...prev]
        next[idx] = saved
        return next
      }
      return [...prev, saved]
    })
    loadData()
  }

  const handleCameraDeleted = (deletedCameraId: string) => {
    setCameras((prev) => prev.filter((c) => c.cameraId !== deletedCameraId))
    setTasks((prev) => prev.filter((t) => t.cameraId !== deletedCameraId))
  }

  // 统计数据
  const totalCount = cameras.length
  const onlineCount = useMemo(
    () => cameras.filter((c) => normalizeProbeStatus(c.lastProbeStatus) === 'healthy').length,
    [cameras],
  )
  const degradedCount = useMemo(
    () => cameras.filter((c) => normalizeProbeStatus(c.lastProbeStatus) === 'degraded').length,
    [cameras],
  )
  const offlineCount = useMemo(
    () => cameras.filter((c) => normalizeProbeStatus(c.lastProbeStatus) === 'offline').length,
    [cameras],
  )

  // 筛选过滤
  const filteredCameras = useMemo(() => {
    const q = searchQuery.toLowerCase().trim()
    return cameras.filter((cam) => {
      const matchesSearch =
        !q ||
        cam.name?.toLowerCase().includes(q) ||
        cam.cameraId?.toLowerCase().includes(q) ||
        cam.rtspUrl?.toLowerCase().includes(q)

      if (!matchesSearch) return false

      if (statusFilter !== 'all') {
        return normalizeProbeStatus(cam.lastProbeStatus) === statusFilter
      }
      return true
    })
  }, [cameras, searchQuery, statusFilter])

  // 检查摄像头是否已绑定任务
  const taskMap = useMemo(() => {
    const map = new Map<string, TaskSummaryDto>()
    for (const item of tasks) {
      map.set(item.cameraId, item)
    }
    return map
  }, [tasks])

  return (
    <div className="flex h-full flex-col gap-4 bg-[var(--bg-primary)] p-4 text-[var(--text-primary)] select-none">
      {/* 顶部操作工具栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-2xl p-3.5 shadow-xs">
        <div className="flex items-center gap-3.5">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs">
            <Video className="h-5 w-5" />
          </div>
          <div>
            <h2 className="text-base font-bold text-[var(--text-primary)]">
              {t('manage.title', { defaultValue: '视频流设备管理' })}
            </h2>
            <div className="mt-0.5 flex items-center gap-3 font-mono text-xs">
              <span className="flex items-center gap-1 text-[var(--text-muted)]">
                <span>{t('manage.totalDevices', { defaultValue: '设备总数' })}:</span>
                <strong className="font-semibold text-[var(--text-primary)]">{totalCount}</strong>
              </span>
              <span className="text-[var(--border-strong)]">/</span>
              <span className="flex items-center gap-1 text-emerald-500">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />
                <span>{t('manage.onlineDevices', { defaultValue: '在线' })}:</span>
                <strong className="font-semibold">{onlineCount}</strong>
              </span>
              {degradedCount > 0 && (
                <>
                  <span className="text-[var(--border-strong)]">/</span>
                  <span className="flex items-center gap-1 text-amber-500">
                    <span>{t('manage.degradedDevices', { defaultValue: '抖动' })}:</span>
                    <strong className="font-semibold">{degradedCount}</strong>
                  </span>
                </>
              )}
              {offlineCount > 0 && (
                <>
                  <span className="text-[var(--border-strong)]">/</span>
                  <span className="flex items-center gap-1 text-rose-500">
                    <span>{t('manage.offlineDevices', { defaultValue: '离线' })}:</span>
                    <strong className="font-semibold">{offlineCount}</strong>
                  </span>
                </>
              )}
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
            onClick={() => {
              setCameraToEdit(null)
              setIsCameraModalOpen(true)
            }}
            className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-3.5 py-1.5 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t('manage.addCamera', { defaultValue: '接入设备' })}</span>
          </button>
        </div>
      </div>

      {/* 搜索与过滤筛选栏 */}
      {cameras.length > 0 && (
        <div className="frosted-glass flex items-center justify-between gap-3 rounded-xl px-4 py-2.5 shadow-xs">
          <div className="flex flex-1 items-center gap-2 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs">
            <Search className="h-3.5 w-3.5 text-[var(--text-muted)]" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t('manage.searchPlaceholder', {
                defaultValue: '按设备名称、ID 或 RTSP 地址搜索...',
              })}
              className="w-full bg-transparent text-[var(--text-primary)] outline-none placeholder:text-[var(--text-muted)]"
            />
          </div>

          <div className="flex items-center gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-xs">
            <button
              type="button"
              onClick={() => setStatusFilter('all')}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                statusFilter === 'all'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              {t('manage.filterAll', { defaultValue: '全部状态' })} ({totalCount})
            </button>
            <button
              type="button"
              onClick={() => setStatusFilter('online')}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                statusFilter === 'online'
                  ? 'bg-emerald-500 text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-emerald-500'
              }`}
            >
              {t('manage.onlineDevices', { defaultValue: '在线' })} ({onlineCount})
            </button>
            <button
              type="button"
              onClick={() => setStatusFilter('offline')}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                statusFilter === 'offline'
                  ? 'bg-rose-500 text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-rose-500'
              }`}
            >
              {t('manage.offlineDevices', { defaultValue: '离线' })} ({offlineCount})
            </button>
          </div>
        </div>
      )}

      {/* 设备网格列表 */}
      <div className="frosted-glass flex-1 overflow-auto rounded-2xl p-4 shadow-xs">
        {cameras.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-24 text-center text-[var(--text-muted)]">
            <Video className="mb-3 h-10 w-10 opacity-40" />
            <h3 className="text-base font-bold text-[var(--text-secondary)]">
              {t('manage.emptyTitle', { defaultValue: '暂无接入的网络摄像头' })}
            </h3>
            <p className="mt-1 max-w-sm text-xs leading-relaxed opacity-75">
              {t('manage.emptyDesc', {
                defaultValue: '录入标准 RTSP 视频流，系统将自动发起异步握手探活与 SPS 解析。',
              })}
            </p>
            <button
              type="button"
              onClick={() => {
                setCameraToEdit(null)
                setIsCameraModalOpen(true)
              }}
              className="mt-5 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95"
            >
              <Plus className="h-4 w-4" />
              <span>{t('manage.addFirstCamera', { defaultValue: '接入首路摄像头' })}</span>
            </button>
          </div>
        ) : filteredCameras.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-20 text-center text-[var(--text-muted)]">
            <Search className="mb-2 h-8 w-8 opacity-40" />
            <p className="text-sm font-medium text-[var(--text-secondary)]">
              {t('manage.noMatchingCameras', { defaultValue: '未找到匹配的摄像头设备' })}
            </p>
            <p className="mt-1 text-xs opacity-75">
              {t('manage.clearFilterHint', {
                defaultValue: '尝试清除搜索词或更改状态过滤条件',
              })}
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
            <AnimatePresence mode="popLayout">
              {filteredCameras.map((camera) => {
                const probeBadge = getProbeBadge(camera.lastProbeStatus, t)
                const boundTask = taskMap.get(camera.cameraId)
                const isProbing = probingCameraId === camera.cameraId
                const isCopied = copiedCameraId === camera.cameraId

                return (
                  <motion.div
                    key={camera.id}
                    layout
                    initial={{ opacity: 0, scale: 0.98 }}
                    animate={{ opacity: 1, scale: 1 }}
                    exit={{ opacity: 0, scale: 0.98 }}
                    transition={{
                      duration: motionTokens.duration.fast,
                      ease: motionTokens.easing.smooth,
                    }}
                    className="flex flex-col justify-between rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] p-4 shadow-xs transition-all hover:border-[var(--accent)]/40 hover:shadow-md"
                  >
                    <div>
                      {/* 卡片头部：状态徽标与标识 */}
                      <div className="flex items-start justify-between gap-2">
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <span
                              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 font-mono text-xs font-semibold ${probeBadge.badgeBg}`}
                            >
                              <span className={`h-1.5 w-1.5 rounded-full ${probeBadge.dotClass}`} />
                              <span>{probeBadge.text}</span>
                            </span>
                            <span className="rounded bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-muted)]">
                              {camera.cameraId}
                            </span>
                          </div>
                          <h3
                            className="mt-1.5 truncate text-base font-bold text-[var(--text-primary)]"
                            title={camera.name || camera.cameraId}
                          >
                            {camera.name || camera.cameraId}
                          </h3>
                        </div>

                        {/* 快捷操作按钮 */}
                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            onClick={() => {
                              setCameraToEdit(camera)
                              setIsCameraModalOpen(true)
                            }}
                            title={tc('actions.edit')}
                            className="rounded-lg p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                          >
                            <Pencil className="h-3.5 w-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={() => setCameraToDelete(camera)}
                            title={tc('actions.delete')}
                            className="rounded-lg p-1.5 text-[var(--text-muted)] transition-colors hover:bg-rose-500/10 hover:text-rose-500"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </div>

                      {/* 规格参数胶囊 */}
                      <div className="mt-3 flex items-center gap-2 font-mono text-xs">
                        <span className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 font-bold text-[var(--accent)]">
                          {camera.lastCodec?.toUpperCase() || 'H.264'}
                        </span>
                        <span className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 text-[var(--text-secondary)]">
                          {camera.lastWidth && camera.lastHeight
                            ? `${camera.lastWidth}x${camera.lastHeight}`
                            : '1080P'}
                        </span>
                        <span className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 text-emerald-500">
                          {camera.lastFps ? camera.lastFps.toFixed(1) : '25.0'} FPS
                        </span>
                      </div>

                      {/* RTSP 串流地址与复制 */}
                      <div className="mt-3 space-y-1.5">
                        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5">
                          <div className="flex items-center justify-between text-[11px]">
                            <span className="font-semibold text-[var(--text-muted)]">
                              {t('manage.mainStreamLabel', { defaultValue: '主码流' })}
                            </span>
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation()
                                void handleCopyRtsp(camera.cameraId, camera.rtspUrl)
                              }}
                              className="flex items-center gap-1 text-[11px] text-[var(--text-secondary)] hover:text-[var(--accent)]"
                            >
                              {isCopied ? (
                                <>
                                  <Check className="h-3 w-3 text-emerald-500" />
                                  <span className="text-emerald-500">
                                    {t('manage.copied', { defaultValue: '已复制' })}
                                  </span>
                                </>
                              ) : (
                                <>
                                  <Copy className="h-3 w-3" />
                                  <span>{t('manage.copyUrl', { defaultValue: '复制' })}</span>
                                </>
                              )}
                            </button>
                          </div>
                          <p
                            className="mt-1 truncate font-mono text-xs text-[var(--text-secondary)]"
                            title={camera.rtspUrl}
                          >
                            {camera.rtspUrl}
                          </p>
                        </div>
                      </div>

                      {/* AI 任务关联状态 */}
                      <div className="mt-3 flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-2 text-xs">
                        <div className="flex items-center gap-2">
                          {boundTask ? (
                            <>
                              <ShieldCheck className="h-4 w-4 text-emerald-500" />
                              <div className="flex flex-col">
                                <span className="font-semibold text-[var(--text-primary)]">
                                  {t('manage.aiTaskBound', { defaultValue: '已配置布防任务' })}
                                </span>
                                <span className="font-mono text-[10px] text-[var(--text-muted)]">
                                  {t('manage.rulesSummary', {
                                    count: boundTask.rulesCount,
                                    defaultValue: `${boundTask.rulesCount} 项空间几何规则`,
                                  })}{' '}
                                  ·{' '}
                                  {boundTask.desiredEnabled
                                    ? t('manage.armedStatus', { defaultValue: '布防中' })
                                    : t('manage.disarmedStatus', { defaultValue: '未布防' })}
                                </span>
                              </div>
                            </>
                          ) : (
                            <>
                              <ShieldAlert className="h-4 w-4 text-slate-400" />
                              <span className="text-[var(--text-muted)]">
                                {t('manage.aiTaskUnbound', { defaultValue: '未分配 AI 任务' })}
                              </span>
                            </>
                          )}
                        </div>

                        {onNavigateToTasks && (
                          <button
                            type="button"
                            onClick={() => onNavigateToTasks(camera)}
                            className="text-xs font-semibold text-[var(--accent)] hover:underline"
                          >
                            {boundTask
                              ? t('manage.goToTask', { defaultValue: '前往布防' })
                              : t('manage.createTask', { defaultValue: '创建布防' })}
                          </button>
                        )}
                      </div>
                    </div>

                    {/* 卡片底栏：单次探活与详情 */}
                    <div className="mt-4 flex items-center justify-between border-t border-[var(--border)] pt-3 text-xs">
                      <span className="font-mono text-[11px] text-[var(--text-muted)]">
                        {camera.remark || camera.protocol?.toUpperCase() || 'RTSP'}
                      </span>
                      {(() => {
                        const feedback = probeFeedback[camera.cameraId]
                        return (
                          <button
                            type="button"
                            onClick={() => handleManualProbe(camera)}
                            disabled={isProbing}
                            className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-medium transition-all ${
                              feedback === 'success'
                                ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400'
                                : feedback === 'failed'
                                  ? 'border-rose-500/30 bg-rose-500/10 text-rose-400'
                                  : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]'
                            } disabled:opacity-50`}
                          >
                            {isProbing ? (
                              <RefreshCw className="h-3 w-3 animate-spin text-[var(--accent)]" />
                            ) : feedback === 'success' ? (
                              <Check className="h-3 w-3 text-emerald-400" />
                            ) : feedback === 'failed' ? (
                              <AlertCircle className="h-3 w-3 text-rose-400" />
                            ) : (
                              <Radio className="h-3 w-3" />
                            )}
                            <span>
                              {isProbing
                                ? t('manage.probing', { defaultValue: '探活中...' })
                                : feedback === 'success'
                                  ? t('manage.probeSuccess', { defaultValue: '探活成功' })
                                  : feedback === 'failed'
                                    ? t('manage.probeFailed', { defaultValue: '探活失败' })
                                    : t('manage.probeAction', { defaultValue: '探活' })}
                            </span>
                          </button>
                        )
                      })()}
                    </div>
                  </motion.div>
                )
              })}
            </AnimatePresence>
          </div>
        )}
      </div>

      {/* 摄像头添加/编辑模态框 */}
      <CameraModal
        isOpen={isCameraModalOpen}
        camera={cameraToEdit}
        onClose={() => {
          setIsCameraModalOpen(false)
          setCameraToEdit(null)
        }}
        onSuccess={handleCameraSaved}
      />

      {/* 摄像头删除确认模态框 */}
      <DeleteCameraModal
        isOpen={Boolean(cameraToDelete)}
        camera={cameraToDelete}
        onClose={() => setCameraToDelete(null)}
        onSuccess={handleCameraDeleted}
      />

      {/* 探活结果悬浮 Toast 提示 (顶部居中 + 100% 纯色不透光，杜绝右上角遮挡按钮与底色穿透) */}
      <AnimatePresence>
        {toast && (
          <motion.div
            key={toast.id}
            initial={{ opacity: 0, y: -20, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -20, scale: 0.96 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
            className="fixed top-6 left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-2xl border border-zinc-700 bg-zinc-900 px-4 py-3 text-zinc-100 shadow-2xl ring-1 ring-white/10 dark:border-zinc-700 dark:bg-zinc-900"
          >
            {toast.type === 'success' ? (
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-xl bg-emerald-500/20 text-emerald-400">
                <CheckCircle2 className="h-4 w-4" />
              </div>
            ) : (
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-xl bg-rose-500/20 text-rose-400">
                <AlertCircle className="h-4 w-4" />
              </div>
            )}
            <div className="flex flex-col">
              <span className="text-xs font-bold tracking-tight text-white">{toast.title}</span>
              <span className="font-mono text-xs text-zinc-400">{toast.message}</span>
            </div>
            <button
              onClick={() => setToast(null)}
              className="ml-2 rounded-lg p-1 text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-white"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
