import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  AlertCircle,
  Camera as CameraIcon,
  ChevronDown,
  LayoutGrid,
  List,
  RefreshCw,
  RotateCcw,
  Search,
  ShieldAlert,
  UserCheck,
  Volume2,
  VolumeX,
  X,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
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
  type RecognitionStatus,
  type RecognitionStatusChangedPayload,
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
import { isAlarmSoundEnabled, playAlarmAlertSound, setAlarmSoundEnabled } from './sound'
import { resolveEffectiveTimeRange } from './utils'

type EvidenceTab = 'recognition' | 'alarms' | 'captures'

function getInitialTodayRange(): DateTimeRangeValue {
  const todayStart = new Date()
  todayStart.setHours(0, 0, 0, 0)
  return {
    quickPreset: 'today',
    startTime: todayStart.getTime(),
    endTime: undefined,
  }
}

function matchesTimeRange(timeRange: DateTimeRangeValue, timestamp: number): boolean {
  if (timeRange.quickPreset === 'all' || (!timeRange.startTime && !timeRange.endTime)) {
    return true
  }
  if (timeRange.quickPreset === 'today') {
    return true
  }
  const { startTime, endTime } = resolveEffectiveTimeRange(timeRange)
  const afterStart = startTime === undefined || timestamp >= startTime
  const beforeEnd = endTime === undefined || timestamp <= endTime + 60_000
  return afterStart && beforeEnd
}

