import React, { Fragment, useEffect, useMemo, useState } from 'react'
import { Plus, Radio, Search, Video, X } from 'lucide-react'
import { AnimatePresence } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { RefreshButton } from '@/components/RefreshButton'
import { PageHeader } from '@/components/ui/PageHeader'
import { SearchInput } from '@/components/ui/SearchInput'
import { cameraApi, gb28181Api, taskApi } from '@/lib/api'
import { cn } from '@/lib/utils'
import { toast } from '@/stores/toast'
import type { Camera, Gb28181Device, TaskSummaryDto } from '@/types'
import { normalizeProbeStatus } from './cameraStatus'
import { CameraModal } from './components/CameraModal'
import { CameraDeviceTile } from './components/CameraDeviceTile'
import { CameraDetailDrawer } from './components/CameraDetailDrawer'
import { DeleteCameraModal } from './components/DeleteCameraModal'
import { BatchImportGbModal } from './components/BatchImportGbModal'
import type { CameraModelType } from './components/illustrations/types'

export type CamerasPageProps = Record<string, never>

export function CamerasPage(): React.ReactElement {
  const { t } = useTranslation('camera')
  const { t: tc } = useTranslation('common')

  const [cameras, setCameras] = useState<Camera[]>([])
  const [tasks, setTasks] = useState<TaskSummaryDto[]>([])
  const [isLoading, setIsLoading] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [protocolFilter, setProtocolFilter] = useState<'all' | 'rtsp' | 'gb28181'>('all')
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(12)
  const [gbDevices, setGbDevices] = useState<Gb28181Device[]>([])
  const [isBatchImportOpen, setIsBatchImportOpen] = useState(false)
  const [bannerDismissed, setBannerDismissed] = useState(false)
  const [modelTypeOverrides, setModelTypeOverrides] = useState<Record<string, CameraModelType>>({})
  const [probingCameraId, setProbingCameraId] = useState<string | null>(null)
  const [selectedCameraForDetail, setSelectedCameraForDetail] = useState<Camera | null>(null)

  // 模态框状态
  const [isCameraModalOpen, setIsCameraModalOpen] = useState(false)
  const [cameraToEdit, setCameraToEdit] = useState<Camera | null>(null)
  const [cameraToDelete, setCameraToDelete] = useState<Camera | null>(null)
  const [probeFeedback, setProbeFeedback] = useState<Record<string, 'success' | 'failed'>>({})

  const loadData = async (): Promise<void> => {
    setIsLoading(true)
    try {
      const [cams, devs, taskList] = await Promise.all([
        cameraApi.list(),
        gb28181Api.listDevices().catch(() => [] as Gb28181Device[]),
        taskApi.list().catch(() => [] as TaskSummaryDto[]),
      ])
      setCameras(cams)
      setGbDevices(devs)
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
                lastProbeAt: Date.now(),
              }
            : c,
        ),
      )
      setSelectedCameraForDetail((curr) =>
        curr?.cameraId === camera.cameraId
          ? {
              ...curr,
              lastProbeStatus: 'healthy',
              lastCodec: res.codec || curr.lastCodec,
              lastWidth: res.width || curr.lastWidth,
              lastHeight: res.height || curr.lastHeight,
              lastFps: res.fps || curr.lastFps,
              lastProbeAt: Date.now(),
            }
          : curr,
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

      toast.success(
        details
          ? `${camera.name} (${details})`
          : `${camera.name} ${t('status.online', { defaultValue: '在线' })}`,
        {
          title: t('manage.probeSuccess', { defaultValue: '探活成功' }),
          category: t('manage.probeAction', { defaultValue: '探活' }),
        },
      )
    } catch {
      setCameras((prev) =>
        prev.map((c) => (c.cameraId === camera.cameraId ? { ...c, lastProbeStatus: 'failed' } : c)),
      )
      setSelectedCameraForDetail((curr) =>
        curr?.cameraId === camera.cameraId ? { ...curr, lastProbeStatus: 'failed' } : curr,
      )
      setProbeFeedback((prev) => ({ ...prev, [camera.cameraId]: 'failed' }))
      setTimeout(() => {
        setProbeFeedback((prev) => {
          const next = { ...prev }
          delete next[camera.cameraId]
          return next
        })
      }, 3000)

      toast.error(
        `${camera.name}: ${t('manage.probeFailedDesc', { defaultValue: 'RTSP 连接超时或鉴权失败，请检查网络与流地址' })}`,
        {
          title: t('manage.probeFailed', { defaultValue: '探活失败' }),
          category: t('manage.probeAction', { defaultValue: '探活' }),
        },
      )
    } finally {
      setProbingCameraId(null)
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
    setSelectedCameraForDetail((curr) =>
      curr?.cameraId === saved.cameraId ? { ...curr, ...saved } : curr,
    )
    loadData()
  }

  const handleCameraDeleted = (deletedCameraId: string) => {
    setCameras((prev) => prev.filter((c) => c.cameraId !== deletedCameraId))
    setTasks((prev) => prev.filter((task) => task.cameraId !== deletedCameraId))
    if (selectedCameraForDetail?.cameraId === deletedCameraId) {
      setSelectedCameraForDetail(null)
    }
  }

  const taskMap = useMemo(() => {
    const map = new Map<string, TaskSummaryDto>()
    for (const task of tasks) map.set(task.cameraId, task)
    return map
  }, [tasks])

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

  // 标题栏遥测行：总数始终展示，其余状态仅在非零时出现（顺序固定，避免数字跳动）
  const statusMetrics = [
    {
      key: 'total',
      label: t('manage.totalDevices', { defaultValue: '设备总数' }),
      value: totalCount,
      dotClass: null,
      labelClass: 'text-[var(--text-muted)]',
      valueClass: 'text-[var(--text-primary)]',
    },
    {
      key: 'online',
      label: t('manage.onlineDevices', { defaultValue: '在线' }),
      value: onlineCount,
      dotClass: 'bg-status-success animate-pulse',
      labelClass: 'text-status-success',
      valueClass: 'text-status-success',
    },
    {
      key: 'degraded',
      label: t('manage.degradedDevices', { defaultValue: '抖动' }),
      value: degradedCount,
      dotClass: null,
      labelClass: 'text-status-warning',
      valueClass: 'text-status-warning',
    },
    {
      key: 'offline',
      label: t('manage.offlineDevices', { defaultValue: '离线' }),
      value: offlineCount,
      dotClass: null,
      labelClass: 'text-[var(--status-danger)]',
      valueClass: 'text-[var(--status-danger)]',
    },
  ].filter((metric) => metric.key === 'total' || metric.value > 0)

  // 筛选过滤
  const filteredCameras = useMemo(() => {
    const q = searchQuery.toLowerCase().trim()
    return cameras.filter((cam) => {
      if (protocolFilter !== 'all' && cam.protocol !== protocolFilter) {
        return false
      }

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
  }, [cameras, searchQuery, statusFilter, protocolFilter])

  // 分页切片计算
  const totalPages = Math.max(1, Math.ceil(filteredCameras.length / pageSize))

  // 安全页码截断（防止删除最后一条记录后停留在空白越界页）
  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages)
    }
  }, [page, totalPages])

  const paginatedCameras = useMemo(() => {
    if (pageSize >= 999) return filteredCameras
    const start = (page - 1) * pageSize
    return filteredCameras.slice(start, start + pageSize)
  }, [filteredCameras, page, pageSize])

  // 聚合统计国标设备未纳管通道与全量纳管指标
  const { unmanagedChannels, totalChannelsCount, importedChannelsCount } = useMemo(() => {
    const list: { device: Gb28181Device; channelId: string; name: string }[] = []
    let total = 0
    let imported = 0
    for (const dev of gbDevices) {
      for (const ch of dev.channels || []) {
        total++
        if (ch.isImported) {
          imported++
        } else {
          list.push({ device: dev, channelId: ch.channelId, name: ch.name })
        }
      }
    }
    return { unmanagedChannels: list, totalChannelsCount: total, importedChannelsCount: imported }
  }, [gbDevices])

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 text-[var(--text-primary)] select-none">
      {/* 顶部操作工具栏 */}
      <PageHeader
        icon={Video}
        title={t('manage.title', { defaultValue: '视频流设备管理' })}
        subtitle={
          <div className="flex items-center gap-3 font-mono text-xs">
            {statusMetrics.map((metric, index) => (
              <Fragment key={metric.key}>
                {index > 0 && <span className="text-[var(--border-strong)]">/</span>}
                <span className={cn('flex items-center gap-1', metric.labelClass)}>
                  {metric.dotClass && (
                    <span className={cn('h-1.5 w-1.5 rounded-full', metric.dotClass)} />
                  )}
                  <span>{metric.label}:</span>
                  <strong className={cn('font-semibold', metric.valueClass)}>{metric.value}</strong>
                </span>
              </Fragment>
            ))}
          </div>
        }
        actions={
          <>
            <RefreshButton onClick={loadData} loading={isLoading} label={tc('actions.refresh')} />
            <button
              type="button"
              onClick={() => {
                setCameraToEdit(null)
                setIsCameraModalOpen(true)
              }}
              className="page-action-btn page-action-btn--primary"
            >
              <Plus className="h-4 w-4" />
              <span>{t('manage.addCamera', { defaultValue: '接入设备' })}</span>
            </button>
          </>
        }
      />

      {/* 国标新通道发现提示横幅 */}
      {unmanagedChannels.length > 0 && !bannerDismissed && (
        <div className="frosted-glass border-status-info/30 bg-status-info/10 flex items-center justify-between gap-4 rounded-xl border p-3.5 shadow-xs">
          <div className="flex items-center gap-3">
            <div className="bg-status-info/20 text-status-info flex h-9 w-9 items-center justify-center rounded-lg">
              <Radio className="h-4.5 w-4.5" />
            </div>
            <div>
              <div className="text-xs font-semibold text-[var(--text-primary)]">
                {t('discovery.bannerTitle', {
                  name: unmanagedChannels[0].device.name || unmanagedChannels[0].device.deviceId,
                  total: unmanagedChannels.length,
                  defaultValue: `检测到新注册的国标设备「${unmanagedChannels[0].device.name || unmanagedChannels[0].device.deviceId}」，发现 ${unmanagedChannels.length} 个可用通道`,
                })}
              </div>
              <div className="text-[11px] text-[var(--text-muted)]">
                {t('discovery.bannerStatus', {
                  imported: importedChannelsCount,
                  total: totalChannelsCount,
                  defaultValue: `当前已纳管: ${importedChannelsCount} / ${totalChannelsCount} 通道`,
                })}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setIsBatchImportOpen(true)}
              className="page-action-btn page-action-btn--soft-info h-8 rounded-lg !px-3 text-xs"
            >
              {t('discovery.batchImport', { defaultValue: '批量纳管通道' })}
            </button>
            <button
              type="button"
              onClick={() => setBannerDismissed(true)}
              className="rounded-lg p-1.5 text-[var(--text-muted)] hover:bg-[var(--bg-secondary)]"
              title={t('discovery.ignore', { defaultValue: '忽略' })}
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>
      )}

      {/* 搜索与过滤筛选栏 */}
      {cameras.length > 0 && (
        <div className="frosted-glass flex items-center justify-between gap-3 rounded-xl px-4 py-2.5 shadow-xs">
          <SearchInput
            showKbdHint
            value={searchQuery}
            onChange={(val) => {
              setSearchQuery(val)
              setPage(1)
            }}
            onClear={() => {
              setSearchQuery('')
              setPage(1)
            }}
            placeholder={t('manage.searchPlaceholder', {
              defaultValue: '按设备名称、ID 或 RTSP 地址搜索...',
            })}
            aria-label={t('manage.searchPlaceholder', {
              defaultValue: '按设备名称、ID 或 RTSP 地址搜索...',
            })}
            clearAriaLabel={t('manage.clearSearch')}
            containerClassName="flex-1"
          />

          <div className="flex items-center gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-xs">
            <button
              type="button"
              onClick={() => {
                setProtocolFilter('all')
                setPage(1)
              }}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                protocolFilter === 'all'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              {t('protocol.all', { defaultValue: '全部协议' })}
            </button>
            <button
              type="button"
              onClick={() => {
                setProtocolFilter('rtsp')
                setPage(1)
              }}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                protocolFilter === 'rtsp'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              {t('protocol.rtsp', { defaultValue: 'RTSP' })}
            </button>
            <button
              type="button"
              onClick={() => {
                setProtocolFilter('gb28181')
                setPage(1)
              }}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                protocolFilter === 'gb28181'
                  ? 'bg-status-info-solid text-white shadow-xs'
                  : 'hover:text-status-info text-[var(--text-secondary)]'
              }`}
            >
              {t('protocol.gb28181', { defaultValue: '国标 28181' })}
            </button>
          </div>

          <div className="flex items-center gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-xs">
            <button
              type="button"
              onClick={() => {
                setStatusFilter('all')
                setPage(1)
              }}
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
              onClick={() => {
                setStatusFilter('online')
                setPage(1)
              }}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                statusFilter === 'online'
                  ? 'bg-status-success-solid text-white shadow-xs'
                  : 'hover:text-status-success text-[var(--text-secondary)]'
              }`}
            >
              {t('manage.onlineDevices', { defaultValue: '在线' })} ({onlineCount})
            </button>
            <button
              type="button"
              onClick={() => {
                setStatusFilter('degraded')
                setPage(1)
              }}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                statusFilter === 'degraded'
                  ? 'bg-status-warning-solid text-white shadow-xs'
                  : 'hover:text-status-warning text-[var(--text-secondary)]'
              }`}
            >
              {t('manage.degradedDevices', { defaultValue: '抖动' })} ({degradedCount})
            </button>
            <button
              type="button"
              onClick={() => {
                setStatusFilter('offline')
                setPage(1)
              }}
              className={`rounded-lg px-2.5 py-1 font-medium transition-colors ${
                statusFilter === 'offline'
                  ? 'bg-[var(--status-danger-solid)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--status-danger)]'
              }`}
            >
              {t('manage.offlineDevices', { defaultValue: '离线' })} ({offlineCount})
            </button>
          </div>
        </div>
      )}

      {/* 设备网格列表 */}
      <div className="flex-1 overflow-auto pr-1">
        {cameras.length === 0 ? (
          <div className="frosted-glass flex flex-col items-center justify-center rounded-2xl border border-[var(--border)] py-24 text-center text-[var(--text-muted)]">
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
              className="page-action-btn page-action-btn--primary mt-5"
            >
              <Plus className="h-4 w-4" />
              <span>{t('manage.addFirstCamera', { defaultValue: '接入首路摄像头' })}</span>
            </button>
          </div>
        ) : filteredCameras.length === 0 ? (
          <div className="frosted-glass flex flex-col items-center justify-center rounded-2xl border border-[var(--border)] py-20 text-center text-[var(--text-muted)]">
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
          <div className="grid grid-cols-[repeat(auto-fill,minmax(270px,1fr))] gap-4 sm:gap-4.5">
            <AnimatePresence mode="popLayout">
              {paginatedCameras.map((camera) => (
                <CameraDeviceTile
                  key={camera.id}
                  camera={camera}
                  cameraType={modelTypeOverrides[camera.cameraId]}
                  task={taskMap.get(camera.cameraId)}
                  isProbing={probingCameraId === camera.cameraId}
                  probeFeedback={probeFeedback[camera.cameraId]}
                  onManualProbe={handleManualProbe}
                  onEdit={(cam) => {
                    setCameraToEdit(cam)
                    setIsCameraModalOpen(true)
                  }}
                  onDetail={(cam) => setSelectedCameraForDetail(cam)}
                  onClick={(cam) => setSelectedCameraForDetail(cam)}
                  onDelete={(cam) => setCameraToDelete(cam)}
                />
              ))}
            </AnimatePresence>
          </div>
        )}
      </div>

      {/* 分页控制栏 (常驻吸底工规条，支持每页条数选择) */}
      <div className="frosted-glass flex shrink-0 flex-wrap items-center justify-between gap-3 rounded-2xl px-4 py-2.5 text-xs text-[var(--text-secondary)] shadow-xs">
        <div className="flex items-center gap-3">
          <span>{t('pagination.page', { current: page })}</span>
          {searchQuery || statusFilter !== 'all' || protocolFilter !== 'all' ? (
            <span className="text-status-success font-mono font-semibold">
              ({t('pagination.pageFiltered', { count: filteredCameras.length })})
            </span>
          ) : totalCount > 0 ? (
            <span className="font-mono text-[var(--text-muted)]">
              ({t('pagination.total', { total: totalCount })})
            </span>
          ) : null}

          {/* 每页条数选择器 */}
          <div className="flex items-center gap-1.5 border-l border-[var(--border)] pl-3">
            <select
              value={pageSize}
              onChange={(e) => {
                const next = Number(e.target.value)
                setPageSize(next)
                setPage(1)
              }}
              aria-label={t('pagination.pageSize')}
              title={t('pagination.pageSize')}
              className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-xs text-[var(--text-primary)] transition-all outline-none hover:border-[var(--accent)] focus:border-[var(--accent)]"
            >
              {[6, 12, 24, 48, 999].map((size) => (
                <option key={size} value={size}>
                  {size === 999
                    ? t('pagination.all', { defaultValue: '全部显示' })
                    : t('pagination.perPage', { count: size, defaultValue: `${size} 台/页` })}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={page <= 1 || isLoading}
            onClick={() => setPage((p) => Math.max(1, p - 1))}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-40"
          >
            {t('pagination.prev')}
          </button>
          <span className="px-1 font-mono font-semibold text-[var(--text-primary)]">
            {page} / {totalPages}
          </span>
          <button
            type="button"
            disabled={page >= totalPages || isLoading}
            onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-40"
          >
            {t('pagination.next')}
          </button>
        </div>
      </div>

      {/* 摄像头完整详情抽屉（modal-layer--drawer 层级） */}
      <CameraDetailDrawer
        camera={selectedCameraForDetail}
        task={selectedCameraForDetail ? taskMap.get(selectedCameraForDetail.cameraId) : undefined}
        onClose={() => setSelectedCameraForDetail(null)}
        onManualProbe={handleManualProbe}
        onModelTypeChange={(cameraId, type) => {
          setModelTypeOverrides((prev) => ({ ...prev, [cameraId]: type }))
        }}
        onEdit={(cam) => {
          setCameraToEdit(cam)
          setIsCameraModalOpen(true)
        }}
        onDelete={(cam) => setCameraToDelete(cam)}
        isProbing={probingCameraId === selectedCameraForDetail?.cameraId}
        probeFeedback={
          selectedCameraForDetail ? probeFeedback[selectedCameraForDetail.cameraId] : undefined
        }
      />

      {/* 摄像头添加/编辑模态框（modal-backdrop--top，层叠在详情抽屉之上） */}
      <CameraModal
        isOpen={isCameraModalOpen}
        camera={cameraToEdit}
        onClose={() => {
          setIsCameraModalOpen(false)
          setCameraToEdit(null)
        }}
        onSuccess={handleCameraSaved}
      />

      {/* 摄像头删除确认模态框（modal-backdrop--top） */}
      <DeleteCameraModal
        isOpen={Boolean(cameraToDelete)}
        camera={cameraToDelete}
        onClose={() => setCameraToDelete(null)}
        onSuccess={handleCameraDeleted}
      />

      {/* 批量纳管国标通道抽屉 */}
      <BatchImportGbModal
        isOpen={isBatchImportOpen}
        onClose={() => setIsBatchImportOpen(false)}
        onSuccess={loadData}
        devices={gbDevices}
      />
    </div>
  )
}
