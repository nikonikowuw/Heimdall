import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  AlertCircle,
  Camera as CameraIcon,
  LayoutGrid,
  List,
  RotateCcw,
  UserCheck,
  Volume2,
  VolumeX,
  X,
  type LucideIcon,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { DateTimeRangePicker } from '@/components/DateTimeRangePicker'
import { RefreshButton } from '@/components/RefreshButton'
import { SearchInput } from '@/components/ui/SearchInput'
import { SelectField } from '@/components/ui/SelectField'
import { useDebounce } from '@/hooks/use-debounce'
import { alarmApi, cameraApi, evidenceApi } from '@/lib/api'
import { resolveEffectiveTimeRange, type DateTimeRangeValue } from '@/lib/dateRange'
import { cn } from '@/lib/utils'
import { wsClient } from '@/lib/wsClient'
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
} from '@/types'
import { AlarmLightboxModal } from './components/AlarmLightboxModal'
import { AlarmsContent, type ViewMode } from './components/AlarmsContent'
import { BatchActionBar } from './components/BatchActionBar'
import { CaptureLightboxModal } from './components/CaptureLightboxModal'
import { CapturesContent } from './components/CapturesContent'
import { CropLightboxModal } from './components/CropLightboxModal'
import { RealtimeAlarmBanner } from './components/RealtimeAlarmBanner'
import { RecognitionContent } from './components/RecognitionContent'
import { RecognitionReviewModal } from './components/RecognitionReviewModal'
import { PageHeader } from '@/components/ui/PageHeader'
import {
  FILTER_ALL,
  type AlarmStatusFilter,
  type RecognitionStatusFilter,
  type RuleTypeFilter,
  type SeverityFilter,
  type TargetLabelFilter,
} from './filters'
import { isAlarmSoundEnabled, playAlarmAlertSound, setAlarmSoundEnabled } from './sound'
import { matchesSearchTerm } from './utils'

type EvidenceTab = 'recognition' | 'alarms' | 'captures'

interface EvidenceTabSpec {
  key: EvidenceTab
  labelKey: `tabs.${EvidenceTab}`
  /**
   * Tab 图标：标题栏与药丸共用同一个。
   * 违规告警用 AlertCircle 对齐左侧全局导航栏的页面身份；
   * ShieldAlert 在本仓已归属「屏蔽遮罩」绘图工具与告警状态徽标，不复用为页面/Tab 身份。
   */
  icon: LucideIcon
  /** 激活态边框/底色/文字三件套，同时用于标题栏图标外壳与药丸 Tab */
  activeClass: string
  /** 图标随状态着色（未激活时也保留语义色） */
  iconClass: string
  /** 激活态计数徽章底色 */
  badgeClass: string
}

/**
 * 证据三支柱的统一定义。图标、配色与激活样式只在此声明一次，
 * 标题栏与药丸 Tab 共用，避免同一套条件分支在两处各写一遍。
 */
const EVIDENCE_TABS: readonly EvidenceTabSpec[] = [
  {
    key: 'recognition',
    labelKey: 'tabs.recognition',
    icon: UserCheck,
    activeClass:
      'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)] shadow-xs',
    iconClass: 'text-[var(--status-success)]',
    badgeClass: 'bg-[var(--status-success-soft)] text-[var(--status-success)]',
  },
  {
    key: 'alarms',
    labelKey: 'tabs.alarms',
    icon: AlertCircle,
    activeClass:
      'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)] shadow-xs',
    iconClass: 'text-[var(--status-danger)]',
    badgeClass: 'bg-[var(--status-danger-soft)] text-[var(--status-danger)]',
  },
  {
    key: 'captures',
    labelKey: 'tabs.captures',
    icon: CameraIcon,
    activeClass:
      'border-[var(--status-info-border)] bg-[var(--status-info-soft)] text-[var(--status-info)] shadow-xs',
    iconClass: 'text-[var(--status-info)]',
    badgeClass: 'bg-[var(--status-info-soft)] text-[var(--status-info)]',
  },
]

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

