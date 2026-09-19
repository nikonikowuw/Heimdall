import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import {
  Users,
  UserPlus,
  Search,
  RotateCw,
  RefreshCw,
  Cpu,
  Layers,
  CheckCircle2,
  AlertCircle,
  Loader2,
  FileText,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { personnelApi } from '../../lib/api'
import { wsClient } from '../../lib/wsClient'
import type { PersonnelItem, PersonnelStats, ReextractProgress } from '../../types'
import { PersonnelCard } from './components/PersonnelCard'
import { PersonnelModal } from './components/PersonnelModal'
import { PersonnelDetailDrawer } from './components/PersonnelDetailDrawer'
import { DeleteConfirmModal } from './components/DeleteConfirmModal'
import { ReextractModal } from './components/ReextractModal'

export function PersonnelPage(): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])

  const [items, setItems] = useState<PersonnelItem[]>([])
  const [total, setTotal] = useState<number>(0)
  const [stats, setStats] = useState<PersonnelStats>({
    totalPersonnel: 0,
    totalFaces: 0,
    algoReady: false,
  })

  const [searchKeyword, setSearchKeyword] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [page, setPage] = useState(1)
  const [limit, setLimit] = useState(24)

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

  // 任务完成即时通知 Toast 状态
  const [toast, setToast] = useState<{
    id: number
    title: string
    message: string
    type: 'success' | 'warning' | 'error'
  } | null>(null)

  const lastStatusRef = useRef<string | null>(null)

  const loadData = useCallback(async () => {
    setIsLoading(true)
    try {
      const [listRes, statsRes] = await Promise.all([
        personnelApi.list({
          keyword: searchKeyword.trim() || undefined,
          limit,
          offset: (page - 1) * limit,
        }),
        personnelApi.getStats().catch(() => ({
          totalPersonnel: 0,
          totalFaces: 0,
          algoReady: false,
        })),
      ])
      setItems(listRes.items)
      setTotal(listRes.total)
      setStats(statsRes)
    } catch {
      // 忽略或通过页面空状态承载加载失败
    } finally {
      setIsLoading(false)
    }
  }, [searchKeyword, page, limit])

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

  // 订阅 WebSocket 完成广播通知
  useEffect(() => {
    const unsub = wsClient.subscribe<ReextractProgress>('personnel.reextract.finished', (data) => {
      setReextractProgress(data)
      loadData()
      setToast({
        id: Date.now(),
        title:
          data.status === 'completed'
            ? t('reextract.completedToastTitle')
            : t('reextract.failedToastTitle'),
        message:
          data.status === 'completed'
            ? t('reextract.completedToastDesc', {
                total: data.total,
                succeeded: data.succeeded,
                failed: data.failed,
              })
            : data.errorMessage || t('reextract.failedToastDesc'),
        type: data.status === 'completed' ? (data.failed > 0 ? 'warning' : 'success') : 'error',
      })
    })
    return unsub
  }, [loadData, t])

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
          setToast({
            id: Date.now(),
            title:
              res.status === 'completed'
                ? t('reextract.completedToastTitle')
                : t('reextract.failedToastTitle'),
            message:
              res.status === 'completed'
                ? t('reextract.completedToastDesc', {
                    total: res.total,
                    succeeded: res.succeeded,
                    failed: res.failed,
                  })
                : res.errorMessage || t('reextract.failedToastDesc'),
            type: res.status === 'completed' ? (res.failed > 0 ? 'warning' : 'success') : 'error',
          })
        }
        lastStatusRef.current = res.status
      } catch {
        // 轮询容错
      }
    }, 1000)

    return () => clearInterval(timer)
  }, [isTaskRunning, loadData, t])

  // Toast 自动消退倒计时
  useEffect(() => {
    if (!toast) return
    const timer = setTimeout(() => {
      setToast(null)
    }, 6000)
    return () => clearTimeout(timer)
  }, [toast])

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
    if (!deleteTarget) return
    setIsDeleting(true)
    try {
      await personnelApi.delete(deleteTarget.subjectId)
      setDeleteTarget(null)
      loadData()
    } catch {
      // 忽略删除网络错误，依赖重新加载状态对齐
    } finally {
      setIsDeleting(false)
    }
  }

  const handleOpenReextract = () => {
    setReextractError(null)
    setReextractModalMode('confirm')
    setIsReextractModalOpen(true)
  }

  const handleOpenReport = () => {
    setReextractError(null)
    setReextractModalMode('report')
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

  const handleModalSuccess = () => {
    loadData()
  }

  const totalPages = Math.max(1, Math.ceil(total / limit))

  // 安全页码截断（防止最后一页删除后停留在空白页）
  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages)
    }
  }, [page, totalPages])

  // 重提特征按钮工具提示与进度推导
  let reextractTooltip = t('actions.reextractFeatures')
  if (!stats.algoReady) {
    reextractTooltip = t('reextract.algoDisabledTooltip')
  } else if (stats.totalFaces === 0) {
    reextractTooltip = t('reextract.noFaces')
  }

  const reextractPercent = useMemo(() => {
    if (!reextractProgress || reextractProgress.total <= 0) return 0
    return Math.min(100, Math.round((reextractProgress.processed / reextractProgress.total) * 100))
  }, [reextractProgress])

  return (
    <div className="flex h-full flex-col gap-4 text-[var(--text-primary)] select-none">
      <div className="shrink-0 space-y-4">
        {/* 顶部标题与统计概览 */}
        <div className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <h1 className="flex items-center gap-2.5 text-2xl font-bold tracking-tight text-[var(--text-primary)]">
              <Users className="h-6 w-6 text-emerald-500" />
              {t('title')}
            </h1>
            <p className="mt-1 text-xs text-[var(--text-muted)]">{t('subtitle')}</p>
          </div>

          {/* 统计指标卡 */}
          <div className="flex flex-wrap items-center gap-3">
            <div className="frosted-glass flex items-center gap-3 rounded-2xl px-4 py-2.5 shadow-xs">
              <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-emerald-500/10 text-emerald-400">
                <Users className="h-4 w-4" />
              </div>
              <div>
                <p className="text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
                  {t('stats.totalPersonnel')}
                </p>
                <p className="text-base font-bold text-[var(--text-primary)]">
                  {stats.totalPersonnel}
                </p>
              </div>
            </div>

            <div className="frosted-glass flex items-center gap-3 rounded-2xl px-4 py-2.5 shadow-xs">
              <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-cyan-500/10 text-cyan-400">
                <Layers className="h-4 w-4" />
              </div>
              <div>
                <p className="text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
                  {t('stats.totalFaces')}
                </p>
                <p className="text-base font-bold text-[var(--text-primary)]">{stats.totalFaces}</p>
              </div>
            </div>

            <div className="frosted-glass flex items-center gap-3 rounded-2xl px-4 py-2.5 shadow-xs">
              <div
                className={`flex h-8 w-8 items-center justify-center rounded-xl ${
                  stats.algoReady
                    ? 'bg-emerald-500/10 text-emerald-400'
                    : 'bg-amber-500/10 text-amber-400'
                }`}
              >
                <Cpu className="h-4 w-4" />
              </div>
              <div>
                <p className="text-[10px] tracking-wider text-[var(--text-muted)] uppercase">
                  {t('stats.algoStatus')}
                </p>
                <div className="flex items-center gap-1.5 text-xs font-semibold">
                  {stats.algoReady ? (
                    <span className="flex items-center gap-1 text-emerald-400">
                      <CheckCircle2 className="h-3.5 w-3.5" />
                      {t('stats.algoReady')}
                    </span>
                  ) : (
                    <span className="flex items-center gap-1 text-amber-400">
                      <AlertCircle className="h-3.5 w-3.5" />
                      {t('stats.algoNotReady')}
                    </span>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* 搜索与快捷操作栏 */}
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="group/search relative max-w-md flex-1">
            <Search className="pointer-events-none absolute top-1/2 left-3.5 h-4 w-4 -translate-y-1/2 text-[var(--text-muted)] transition-colors group-focus-within/search:text-emerald-500" />
            <input
              type="text"
              data-search-input="true"
              value={searchKeyword}
              onChange={(e) => {
                setSearchKeyword(e.target.value)
                setPage(1)
              }}
              placeholder={t('actions.searchPlaceholder')}
              className="w-full rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] py-2 pr-9 pl-10 text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] shadow-xs backdrop-blur-md transition-all hover:border-[var(--border-strong)] focus:border-emerald-500/80 focus:bg-[var(--bg-surface)] focus:shadow-[0_0_16px_rgba(16,185,129,0.15)] focus:ring-2 focus:ring-emerald-500/20 focus:outline-none"
            />
            {searchKeyword ? (
              <button
                type="button"
                onClick={() => {
                  setSearchKeyword('')
                  setPage(1)
                }}
                className="absolute top-1/2 right-3 -translate-y-1/2 rounded-md p-0.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)]"
                title={t('actions.clearSearch')}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            ) : (
              <kbd className="pointer-events-none absolute top-1/2 right-3 hidden -translate-y-1/2 rounded border border-[var(--border)]/70 bg-[var(--bg-surface)]/70 px-1 font-mono text-[10px] text-[var(--text-muted)] shadow-2xs sm:inline-block">
                /
              </kbd>
            )}
          </div>

          <div className="flex items-center gap-2">
            {/* 最近一次重提任务报告查看模块 */}
            {reextractProgress &&
              (reextractProgress.status === 'completed' || reextractProgress.status === 'failed') &&
              !isTaskRunning && (
                <button
                  type="button"
                  onClick={handleOpenReport}
                  className={`inline-flex items-center gap-1.5 rounded-xl border px-3 py-2 text-xs font-medium transition-colors ${
                    reextractProgress.failed > 0
                      ? 'border-amber-500/40 bg-amber-500/10 text-amber-400 hover:bg-amber-500/20'
                      : 'border-emerald-500/30 bg-emerald-500/5 text-emerald-400 hover:bg-emerald-500/15'
                  }`}
                  title={t('reextract.viewReportTooltip')}
                >
                  <FileText className="h-3.5 w-3.5" />
                  <span className="hidden sm:inline">
                    {t('reextract.lastReportBadge', {
                      succeeded: reextractProgress.succeeded,
                      failed: reextractProgress.failed,
                    })}
                  </span>
                  <span className="sm:hidden">{t('reextract.viewReport')}</span>
                </button>
              )}

            {/* 一键重新提取人脸特征（支持后台异步任务感知） */}
            <button
              type="button"
              onClick={isTaskRunning ? handleOpenReport : handleOpenReextract}
              disabled={(!stats.algoReady || stats.totalFaces === 0) && !isTaskRunning}
              className={`inline-flex items-center gap-1.5 rounded-xl border px-3 py-2 text-sm font-medium transition-colors ${
                isTaskRunning
                  ? 'border-emerald-500/50 bg-emerald-500/10 text-emerald-400 hover:bg-emerald-500/20'
                  : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] hover:border-emerald-500/40 hover:text-emerald-400 disabled:cursor-not-allowed disabled:opacity-40'
              }`}
              title={reextractTooltip}
            >
              <RefreshCw
                className={`h-4 w-4 ${isTaskRunning ? 'animate-spin text-emerald-400' : ''}`}
              />
              <span className="hidden sm:inline">
                {isTaskRunning
                  ? t('reextract.runningBadge', { percent: reextractPercent })
                  : t('actions.reextractFeatures')}
              </span>
            </button>

            <button
              type="button"
              onClick={() => loadData()}
              disabled={isLoading}
              className="flex h-9 w-9 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] transition-colors hover:border-emerald-500/40 hover:text-emerald-400"
              title={t('actions.refresh')}
            >
              <RotateCw className={`h-4 w-4 ${isLoading ? 'animate-spin' : ''}`} />
            </button>

            <button
              type="button"
              onClick={handleOpenRegister}
              className="inline-flex items-center gap-2 rounded-xl bg-emerald-500 px-4 py-2 text-sm font-semibold text-black shadow-sm transition-all hover:bg-emerald-400"
            >
              <UserPlus className="h-4 w-4" />
              {t('actions.register')}
            </button>
          </div>
        </div>
      </div>

      {/* 主体卡片网格视口 (纯通透滚动容器) */}
      <div className="flex-1 overflow-y-auto pr-1">
        {isLoading && items.length === 0 ? (
          <div className="flex h-64 items-center justify-center text-[var(--text-muted)]">
            <Loader2 className="h-8 w-8 animate-spin text-emerald-500" />
          </div>
        ) : items.length === 0 ? (
          <div className="flex flex-col items-center justify-center rounded-3xl border border-dashed border-[var(--border)] bg-[var(--bg-secondary)]/50 py-20 text-center">
            <div className="mb-3 flex h-14 w-14 items-center justify-center rounded-2xl bg-emerald-500/10 text-emerald-400">
              <Users className="h-7 w-7 opacity-70" />
            </div>
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              {searchKeyword ? t('empty.noSearchResult') : t('empty.title')}
            </h3>
            <p className="mt-1 max-w-sm text-xs text-[var(--text-muted)]">{t('empty.desc')}</p>
            {!searchKeyword && (
              <button
                type="button"
                onClick={handleOpenRegister}
                className="mt-5 inline-flex items-center gap-2 rounded-xl bg-emerald-500 px-4 py-2 text-sm font-semibold text-black shadow-sm hover:bg-emerald-400"
              >
                <UserPlus className="h-4 w-4" />
                {t('actions.register')}
              </button>
            )}
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {items.map((person) => (
              <PersonnelCard
                key={person.id}
                person={person}
                onView={handleOpenView}
                onEdit={handleOpenEdit}
                onDelete={handleOpenDelete}
                onAddFace={handleOpenAddFace}
              />
            ))}
          </div>
        )}
      </div>

      {/* 分页控制栏 (常驻吸底工规条，支持每页条数选择) */}
      <div className="frosted-glass flex shrink-0 flex-wrap items-center justify-between gap-3 rounded-2xl px-4 py-2.5 text-xs text-[var(--text-secondary)] shadow-xs">
        <div className="flex items-center gap-3">
          <span>{t('pagination.page', { current: page })}</span>
          <span className="font-mono text-[var(--text-muted)]">
            ({t('pagination.total', { total })})
          </span>

          {/* 每页条数选择器 */}
          <div className="flex items-center gap-1.5 border-l border-[var(--border)] pl-3">
            <select
              value={limit}
              onChange={(e) => {
                const next = Number(e.target.value)
                setLimit(next)
                setPage(1)
              }}
              className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-xs text-[var(--text-primary)] transition-all outline-none hover:border-emerald-500 focus:border-emerald-500"
              title={t('pagination.pageSize')}
            >
              {[12, 24, 48, 96].map((size) => (
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
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-emerald-500 disabled:opacity-40"
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
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-emerald-500 disabled:opacity-40"
          >
            {t('pagination.next')}
          </button>
        </div>
      </div>

      {/* 录入/编辑弹窗 */}
      <PersonnelModal
        isOpen={isModalOpen}
        onClose={() => setIsModalOpen(false)}
        onSuccess={handleModalSuccess}
        editTarget={editTarget}
        onManagePhotos={handleOpenAddFace}
      />

      {/* 详情与多图抽屉 */}
      <PersonnelDetailDrawer
        isOpen={Boolean(drawerSubjectId)}
        subjectId={drawerSubjectId}
        autoOpenUpload={drawerAutoOpenUpload}
        onClose={() => {
          setDrawerSubjectId(null)
          setDrawerAutoOpenUpload(false)
        }}
        onUpdate={loadData}
      />

      {/* 删除确认弹窗 */}
      <DeleteConfirmModal
        isOpen={Boolean(deleteTarget)}
        target={deleteTarget}
        isDeleting={isDeleting}
        onClose={() => setDeleteTarget(null)}
        onConfirm={handleConfirmDelete}
      />

      {/* 一键重新提取特征确认与实时进度弹窗 */}
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

      {/* 任务完成即时反馈 Toast 提示卡片 */}
      {toast && (
        <div className="animate-in slide-in-from-top-4 fade-in fixed top-5 right-5 z-50 flex max-w-md items-start gap-3 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4 shadow-xl backdrop-blur-md">
          <div
            className={`mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg ${
              toast.type === 'error'
                ? 'bg-rose-500/10 text-rose-400'
                : toast.type === 'warning'
                  ? 'bg-amber-500/10 text-amber-400'
                  : 'bg-emerald-500/10 text-emerald-400'
            }`}
          >
            {toast.type === 'error' || toast.type === 'warning' ? (
              <AlertCircle className="h-4 w-4" />
            ) : (
              <CheckCircle2 className="h-4 w-4" />
            )}
          </div>
          <div className="flex-1">
            <h4 className="text-xs font-semibold text-[var(--text-primary)]">{toast.title}</h4>
            <p className="mt-0.5 text-xs text-[var(--text-secondary)]">{toast.message}</p>
            <div className="mt-2 flex items-center gap-2">
              <button
                type="button"
                onClick={() => {
                  setToast(null)
                  handleOpenReport()
                }}
                className="text-xs font-medium text-emerald-400 hover:underline"
              >
                {t('reextract.viewReport')}
              </button>
            </div>
          </div>
          <button
            type="button"
            onClick={() => setToast(null)}
            className="text-[var(--text-muted)] hover:text-[var(--text-primary)]"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}
    </div>
  )
}
