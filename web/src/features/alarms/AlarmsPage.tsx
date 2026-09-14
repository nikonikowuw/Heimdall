import React, { useCallback, useEffect, useMemo, useState } from 'react'
import {
  AlertCircle,
  Camera as CameraIcon,
  Filter,
  LayoutGrid,
  List,
  RefreshCw,
  ShieldAlert,
  UserCheck,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { alarmApi, cameraApi, evidenceApi } from '../../lib/api'
import { wsClient } from '../../lib/wsClient'
import {
  type AlarmRecord,
  type AlarmSeverity,
  type AlarmStatus,
  type Camera,
  type CaptureRecord,
  type FaceCandidateItem,
  type RecognitionRecord,
  WS_TOPICS,
} from '../../types'
import { AlarmLightboxModal } from './components/AlarmLightboxModal'
import { AlarmsContent, type ViewMode } from './components/AlarmsContent'
import { BatchActionBar } from './components/BatchActionBar'
import { CaptureLightboxModal } from './components/CaptureLightboxModal'
import { CapturesContent } from './components/CapturesContent'
import { CropLightboxModal } from './components/CropLightboxModal'
import { DateTimeRangePicker, type DateTimeRangeValue } from './components/DateTimeRangePicker'
import { RealtimeAlarmBanner } from './components/RealtimeAlarmBanner'
import { RecognitionContent } from './components/RecognitionContent'
import { RecognitionReviewModal } from './components/RecognitionReviewModal'

type EvidenceTab = 'alarms' | 'captures' | 'recognition'

function getInitialTodayRange(): DateTimeRangeValue {
  const todayStart = new Date()
  todayStart.setHours(0, 0, 0, 0)
  return {
    quickPreset: 'today',
    startTime: todayStart.getTime(),
    endTime: Date.now(),
  }
}

function matchesTimeRange(timeRange: DateTimeRangeValue, timestamp: number): boolean {
  if (timeRange.quickPreset === 'all' || (!timeRange.startTime && !timeRange.endTime)) {
    return true
  }
  if (timeRange.quickPreset === 'today') {
    return true
  }
  const afterStart = timeRange.startTime === undefined || timestamp >= timeRange.startTime
  const beforeEnd = timeRange.endTime === undefined || timestamp <= timeRange.endTime + 60_000
  return afterStart && beforeEnd
}

export function AlarmsPage(): React.ReactElement {
  const { t } = useTranslation('alarm')
  const [activeTab, setActiveTab] = useState<EvidenceTab>('alarms')
  const [viewMode, setViewMode] = useState<ViewMode>('cards')

  // 基础数据与通道
  const [cameras, setCameras] = useState<Camera[]>([])
  const [selectedCameraId, setSelectedCameraId] = useState<string>('')
  const [selectedTargetLabel, setSelectedTargetLabel] = useState<string>('')
  const [selectedSeverity, setSelectedSeverity] = useState<string>('all')
  const [selectedStatus, setSelectedStatus] = useState<string>('all')

  // 时间维度筛选 (默认查询当前最新记录 - 今天)
  const [timeRange, setTimeRange] = useState<DateTimeRangeValue>(getInitialTodayRange)

  // 分页与总数 (支持动态选择每页条数)
  const [page, setPage] = useState<number>(1)
  const [pageSize, setPageSize] = useState<number>(20)
  const [totalCount, setTotalCount] = useState<number>(0)
  const [tabCounts, setTabCounts] = useState<Record<EvidenceTab, number>>({
    alarms: 0,
    captures: 0,
    recognition: 0,
  })

  // 数据列表
  const [alarms, setAlarms] = useState<AlarmRecord[]>([])
  const [captures, setCaptures] = useState<CaptureRecord[]>([])
  const [recognitions, setRecognitions] = useState<RecognitionRecord[]>([])

  // 多选与批量操作
  const [selectedAlarmIds, setSelectedAlarmIds] = useState<Set<number>>(new Set())
  const [isBatchProcessing, setIsBatchProcessing] = useState<boolean>(false)

  // 实时未读告警通知
  const [unreadRealtimeCount, setUnreadRealtimeCount] = useState<number>(0)

  // 界面状态
  const [isLoading, setIsLoading] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  // 模态框灯箱
  const [lightboxAlarm, setLightboxAlarm] = useState<AlarmRecord | null>(null)
  const [lightboxCapture, setLightboxCapture] = useState<CaptureRecord | null>(null)
  const [cropPreviewAlarm, setCropPreviewAlarm] = useState<AlarmRecord | null>(null)
  const [reviewModalRec, setReviewModalRec] = useState<RecognitionRecord | null>(null)

  // 挂载时加载摄像头字典并拉取各 Tab 初始总数
  useEffect(() => {
    let isMounted = true
    cameraApi
      .list()
      .then((list) => {
        if (isMounted) setCameras(list)
      })
      .catch(() => {})

    const initialRange = getInitialTodayRange()
    Promise.all([
      alarmApi
        .count({ startTime: initialRange.startTime, endTime: initialRange.endTime })
        .catch(() => ({ total: 0 })),
      evidenceApi
        .countCaptures({ startTime: initialRange.startTime, endTime: initialRange.endTime })
        .catch(() => ({ total: 0 })),
      evidenceApi
        .countRecognitions({ startTime: initialRange.startTime, endTime: initialRange.endTime })
        .catch(() => ({ total: 0 })),
    ]).then(([alarmsRes, capturesRes, recsRes]) => {
      if (!isMounted) return
      setTabCounts({
        alarms: alarmsRes.total,
        captures: capturesRes.total,
        recognition: recsRes.total,
      })
    })

    return () => {
      isMounted = false
    }
  }, [])

  const cameraNameMap = useMemo(() => {
    const map: Record<string, string> = {}
    for (const c of cameras) {
      map[c.cameraId] = c.name
    }
    return map
  }, [cameras])

  const handleSwitchTab = (tab: EvidenceTab) => {
    setActiveTab(tab)
    setSelectedStatus('all')
    setTotalCount(tabCounts[tab] || 0)
    setPage(1)
  }

  // 数据加载函数
  const loadData = useCallback(async () => {
    setIsLoading(true)
    setErrorMessage(null)
    try {
      const camId = selectedCameraId || undefined
      const targetLbl = selectedTargetLabel || undefined
      const severityParam = selectedSeverity === 'all' ? undefined : selectedSeverity
      const statusParam = selectedStatus === 'all' ? undefined : selectedStatus
      const startMs = timeRange.startTime
      const endMs = timeRange.endTime
      const offset = (page - 1) * pageSize

      if (activeTab === 'alarms') {
        const [list, countRes] = await Promise.all([
          alarmApi.list({
            cameraId: camId,
            status: statusParam,
            targetLabel: targetLbl,
            severity: severityParam,
            startTime: startMs,
            endTime: endMs,
            limit: pageSize,
            offset,
          }),
          alarmApi.count({
            cameraId: camId,
            status: statusParam,
            targetLabel: targetLbl,
            severity: severityParam,
            startTime: startMs,
            endTime: endMs,
          }),
        ])
        setAlarms(list)
        setTotalCount(countRes.total)
        setTabCounts((prev) => ({ ...prev, alarms: countRes.total }))
      } else if (activeTab === 'captures') {
        const [list, countRes] = await Promise.all([
          evidenceApi.listCaptures({
            cameraId: camId,
            targetLabel: targetLbl,
            startTime: startMs,
            endTime: endMs,
            limit: pageSize,
            offset,
          }),
          evidenceApi.countCaptures({
            cameraId: camId,
            targetLabel: targetLbl,
            startTime: startMs,
            endTime: endMs,
          }),
        ])
        setCaptures(list)
        setTotalCount(countRes.total)
        setTabCounts((prev) => ({ ...prev, captures: countRes.total }))
      } else if (activeTab === 'recognition') {
        const [list, countRes] = await Promise.all([
          evidenceApi.listRecognitions({
            cameraId: camId,
            status: statusParam,
            startTime: startMs,
            endTime: endMs,
            limit: pageSize,
            offset,
          }),
          evidenceApi.countRecognitions({
            cameraId: camId,
            status: statusParam,
            startTime: startMs,
            endTime: endMs,
          }),
        ])
        setRecognitions(list)
        setTotalCount(countRes.total)
        setTabCounts((prev) => ({ ...prev, recognition: countRes.total }))
      }
      setSelectedAlarmIds(new Set())
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [
    activeTab,
    selectedCameraId,
    selectedTargetLabel,
    selectedSeverity,
    selectedStatus,
    timeRange,
    page,
    pageSize,
  ])

  useEffect(() => {
    loadData()
  }, [loadData])

  // 实时 WebSocket 订阅：新告警触发 & 状态变更
  useEffect(() => {
    const unsubAlarm = wsClient.subscribe<{
      id: number
      eventId: string
      cameraId: string
      cameraName: string
      algorithmId: string
      alarmTypeId: string
      targetLabel: string
      ruleType: string
      severity: AlarmSeverity
      cropImageRelPath: string
      imageRelPath: string
      occurredAt: number
    }>(WS_TOPICS.ALARM_TRIGGERED, (p) => {
      if (!p) return

      if (activeTab !== 'alarms') return

      // 若当前在第 1 页且无冲突筛选，平滑 prepend 到列表顶部
      const matchesCamera = !selectedCameraId || selectedCameraId === p.cameraId
      const matchesTarget = !selectedTargetLabel || selectedTargetLabel === p.targetLabel
      const matchesSeverity = selectedSeverity === 'all' || selectedSeverity === p.severity
      const matchesStatus = selectedStatus === 'all' || selectedStatus === 'unprocessed'
      const isLiveTime = matchesTimeRange(timeRange, p.occurredAt)

      if (
        page === 1 &&
        matchesCamera &&
        matchesTarget &&
        matchesSeverity &&
        matchesStatus &&
        isLiveTime
      ) {
        const newRecord: AlarmRecord = {
          id: p.id,
          eventId: p.eventId,
          cameraId: p.cameraId,
          alarmTypeId: p.alarmTypeId,
          occurredAt: p.occurredAt,
          targetLabel: p.targetLabel,
          confidence: 1.0,
          trackId: 0,
          bboxJson: '{}',
          imageId: '',
          imageRelPath: p.imageRelPath || '',
          cropImageId: '',
          cropImageRelPath: p.cropImageRelPath || '',
          ruleType: p.ruleType,
          severity: p.severity,
          status: 'unprocessed',
          handledAt: null,
          createdAt: Date.now(),
        }
        setAlarms((prev) => {
          if (prev.some((a) => a.id === p.id || a.eventId === p.eventId)) return prev
          return [newRecord, ...prev.slice(0, pageSize - 1)]
        })
        setTotalCount((c) => c + 1)
        setTabCounts((prev) => ({ ...prev, alarms: prev.alarms + 1 }))
      } else {
        setUnreadRealtimeCount((c) => c + 1)
        setTabCounts((prev) => ({ ...prev, alarms: prev.alarms + 1 }))
      }
    })

    const unsubStatus = wsClient.subscribe<{
      id?: number
      eventId?: string
      status?: AlarmStatus
      handledAt?: number
    }>(WS_TOPICS.ALARM_STATUS_CHANGED, (p) => {
      if (!p || !p.status) return
      const nextStatus = p.status
      const nextHandledAt = p.handledAt ?? Date.now()

      setAlarms((prev) =>
        prev.map((a) => {
          if ((p.id && a.id === p.id) || (p.eventId && a.eventId === p.eventId)) {
            return { ...a, status: nextStatus, handledAt: nextHandledAt }
          }
          return a
        }),
      )

      setLightboxAlarm((prev) => {
        if (!prev) return null
        if ((p.id && prev.id === p.id) || (p.eventId && prev.eventId === p.eventId)) {
          return { ...prev, status: nextStatus, handledAt: nextHandledAt }
        }
        return prev
      })
    })

    return () => {
      unsubAlarm()
      unsubStatus()
    }
  }, [
    activeTab,
    selectedCameraId,
    selectedTargetLabel,
    selectedSeverity,
    selectedStatus,
    timeRange,
    page,
    pageSize,
  ])

  const handleFilterChange = (setter: (v: string) => void, val: string): void => {
    setter(val)
    setPage(1)
  }

  // 单条告警状态切换
  const handleToggleAlarmStatus = async (alarm: AlarmRecord): Promise<void> => {
    const nextStatus: AlarmStatus = alarm.status === 'processed' ? 'unprocessed' : 'processed'
    try {
      const updated = await alarmApi.updateStatus(alarm.id, nextStatus)
      setAlarms((prev) => prev.map((a) => (a.id === alarm.id ? updated : a)))
      if (lightboxAlarm && lightboxAlarm.id === alarm.id) {
        setLightboxAlarm(updated)
      }
      if (selectedStatus !== 'all') {
        loadData()
      }
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err))
    }
  }

  // 批量操作处理
  const handleToggleSelectAlarm = (id: number, selected: boolean): void => {
    setSelectedAlarmIds((prev) => {
      const next = new Set(prev)
      if (selected) {
        next.add(id)
      } else {
        next.delete(id)
      }
      return next
    })
  }

  const handleToggleSelectAll = (selected: boolean): void => {
    if (selected) {
      const allIds = new Set(alarms.map((a) => a.id))
      setSelectedAlarmIds(allIds)
    } else {
      setSelectedAlarmIds(new Set())
    }
  }

  const handleBatchStatus = async (status: AlarmStatus): Promise<void> => {
    if (selectedAlarmIds.size === 0) return
    setIsBatchProcessing(true)
    try {
      const ids = Array.from(selectedAlarmIds)
      const updatedList = await alarmApi.batchUpdateStatus(ids, status)
      const updatedMap = new Map(updatedList.map((item) => [item.id, item]))
      setAlarms((prev) => prev.map((a) => updatedMap.get(a.id) || a))
      setSelectedAlarmIds(new Set())
      if (selectedStatus !== 'all') {
        loadData()
      }
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err))
    } finally {
      setIsBatchProcessing(false)
    }
  }

  // 人脸识别复核处理
  const handleReviewRecognition = async (
    rec: RecognitionRecord,
    status: 'confirmed' | 'rejected',
    candidate?: FaceCandidateItem,
  ): Promise<void> => {
    try {
      const updated = await evidenceApi.reviewRecognition(rec.recognitionId, {
        status,
        subjectId: candidate?.subjectId ?? (status === 'confirmed' ? rec.subjectId : undefined),
        subjectName:
          candidate?.subjectName ?? (status === 'confirmed' ? rec.subjectName : undefined),
        photoRelPath: candidate?.photoRelPath,
        similarity: candidate?.similarity ?? (status === 'confirmed' ? rec.similarity : undefined),
      })
      setRecognitions((prev) =>
        prev.map((r) => (r.recognitionId === rec.recognitionId ? updated : r)),
      )
      if (reviewModalRec && reviewModalRec.recognitionId === rec.recognitionId) {
        setReviewModalRec(null)
      }
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err))
    }
  }

  const totalPages = Math.max(1, Math.ceil(totalCount / pageSize))

  return (
    <div className="flex h-full flex-col gap-3 bg-[var(--bg-primary)] p-4 text-[var(--text-primary)] select-none">
      {/* 实时新告警浮条 */}
      <RealtimeAlarmBanner
        count={unreadRealtimeCount}
        onViewNew={() => {
          setUnreadRealtimeCount(0)
          setSelectedCameraId('')
          setSelectedTargetLabel('')
          setSelectedSeverity('all')
          setSelectedStatus('all')
          setTimeRange(getInitialTodayRange())
          setPage(1)
        }}
        onDismiss={() => setUnreadRealtimeCount(0)}
        t={t}
      />

      {/* 顶部控制栏与三重视图切换 */}
      <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl p-3 shadow-xs">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-rose-500/10 text-rose-500">
            <ShieldAlert className="h-5 w-5" />
          </div>
          <div>
            <h2 className="text-sm font-semibold text-[var(--text-primary)]">{t('title')}</h2>
            <p className="text-xs text-[var(--text-muted)]">
              {t(`tabs.${activeTab}`)} · {t('subtitleSuffix')}
            </p>
          </div>
        </div>

        {/* 证据分类 Tab 切换器 */}
        <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-xs">
          {(
            [
              {
                key: 'alarms' as const,
                label: t('tabs.alarms'),
                icon: AlertCircle,
                activeClass: 'border border-rose-500/30 bg-rose-500/15 text-rose-500 shadow-xs',
                iconColor: 'text-rose-500',
                badgeBg: 'bg-rose-500/10 text-rose-500',
                badgeCount: activeTab === 'alarms' ? totalCount : tabCounts.alarms,
              },
              {
                key: 'captures' as const,
                label: t('tabs.captures'),
                icon: CameraIcon,
                activeClass: 'border border-cyan-500/30 bg-cyan-500/15 text-cyan-500 shadow-xs',
                iconColor: 'text-cyan-500',
                badgeBg: 'bg-cyan-500/10 text-cyan-500',
                badgeCount: activeTab === 'captures' ? totalCount : tabCounts.captures,
              },
              {
                key: 'recognition' as const,
                label: t('tabs.recognition'),
                icon: UserCheck,
                activeClass:
                  'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500 shadow-xs',
                iconColor: 'text-emerald-500',
                badgeBg: 'bg-emerald-500/10 text-emerald-500',
                badgeCount: activeTab === 'recognition' ? totalCount : tabCounts.recognition,
              },
            ] as const
          ).map((tab) => {
            const Icon = tab.icon
            const isActive = activeTab === tab.key
            return (
              <button
                key={tab.key}
                type="button"
                onClick={() => handleSwitchTab(tab.key)}
                className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all ${
                  isActive
                    ? tab.activeClass
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                <Icon className={`h-3.5 w-3.5 ${tab.iconColor}`} />
                <span>{tab.label}</span>
                {tab.badgeCount > 0 && (
                  <span
                    className={`py-0.2 ml-1 rounded-full px-1.5 font-mono text-[10px] font-bold ${
                      isActive ? tab.badgeBg : 'bg-[var(--bg-secondary)] text-[var(--text-muted)]'
                    }`}
                  >
                    {tab.badgeCount}
                  </span>
                )}
              </button>
            )
          })}
        </div>
      </div>

      {/* 筛选工具条与视图切换 */}
      <div className="frosted-glass relative z-20 flex flex-wrap items-center justify-between gap-3 rounded-2xl p-3 shadow-xs">
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <Filter className="h-3.5 w-3.5 text-[var(--text-muted)]" />

          {/* 通道筛选 */}
          <select
            value={selectedCameraId}
            onChange={(e) => handleFilterChange(setSelectedCameraId, e.target.value)}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
          >
            <option value="">{t('filter.allCameras')}</option>
            {cameras.map((c) => (
              <option key={c.id} value={c.cameraId}>
                {c.name || c.cameraId}
              </option>
            ))}
          </select>

          {/* 目标类别筛选 */}
          {activeTab !== 'recognition' && (
            <select
              value={selectedTargetLabel}
              onChange={(e) => handleFilterChange(setSelectedTargetLabel, e.target.value)}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
            >
              <option value="">{t('filter.allTargets')}</option>
              <option value="person">{t('filter.person')}</option>
              <option value="car">{t('filter.car')}</option>
              <option value="bicycle">{t('filter.bicycle')}</option>
            </select>
          )}

          {/* 严重级别筛选 */}
          {activeTab === 'alarms' && (
            <select
              value={selectedSeverity}
              onChange={(e) => handleFilterChange(setSelectedSeverity, e.target.value)}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
            >
              <option value="all">{t('filter.allSeverities')}</option>
              <option value="warning">{t('filter.severityWarning')}</option>
              <option value="critical">{t('filter.severityCritical')}</option>
            </select>
          )}

          {/* 告警状态筛选 */}
          {activeTab === 'alarms' && (
            <select
              value={selectedStatus}
              onChange={(e) => handleFilterChange(setSelectedStatus, e.target.value)}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
            >
              <option value="all">{t('statusFilter.all')}</option>
              <option value="unprocessed">{t('statusFilter.unprocessed')}</option>
              <option value="processed">{t('statusFilter.processed')}</option>
            </select>
          )}

          {/* 识别对账状态筛选 */}
          {activeTab === 'recognition' && (
            <select
              value={selectedStatus}
              onChange={(e) => handleFilterChange(setSelectedStatus, e.target.value)}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
            >
              <option value="all">{t('statusFilter.all')}</option>
              <option value="confirmed">{t('statusFilter.confirmed')}</option>
              <option value="pending_review">{t('statusFilter.pendingReview')}</option>
              <option value="rejected">{t('statusFilter.rejected')}</option>
            </select>
          )}

          {/* 秒级精细时间选择器 */}
          <DateTimeRangePicker
            value={timeRange}
            onChange={(val) => {
              setTimeRange(val)
              setPage(1)
            }}
            t={t}
          />
        </div>

        <div className="flex items-center gap-2">
          {/* 卡片与表格视图切换 */}
          {activeTab === 'alarms' && (
            <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-0.5 text-xs">
              <button
                type="button"
                onClick={() => setViewMode('cards')}
                className={`flex items-center gap-1 rounded-lg px-2 py-1 transition-all ${
                  viewMode === 'cards'
                    ? 'bg-[var(--accent)] text-white shadow-xs'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
                title={t('views.cards')}
              >
                <LayoutGrid className="h-3.5 w-3.5" />
                <span className="text-[11px]">{t('views.cards')}</span>
              </button>
              <button
                type="button"
                onClick={() => setViewMode('table')}
                className={`flex items-center gap-1 rounded-lg px-2 py-1 transition-all ${
                  viewMode === 'table'
                    ? 'bg-[var(--accent)] text-white shadow-xs'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
                title={t('views.table')}
              >
                <List className="h-3.5 w-3.5" />
                <span className="text-[11px]">{t('views.table')}</span>
              </button>
            </div>
          )}

          <button
            type="button"
            onClick={loadData}
            disabled={isLoading}
            className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-50"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
            <span>{t('filter.refresh')}</span>
          </button>
        </div>
      </div>

      {/* 错误提示 */}
      {errorMessage && (
        <div className="flex items-center gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-500">
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span>{errorMessage}</span>
        </div>
      )}

      {/* 主视图内容区 */}
      <div className="frosted-glass flex-1 overflow-auto rounded-2xl p-4 shadow-xs">
        {activeTab === 'alarms' && (
          <AlarmsContent
            alarms={alarms}
            totalCount={totalCount}
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            selectedAlarmIds={selectedAlarmIds}
            onToggleSelectAlarm={handleToggleSelectAlarm}
            onToggleSelectAll={handleToggleSelectAll}
            onSelect={setLightboxAlarm}
            onSelectCrop={setCropPreviewAlarm}
            onToggleStatus={handleToggleAlarmStatus}
            t={t}
          />
        )}

        {activeTab === 'captures' && (
          <CapturesContent
            captures={captures}
            cameraNameMap={cameraNameMap}
            onSelect={setLightboxCapture}
            t={t}
          />
        )}

        {activeTab === 'recognition' && (
          <RecognitionContent
            recognitions={recognitions}
            cameraNameMap={cameraNameMap}
            onOpenReview={setReviewModalRec}
            onQuickReview={(recognition, status) => handleReviewRecognition(recognition, status)}
            t={t}
          />
        )}
      </div>

      {/* 分页控制栏 (支持每页条数选择器) */}
      <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl px-4 py-2.5 text-xs text-[var(--text-secondary)] shadow-xs">
        <div className="flex items-center gap-3">
          <span>{t('pagination.page', { current: page })}</span>
          {totalCount > 0 && (
            <span className="font-mono text-[var(--text-muted)]">
              ({t('pagination.total', { total: totalCount })})
            </span>
          )}

          {/* 每页条数选择器 */}
          <div className="flex items-center gap-1.5 border-l border-[var(--border)] pl-3">
            <select
              value={pageSize}
              onChange={(e) => {
                const next = Number(e.target.value)
                setPageSize(next)
                setPage(1)
              }}
              className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-xs text-[var(--text-primary)] transition-all outline-none hover:border-[var(--accent)] focus:border-[var(--accent)]"
              title={t('pagination.pageSize')}
            >
              {[10, 20, 50, 100].map((size) => (
                <option key={size} value={size}>
                  {t('pagination.perPage', { count: size })}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setPage((p) => Math.max(1, p - 1))}
            disabled={page === 1 || isLoading}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-40"
          >
            {t('pagination.prev')}
          </button>
          <span className="px-1 font-mono font-semibold text-[var(--text-primary)]">
            {page} / {totalPages}
          </span>
          <button
            type="button"
            onClick={() => setPage((p) => p + 1)}
            disabled={page >= totalPages || isLoading}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-40"
          >
            {t('pagination.next')}
          </button>
        </div>
      </div>

      {/* 底部浮动批量操作条 */}
      <BatchActionBar
        selectedCount={selectedAlarmIds.size}
        isProcessing={isBatchProcessing}
        onMarkProcessed={() => handleBatchStatus('processed')}
        onMarkUnprocessed={() => handleBatchStatus('unprocessed')}
        onClearSelection={() => setSelectedAlarmIds(new Set())}
        t={t}
      />

      {/* 告警大图灯箱 Modal (高精度 BBox 绘制 + Esc 快速退出) */}
      {lightboxAlarm && (
        <AlarmLightboxModal
          alarm={lightboxAlarm}
          cameraName={cameraNameMap[lightboxAlarm.cameraId]}
          onClose={() => setLightboxAlarm(null)}
          onToggleStatus={() => handleToggleAlarmStatus(lightboxAlarm)}
          onSelectCrop={() => setCropPreviewAlarm(lightboxAlarm)}
          t={t}
        />
      )}

      {/* 抓拍大图灯箱 Modal */}
      {lightboxCapture && (
        <CaptureLightboxModal
          capture={lightboxCapture}
          cameraName={cameraNameMap[lightboxCapture.cameraId]}
          onClose={() => setLightboxCapture(null)}
          t={t}
        />
      )}

      {/* 识别对账 Top-5 候选人核验 Modal */}
      {reviewModalRec && (
        <RecognitionReviewModal
          recognition={reviewModalRec}
          cameraName={cameraNameMap[reviewModalRec.cameraId]}
          onClose={() => setReviewModalRec(null)}
          onReview={handleReviewRecognition}
          t={t}
        />
      )}

      {/* 特写大图灯箱 Modal */}
      {cropPreviewAlarm && (
        <CropLightboxModal
          alarm={cropPreviewAlarm}
          onClose={() => setCropPreviewAlarm(null)}
          t={t}
        />
      )}
    </div>
  )
}
