import React, { useState, useEffect, useCallback } from 'react'
import {
  Users,
  UserPlus,
  Search,
  RotateCw,
  Cpu,
  Layers,
  CheckCircle2,
  AlertCircle,
  Loader2,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { personnelApi } from '../../lib/api'
import type { PersonnelItem, PersonnelStats } from '../../types'
import { PersonnelCard } from './components/PersonnelCard'
import { PersonnelModal } from './components/PersonnelModal'
import { PersonnelDetailDrawer } from './components/PersonnelDetailDrawer'
import { DeleteConfirmModal } from './components/DeleteConfirmModal'

export const PersonnelPage: React.FC = () => {
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
  const limit = 24

  // 模态框与抽屉状态
  const [isModalOpen, setIsModalOpen] = useState(false)
  const [editTarget, setEditTarget] = useState<PersonnelItem | null>(null)

  const [drawerSubjectId, setDrawerSubjectId] = useState<string | null>(null)
  const [drawerAutoOpenUpload, setDrawerAutoOpenUpload] = useState(false)

  const [deleteTarget, setDeleteTarget] = useState<PersonnelItem | null>(null)
  const [isDeleting, setIsDeleting] = useState(false)

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

  const handleModalSuccess = () => {
    loadData()
  }

  const totalPages = Math.ceil(total / limit)

  return (
    <div className="flex h-full flex-col space-y-6 overflow-y-auto bg-[var(--bg-primary)] p-6 md:p-8">
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
          <div className="flex items-center gap-3 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-2.5 shadow-xs">
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

          <div className="flex items-center gap-3 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-2.5 shadow-xs">
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

          <div className="flex items-center gap-3 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-2.5 shadow-xs">
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
        <div className="relative max-w-md flex-1">
          <Search className="absolute top-1/2 left-3.5 h-4 w-4 -translate-y-1/2 text-[var(--text-muted)]" />
          <input
            type="text"
            value={searchKeyword}
            onChange={(e) => {
              setSearchKeyword(e.target.value)
              setPage(1)
            }}
            placeholder={t('actions.searchPlaceholder')}
            className="w-full rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] py-2 pr-4 pl-10 text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] shadow-xs focus:border-emerald-500/60 focus:outline-none"
          />
        </div>

        <div className="flex items-center gap-2">
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

      {/* 主体卡片网格 */}
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

      {/* 分页控制 */}
      {totalPages > 1 && (
        <div className="flex items-center justify-between border-t border-[var(--border)] pt-4 text-xs text-[var(--text-muted)]">
          <span>共 {total} 位人员</span>
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={page <= 1}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              className="rounded-lg border border-[var(--border)] px-3 py-1.5 hover:bg-[var(--bg-secondary)] disabled:opacity-40"
            >
              上一页
            </button>
            <span>
              {page} / {totalPages}
            </span>
            <button
              type="button"
              disabled={page >= totalPages}
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              className="rounded-lg border border-[var(--border)] px-3 py-1.5 hover:bg-[var(--bg-secondary)] disabled:opacity-40"
            >
              下一页
            </button>
          </div>
        </div>
      )}

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
    </div>
  )
}
