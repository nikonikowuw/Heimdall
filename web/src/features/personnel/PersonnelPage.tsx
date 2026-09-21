import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import {
  ArrowUpDown,
  ChevronLeft,
  ChevronRight,
  FileText,
  LayoutGrid,
  List,
  RefreshCw,
  RotateCw,
  Search,
  UserPlus,
  Users,
  X,
} from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { personnelApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import { wsClient } from '@/lib/wsClient'
import type { PersonnelDetail, PersonnelItem, PersonnelStats, ReextractProgress } from '@/types'
import { BatchDeleteModal } from './components/BatchDeleteModal'
import { DeleteConfirmModal } from './components/DeleteConfirmModal'
import { PersonnelBatchBar } from './components/PersonnelBatchBar'
import { PersonnelCard, PersonnelCardSkeleton } from './components/PersonnelCard'
import { PersonnelDetailDrawer } from './components/PersonnelDetailDrawer'
import { PersonnelModal } from './components/PersonnelModal'
import { PersonnelStatsGrid } from './components/PersonnelStatsGrid'
import { PersonnelTable, PersonnelTableSkeleton } from './components/PersonnelTable'
import { toast } from '@/stores/toast'
import type { PersonnelNotice } from './components/PersonnelToast'
import { ReextractModal } from './components/ReextractModal'

const PAGE_SIZE_OPTIONS = [12, 24, 48, 96]

const HEADER_ACTION_CLASS =
  'inline-flex h-9 items-center justify-center gap-1.5 rounded-xl border text-xs font-medium transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95 disabled:cursor-not-allowed disabled:opacity-40'

const PAGER_CLASS =
  'inline-flex h-8 items-center justify-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 text-xs font-medium text-[var(--text-secondary)] transition-all hover:border-[var(--accent)]/40 hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-40 disabled:hover:border-[var(--border)] disabled:hover:text-[var(--text-secondary)]'

type ViewMode = 'grid' | 'table'
type SampleFilter = 'all' | 'saturated' | 'incomplete' | 'empty'
type SortBy = 'createdDesc' | 'createdAsc' | 'nameAsc' | 'faceCountDesc' | 'faceCountAsc'

export function PersonnelPage(): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()

  const [items, setItems] = useState<PersonnelItem[]>([])
  const [total, setTotal] = useState<number>(0)
  const [stats, setStats] = useState<PersonnelStats>({
    totalPersonnel: 0,
    totalFaces: 0,
    algoReady: false,
  })

  // 视图与筛选
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    if (typeof window === 'undefined' || typeof localStorage === 'undefined') {
      return 'grid'
    }
    try {
      const saved = localStorage.getItem('heimdall_personnel_view_mode')
      return saved === 'table' ? 'table' : 'grid'
    } catch {
      return 'grid'
    }
  })
  const [sampleFilter, setSampleFilter] = useState<SampleFilter>('all')
  const [sortBy, setSortBy] = useState<SortBy>('createdDesc')

  const [searchKeyword, setSearchKeyword] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [page, setPage] = useState(1)
  const [limit, setLimit] = useState(24)

  // 多选与批量操作
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [isBatchDeleteModalOpen, setIsBatchDeleteModalOpen] = useState(false)
  const [isBatchDeleting, setIsBatchDeleting] = useState(false)
  const [batchDeleteProgress, setBatchDeleteProgress] = useState<{
    current: number
    total: number
  } | null>(null)

  // 模态框与抽屉状态
  const [isModalOpen, setIsModalOpen] = useState(false)
  const [editTarget, setEditTarget] = useState<PersonnelItem | null>(null)

  const [drawerSubjectId, setDrawerSubjectId] = useState<string | null>(null)
  const [drawerAutoOpenUpload, setDrawerAutoOpenUpload] = useState(false)

  const [deleteTarget, setDeleteTarget] = useState<PersonnelItem | null>(null)
  const [isDeleting, setIsDeleting] = useState(false)

  // 一键重新提取特征状态与执行报告
  const [isReextractModalOpen, setIsReextractModalOpen] = useState(false)
  const [reextractModalMode, setReextractModalMode] = useState<'confirm' | 'report'>('confirm')
  const [isStartingReextract, setIsStartingReextract] = useState(false)
  const [reextractProgress, setReextractProgress] = useState<ReextractProgress | null>(null)
  const [reextractError, setReextractError] = useState<string | null>(null)

  // 统计接口失败说明：与算法未就绪区分开，避免网络故障被误读为算法缺失
  const [statsError, setStatsError] = useState<string | null>(null)
  // 是否拿到过至少一次有效统计：决定数值卡显示真实值还是占位符
  const [hasStats, setHasStats] = useState(false)

  const lastStatusRef = useRef<string | null>(null)
  const searchInputRef = useRef<HTMLInputElement>(null)

  const handleOpenReport = useCallback(() => {
    setReextractError(null)
    setReextractModalMode('report')
    setIsReextractModalOpen(true)
  }, [])

  const pushNotice = useCallback(
    (
      type: PersonnelNotice['type'],
      title: string,
      message: string,
      action?: PersonnelNotice['action'],
    ) => {
      toast.show({
        type,
        category: t('common:nav.personnel', { defaultValue: '人员底库' }),
        title,
        message,
        duration: 5000,
        action:
          action === 'report'
            ? {
                label: t('reextract.viewReport', { defaultValue: '查看报告' }),
                onClick: handleOpenReport,
                primary: true,
              }
            : undefined,
      })
    },
    [handleOpenReport, t],
  )

  const handleSetViewMode = (mode: ViewMode) => {
    setViewMode(mode)
    try {
      localStorage.setItem('heimdall_personnel_view_mode', mode)
    } catch {
      // 容错
    }
  }

  const loadData = useCallback(async () => {
    setIsLoading(true)
    try {
      const [listRes, statsRes] = await Promise.all([
        personnelApi.list({
          keyword: searchKeyword.trim() || undefined,
          limit,
          offset: (page - 1) * limit,
        }),
        personnelApi.getStats().catch(() => null),
      ])
      setItems(listRes.items)
      setTotal(listRes.total)
      if (statsRes) {
        setStats(statsRes)
        setHasStats(true)
        setStatsError(null)
      } else {
        setStatsError(t('stats.loadFailed'))
      }
    } catch {
      // 列表失败保持上次结果，由空状态与刷新按钮承载重试
    } finally {
      setIsLoading(false)
    }
  }, [searchKeyword, page, limit, t])

  useEffect(() => {
    loadData()
  }, [loadData])

  // 初次加载探测一次后台任务状态
  useEffect(() => {
    personnelApi
      .getReextractStatus()
      .then((res) => {
        setReextractProgress(res)
        lastStatusRef.current = res.status
      })
      .catch(() => {})
  }, [])

  // 键盘快捷键监听：/ 聚焦搜索框；Escape 取消多选
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
        if (e.key === 'Escape') {
          ;(e.target as HTMLElement).blur()
        }
        return
      }

      if (e.key === '/') {
        e.preventDefault()
        searchInputRef.current?.focus()
      } else if (e.key === 'Escape' && selectedIds.size > 0) {
        setSelectedIds(new Set())
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [selectedIds.size])

  const buildReextractNotice = useCallback(
    (
      data: ReextractProgress,
    ): { type: PersonnelNotice['type']; title: string; message: string } => {
      if (data.status === 'completed') {
        return {
          type: data.failed > 0 ? 'warning' : 'success',
          title: t('reextract.completedToastTitle'),
          message: t('reextract.completedToastDesc', {
            total: data.total,
            succeeded: data.succeeded,
            failed: data.failed,
          }),
        }
      }
      return {
        type: 'error',
        title: t('reextract.failedToastTitle'),
        message: data.errorMessage || t('reextract.failedToastDesc'),
      }
    },
    [t],
  )

  // 订阅 WebSocket 完成广播通知
  useEffect(() => {
    const unsub = wsClient.subscribe<ReextractProgress>('personnel.reextract.finished', (data) => {
      setReextractProgress(data)
      loadData()
      const built = buildReextractNotice(data)
      pushNotice(built.type, built.title, built.message, 'report')
    })
    return unsub
  }, [loadData, pushNotice, buildReextractNotice])

  // 轮询后台重提任务状态
  const isTaskRunning = reextractProgress?.status === 'running'
  useEffect(() => {
    if (!isTaskRunning) return

    const timer = setInterval(async () => {
      try {
        const res = await personnelApi.getReextractStatus()
        setReextractProgress(res)

        // 状态从 running 转为已完成或失败
        if (lastStatusRef.current === 'running' && res.status !== 'running') {
          loadData()
          const built = buildReextractNotice(res)
          pushNotice(built.type, built.title, built.message, 'report')
        }
        lastStatusRef.current = res.status
      } catch {
        // 轮询容错
      }
    }, 1000)

    return () => clearInterval(timer)
  }, [isTaskRunning, loadData, pushNotice, buildReextractNotice])

  const handleOpenRegister = () => {
    setEditTarget(null)
    setIsModalOpen(true)
  }

  const handleOpenEdit = (person: PersonnelItem) => {
    setEditTarget(person)
    setIsModalOpen(true)
  }

  const handleOpenView = (person: PersonnelItem) => {
    setDrawerSubjectId(person.subjectId)
    setDrawerAutoOpenUpload(false)
  }

  const handleOpenAddFace = (person: PersonnelItem) => {
    setDrawerSubjectId(person.subjectId)
    setDrawerAutoOpenUpload(true)
  }

  const handleOpenDelete = (person: PersonnelItem) => {
    setDeleteTarget(person)
  }

  const handleConfirmDelete = async () => {
    const target = deleteTarget
    if (!target) return
    setIsDeleting(true)
    try {
      await personnelApi.delete(target.subjectId)
      setDeleteTarget(null)
      setSelectedIds((prev) => {
        const next = new Set(prev)
        next.delete(target.subjectId)
        return next
      })
      pushNotice(
        'success',
        t('toast.deleted'),
        `${target.name} · ${t('card.subjectId')} ${target.subjectId}`,
      )
      loadData()
    } catch {
      pushNotice('error', t('errors.failedToDelete'), target.name)
    } finally {
      setIsDeleting(false)
    }
  }

  // 批量勾选与管理
  const handleToggleSelect = (subjectId: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev)
      if (next.has(subjectId)) {
        next.delete(subjectId)
      } else {
        next.add(subjectId)
      }
      return next
    })
  }

  // 针对当前已过滤项进行排序与切片
  const displayItems = useMemo(() => {
    let result = [...items]

    // 样本健康度过滤
    if (sampleFilter === 'saturated') {
      result = result.filter((p) => p.faceCount >= 5)
    } else if (sampleFilter === 'incomplete') {
      result = result.filter((p) => p.faceCount >= 1 && p.faceCount < 5)
    } else if (sampleFilter === 'empty') {
      result = result.filter((p) => p.faceCount === 0)
    }

    // 多维排序
    result.sort((a, b) => {
      switch (sortBy) {
        case 'createdAsc':
          return a.createdAt - b.createdAt
        case 'nameAsc':
          return a.name.localeCompare(b.name)
        case 'faceCountDesc':
          return b.faceCount - a.faceCount
        case 'faceCountAsc':
          return a.faceCount - b.faceCount
        case 'createdDesc':
        default:
          return b.createdAt - a.createdAt
      }
    })

    return result
  }, [items, sampleFilter, sortBy])

  const isAllSelected =
    displayItems.length > 0 && displayItems.every((item) => selectedIds.has(item.subjectId))

  const handleToggleSelectAll = () => {
    if (isAllSelected) {
      // 取消勾选当页可见项
      setSelectedIds((prev) => {
        const next = new Set(prev)
        displayItems.forEach((item) => next.delete(item.subjectId))
        return next
      })
    } else {
      // 全选当页可见项
      setSelectedIds((prev) => {
        const next = new Set(prev)
        displayItems.forEach((item) => next.add(item.subjectId))
        return next
      })
    }
  }

  const handleClearSelection = () => {
    setSelectedIds(new Set())
  }

  const selectedTargets = useMemo(() => {
    return items.filter((item) => selectedIds.has(item.subjectId))
  }, [items, selectedIds])

  const handleBatchDeleteConfirm = async () => {
    if (selectedTargets.length === 0) return
    setIsBatchDeleting(true)
    let succeeded = 0
    let failed = 0

    const totalToDelete = selectedTargets.length
    for (let i = 0; i < totalToDelete; i++) {
      const target = selectedTargets[i]
      setBatchDeleteProgress({ current: i + 1, total: totalToDelete })
      try {
        await personnelApi.delete(target.subjectId)
        succeeded++
      } catch {
        failed++
      }
    }

    setIsBatchDeleting(false)
    setIsBatchDeleteModalOpen(false)
    setBatchDeleteProgress(null)
    setSelectedIds(new Set())

    if (failed === 0) {
      pushNotice('success', t('batch.batchDeleteSuccess', { count: succeeded }), '')
    } else {
      pushNotice('warning', t('batch.batchDeletePartial', { succeeded, failed }), '')
    }

    loadData()
  }

  const handleOpenReextract = () => {
    setReextractError(null)
    setReextractModalMode('confirm')
    setIsReextractModalOpen(true)
  }

  const handleConfirmReextract = async () => {
    setIsStartingReextract(true)
    setReextractError(null)
    lastStatusRef.current = 'running'
    try {
      const initial = await personnelApi.startReextract()
      setReextractProgress(initial)
    } catch (err: unknown) {
      setReextractError(err instanceof Error ? err.message : t('errors.reextractFailed'))
    } finally {
      setIsStartingReextract(false)
    }
  }

  const handleCloseReextractModal = () => {
    setIsReextractModalOpen(false)
    setReextractError(null)
  }

  const handleModalSuccess = (detail: PersonnelDetail) => {
    const isEditFlow = Boolean(editTarget)
    pushNotice(
      'success',
      isEditFlow ? t('toast.updated') : t('toast.registered'),
      `${detail.name} · ${t('card.subjectId')} ${detail.subjectId}`,
    )
    loadData()
  }

  const handleDrawerNotify = useCallback(
    (payload: { title: string; message: string }) => {
      pushNotice('success', payload.title, payload.message)
    },
    [pushNotice],
  )

  const totalPages = Math.max(1, Math.ceil(total / limit))

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages)
    }
  }, [page, totalPages])

  let reextractTooltip = t('actions.reextractFeatures')
  if (!hasStats && statsError === null) {
    reextractTooltip = t('common:loading')
  } else if (statsError !== null) {
    reextractTooltip = statsError
  } else if (!stats.algoReady) {
    reextractTooltip = t('reextract.algoDisabledTooltip')
  } else if (stats.totalFaces === 0) {
    reextractTooltip = t('reextract.noFaces')
  }

  const reextractPercent = useMemo(() => {
    if (!reextractProgress || reextractProgress.total <= 0) return 0
    return Math.min(100, Math.round((reextractProgress.processed / reextractProgress.total) * 100))
  }, [reextractProgress])

  const hasReport =
    reextractProgress !== null &&
    (reextractProgress.status === 'completed' || reextractProgress.status === 'failed') &&
    !isTaskRunning

  const renderListContent = (): React.ReactElement => {
    if (isLoading && items.length === 0) {
      if (viewMode === 'grid') {
        return (
          <div className="grid grid-cols-1 gap-3.5 pt-1.5 pb-2 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
            {[1, 2, 3, 4, 5, 6, 7, 8].map((i) => (
              <PersonnelCardSkeleton key={i} />
            ))}
          </div>
        )
      }
      return <PersonnelTableSkeleton />
    }

    if (displayItems.length === 0) {
      const hasFilterOrSearch = Boolean(searchKeyword || sampleFilter !== 'all')
      let emptyTitle = t('empty.title')
      let emptyDesc = t('empty.desc')
      if (searchKeyword) {
        emptyTitle = t('empty.noSearchResult')
        emptyDesc = t('empty.searchHint')
      } else if (sampleFilter !== 'all') {
        emptyTitle = t('filters.noMatchTitle')
        emptyDesc = t('filters.noMatchDesc')
      }

      return (
        <div className="frosted-glass flex flex-col items-center justify-center rounded-3xl py-20 text-center">
          <div className="mb-3 flex h-14 w-14 items-center justify-center rounded-2xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)]">
            {hasFilterOrSearch ? <Search className="h-6 w-6" /> : <Users className="h-6 w-6" />}
          </div>
          <h3 className="text-base font-bold text-[var(--text-primary)]">{emptyTitle}</h3>
          <p className="mt-1 max-w-sm text-xs leading-relaxed text-[var(--text-muted)]">
            {emptyDesc}
          </p>
          {hasFilterOrSearch ? (
            <button
              type="button"
              onClick={() => {
                setSearchKeyword('')
                setSampleFilter('all')
                setPage(1)
              }}
              className="mt-5 inline-flex h-9 items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)]/40 hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            >
              <X className="h-3.5 w-3.5" aria-hidden="true" />
              {t('filters.resetFilters')}
            </button>
          ) : (
            <button
              type="button"
              onClick={handleOpenRegister}
              className="mt-5 inline-flex h-9 items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95"
            >
              <UserPlus className="h-4 w-4" aria-hidden="true" />
              {t('actions.register')}
            </button>
          )}
        </div>
      )
    }

    if (viewMode === 'grid') {
      return (
        <div className="grid grid-cols-1 gap-3.5 pt-1.5 pb-2 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
          {displayItems.map((person) => (
            <PersonnelCard
              key={person.id}
              person={person}
              isSelected={selectedIds.has(person.subjectId)}
              onToggleSelect={handleToggleSelect}
              onView={handleOpenView}
              onEdit={handleOpenEdit}
              onDelete={handleOpenDelete}
              onAddFace={handleOpenAddFace}
            />
          ))}
        </div>
      )
    }

    return (
      <PersonnelTable
        items={displayItems}
        selectedIds={selectedIds}
        onToggleSelect={handleToggleSelect}
        onToggleSelectAll={handleToggleSelectAll}
        isAllSelected={isAllSelected}
        onView={handleOpenView}
        onEdit={handleOpenEdit}
        onDelete={handleOpenDelete}
        onAddFace={handleOpenAddFace}
      />
    )
  }

  return (
    <div className="relative flex h-full flex-col gap-4 text-[var(--text-primary)] select-none">
      <div className="shrink-0 space-y-3.5">
        {/* ── 1. 紧凑型 SaaS 指挥台：标题 + 遥测状态 + 全局操作 ── */}
        <motion.header
          initial={reduceMotion ? false : { opacity: 0, y: motionTokens.distance.sm }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }}
          className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl p-3.5 shadow-xs"
        >
          <div className="flex min-w-0 items-center gap-3.5">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)] shadow-xs">
              <Users className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2.5">
                <h1 className="truncate text-base font-bold tracking-tight text-[var(--text-primary)] sm:text-lg">
                  {t('title')}
                </h1>
                {/* 紧凑型遥测标签 */}
                <span className="font-data hidden items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg-secondary)]/80 px-2 py-0.5 text-[11px] font-medium text-[var(--text-secondary)] sm:inline-flex">
                  <span className="text-[var(--text-muted)]">{t('pagination.totalCount')}:</span>
                  <strong className="text-[var(--text-primary)]">{total}</strong>
                </span>
              </div>
              <p className="mt-0.5 truncate text-xs text-[var(--text-muted)]">{t('subtitle')}</p>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {/* 算法就绪状态：微脉冲呼吸灯指示 */}
            <span
              title={t('stats.algoStatus')}
              className={`hidden max-w-[15rem] items-center gap-1.5 truncate rounded-xl border px-2.5 py-1.5 text-[11px] font-medium lg:inline-flex ${
                !hasStats
                  ? 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)]'
                  : stats.algoReady
                    ? 'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)]'
                    : 'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)]'
              }`}
            >
              <span
                aria-hidden="true"
                className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                  !hasStats
                    ? 'bg-[var(--text-muted)]'
                    : stats.algoReady
                      ? 'animate-pulse bg-[var(--status-success)] shadow-[0_0_6px_var(--status-success-soft)]'
                      : 'bg-[var(--status-warning)]'
                }`}
              />
              <span className="truncate">
                {!hasStats
                  ? t('stats.algoUnknown')
                  : stats.algoReady
                    ? t('stats.algoReady')
                    : t('stats.algoNotReady')}
              </span>
            </span>

            {/* 最近一次重提任务报告入口 */}
            {hasReport && reextractProgress && (
              <button
                type="button"
                onClick={handleOpenReport}
                title={t('reextract.viewReportTooltip')}
                className={`${HEADER_ACTION_CLASS} ${
                  reextractProgress.failed > 0
                    ? 'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)] hover:bg-[var(--status-warning-soft)]'
                    : 'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)] hover:bg-[var(--status-success-soft)]'
                }`}
              >
                <FileText className="h-3.5 w-3.5" aria-hidden="true" />
                <span className="hidden sm:inline">
                  {t('reextract.lastReportBadge', {
                    succeeded: reextractProgress.succeeded,
                    failed: reextractProgress.failed,
                  })}
                </span>
                <span className="sm:hidden">{t('reextract.viewReport')}</span>
              </button>
            )}

            {/* 一键重新提取人脸特征 */}
            <button
              type="button"
              onClick={isTaskRunning ? handleOpenReport : handleOpenReextract}
              disabled={(!stats.algoReady || stats.totalFaces === 0) && !isTaskRunning}
              title={reextractTooltip}
              aria-label={reextractTooltip}
              className={`${HEADER_ACTION_CLASS} ${
                isTaskRunning
                  ? 'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)] hover:bg-[var(--status-success-soft)]'
                  : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] hover:border-[var(--accent)]/40 hover:text-[var(--accent)]'
              }`}
            >
              <RefreshCw
                className={`h-3.5 w-3.5 ${isTaskRunning ? 'animate-spin' : ''}`}
                aria-hidden="true"
              />
              <span className="hidden sm:inline">
                {isTaskRunning
                  ? t('reextract.runningBadge', { percent: reextractPercent })
                  : t('actions.reextractShort', { defaultValue: t('actions.reextractFeatures') })}
              </span>
            </button>

            {/* 刷新 */}
            <button
              type="button"
              onClick={() => loadData()}
              disabled={isLoading}
              aria-label={t('actions.refresh')}
              title={t('actions.refresh')}
              className={`${HEADER_ACTION_CLASS} w-9 border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] hover:border-[var(--accent)]/40 hover:text-[var(--accent)]`}
            >
              <RotateCw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
            </button>

            {/* 录入新人员 */}
            <button
              type="button"
              onClick={handleOpenRegister}
              title={t('actions.register')}
              className="inline-flex h-9 items-center justify-center gap-1.5 rounded-xl bg-[var(--accent)] px-3.5 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95"
            >
              <UserPlus className="h-4 w-4" aria-hidden="true" />
              <span>{t('actions.register')}</span>
            </button>
          </div>
        </motion.header>

        {/* ── 2. 底库指标卡 ── */}
        <PersonnelStatsGrid stats={stats} hasData={hasStats} error={statsError} />

        {/* ── 3. 现代化 SaaS 一体化操作检索工具栏 ── */}
        <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl p-2.5 shadow-xs">
          <div className="flex flex-1 flex-wrap items-center gap-2.5 sm:flex-nowrap">
            {/* 搜索框 */}
            <div className="group/search relative min-w-[200px] flex-1 sm:max-w-xs">
              <Search
                className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)] transition-colors group-focus-within/search:text-[var(--accent)]"
                aria-hidden="true"
              />
              <input
                ref={searchInputRef}
                type="text"
                data-search-input="true"
                value={searchKeyword}
                onChange={(e) => {
                  setSearchKeyword(e.target.value)
                  setPage(1)
                }}
                placeholder={t('actions.searchPlaceholder')}
                aria-label={t('actions.searchPlaceholder')}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/70 py-1.5 pr-8 pl-8 text-xs text-[var(--text-primary)] backdrop-blur-md transition-all select-text placeholder:text-[var(--text-muted)] hover:border-[var(--border-strong)] focus:border-[var(--accent)] focus:bg-[var(--bg-surface)] focus:ring-2 focus:ring-[var(--accent-soft)] focus:outline-none"
              />
              {searchKeyword ? (
                <button
                  type="button"
                  onClick={() => {
                    setSearchKeyword('')
                    setPage(1)
                  }}
                  aria-label={t('actions.clearSearch')}
                  title={t('actions.clearSearch')}
                  className="absolute top-1/2 right-2 flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded-md text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                >
                  <X className="h-3 w-3" />
                </button>
              ) : (
                <kbd className="pointer-events-none absolute top-1/2 right-2 hidden -translate-y-1/2 rounded border border-[var(--border)] bg-[var(--bg-surface)] px-1 font-mono text-[10px] text-[var(--text-muted)] shadow-2xs sm:inline-block">
                  /
                </kbd>
              )}
            </div>

            {/* 样本健康度分段筛选胶囊 (Segmented Pills) */}
            <div className="hidden items-center rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/60 p-0.5 xl:flex">
              {(
                [
                  { id: 'all', label: t('filters.all') },
                  { id: 'saturated', label: t('filters.saturated') },
                  { id: 'incomplete', label: t('filters.incomplete') },
                  { id: 'empty', label: t('filters.empty') },
                ] as const
              ).map((filter) => (
                <button
                  key={filter.id}
                  type="button"
                  onClick={() => setSampleFilter(filter.id)}
                  className={`rounded-lg px-2.5 py-1 text-[11px] font-medium transition-all ${
                    sampleFilter === filter.id
                      ? 'bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs'
                      : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                  }`}
                >
                  {filter.label}
                </button>
              ))}
            </div>
          </div>

          {/* 右侧：排序 + 视图切换 + 加载/计数 */}
          <div className="flex items-center gap-2">
            {/* 排序选择器 */}
            <div className="flex items-center gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 px-2 py-1 text-[11px] text-[var(--text-secondary)]">
              <ArrowUpDown className="h-3 w-3 text-[var(--text-muted)]" />
              <select
                value={sortBy}
                onChange={(e) => setSortBy(e.target.value as SortBy)}
                aria-label={t('sort.label')}
                className="bg-transparent text-[11px] font-medium text-[var(--text-primary)] focus:outline-none"
              >
                <option value="createdDesc">{t('sort.createdDesc')}</option>
                <option value="createdAsc">{t('sort.createdAsc')}</option>
                <option value="nameAsc">{t('sort.nameAsc')}</option>
                <option value="faceCountDesc">{t('sort.faceCountDesc')}</option>
                <option value="faceCountAsc">{t('sort.faceCountAsc')}</option>
              </select>
            </div>

            {/* 视图模式切换：Grid / Table */}
            <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/60 p-0.5">
              <button
                type="button"
                onClick={() => handleSetViewMode('grid')}
                title={t('viewMode.grid')}
                aria-label={t('viewMode.grid')}
                className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
                  viewMode === 'grid'
                    ? 'bg-[var(--bg-surface)] text-[var(--accent)] shadow-xs'
                    : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                }`}
              >
                <LayoutGrid className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                onClick={() => handleSetViewMode('table')}
                title={t('viewMode.table')}
                aria-label={t('viewMode.table')}
                className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
                  viewMode === 'table'
                    ? 'bg-[var(--bg-surface)] text-[var(--accent)] shadow-xs'
                    : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                }`}
              >
                <List className="h-3.5 w-3.5" />
              </button>
            </div>

            {isLoading && (
              <span className="flex items-center gap-1 text-[11px] text-[var(--text-muted)]">
                <RotateCw className="h-3 w-3 animate-spin" aria-hidden="true" />
              </span>
            )}
            <span className="font-data text-[11px] text-[var(--text-muted)] tabular-nums">
              {t('pagination.total', { total })}
            </span>
          </div>
        </div>
      </div>

      {/* ── 4. 主体列表区 (支持 Grid 与 Table 双视图) ── */}
      <div
        aria-busy={isLoading}
        className={`flex-1 overflow-y-auto px-1.5 pt-3 pb-24 transition-opacity duration-200 ${
          isLoading && items.length > 0 ? 'opacity-60' : 'opacity-100'
        }`}
      >
        {renderListContent()}
      </div>

      {/* ── 5. 分页控制条 ── */}
      <div className="frosted-glass flex shrink-0 flex-wrap items-center justify-between gap-3 rounded-2xl px-3.5 py-2.5 text-xs text-[var(--text-secondary)] shadow-xs">
        <div className="flex flex-wrap items-center gap-3">
          <span className="font-data tabular-nums">{t('pagination.total', { total })}</span>
          <div className="flex items-center gap-1.5 border-l border-[var(--border)] pl-3">
            <label
              htmlFor="personnel-page-size"
              className="text-[10px] font-medium tracking-wider text-[var(--text-muted)] uppercase"
            >
              {t('pagination.pageSize')}
            </label>
            <select
              id="personnel-page-size"
              value={limit}
              onChange={(e) => {
                setLimit(Number(e.target.value))
                setPage(1)
              }}
              className="font-data h-8 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 text-xs text-[var(--text-primary)] transition-colors hover:border-[var(--border-strong)] focus:border-[var(--accent)] focus:outline-none"
            >
              {PAGE_SIZE_OPTIONS.map((size) => (
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
            disabled={page <= 1 || isLoading}
            onClick={() => setPage((p) => Math.max(1, p - 1))}
            aria-label={t('pagination.prev')}
            className={PAGER_CLASS}
          >
            <ChevronLeft className="h-3.5 w-3.5" aria-hidden="true" />
            <span className="hidden sm:inline">{t('pagination.prev')}</span>
          </button>
          <span className="font-data px-1.5 font-semibold text-[var(--text-primary)] tabular-nums">
            {page} / {totalPages}
          </span>
          <button
            type="button"
            disabled={page >= totalPages || isLoading}
            onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
            aria-label={t('pagination.next')}
            className={PAGER_CLASS}
          >
            <span className="hidden sm:inline">{t('pagination.next')}</span>
            <ChevronRight className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        </div>
      </div>

      {/* ── 6. 悬浮式批量操作坞 ── */}
      <PersonnelBatchBar
        selectedCount={selectedIds.size}
        isAllSelected={isAllSelected}
        onToggleSelectAll={handleToggleSelectAll}
        onClearSelection={handleClearSelection}
        onBatchDelete={() => setIsBatchDeleteModalOpen(true)}
      />

      {/* ── 7. 浮层与即时反馈 ── */}
      <PersonnelModal
        isOpen={isModalOpen}
        onClose={() => setIsModalOpen(false)}
        onSuccess={handleModalSuccess}
        editTarget={editTarget}
        onManagePhotos={handleOpenAddFace}
      />

      <PersonnelDetailDrawer
        isOpen={Boolean(drawerSubjectId)}
        subjectId={drawerSubjectId}
        autoOpenUpload={drawerAutoOpenUpload}
        onClose={() => {
          setDrawerSubjectId(null)
          setDrawerAutoOpenUpload(false)
        }}
        onUpdate={loadData}
        onNotify={handleDrawerNotify}
      />

      <DeleteConfirmModal
        isOpen={Boolean(deleteTarget)}
        target={deleteTarget}
        isDeleting={isDeleting}
        onClose={() => setDeleteTarget(null)}
        onConfirm={handleConfirmDelete}
      />

      <BatchDeleteModal
        isOpen={isBatchDeleteModalOpen}
        targets={selectedTargets}
        isDeleting={isBatchDeleting}
        currentProgress={batchDeleteProgress}
        onClose={() => setIsBatchDeleteModalOpen(false)}
        onConfirm={handleBatchDeleteConfirm}
      />

      <ReextractModal
        isOpen={isReextractModalOpen}
        isGlobal={true}
        initialMode={reextractModalMode}
        isStarting={isStartingReextract}
        progress={reextractProgress}
        error={reextractError}
        onClose={handleCloseReextractModal}
        onConfirm={handleConfirmReextract}
      />
    </div>
  )
}
