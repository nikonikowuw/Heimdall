import React, { useState } from 'react'
import { Check, Copy, Eye, ImagePlus, Pencil, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { evidenceApi } from '../../../lib/api'
import { copyToClipboard } from '../../../lib/utils'
import type { PersonnelItem } from '../../../types'

export interface PersonnelTableProps {
  items: PersonnelItem[]
  selectedIds: Set<string>
  onToggleSelect: (subjectId: string) => void
  onToggleSelectAll: () => void
  isAllSelected: boolean
  onView: (person: PersonnelItem) => void
  onEdit: (person: PersonnelItem) => void
  onDelete: (person: PersonnelItem) => void
  onAddFace?: (person: PersonnelItem) => void
}

const AVATAR_PALETTES = [
  {
    bg: 'from-blue-500/15 to-indigo-500/15 dark:from-blue-500/25 dark:to-indigo-500/25',
    text: 'text-indigo-600 dark:text-indigo-400',
  },
  {
    bg: 'from-emerald-500/15 to-teal-500/15 dark:from-emerald-500/25 dark:to-teal-500/25',
    text: 'text-emerald-600 dark:text-emerald-400',
  },
  {
    bg: 'from-violet-500/15 to-purple-500/15 dark:from-violet-500/25 dark:to-purple-500/25',
    text: 'text-violet-600 dark:text-violet-400',
  },
  {
    bg: 'from-amber-500/15 to-orange-500/15 dark:from-amber-500/25 dark:to-orange-500/25',
    text: 'text-amber-600 dark:text-amber-400',
  },
  {
    bg: 'from-cyan-500/15 to-sky-500/15 dark:from-cyan-500/25 dark:to-sky-500/25',
    text: 'text-cyan-600 dark:text-cyan-400',
  },
]

function getAvatarInitials(name: string): string {
  if (!name) return '?'
  const trimmed = name.trim()
  if (trimmed.length <= 2) return trimmed
  if (trimmed.length === 3) return trimmed.slice(1)
  return trimmed.slice(0, 2)
}

function getAvatarPalette(name: string) {
  let hash = 0
  for (let i = 0; i < name.length; i++) hash = (hash << 5) - hash + name.charCodeAt(i)
  return AVATAR_PALETTES[Math.abs(hash) % AVATAR_PALETTES.length]
}

function formatDate(ts: number): string {
  const d = new Date(ts)
  const y = d.getFullYear()
  const m = String(d.getMonth() + 1).padStart(2, '0')
  const day = String(d.getDate()).padStart(2, '0')
  return `${y}-${m}-${day}`
}

/** 5 段式微型样本点阵槽：体现工业边缘硬件的量化刻度感 */
export function SampleHealthDots({ count }: { count: number }): React.ReactElement {
  const safeCount = Math.max(0, Math.min(5, count))
  const isSaturated = safeCount >= 5

  let textColor = 'text-[var(--text-muted)]'
  if (isSaturated) {
    textColor = 'text-emerald-500'
  } else if (safeCount > 0) {
    textColor = 'text-amber-500'
  }

  return (
    <div className="inline-flex items-center gap-2">
      <div className="flex items-center gap-1">
        {[1, 2, 3, 4, 5].map((idx) => {
          let dotColor = 'bg-[var(--border-strong)]/40'
          if (idx <= safeCount) {
            dotColor = isSaturated
              ? 'bg-emerald-500 shadow-[0_0_5px_rgba(16,185,129,0.5)]'
              : 'bg-amber-500 shadow-[0_0_4px_rgba(245,158,11,0.4)]'
          }
          return <span key={idx} className={`h-2 w-2 rounded-xs transition-colors ${dotColor}`} />
        })}
      </div>
      <span className={`font-data text-xs font-semibold tabular-nums ${textColor}`}>
        {safeCount}/5
      </span>
    </div>
  )
}

export function PersonnelTable({
  items,
  selectedIds,
  onToggleSelect,
  onToggleSelectAll,
  isAllSelected,
  onView,
  onEdit,
  onDelete,
  onAddFace,
}: PersonnelTableProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const [copiedId, setCopiedId] = useState<string | null>(null)

  const handleCopy = async (e: React.MouseEvent, text: string, subjectId: string) => {
    e.stopPropagation()
    const success = await copyToClipboard(text)
    if (success) {
      setCopiedId(subjectId)
      setTimeout(() => setCopiedId(null), 1200)
    }
  }

  return (
    <div className="frosted-glass overflow-hidden rounded-2xl border border-[var(--border)] shadow-xs">
      <div className="w-full overflow-x-auto">
        <table className="w-full min-w-[860px] border-collapse text-left text-xs">
          <thead>
            <tr className="border-b border-[var(--border)] bg-[var(--bg-secondary)]/60 text-[11px] font-semibold text-[var(--text-secondary)]">
              {/* 复选框 */}
              <th scope="col" className="w-12 px-3.5 py-3 text-center">
                <input
                  type="checkbox"
                  aria-label={t('table.selectAll')}
                  checked={isAllSelected && items.length > 0}
                  onChange={onToggleSelectAll}
                  className="h-4 w-4 cursor-pointer rounded-md border-[var(--border-strong)] text-[var(--accent)] accent-[var(--accent)] transition-all focus:ring-2 focus:ring-[var(--ring)]"
                />
              </th>

              {/* 人员档案 */}
              <th scope="col" className="px-3.5 py-3 font-medium">
                {t('table.person')}
              </th>

              {/* 工号 */}
              <th scope="col" className="px-3.5 py-3 font-medium">
                {t('table.subjectId')}
              </th>

              {/* 证件号 */}
              <th scope="col" className="px-3.5 py-3 font-medium">
                {t('table.idCard')}
              </th>

              {/* 部门/备注 */}
              <th scope="col" className="px-3.5 py-3 font-medium">
                {t('table.remark')}
              </th>

              {/* 样本健康度 */}
              <th scope="col" className="px-3.5 py-3 font-medium">
                {t('table.sampleHealth')}
              </th>

              {/* 录入时间 */}
              <th scope="col" className="px-3.5 py-3 font-medium">
                {t('table.createdAt')}
              </th>

              {/* 操作 */}
              <th scope="col" className="w-36 px-3.5 py-3 text-right font-medium">
                {t('table.actions')}
              </th>
            </tr>
          </thead>

          <tbody className="divide-y divide-[var(--border)]">
            {items.map((person) => {
              const isSelected = selectedIds.has(person.subjectId)
              const hasPhoto = Boolean(person.primaryPhotoPath)
              const palette = getAvatarPalette(person.name)
              const initials = getAvatarInitials(person.name)
              const avatarUrl = hasPhoto ? evidenceApi.getImageUrl(person.primaryPhotoPath) : ''
              const isSaturated = person.faceCount >= 5

              return (
                <tr
                  key={person.id}
                  onClick={() => onView(person)}
                  className={`group cursor-pointer transition-colors ${
                    isSelected
                      ? 'bg-[var(--accent-soft)]/50'
                      : 'hover:bg-[var(--bg-secondary)]/50 dark:hover:bg-white/[0.02]'
                  }`}
                >
                  {/* 复选框 */}
                  <td className="px-3.5 py-2.5 text-center" onClick={(e) => e.stopPropagation()}>
                    <input
                      type="checkbox"
                      aria-label={t('table.selectRow', { name: person.name })}
                      checked={isSelected}
                      onChange={() => onToggleSelect(person.subjectId)}
                      className="h-4 w-4 cursor-pointer rounded-md border-[var(--border-strong)] text-[var(--accent)] accent-[var(--accent)] transition-all focus:ring-2 focus:ring-[var(--ring)]"
                    />
                  </td>

                  {/* 人员基本信息 (头像 + 姓名) */}
                  <td className="px-3.5 py-2.5 whitespace-nowrap">
                    <div className="flex items-center gap-3">
                      <div className="relative h-12 w-12 shrink-0 overflow-hidden rounded-2xl border border-black/10 bg-[var(--bg-secondary)] shadow-xs dark:border-white/10">
                        {avatarUrl ? (
                          <img
                            src={avatarUrl}
                            alt={person.name}
                            loading="lazy"
                            className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
                          />
                        ) : (
                          <div
                            className={`flex h-full w-full items-center justify-center bg-gradient-to-br ${palette.bg}`}
                          >
                            <span className={`font-mono text-sm font-extrabold ${palette.text}`}>
                              {initials}
                            </span>
                          </div>
                        )}
                      </div>
                      <div className="min-w-0">
                        <div className="flex items-center gap-1.5">
                          <span className="truncate text-sm font-semibold text-[var(--text-primary)] transition-colors group-hover:text-[var(--accent)]">
                            {person.name}
                          </span>
                        </div>
                      </div>
                    </div>
                  </td>

                  {/* 工号 / Subject ID (绝对不折行) */}
                  <td className="px-3.5 py-2.5 whitespace-nowrap">
                    <div className="inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap">
                      <span className="font-data inline-flex items-center rounded-md border border-[var(--border)] bg-[var(--bg-secondary)]/80 px-2 py-0.5 text-xs font-semibold whitespace-nowrap text-[var(--accent)] shadow-2xs select-text">
                        {`#${person.subjectId}`}
                      </span>
                      <button
                        type="button"
                        aria-label={t('common:copy')}
                        onClick={(e) => handleCopy(e, person.subjectId, person.subjectId)}
                        className="rounded p-1 text-[var(--text-muted)] opacity-0 transition-opacity group-hover:opacity-100 hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                        title={t('common:copy')}
                      >
                        {copiedId === person.subjectId ? (
                          <Check className="h-3 w-3 text-emerald-500" />
                        ) : (
                          <Copy className="h-3 w-3" />
                        )}
                      </button>
                    </div>
                  </td>

                  {/* 证件号 */}
                  <td className="px-3.5 py-2.5 whitespace-nowrap">
                    {person.idCard ? (
                      <span className="font-data text-xs whitespace-nowrap text-[var(--text-secondary)] select-text">
                        {person.idCard}
                      </span>
                    ) : (
                      <span className="text-xs text-[var(--text-muted)]">—</span>
                    )}
                  </td>

                  {/* 部门 / 备注 */}
                  <td className="max-w-[200px] truncate px-3.5 py-2.5 whitespace-nowrap">
                    {person.remark ? (
                      <span className="inline-flex items-center rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-0.5 text-xs text-[var(--text-secondary)]">
                        {person.remark}
                      </span>
                    ) : (
                      <span className="text-xs text-[var(--text-muted)]">—</span>
                    )}
                  </td>

                  {/* 样本健康度 */}
                  <td className="px-3.5 py-2.5 whitespace-nowrap">
                    <SampleHealthDots count={person.faceCount} />
                  </td>

                  {/* 录入时间 */}
                  <td className="font-data px-3.5 py-2.5 text-xs whitespace-nowrap text-[var(--text-muted)] tabular-nums">
                    {formatDate(person.createdAt)}
                  </td>

                  {/* 操作列 */}
                  <td className="px-3.5 py-2.5 text-right" onClick={(e) => e.stopPropagation()}>
                    <div className="flex items-center justify-end gap-1">
                      <button
                        type="button"
                        onClick={() => onView(person)}
                        title={t('actions.viewDetails')}
                        aria-label={t('actions.viewDetails')}
                        className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      >
                        <Eye className="h-3.5 w-3.5" />
                      </button>

                      {!isSaturated && onAddFace && (
                        <button
                          type="button"
                          onClick={() => onAddFace(person)}
                          title={t('actions.addFacesShort')}
                          aria-label={t('actions.addFacesShort')}
                          className="flex h-7 w-7 items-center justify-center rounded-lg text-emerald-600 transition-colors hover:bg-emerald-500/10 dark:text-emerald-400"
                        >
                          <ImagePlus className="h-3.5 w-3.5" />
                        </button>
                      )}

                      <button
                        type="button"
                        onClick={() => onEdit(person)}
                        title={t('actions.editInfo')}
                        aria-label={t('actions.editInfo')}
                        className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>

                      <button
                        type="button"
                        onClick={() => onDelete(person)}
                        title={t('actions.delete')}
                        aria-label={t('actions.delete')}
                        className="flex h-7 w-7 items-center justify-center rounded-lg text-rose-500 transition-colors hover:bg-rose-500/10"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}

export function PersonnelTableSkeleton(): React.ReactElement {
  return (
    <div className="frosted-glass animate-pulse overflow-hidden rounded-2xl border border-[var(--border)] shadow-xs">
      <div className="w-full overflow-x-auto">
        <table className="w-full min-w-[860px] border-collapse text-left text-xs">
          <thead>
            <tr className="border-b border-[var(--border)] bg-[var(--bg-secondary)]/60 text-[11px]">
              <th className="w-12 px-3.5 py-3">
                <div className="mx-auto h-4 w-4 rounded bg-[var(--bg-secondary)]" />
              </th>
              <th className="px-3.5 py-3">
                <div className="h-4 w-20 rounded bg-[var(--bg-secondary)]" />
              </th>
              <th className="px-3.5 py-3">
                <div className="h-4 w-16 rounded bg-[var(--bg-secondary)]" />
              </th>
              <th className="px-3.5 py-3">
                <div className="h-4 w-20 rounded bg-[var(--bg-secondary)]" />
              </th>
              <th className="px-3.5 py-3">
                <div className="h-4 w-24 rounded bg-[var(--bg-secondary)]" />
              </th>
              <th className="px-3.5 py-3">
                <div className="h-4 w-20 rounded bg-[var(--bg-secondary)]" />
              </th>
              <th className="px-3.5 py-3">
                <div className="h-4 w-16 rounded bg-[var(--bg-secondary)]" />
              </th>
              <th className="w-36 px-3.5 py-3 text-right">
                <div className="ml-auto h-4 w-12 rounded bg-[var(--bg-secondary)]" />
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--border)]">
            {[1, 2, 3, 4, 5, 6, 7, 8].map((i) => (
              <tr key={i}>
                <td className="px-3.5 py-3 text-center">
                  <div className="mx-auto h-4 w-4 rounded bg-[var(--bg-secondary)]" />
                </td>
                <td className="px-3.5 py-3">
                  <div className="flex items-center gap-3">
                    <div className="h-12 w-12 rounded-2xl bg-[var(--bg-secondary)]" />
                    <div className="h-4 w-20 rounded bg-[var(--bg-secondary)]" />
                  </div>
                </td>
                <td className="px-3.5 py-3">
                  <div className="h-4 w-16 rounded bg-[var(--bg-secondary)]" />
                </td>
                <td className="px-3.5 py-3">
                  <div className="h-4 w-28 rounded bg-[var(--bg-secondary)]" />
                </td>
                <td className="px-3.5 py-3">
                  <div className="h-4 w-20 rounded bg-[var(--bg-secondary)]" />
                </td>
                <td className="px-3.5 py-3">
                  <div className="h-4 w-24 rounded bg-[var(--bg-secondary)]" />
                </td>
                <td className="px-3.5 py-3">
                  <div className="h-4 w-20 rounded bg-[var(--bg-secondary)]" />
                </td>
                <td className="px-3.5 py-3 text-right">
                  <div className="ml-auto h-6 w-20 rounded bg-[var(--bg-secondary)]" />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