/**
 * 实时事件回调依赖的最新过滤快照。
 *
 * 关键字与通道名映射只参与「新事件是否应插入当前列表」的判定，用 ref 承载即可；
 * 若放进订阅 effect 的依赖数组，每次按键和每次通道列表刷新都会拆建 4 条订阅。
 */
interface LiveFilterSnapshot {
  searchQuery: string
  cameraNameMap: Record<string, string>
}

function isAbortError(error: unknown): boolean {
  return (
    typeof error === 'object' && error !== null && 'name' in error && error.name === 'AbortError'
  )
}

export function AlarmsPage(): React.ReactElement {
  const { t } = useTranslation('alarm')
  const [activeTab, setActiveTab] = useState<EvidenceTab>('recognition')
  const [viewMode, setViewMode] = useState<ViewMode>('cards')

  // 基础数据与通道
  const [cameras, setCameras] = useState<Camera[]>([])
  const [selectedCameraId, setSelectedCameraId] = useState<string>('')
  const [selectedTargetLabel, setSelectedTargetLabel] = useState<string>('')
  const [selectedRuleType, setSelectedRuleType] = useState<RuleTypeFilter>(FILTER_ALL)
  const [selectedSeverity, setSelectedSeverity] = useState<SeverityFilter>(FILTER_ALL)
  /*
   * 告警与识别的处理状态分开持有。二者取值域无交集（`unprocessed`/`processed` vs
   *`confirmed`/`pending_review`/`rejected`），共用一份 `string` state 会让
   * 「识别状态下拉出现告警状态选项」成为可编译的写法，并迫使调用点用 `as` 断言
   * 把 `string` 硬塞回字面量联合 —— 断言正是选项表与类型漂移时静默失效的入口。
   */
  const [alarmStatus, setAlarmStatus] = useState<AlarmStatusFilter>(FILTER_ALL)
  const [recognitionStatus, setRecognitionStatus] = useState<RecognitionStatusFilter>(FILTER_ALL)
  // 轨道过滤：与机位联合生效（track_id 仅在单机位追踪器内唯一），用于回看同一个人的一次通行
  const [selectedTrackId, setSelectedTrackId] = useState<number | null>(null)
  const [searchQuery, setSearchQuery] = useState<string>('')
  const debouncedSearchQuery = useDebounce(searchQuery, 300)

  // 时间维度筛选 (默认查询当前最新记录 - 今天)
  const [timeRange, setTimeRange] = useState<DateTimeRangeValue>(getInitialTodayRange)

  // 声音告警开关
  const [soundEnabled, setSoundEnabled] = useState<boolean>(isAlarmSoundEnabled)

  // 分页与总数 (支持动态选择每页条数)
  const [page, setPage] = useState<number>(1)
  const [pageSize, setPageSize] = useState<number>(24)
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
  const dataAbortControllerRef = useRef<AbortController | null>(null)
  // 实时事件回调读取的最新过滤条件（见 LiveFilterSnapshot）
  const liveFilterRef = useRef<LiveFilterSnapshot>({ searchQuery: '', cameraNameMap: {} })

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

  // 供 WS 订阅回调读取最新过滤条件，避免把高频变化的值放进订阅依赖数组
  useEffect(() => {
    liveFilterRef.current = { searchQuery, cameraNameMap }
  }, [searchQuery, cameraNameMap])

  /**
   * 目标类别筛选的候选项。
   *
   * 后端没有 distinct 聚合接口，只能从当前页已加载的记录里汇聚；因此这个列表天然
   * 随筛选与翻页而变。已选中的 `selectedTargetLabel` 必须并回候选项，否则它一旦
   * 离开当前页就会被选不中，而请求仍带着该值 —— 筛选在生效、界面却看不到也清不掉。
   * （SelectField 会在选项缺失时补占位 option，但它只能展示原始代号，补在这里
   * 才能拿到 `filter.*` 的本地化文案。）
   */
  const distinctTargetLabels = useMemo(() => {
    const set = new Set<string>(['person', 'face', 'car', 'bicycle'])
    if (selectedTargetLabel) set.add(selectedTargetLabel)
    for (const a of alarms) {
      if (a.targetLabel?.trim()) set.add(a.targetLabel.trim())
    }
    for (const c of captures) {
      if (c.targetLabel?.trim()) set.add(c.targetLabel.trim())
    }
    return Array.from(set)
  }, [alarms, captures, selectedTargetLabel])

  /**
   * 当前 Tab 生效的状态筛选。
   * 两个 Tab 各自持有独立取值，此处只做投影，避免请求构造处再分流分支。
   */
  const activeStatusFilter = activeTab === 'alarms' ? alarmStatus : recognitionStatus

  const activeFilterCount = useMemo(() => {
    let count = 0
    if (searchQuery.trim()) count++
    if (selectedCameraId) count++
    // 目标类别仅告警与抓拍两个 Tab 生效（与控件渲染条件一致）：识别请求不携带该参数，
    // 若在此处无条件计数，会出现「重置角标显示 1 项筛选，界面上却找不到对应控件」。
    if (activeTab !== 'recognition' && selectedTargetLabel) count++
    if (activeTab === 'alarms' && selectedRuleType !== FILTER_ALL) count++
    if (activeTab === 'alarms' && selectedSeverity !== FILTER_ALL) count++
    if (activeStatusFilter !== FILTER_ALL) count++
    if (selectedTrackId !== null) count++
    if (timeRange.quickPreset !== 'today') count++
    return count
  }, [
    searchQuery,
    selectedCameraId,
    selectedTargetLabel,
    activeTab,
    selectedRuleType,
    selectedSeverity,
    activeStatusFilter,
    selectedTrackId,
    timeRange.quickPreset,
  ])

  // 是否存在活跃的非默认过滤条件
  const hasActiveFilters = activeFilterCount > 0

  const handleResetFilters = useCallback(() => {
    setSearchQuery('')
    setSelectedCameraId('')
    setSelectedTargetLabel('')
    setSelectedRuleType(FILTER_ALL)
    setSelectedSeverity(FILTER_ALL)
    setAlarmStatus(FILTER_ALL)
    setRecognitionStatus(FILTER_ALL)
    setSelectedTrackId(null)
    setTimeRange(getInitialTodayRange())
    setPage(1)
  }, [])

  /**
   * 点击轨道号：按「机位 + 轨道」回看同一个人的一次通行。
   *
   * 轨道号只在单机位追踪器内唯一，因此必须同时锁定机位，否则会把其他通道的
   * 同号轨道混进同一次「行迹」里。筛选在服务端执行（列表分页 + 客户端过滤会截断结果）。
   */
  const handleSelectTrack = useCallback((trackId: number, cameraId: string) => {
    setSelectedTrackId(trackId)
    if (cameraId) setSelectedCameraId(cameraId)
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
    // 状态筛选按 Tab 清空：告警与识别的取值域无交集，保留旧值只会让下一帧请求带上非法参数
    setAlarmStatus(FILTER_ALL)
    setRecognitionStatus(FILTER_ALL)
    setSelectedTrackId(null)
    setSelectedRuleType(FILTER_ALL)
    pendingAlarmsRef.current = []
    pendingCountRef.current = 0
    setTotalCount(tabCounts[tab] || 0)
    setPage(1)
  }

  // 数据加载函数
  const loadData = useCallback(async () => {
    // 代次 + AbortController 只保留一套机制：控制器身份即「本次请求是否为当前请求」，
    // 被新请求接管（ref 换成新控制器）或已 abort 时一律不再写状态。
    dataAbortControllerRef.current?.abort()
    const controller = new AbortController()
    dataAbortControllerRef.current = controller
    const { signal } = controller
    const isCurrentRequest = (): boolean =>
      dataAbortControllerRef.current === controller && !signal.aborted

    setIsLoading(true)
    setErrorMessage(null)
    try {
      const camId = selectedCameraId || undefined
      const targetLbl = selectedTargetLabel || undefined
      const ruleTypeParam = selectedRuleType === FILTER_ALL ? undefined : selectedRuleType
      const severityParam = selectedSeverity === FILTER_ALL ? undefined : selectedSeverity
      const statusParam = activeStatusFilter === FILTER_ALL ? undefined : activeStatusFilter
      const keyword = debouncedSearchQuery.trim() || undefined
      const { startTime: startMs, endTime: endMs } = resolveEffectiveTimeRange(timeRange)
      const offset = (page - 1) * pageSize

      // 各分支只负责取数与投放本页数据；总数与徽标在同处结算，避免三份重复守卫
      let refreshedTotal: number | null = null

      if (activeTab === 'alarms') {
        const [list, countRes] = await Promise.all([
          alarmApi.list(
            {
              cameraId: camId,
              status: statusParam,
              targetLabel: targetLbl,
              ruleType: ruleTypeParam,
              severity: severityParam,
              q: keyword,
              startTime: startMs,
              endTime: endMs,
              limit: pageSize,
              offset,
            },
            signal,
          ),
          alarmApi.count(
            {
              cameraId: camId,
              status: statusParam,
              targetLabel: targetLbl,
              ruleType: ruleTypeParam,
              severity: severityParam,
              q: keyword,
              startTime: startMs,
              endTime: endMs,
            },
            signal,
          ),
        ])
        if (!isCurrentRequest()) return
        setAlarms(list)
        refreshedTotal = countRes.total
      } else if (activeTab === 'captures') {
        const [list, countRes] = await Promise.all([
          evidenceApi.listCaptures(
            {
              cameraId: camId,
              targetLabel: targetLbl,
              q: keyword,
              trackId: selectedTrackId ?? undefined,
              startTime: startMs,
              endTime: endMs,
              limit: pageSize,
              offset,
            },
            signal,
          ),
          evidenceApi.countCaptures(
            {
              cameraId: camId,
              targetLabel: targetLbl,
              q: keyword,
              trackId: selectedTrackId ?? undefined,
              startTime: startMs,
              endTime: endMs,
            },
            signal,
          ),
        ])
        if (!isCurrentRequest()) return
        setCaptures(list)
        refreshedTotal = countRes.total
      } else if (activeTab === 'recognition') {
        const [list, countRes] = await Promise.all([
          evidenceApi.listRecognitions(
            {
              cameraId: camId,
              status: statusParam,
              q: keyword,
              startTime: startMs,
              endTime: endMs,
              limit: pageSize,
              offset,
            },
            signal,
          ),
          evidenceApi.countRecognitions(
            {
              cameraId: camId,
              status: statusParam,
              q: keyword,
              startTime: startMs,
              endTime: endMs,
            },
            signal,
          ),
        ])
        if (!isCurrentRequest()) return
        setRecognitions(list)
        refreshedTotal = countRes.total
      }
      if (refreshedTotal === null) return
      setTotalCount(refreshedTotal)
      // 关键字生效时 totalCount 是筛选命中数，不得污染「未筛选总数」徽标
      if (!keyword) {
        setTabCounts((prev) => ({ ...prev, [activeTab]: refreshedTotal }))
      }
      setSelectedAlarmIds(new Set())
    } catch (err) {
      if (!isCurrentRequest() || isAbortError(err)) return
      setErrorMessage(err instanceof Error ? err.message : String(err))
    } finally {
      // 仅当前请求能结束加载态：被接管的请求在到达这里前已将 ref 交给新控制器
      if (dataAbortControllerRef.current === controller) {
        dataAbortControllerRef.current = null
        setIsLoading(false)
      }
    }
  }, [
    activeTab,
    selectedCameraId,
    selectedTargetLabel,
    selectedRuleType,
    selectedSeverity,
    activeStatusFilter,
    selectedTrackId,
    timeRange,
    debouncedSearchQuery,
    page,
    pageSize,
  ])

  useEffect(() => {
    void loadData()
    return () => {
      dataAbortControllerRef.current?.abort()
    }
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
      const matchesRule = selectedRuleType === FILTER_ALL || selectedRuleType === p.ruleType
      const matchesSeverity = selectedSeverity === FILTER_ALL || selectedSeverity === p.severity
      const matchesStatus = alarmStatus === FILTER_ALL || alarmStatus === 'unprocessed'
      const isLiveTime = matchesTimeRange(timeRange, p.occurredAt)
      const matchesSearch = matchesSearchTerm(liveFilterRef.current.searchQuery, [
        p.eventId,
        p.cameraId,
        p.cameraName,
        p.targetLabel,
        p.ruleType,
        p.alarmTypeId,
      ])

      if (
        page === 1 &&
        matchesCamera &&
        matchesTarget &&
        matchesRule &&
        matchesSeverity &&
        matchesStatus &&
        isLiveTime &&
        matchesSearch
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
        const matchesStatus = recognitionStatus === FILTER_ALL || recognitionStatus === p.status
        const isLiveTime = matchesTimeRange(timeRange, p.recognizedAt)
        const matchesSearch = matchesSearchTerm(liveFilterRef.current.searchQuery, [
          p.subjectName,
          p.subjectId,
          p.cameraId,
          liveFilterRef.current.cameraNameMap[p.cameraId],
          p.recognitionId,
        ])

        if (page === 1 && matchesCamera && matchesStatus && isLiveTime && matchesSearch) {
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
    alarmStatus,
    recognitionStatus,
    soundEnabled,
    timeRange,
    page,
    pageSize,
  ])

  // 显式点击刷新处理
  const handleRefresh = useCallback(() => {
    void loadData()

    const { startTime: startMs, endTime: endMs } = resolveEffectiveTimeRange(timeRange)
    const camId = selectedCameraId || undefined
    const targetLbl = selectedTargetLabel || undefined
    const ruleTypeParam = selectedRuleType === FILTER_ALL ? undefined : selectedRuleType
    const severityParam = selectedSeverity === FILTER_ALL ? undefined : selectedSeverity
    const statusParam = activeStatusFilter === FILTER_ALL ? undefined : activeStatusFilter

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
    activeStatusFilter,
  ])

  // 单条告警状态切换
  const handleToggleAlarmStatus = useCallback(
    async (alarm: AlarmRecord): Promise<void> => {
      const nextStatus: AlarmStatus = alarm.status === 'processed' ? 'unprocessed' : 'processed'
      try {
        const updated = await alarmApi.updateStatus(alarm.id, nextStatus)
        setAlarms((prev) => prev.map((a) => (a.id === alarm.id ? updated : a)))
        setLightboxAlarm((prev) => (prev && prev.id === alarm.id ? updated : prev))
        if (alarmStatus !== FILTER_ALL) {
          void loadData()
        }
      } catch (err) {
        setErrorMessage(err instanceof Error ? err.message : String(err))
      }
    },
    [alarmStatus, loadData],
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
      if (alarmStatus !== FILTER_ALL) {
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

  const totalPages = Math.max(1, Math.ceil(totalCount / pageSize))
  // EVIDENCE_TABS 覆盖全部 EvidenceTab，回退分支仅为满足类型收窄
  const activeTabSpec = EVIDENCE_TABS.find((tab) => tab.key === activeTab) ?? EVIDENCE_TABS[0]

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 text-[var(--text-primary)] select-none">
      {/* 顶部控制栏与三重视图切换 */}
      <PageHeader
        icon={activeTabSpec.icon}
        iconClassName={cn('transition-colors duration-200', activeTabSpec.activeClass)}
        title={t('title')}
        subtitle={`${t(activeTabSpec.labelKey)} · ${t('subtitleSuffix')}`}
        actions={
          <>
            {/* 声音告警开关 */}
            <motion.button
              type="button"
              whileTap={{ scale: 0.96 }}
              onClick={handleToggleSound}
              className={`flex items-center gap-1.5 rounded-xl border px-3 py-1.5 text-xs font-medium transition-all ${
                soundEnabled
                  ? 'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)] shadow-xs'
                  : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-muted)] hover:text-[var(--text-primary)]'
              }`}
              title={soundEnabled ? t('sound.enabled') : t('sound.disabled')}
              aria-label={t('sound.toggleAlert')}
            >
              {soundEnabled ? (
                <Volume2 className="h-3.5 w-3.5 text-[var(--status-danger)]" />
              ) : (
                <VolumeX className="h-3.5 w-3.5" />
              )}
              <span className="hidden sm:inline">
                {soundEnabled ? t('sound.enabled') : t('sound.disabled')}
              </span>
            </motion.button>

            {/* 证据分类 Tab 切换器 (顺序: 识别对账 -> 违规告警 -> 轨迹抓拍) */}
            <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-xs">
              {EVIDENCE_TABS.map((tab) => {
                const Icon = tab.icon
                const isActive = activeTab === tab.key
                const badgeCount = tabCounts[tab.key]
                return (
                  <button
                    key={tab.key}
                    type="button"
                    onClick={() => handleSwitchTab(tab.key)}
                    className={cn(
                      'flex items-center gap-1.5 rounded-lg border px-3 py-1.5 font-medium transition-colors duration-150',
                      isActive
                        ? tab.activeClass
                        : 'border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)]',
                    )}
                  >
                    <Icon className={cn('h-3.5 w-3.5', tab.iconClass)} />
                    <span>{t(tab.labelKey)}</span>
                    {badgeCount > 0 && (
                      <span
                        className={cn(
                          'ml-1 rounded-full px-1.5 font-mono text-[10px] font-bold tabular-nums',
                          isActive
                            ? tab.badgeClass
                            : 'bg-[var(--bg-secondary)] text-[var(--text-muted)]',
                        )}
                      >
                        {badgeCount}
                      </span>
                    )}
                  </button>
                )
              })}
            </div>
          </>
        }
      />

      {/* 实时新告警浮条（置于标题栏下方，不推动顶部锚定位置） */}
      <RealtimeAlarmBanner
        count={unreadRealtimeCount}
        onViewNew={() => {
          setUnreadRealtimeCount(0)
          handleResetFilters()
        }}
        onDismiss={() => setUnreadRealtimeCount(0)}
        t={t}
      />

      {/* 现代毛玻璃搜索与筛选控制工作台 (零抖动单行无缝排布) */}
      <div className="frosted-glass relative z-20 flex min-h-[52px] items-center justify-between gap-3 rounded-2xl p-2.5 shadow-xs">
        <div className="flex flex-1 [scrollbar-width:none] items-center gap-2 overflow-x-auto text-xs [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden">
          {/* 全能搜索框 (Omni-Search Bar) */}
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
            placeholder={
              activeTab === 'recognition'
                ? t('search.placeholderRecognition')
                : activeTab === 'alarms'
                  ? t('search.placeholderAlarms')
                  : t('search.placeholderCaptures')
            }
            clearAriaLabel={t('search.clear')}
            containerClassName="min-w-[220px] flex-1 sm:max-w-xs"
          />

          <div className="hidden h-4 w-px bg-[var(--border)]/60 sm:block" />

          {/* 筛选下拉：维度、选项与可访问名称由 SelectField 统一承载，
 强调态表示该维度已收敛当前视图 */}
          <SelectField
            label={t('filter.allCameras')}
            value={selectedCameraId}
            emphasis={Boolean(selectedCameraId)}
            onChange={(cameraId) => {
              setSelectedCameraId(cameraId)
              // 轨道号仅在单机位追踪器内唯一，换通道后必须一并清除
              setSelectedTrackId(null)
              setPage(1)
            }}
            allOption={{ value: '', label: t('filter.allCameras') }}
            options={cameras.map((camera) => ({
              value: camera.cameraId,
              label: camera.name || camera.cameraId,
            }))}
          />

          {/* 动态目标类别筛选。候选项由当前视图已出现的标签汇聚而来 */}
          {activeTab !== 'recognition' && (
            <SelectField<TargetLabelFilter>
              label={t('filter.allTargets')}
              value={selectedTargetLabel}
              emphasis={Boolean(selectedTargetLabel)}
              onChange={(label) => {
                setSelectedTargetLabel(label)
                setPage(1)
              }}
              allOption={{ value: '', label: t('filter.allTargets') }}
              options={distinctTargetLabels.map((label) => ({
                value: label,
                label: t(`filter.${label}`, { defaultValue: label }),
              }))}
            />
          )}

          {/* 规则类型筛选 (仅违规告警生效) */}
          {activeTab === 'alarms' && (
            <SelectField<RuleTypeFilter>
              label={t('filter.allRuleTypes')}
              value={selectedRuleType}
              emphasis={selectedRuleType !== FILTER_ALL}
              onChange={(ruleType) => {
                setSelectedRuleType(ruleType)
                setPage(1)
              }}
              options={[
                { value: FILTER_ALL, label: t('filter.allRuleTypes') },
                { value: 'roi', label: t('filter.ruleRoi') },
                { value: 'line', label: t('filter.ruleLine') },
              ]}
            />
          )}

          {/* 严重级别筛选 */}
          {activeTab === 'alarms' && (
            <SelectField<SeverityFilter>
              label={t('filter.allSeverities')}
              value={selectedSeverity}
              emphasis={selectedSeverity !== FILTER_ALL}
              onChange={(severity) => {
                setSelectedSeverity(severity)
                setPage(1)
              }}
              options={[
                { value: FILTER_ALL, label: t('filter.allSeverities') },
                { value: 'warning', label: t('filter.severityWarning') },
                { value: 'critical', label: t('filter.severityCritical') },
              ]}
            />
          )}

          {/*
 处理状态筛选。告警与识别各自持有一份类型化 state，
 因此无需任何`as` 断言，也不可能把一侧的取值域泄露到另一侧。
 */}
          {activeTab === 'alarms' && (
            <SelectField<AlarmStatusFilter>
              label={t('statusFilter.all')}
              value={alarmStatus}
              emphasis={alarmStatus !== FILTER_ALL}
              onChange={(status) => {
                setAlarmStatus(status)
                setPage(1)
              }}
              options={[
                { value: FILTER_ALL, label: t('statusFilter.all') },
                { value: 'unprocessed', label: t('statusFilter.unprocessed') },
                { value: 'processed', label: t('statusFilter.processed') },
              ]}
            />
          )}

          {activeTab === 'recognition' && (
            <SelectField<RecognitionStatusFilter>
              label={t('statusFilter.all')}
              value={recognitionStatus}
              emphasis={recognitionStatus !== FILTER_ALL}
              onChange={(status) => {
                setRecognitionStatus(status)
                setPage(1)
              }}
              options={[
                { value: FILTER_ALL, label: t('statusFilter.all') },
                { value: 'confirmed', label: t('statusFilter.confirmed') },
                { value: 'pending_review', label: t('statusFilter.pendingReview') },
                { value: 'rejected', label: t('statusFilter.rejected') },
              ]}
            />
          )}

          {activeTab === 'recognition' && (
            <SelectField<RecognitionStatusFilter>
              label={t('statusFilter.all')}
              value={recognitionStatus}
              emphasis={recognitionStatus !== FILTER_ALL}
              onChange={(status) => {
                setRecognitionStatus(status)
                setPage(1)
              }}
              options={[
                { value: FILTER_ALL, label: t('statusFilter.all') },
                { value: 'confirmed', label: t('statusFilter.confirmed') },
                { value: 'pending_review', label: t('statusFilter.pendingReview') },
                { value: 'rejected', label: t('statusFilter.rejected') },
              ]}
            />
          )}

          {/* 轨道筛选徽标：仅抓拍 Tab 会出现取值 */}
          {activeTab === 'captures' && selectedTrackId !== null && (
            <button
              type="button"
              onClick={() => {
                setSelectedTrackId(null)
                setPage(1)
              }}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--status-info-border)] bg-[var(--status-info-soft)] px-2.5 py-1.5 font-mono text-xs font-semibold text-[var(--status-info)] shadow-2xs backdrop-blur-md transition-all hover:border-[var(--status-info-border)] hover:bg-[var(--status-info-soft)]"
              title={t('trackFilter.clear')}
            >
              <span>{t('trackFilter.active', { trackId: selectedTrackId })}</span>
              <X className="h-3 w-3" />
            </button>
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
              className="flex items-center gap-1.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] px-2.5 py-1.5 text-xs font-semibold text-[var(--status-danger)] shadow-2xs backdrop-blur-md transition-all hover:border-[var(--status-danger-border)] hover:bg-[var(--status-danger-soft)]"
              title={t('filter.reset')}
            >
              <RotateCcw className="h-3 w-3" />
              <span>{t('filter.reset')}</span>
              <span className="rounded-full bg-[var(--status-danger-soft)] px-1.5 font-mono text-[10px] font-bold text-[var(--status-danger)]">
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

          <RefreshButton onClick={handleRefresh} loading={isLoading} label={t('filter.refresh')} />
        </div>
      </div>

      {/* 错误提示 */}
      {errorMessage && (
        <div className="flex items-center gap-2 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3 text-xs text-[var(--status-danger)]">
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span>{errorMessage}</span>
        </div>
      )}

      {/* 主视图内容区 (带平滑淡入微动效，卡片模式纯通透呼吸感，表格模式紧凑包裹) */}
      <div
        key={activeTab}
        className={`animate-tab-fade flex-1 overflow-auto ${
          viewMode === 'table' ? 'frosted-glass rounded-2xl p-0 shadow-xs' : 'pr-1'
        }`}
      >
        {activeTab === 'alarms' && (
          <AlarmsContent
            alarms={alarms}
            totalCount={totalCount}
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            selectedAlarmIds={selectedAlarmIds}
            hasActiveFilters={hasActiveFilters}
            searchQuery={searchQuery}
            onResetFilters={handleResetFilters}
            onClearSearch={() => {
              setSearchQuery('')
              setPage(1)
            }}
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
            captures={captures}
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            hasActiveFilters={hasActiveFilters}
            searchQuery={searchQuery}
            onResetFilters={handleResetFilters}
            onClearSearch={() => {
              setSearchQuery('')
              setPage(1)
            }}
            onSelect={setLightboxCapture}
            onSelectTrack={handleSelectTrack}
            t={t}
          />
        )}

        {activeTab === 'recognition' && (
          <RecognitionContent
            recognitions={recognitions}
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            hasActiveFilters={hasActiveFilters}
            searchQuery={searchQuery}
            onResetFilters={handleResetFilters}
            onClearSearch={() => {
              setSearchQuery('')
              setPage(1)
            }}
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
              aria-label={t('pagination.pageSize')}
              title={t('pagination.pageSize')}
              className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-xs text-[var(--text-primary)] transition-all outline-none hover:border-[var(--accent)] focus:border-[var(--accent)]"
            >
              {[12, 24, 48].map((size) => (
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