export function AlarmsPage(): React.ReactElement {
  const { t } = useTranslation('alarm')
  const [activeTab, setActiveTab] = useState<EvidenceTab>('recognition')
  const [viewMode, setViewMode] = useState<ViewMode>('cards')

  // 基础数据与通道
  const [cameras, setCameras] = useState<Camera[]>([])
  const [selectedCameraId, setSelectedCameraId] = useState<string>('')
  const [selectedTargetLabel, setSelectedTargetLabel] = useState<string>('')
  const [selectedRuleType, setSelectedRuleType] = useState<string>('all')
  const [selectedSeverity, setSelectedSeverity] = useState<string>('all')
  const [selectedStatus, setSelectedStatus] = useState<string>('all')
  const [searchQuery, setSearchQuery] = useState<string>('')

  // 时间维度筛选 (默认查询当前最新记录 - 今天)
  const [timeRange, setTimeRange] = useState<DateTimeRangeValue>(getInitialTodayRange)

  // 声音告警开关
  const [soundEnabled, setSoundEnabled] = useState<boolean>(isAlarmSoundEnabled)

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

  // 实时事件微批次注入缓冲 (Micro-batch Ingestion Buffer)
  const pendingAlarmsRef = useRef<AlarmRecord[]>([])
  const pendingCountRef = useRef<number>(0)

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
    const { startTime: startMs, endTime: endMs } = resolveEffectiveTimeRange(initialRange)
    Promise.all([
      alarmApi.count({ startTime: startMs, endTime: endMs }).catch(() => ({ total: 0 })),
      evidenceApi.countCaptures({ startTime: startMs, endTime: endMs }).catch(() => ({ total: 0 })),
      evidenceApi
        .countRecognitions({ startTime: startMs, endTime: endMs })
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

  // 动态汇聚已出现的所有目标标签 (消除硬编码)
  const distinctTargetLabels = useMemo(() => {
    const set = new Set<string>()
    set.add('person')
    set.add('car')
    set.add('bicycle')
    for (const a of alarms) {
      if (a.targetLabel && a.targetLabel.trim()) set.add(a.targetLabel.trim())
    }
    for (const c of captures) {
      if (c.targetLabel && c.targetLabel.trim()) set.add(c.targetLabel.trim())
    }
    return Array.from(set)
  }, [alarms, captures])

  // 是否存在活跃的非默认过滤条件
  const hasActiveFilters = Boolean(
    searchQuery.trim() ||
    selectedCameraId ||
    selectedTargetLabel ||
    (activeTab === 'alarms' && selectedRuleType !== 'all') ||
    (activeTab === 'alarms' && selectedSeverity !== 'all') ||
    selectedStatus !== 'all' ||
    timeRange.quickPreset !== 'today',
  )

  const activeFilterCount = useMemo(() => {
    let count = 0
    if (searchQuery.trim()) count++
    if (selectedCameraId) count++
    if (selectedTargetLabel) count++
    if (activeTab === 'alarms' && selectedRuleType !== 'all') count++
    if (activeTab === 'alarms' && selectedSeverity !== 'all') count++
    if (selectedStatus !== 'all') count++
    if (timeRange.quickPreset !== 'today') count++
    return count
  }, [
    searchQuery,
    selectedCameraId,
    selectedTargetLabel,
    activeTab,
    selectedRuleType,
    selectedSeverity,
    selectedStatus,
    timeRange.quickPreset,
  ])

  const handleResetFilters = useCallback(() => {
    setSearchQuery('')
    setSelectedCameraId('')
    setSelectedTargetLabel('')
    setSelectedRuleType('all')
    setSelectedSeverity('all')
    setSelectedStatus('all')
    setTimeRange(getInitialTodayRange())
    setPage(1)
  }, [])

  const handleToggleSound = useCallback(() => {
    setSoundEnabled((prev) => {
      const next = !prev
      setAlarmSoundEnabled(next)
      return next
    })
  }, [])

  const handleSwitchTab = (tab: EvidenceTab) => {
    setActiveTab(tab)
    setSelectedStatus('all')
    setSelectedRuleType('all')
    pendingAlarmsRef.current = []
    pendingCountRef.current = 0
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
      const ruleTypeParam = selectedRuleType === 'all' ? undefined : selectedRuleType
      const severityParam = selectedSeverity === 'all' ? undefined : selectedSeverity
      const statusParam = selectedStatus === 'all' ? undefined : selectedStatus
      const { startTime: startMs, endTime: endMs } = resolveEffectiveTimeRange(timeRange)
      const offset = (page - 1) * pageSize

      if (activeTab === 'alarms') {
        const [list, countRes] = await Promise.all([
          alarmApi.list({
            cameraId: camId,
            status: statusParam,
            targetLabel: targetLbl,
            ruleType: ruleTypeParam,
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
            ruleType: ruleTypeParam,
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
    selectedRuleType,
    selectedSeverity,
    selectedStatus,
    timeRange,
    page,
    pageSize,
  ])

  useEffect(() => {
    loadData()
  }, [loadData])

  // 微批次推流消费周期 (每 200ms 合并刷入一次，抵御推流风暴)
  useEffect(() => {
    const interval = setInterval(() => {
      if (pendingAlarmsRef.current.length === 0) return
      const batch = pendingAlarmsRef.current.splice(0)
      const countInc = pendingCountRef.current
      pendingCountRef.current = 0

      if (activeTab === 'alarms') {
        setAlarms((prev) => {
          const existingIds = new Set(prev.map((a) => a.id))
          const newItems = batch.filter((a) => !existingIds.has(a.id))
          if (newItems.length === 0) return prev
          return [...newItems, ...prev].slice(0, pageSize)
        })
        setTotalCount((c) => c + countInc)
      }
      setTabCounts((prev) => ({ ...prev, alarms: prev.alarms + countInc }))
    }, 200)

    return () => clearInterval(interval)
  }, [pageSize, activeTab])

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

      // 发出告警提示音 (若开启)
      if (soundEnabled) {
        playAlarmAlertSound()
      }

      if (activeTab !== 'alarms') {
        setTabCounts((prev) => ({ ...prev, alarms: prev.alarms + 1 }))
        return
      }

      // 若当前在第 1 页且无冲突筛选，入队批处理微缓冲池
      const matchesCamera = !selectedCameraId || selectedCameraId === p.cameraId
      const matchesTarget = !selectedTargetLabel || selectedTargetLabel === p.targetLabel
      const matchesRule = selectedRuleType === 'all' || selectedRuleType === p.ruleType
      const matchesSeverity = selectedSeverity === 'all' || selectedSeverity === p.severity
      const matchesStatus = selectedStatus === 'all' || selectedStatus === 'unprocessed'
      const isLiveTime = matchesTimeRange(timeRange, p.occurredAt)

      if (
        page === 1 &&
        matchesCamera &&
        matchesTarget &&
        matchesRule &&
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
        pendingAlarmsRef.current.push(newRecord)
        pendingCountRef.current += 1
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

    const unsubRecognition = wsClient.subscribe<RecognitionRecord>(
      WS_TOPICS.RECOGNITION_MATCHED,
      (p) => {
        if (!p) return

        if (activeTab !== 'recognition') {
          setTabCounts((prev) => ({ ...prev, recognition: prev.recognition + 1 }))
          return
        }

        const matchesCamera = !selectedCameraId || selectedCameraId === p.cameraId
        const matchesStatus = selectedStatus === 'all' || selectedStatus === p.status
        const isLiveTime = matchesTimeRange(timeRange, p.recognizedAt)

        if (page === 1 && matchesCamera && matchesStatus && isLiveTime) {
          setRecognitions((prev) => {
            if (prev.some((r) => r.recognitionId === p.recognitionId)) return prev
            return [p, ...prev.slice(0, pageSize - 1)]
          })
          setTotalCount((c) => c + 1)
          setTabCounts((prev) => ({ ...prev, recognition: prev.recognition + 1 }))
        } else {
          setUnreadRealtimeCount((c) => c + 1)
          setTabCounts((prev) => ({ ...prev, recognition: prev.recognition + 1 }))
        }
      },
    )

    const unsubRecStatus = wsClient.subscribe<RecognitionStatusChangedPayload>(
      WS_TOPICS.RECOGNITION_STATUS_CHANGED,
      (p) => {
        if (!p || !p.recognitionId || !p.status) return

        const applyStatusUpdate = <T extends RecognitionRecord>(r: T): T => ({
          ...r,
          status: p.status as RecognitionStatus,
          reviewerId: p.reviewerId ?? r.reviewerId,
          reviewedAt: p.reviewedAt ?? r.reviewedAt,
          subjectId: p.subjectId ?? r.subjectId,
          subjectName: p.subjectName ?? r.subjectName,
          similarity: p.similarity ?? r.similarity,
          registeredPhotoPath: p.registeredPhotoPath ?? r.registeredPhotoPath,
        })

        setRecognitions((prev) =>
          prev.map((r) => (r.recognitionId === p.recognitionId ? applyStatusUpdate(r) : r)),
        )
        setReviewModalRec((prev) =>
          prev && prev.recognitionId === p.recognitionId ? applyStatusUpdate(prev) : prev,
        )
      },
    )

    return () => {
      unsubAlarm()
      unsubStatus()
      unsubRecognition()
      unsubRecStatus()
    }
  }, [
    activeTab,
    selectedCameraId,
    selectedTargetLabel,
    selectedRuleType,
    selectedSeverity,
    selectedStatus,
    soundEnabled,
    timeRange,
    page,
    pageSize,
  ])

  const handleFilterChange = (setter: (v: string) => void, val: string): void => {
    setter(val)
    setPage(1)
  }

  // 显式点击刷新处理
  const handleRefresh = useCallback(() => {
    void loadData()

    const { startTime: startMs, endTime: endMs } = resolveEffectiveTimeRange(timeRange)
    const camId = selectedCameraId || undefined
    const targetLbl = selectedTargetLabel || undefined
    const ruleTypeParam = selectedRuleType === 'all' ? undefined : selectedRuleType
    const severityParam = selectedSeverity === 'all' ? undefined : selectedSeverity
    const statusParam = selectedStatus === 'all' ? undefined : selectedStatus

    Promise.all([
      alarmApi
        .count({
          cameraId: camId,
          status: statusParam,
          targetLabel: targetLbl,
          ruleType: ruleTypeParam,
          severity: severityParam,
          startTime: startMs,
          endTime: endMs,
        })
        .catch(() => null),
      evidenceApi
        .countCaptures({
          cameraId: camId,
          targetLabel: targetLbl,
          startTime: startMs,
          endTime: endMs,
        })
        .catch(() => null),
      evidenceApi
        .countRecognitions({
          cameraId: camId,
          status: statusParam,
          startTime: startMs,
          endTime: endMs,
        })
        .catch(() => null),
    ]).then(([alarmsCount, capturesCount, recsCount]) => {
      setTabCounts((prev) => ({
        alarms: alarmsCount !== null ? alarmsCount.total : prev.alarms,
        captures: capturesCount !== null ? capturesCount.total : prev.captures,
        recognition: recsCount !== null ? recsCount.total : prev.recognition,
      }))
    })
  }, [
    loadData,
    timeRange,
    selectedCameraId,
    selectedTargetLabel,
    selectedRuleType,
    selectedSeverity,
    selectedStatus,
  ])

  // 单条告警状态切换
  const handleToggleAlarmStatus = useCallback(
    async (alarm: AlarmRecord): Promise<void> => {
      const nextStatus: AlarmStatus = alarm.status === 'processed' ? 'unprocessed' : 'processed'
      try {
        const updated = await alarmApi.updateStatus(alarm.id, nextStatus)
        setAlarms((prev) => prev.map((a) => (a.id === alarm.id ? updated : a)))
        setLightboxAlarm((prev) => (prev && prev.id === alarm.id ? updated : prev))
        if (selectedStatus !== 'all') {
          void loadData()
        }
      } catch (err) {
        setErrorMessage(err instanceof Error ? err.message : String(err))
      }
    },
    [selectedStatus, loadData],
  )

  // 批量选择处理
  const handleToggleSelectAlarm = useCallback((id: number, selected: boolean): void => {
    setSelectedAlarmIds((prev) => {
      const next = new Set(prev)
      if (selected) {
        next.add(id)
      } else {
        next.delete(id)
      }
      return next
    })
  }, [])

  const handleToggleSelectAll = useCallback(
    (selected: boolean): void => {
      if (selected) {
        setSelectedAlarmIds(new Set(alarms.map((a) => a.id)))
      } else {
        setSelectedAlarmIds(new Set())
      }
    },
    [alarms],
  )

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
        void loadData()
      }
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err))
    } finally {
      setIsBatchProcessing(false)
    }
  }

  const handleSelectAlarm = useCallback((alarm: AlarmRecord) => {
    setLightboxAlarm(alarm)
  }, [])

  const handleSelectCrop = useCallback((alarm: AlarmRecord) => {
    setCropPreviewAlarm(alarm)
  }, [])

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

  // 根据搜索关键词进行即时多字段模糊匹配
  const filteredRecognitions = useMemo(() => {
    if (!searchQuery.trim()) return recognitions
    const q = searchQuery.trim().toLowerCase()
    return recognitions.filter((r) => {
      const name = (r.subjectName || '').toLowerCase()
      const subjectId = (r.subjectId || '').toLowerCase()
      const camId = (r.cameraId || '').toLowerCase()
      const camName = (cameraNameMap[r.cameraId] || '').toLowerCase()
      const recId = (r.recognitionId || '').toLowerCase()
      return (
        name.includes(q) ||
        subjectId.includes(q) ||
        camId.includes(q) ||
        camName.includes(q) ||
        recId.includes(q)
      )
    })
  }, [recognitions, searchQuery, cameraNameMap])

  const filteredAlarms = useMemo(() => {
    if (!searchQuery.trim()) return alarms
    const q = searchQuery.trim().toLowerCase()
    return alarms.filter((a) => {
      const evtId = (a.eventId || '').toLowerCase()
      const camId = (a.cameraId || '').toLowerCase()
      const camName = (cameraNameMap[a.cameraId] || '').toLowerCase()
      const target = (a.targetLabel || '').toLowerCase()
      const rule = (a.ruleType || '').toLowerCase()
      const alarmType = (a.alarmTypeId || '').toLowerCase()
      return (
        evtId.includes(q) ||
        camId.includes(q) ||
        camName.includes(q) ||
        target.includes(q) ||
        rule.includes(q) ||
        alarmType.includes(q)
      )
    })
  }, [alarms, searchQuery, cameraNameMap])

  const filteredCaptures = useMemo(() => {
    if (!searchQuery.trim()) return captures
    const q = searchQuery.trim().toLowerCase()
    return captures.filter((c) => {
      const capId = (c.captureId || '').toLowerCase()
      const camId = (c.cameraId || '').toLowerCase()
      const camName = (cameraNameMap[c.cameraId] || '').toLowerCase()
      const target = (c.targetLabel || '').toLowerCase()
      return capId.includes(q) || camId.includes(q) || camName.includes(q) || target.includes(q)
    })
  }, [captures, searchQuery, cameraNameMap])

  const isSearching = Boolean(searchQuery.trim())
  const currentFilteredCount =
    activeTab === 'recognition'
      ? filteredRecognitions.length
      : activeTab === 'captures'
        ? filteredCaptures.length
        : filteredAlarms.length

  const effectiveTotalCount = isSearching ? currentFilteredCount : totalCount
  const totalPages = Math.max(1, Math.ceil(effectiveTotalCount / pageSize))

  return (
    <div className="flex h-full flex-col gap-3 bg-[var(--bg-primary)] p-4 text-[var(--text-primary)] select-none">
      {/* 实时新告警浮条 */}
      <RealtimeAlarmBanner
        count={unreadRealtimeCount}
        onViewNew={() => {
          setUnreadRealtimeCount(0)
          handleResetFilters()
        }}
        onDismiss={() => setUnreadRealtimeCount(0)}
        t={t}
      />

      {/* 顶部控制栏与三重视图切换 */}
      <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl p-3 shadow-xs">
        <div className="flex items-center gap-3">
          <div
            className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl shadow-xs transition-colors duration-200 ${
              activeTab === 'recognition'
                ? 'bg-emerald-500/10 text-emerald-500'
                : activeTab === 'captures'
                  ? 'bg-cyan-500/10 text-cyan-500'
                  : 'bg-rose-500/10 text-rose-500'
            }`}
          >
            {activeTab === 'recognition' ? (
              <UserCheck className="h-5 w-5" />
            ) : activeTab === 'captures' ? (
              <CameraIcon className="h-5 w-5" />
            ) : (
              <ShieldAlert className="h-5 w-5" />
            )}
          </div>
          <div className="min-w-[170px]">
            <h2 className="text-sm font-semibold text-[var(--text-primary)]">{t('title')}</h2>
            <p className="text-xs text-[var(--text-muted)]">
              {t(`tabs.${activeTab}`)} · {t('subtitleSuffix')}
            </p>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* 声音告警开关 */}
          <motion.button
            type="button"
            whileTap={{ scale: 0.96 }}
            onClick={handleToggleSound}
            className={`flex items-center gap-1.5 rounded-xl border px-3 py-1.5 text-xs font-medium transition-all ${
              soundEnabled
                ? 'border-rose-500/30 bg-rose-500/10 text-rose-500 shadow-xs'
                : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-muted)] hover:text-[var(--text-primary)]'
            }`}
            title={soundEnabled ? t('sound.enabled') : t('sound.disabled')}
            aria-label={t('sound.toggleAlert')}
          >
            {soundEnabled ? (
              <Volume2 className="h-3.5 w-3.5 animate-pulse text-rose-500" />
            ) : (
              <VolumeX className="h-3.5 w-3.5" />
            )}
            <span className="hidden sm:inline">
              {soundEnabled ? t('sound.enabled') : t('sound.disabled')}
            </span>
          </motion.button>

          {/* 证据分类 Tab 切换器 (顺序: 识别对账 -> 违规告警 -> 轨迹抓拍) */}
          <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-xs">
            {(
              [
                {
                  key: 'recognition' as const,
                  label: t('tabs.recognition'),
                  icon: UserCheck,
                  activeClass: 'border-emerald-500/30 bg-emerald-500/15 text-emerald-500 shadow-xs',
                  iconColor: 'text-emerald-500',
                  badgeBg: 'bg-emerald-500/10 text-emerald-500',
                  badgeCount: activeTab === 'recognition' ? totalCount : tabCounts.recognition,
                },
                {
                  key: 'alarms' as const,
                  label: t('tabs.alarms'),
                  icon: AlertCircle,
                  activeClass: 'border-rose-500/30 bg-rose-500/15 text-rose-500 shadow-xs',
                  iconColor: 'text-rose-500',
                  badgeBg: 'bg-rose-500/10 text-rose-500',
                  badgeCount: activeTab === 'alarms' ? totalCount : tabCounts.alarms,
                },
                {
                  key: 'captures' as const,
                  label: t('tabs.captures'),
                  icon: CameraIcon,
                  activeClass: 'border-cyan-500/30 bg-cyan-500/15 text-cyan-500 shadow-xs',
                  iconColor: 'text-cyan-500',
                  badgeBg: 'bg-cyan-500/10 text-cyan-500',
                  badgeCount: activeTab === 'captures' ? totalCount : tabCounts.captures,
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
                  className={`flex items-center gap-1.5 rounded-lg border px-3 py-1.5 font-medium transition-colors duration-150 ${
                    isActive
                      ? tab.activeClass
                      : 'border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                  }`}
                >
                  <Icon className={`h-3.5 w-3.5 ${tab.iconColor}`} />
                  <span>{tab.label}</span>
                  {tab.badgeCount > 0 && (
                    <span
                      className={`py-0.2 ml-1 rounded-full px-1.5 font-mono text-[10px] font-bold tabular-nums ${
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
      </div>

      {/* 现代毛玻璃搜索与筛选控制工作台 (零抖动单行无缝排布) */}
      <div className="frosted-glass relative z-20 flex min-h-[52px] items-center justify-between gap-3 rounded-2xl p-2.5 shadow-xs">
        <div className="flex flex-1 [scrollbar-width:none] items-center gap-2 overflow-x-auto text-xs [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden">
          {/* 全能搜索框 (Omni-Search Bar) */}
          <div className="group/search relative flex min-w-[220px] flex-1 items-center sm:max-w-xs">
            <Search className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)] transition-colors group-focus-within/search:text-[var(--accent)]" />
            <input
              type="text"
              data-search-input="true"
              value={searchQuery}
              onChange={(e) => {
                setSearchQuery(e.target.value)
                setPage(1)
              }}
              placeholder={
                activeTab === 'recognition'
                  ? t('search.placeholderRecognition')
                  : activeTab === 'alarms'
                    ? t('search.placeholderAlarms')
                    : t('search.placeholderCaptures')
              }
              className="w-full rounded-xl border border-[var(--border)]/80 bg-[var(--bg-secondary)]/50 py-1.5 pr-8 pl-9 text-xs text-[var(--text-primary)] backdrop-blur-md transition-all outline-none placeholder:text-[var(--text-muted)] hover:border-[var(--border-strong)] focus:border-[var(--accent)] focus:bg-[var(--bg-surface)] focus:shadow-[0_0_16px_rgba(var(--accent-rgb),0.12)] focus:ring-2 focus:ring-[var(--accent)]/15"
            />
            {searchQuery ? (
              <button
                type="button"
                onClick={() => setSearchQuery('')}
                className="absolute top-1/2 right-2.5 -translate-y-1/2 rounded-md p-0.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                title={t('search.clear')}
              >
                <X className="h-3 w-3" />
              </button>
            ) : (
              <kbd className="py-0.2 pointer-events-none absolute top-1/2 right-2.5 hidden -translate-y-1/2 rounded-md border border-[var(--border)]/70 bg-[var(--bg-surface)]/70 px-1.5 font-mono text-[10px] text-[var(--text-muted)] shadow-2xs sm:inline-block">
                /
              </kbd>
            )}
          </div>

          <div className="hidden h-4 w-px bg-[var(--border)]/60 sm:block" />

          {/* 定制现代下拉筛选胶囊 */}
          {/* 通道筛选 */}
          <div className="group/sel relative inline-flex items-center">
            <select
              value={selectedCameraId}
              onChange={(e) => handleFilterChange(setSelectedCameraId, e.target.value)}
              className={`cursor-pointer appearance-none rounded-xl border py-1.5 pr-7 pl-3 text-xs font-medium backdrop-blur-md transition-all outline-none ${
                selectedCameraId
                  ? 'border-[var(--accent)]/50 bg-[var(--accent-soft)]/20 font-semibold text-[var(--accent)] shadow-2xs'
                  : 'border-[var(--border)]/70 bg-[var(--bg-surface)]/80 text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]'
              }`}
            >
              <option value="" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                {t('filter.allCameras')}
              </option>
              {cameras.map((c) => (
                <option
                  key={c.id}
                  value={c.cameraId}
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {c.name || c.cameraId}
                </option>
              ))}
            </select>
            <ChevronDown
              className={`pointer-events-none absolute top-1/2 right-2 h-3 w-3 -translate-y-1/2 transition-colors ${
                selectedCameraId
                  ? 'text-[var(--accent)]'
                  : 'text-[var(--text-muted)] group-hover/sel:text-[var(--text-primary)]'
              }`}
            />
          </div>

          {/* 动态目标类别筛选 */}
          {activeTab !== 'recognition' && (
            <div className="group/sel relative inline-flex items-center">
              <select
                value={selectedTargetLabel}
                onChange={(e) => handleFilterChange(setSelectedTargetLabel, e.target.value)}
                className={`cursor-pointer appearance-none rounded-xl border py-1.5 pr-7 pl-3 text-xs font-medium backdrop-blur-md transition-all outline-none ${
                  selectedTargetLabel
                    ? 'border-[var(--accent)]/50 bg-[var(--accent-soft)]/20 font-semibold text-[var(--accent)] shadow-2xs'
                    : 'border-[var(--border)]/70 bg-[var(--bg-surface)]/80 text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]'
                }`}
              >
                <option value="" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                  {t('filter.allTargets')}
                </option>
                {distinctTargetLabels.map((lbl) => (
                  <option
                    key={lbl}
                    value={lbl}
                    className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                  >
                    {lbl === 'person'
                      ? t('filter.person')
                      : lbl === 'car'
                        ? t('filter.car')
                        : lbl === 'bicycle'
                          ? t('filter.bicycle')
                          : lbl}
                  </option>
                ))}
              </select>
              <ChevronDown
                className={`pointer-events-none absolute top-1/2 right-2 h-3 w-3 -translate-y-1/2 transition-colors ${
                  selectedTargetLabel
                    ? 'text-[var(--accent)]'
                    : 'text-[var(--text-muted)] group-hover/sel:text-[var(--text-primary)]'
                }`}
              />
            </div>
          )}

          {/* 规则类型筛选 (仅违规告警生效) */}
          {activeTab === 'alarms' && (
            <div className="group/sel relative inline-flex items-center">
              <select
                value={selectedRuleType}
                onChange={(e) => handleFilterChange(setSelectedRuleType, e.target.value)}
                className={`cursor-pointer appearance-none rounded-xl border py-1.5 pr-7 pl-3 text-xs font-medium backdrop-blur-md transition-all outline-none ${
                  selectedRuleType !== 'all'
                    ? 'border-[var(--accent)]/50 bg-[var(--accent-soft)]/20 font-semibold text-[var(--accent)] shadow-2xs'
                    : 'border-[var(--border)]/70 bg-[var(--bg-surface)]/80 text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]'
                }`}
              >
                <option value="all" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                  {t('filter.allRuleTypes')}
                </option>
                <option value="roi" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                  {t('filter.ruleRoi')}
                </option>
                <option value="line" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                  {t('filter.ruleLine')}
                </option>
              </select>
              <ChevronDown
                className={`pointer-events-none absolute top-1/2 right-2 h-3 w-3 -translate-y-1/2 transition-colors ${
                  selectedRuleType !== 'all'
                    ? 'text-[var(--accent)]'
                    : 'text-[var(--text-muted)] group-hover/sel:text-[var(--text-primary)]'
                }`}
              />
            </div>
          )}

          {/* 严重级别筛选 */}
          {activeTab === 'alarms' && (
            <div className="group/sel relative inline-flex items-center">
              <select
                value={selectedSeverity}
                onChange={(e) => handleFilterChange(setSelectedSeverity, e.target.value)}
                className={`cursor-pointer appearance-none rounded-xl border py-1.5 pr-7 pl-3 text-xs font-medium backdrop-blur-md transition-all outline-none ${
                  selectedSeverity !== 'all'
                    ? 'border-[var(--accent)]/50 bg-[var(--accent-soft)]/20 font-semibold text-[var(--accent)] shadow-2xs'
                    : 'border-[var(--border)]/70 bg-[var(--bg-surface)]/80 text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]'
                }`}
              >
                <option value="all" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                  {t('filter.allSeverities')}
                </option>
                <option
                  value="warning"
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {t('filter.severityWarning')}
                </option>
                <option
                  value="critical"
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {t('filter.severityCritical')}
                </option>
              </select>
              <ChevronDown
                className={`pointer-events-none absolute top-1/2 right-2 h-3 w-3 -translate-y-1/2 transition-colors ${
                  selectedSeverity !== 'all'
                    ? 'text-[var(--accent)]'
                    : 'text-[var(--text-muted)] group-hover/sel:text-[var(--text-primary)]'
                }`}
              />
            </div>
          )}

          {/* 告警状态筛选 */}
          {activeTab === 'alarms' && (
            <div className="group/sel relative inline-flex items-center">
              <select
                value={selectedStatus}
                onChange={(e) => handleFilterChange(setSelectedStatus, e.target.value)}
                className={`cursor-pointer appearance-none rounded-xl border py-1.5 pr-7 pl-3 text-xs font-medium backdrop-blur-md transition-all outline-none ${
                  selectedStatus !== 'all'
                    ? 'border-[var(--accent)]/50 bg-[var(--accent-soft)]/20 font-semibold text-[var(--accent)] shadow-2xs'
                    : 'border-[var(--border)]/70 bg-[var(--bg-surface)]/80 text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]'
                }`}
              >
                <option value="all" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                  {t('statusFilter.all')}
                </option>
                <option
                  value="unprocessed"
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {t('statusFilter.unprocessed')}
                </option>
                <option
                  value="processed"
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {t('statusFilter.processed')}
                </option>
              </select>
              <ChevronDown
                className={`pointer-events-none absolute top-1/2 right-2 h-3 w-3 -translate-y-1/2 transition-colors ${
                  selectedStatus !== 'all'
                    ? 'text-[var(--accent)]'
                    : 'text-[var(--text-muted)] group-hover/sel:text-[var(--text-primary)]'
                }`}
              />
            </div>
          )}

          {/* 识别对账状态筛选 */}
          {activeTab === 'recognition' && (
            <div className="group/sel relative inline-flex items-center">
              <select
                value={selectedStatus}
                onChange={(e) => handleFilterChange(setSelectedStatus, e.target.value)}
                className={`cursor-pointer appearance-none rounded-xl border py-1.5 pr-7 pl-3 text-xs font-medium backdrop-blur-md transition-all outline-none ${
                  selectedStatus !== 'all'
                    ? 'border-[var(--accent)]/50 bg-[var(--accent-soft)]/20 font-semibold text-[var(--accent)] shadow-2xs'
                    : 'border-[var(--border)]/70 bg-[var(--bg-surface)]/80 text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]'
                }`}
              >
                <option value="all" className="bg-[var(--bg-surface)] text-[var(--text-primary)]">
                  {t('statusFilter.all')}
                </option>
                <option
                  value="confirmed"
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {t('statusFilter.confirmed')}
                </option>
                <option
                  value="pending_review"
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {t('statusFilter.pendingReview')}
                </option>
                <option
                  value="rejected"
                  className="bg-[var(--bg-surface)] text-[var(--text-primary)]"
                >
                  {t('statusFilter.rejected')}
                </option>
              </select>
              <ChevronDown
                className={`pointer-events-none absolute top-1/2 right-2 h-3 w-3 -translate-y-1/2 transition-colors ${
                  selectedStatus !== 'all'
                    ? 'text-[var(--accent)]'
                    : 'text-[var(--text-muted)] group-hover/sel:text-[var(--text-primary)]'
                }`}
              />
            </div>
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

          {/* 重置全部筛选与搜索徽章 */}
          {hasActiveFilters && (
            <motion.button
              type="button"
              whileTap={{ scale: 0.95 }}
              onClick={handleResetFilters}
              className="flex items-center gap-1.5 rounded-xl border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs font-semibold text-rose-500 shadow-2xs backdrop-blur-md transition-all hover:border-rose-500/60 hover:bg-rose-500/20"
              title={t('filter.reset')}
            >
              <RotateCcw className="h-3 w-3" />
              <span>{t('filter.reset')}</span>
              <span className="py-0.2 rounded-full bg-rose-500/20 px-1.5 font-mono text-[10px] font-bold text-rose-400">
                {activeFilterCount}
              </span>
            </motion.button>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {/* 卡片与表格视图切换 (三 Tab 全面统一支持，消除按钮跳跃) */}
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

          <button
            type="button"
            onClick={handleRefresh}
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

      {/* 主视图内容区 (带平滑淡入微动效，表格模式下无冗余内边距与双重卡片嵌套) */}
      <div
        key={activeTab}
        className={`frosted-glass animate-tab-fade flex-1 overflow-auto rounded-2xl shadow-xs ${
          viewMode === 'table' ? 'p-0' : 'p-4'
        }`}
      >
        {activeTab === 'alarms' && (
          <AlarmsContent
            alarms={filteredAlarms}
            totalCount={totalCount}
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            selectedAlarmIds={selectedAlarmIds}
            hasActiveFilters={hasActiveFilters}
            searchQuery={searchQuery}
            onResetFilters={handleResetFilters}
            onClearSearch={() => setSearchQuery('')}
            onToggleSelectAlarm={handleToggleSelectAlarm}
            onToggleSelectAll={handleToggleSelectAll}
            onSelect={handleSelectAlarm}
            onSelectCrop={handleSelectCrop}
            onToggleStatus={handleToggleAlarmStatus}
            t={t}
          />
        )}

        {activeTab === 'captures' && (
          <CapturesContent
            captures={filteredCaptures}
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            hasActiveFilters={hasActiveFilters}
            searchQuery={searchQuery}
            onResetFilters={handleResetFilters}
            onClearSearch={() => setSearchQuery('')}
            onSelect={setLightboxCapture}
            t={t}
          />
        )}

        {activeTab === 'recognition' && (
          <RecognitionContent
            recognitions={filteredRecognitions}
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            hasActiveFilters={hasActiveFilters}
            searchQuery={searchQuery}
            onResetFilters={handleResetFilters}
            onClearSearch={() => setSearchQuery('')}
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
          {isSearching ? (
            <span className="font-mono font-semibold text-emerald-500">
              ({t('search.pageFiltered', { count: currentFilteredCount })})
            </span>
          ) : (
            totalCount > 0 && (
              <span className="font-mono text-[var(--text-muted)]">
                ({t('pagination.total', { total: totalCount })})
              </span>
            )
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

      {/* 告警大图灯箱 Modal (高精度 BBox + 滚轮平移缩放 + 特写画中画 + Esc 退出) */}
      <AnimatePresence>
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
      </AnimatePresence>

      {/* 抓拍大图灯箱 Modal */}
      <AnimatePresence>
        {lightboxCapture && (
          <CaptureLightboxModal
            capture={lightboxCapture}
            cameraName={cameraNameMap[lightboxCapture.cameraId]}
            onClose={() => setLightboxCapture(null)}
            t={t}
          />
        )}
      </AnimatePresence>

      {/* 识别对账 Top-5 候选人核验 Modal */}
      <AnimatePresence>
        {reviewModalRec && (
          <RecognitionReviewModal
            recognition={reviewModalRec}
            cameraName={cameraNameMap[reviewModalRec.cameraId]}
            onClose={() => setReviewModalRec(null)}
            onReview={handleReviewRecognition}
            t={t}
          />
        )}
      </AnimatePresence>

      {/* 特写大图灯箱 Modal */}
      <AnimatePresence>
        {cropPreviewAlarm && (
          <CropLightboxModal
            alarm={cropPreviewAlarm}
            onClose={() => setCropPreviewAlarm(null)}
            t={t}
          />
        )}
      </AnimatePresence>
    </div>
  )
}
