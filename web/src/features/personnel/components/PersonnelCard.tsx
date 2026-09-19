import React, { useEffect, useRef, useState } from 'react'
import {
  ArrowRight,
  Check,
  Copy,
  CreditCard,
  Eye,
  ImagePlus,
  MoreHorizontal,
  Pencil,
  Sparkles,
  Trash2,
  User,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { evidenceApi } from '@/lib/api'
import { copyToClipboard } from '@/lib/utils'
import type { PersonnelItem } from '@/types'

export interface PersonnelCardProps {
  person: PersonnelItem
  onView: (person: PersonnelItem) => void
  onEdit: (person: PersonnelItem) => void
  onDelete: (person: PersonnelItem) => void
  onAddFace?: (person: PersonnelItem) => void
  isSelected?: boolean
  onToggleSelect?: (subjectId: string) => void
}

const AVATAR_PALETTES = [
  {
    bg: 'from-blue-600/20 via-indigo-500/15 to-sky-500/20 dark:from-blue-600/30 dark:via-indigo-500/25 dark:to-sky-500/30',
    text: 'text-blue-600 dark:text-blue-400',
    ring: 'border-blue-500/30',
  },
  {
    bg: 'from-emerald-600/20 via-teal-500/15 to-cyan-500/20 dark:from-emerald-600/30 dark:via-teal-500/25 dark:to-cyan-500/30',
    text: 'text-emerald-600 dark:text-emerald-400',
    ring: 'border-emerald-500/30',
  },
  {
    bg: 'from-purple-600/20 via-violet-500/15 to-pink-500/20 dark:from-purple-600/30 dark:via-violet-500/25 dark:to-pink-500/30',
    text: 'text-purple-600 dark:text-purple-400',
    ring: 'border-purple-500/30',
  },
  {
    bg: 'from-amber-600/20 via-orange-500/15 to-yellow-500/20 dark:from-amber-600/30 dark:via-orange-500/25 dark:to-yellow-500/30',
    text: 'text-amber-600 dark:text-amber-400',
    ring: 'border-amber-500/30',
  },
  {
    bg: 'from-cyan-600/20 via-sky-500/15 to-indigo-500/20 dark:from-cyan-600/30 dark:via-sky-500/25 dark:to-indigo-500/30',
    text: 'text-cyan-600 dark:text-cyan-400',
    ring: 'border-cyan-500/30',
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

export function PersonnelCard({
  person,
  onView,
  onEdit,
  onDelete,
  onAddFace,
  isSelected = false,
  onToggleSelect,
}: PersonnelCardProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const [menuOpen, setMenuOpen] = useState(false)
  const [copied, setCopied] = useState(false)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const hasPhoto = Boolean(person.primaryPhotoPath)
  const isSaturated = person.faceCount >= 5
  const palette = getAvatarPalette(person.name)
  const initials = getAvatarInitials(person.name)
  const avatarUrl = hasPhoto ? evidenceApi.getImageUrl(person.primaryPhotoPath) : ''

  useEffect(() => {
    if (!menuOpen) return

    const handleClickOutside = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setMenuOpen(false)
      }
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setMenuOpen(false)
    }

    document.addEventListener('mousedown', handleClickOutside)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('mousedown', handleClickOutside)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [menuOpen])

  useEffect(
    () => () => {
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
    },
    [],
  )

  const handleCardClick = () => {
    onView(person)
  }

  const handleCardKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget) return
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onView(person)
    }
  }

  const handleCopyId = async (event: React.MouseEvent) => {
    event.stopPropagation()
    const textToCopy = person.subjectId
    const success = await copyToClipboard(textToCopy)
    if (!success) return

    setCopied(true)
    if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
    copiedTimerRef.current = setTimeout(() => {
      setCopied(false)
      setMenuOpen(false)
    }, 1200)
  }

  return (
    <article
      role="button"
      tabIndex={0}
      onClick={handleCardClick}
      onKeyDown={handleCardKeyDown}
      aria-label={`${person.name} (${person.subjectId})`}
      className={`frosted-glass-interactive group relative flex min-w-0 cursor-pointer flex-col justify-between rounded-2xl border p-3.5 text-left shadow-xs transition-all duration-200 ease-out hover:-translate-y-1 hover:shadow-lg focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
        menuOpen ? 'z-50' : 'hover:z-30'
      } ${
        isSelected
          ? 'z-10 border-[var(--accent)] bg-[var(--accent-soft)]/20 shadow-md ring-2 ring-[var(--accent)]/45'
          : 'z-1 border-[var(--border)] bg-white/80 hover:border-[var(--border-strong)] dark:bg-[var(--bg-surface-solid)]/90'
      }`}
    >
      {/* 顶部微细环境高光线（未选中时展示，选中时让出纯正蓝色高光） */}
      {!isSelected && (
        <div className="pointer-events-none absolute inset-x-0 top-0 h-px rounded-t-2xl bg-gradient-to-r from-transparent via-white/60 to-transparent dark:via-white/10" />
      )}

      {/* ── 核心主体：左侧黄金人像相框 + 右侧饱满信息流 ── */}
      <div className="flex min-w-0 items-start gap-3.5">
        {/* ── 1. 左侧：独立 4:5 肖像相框 (严丝合缝，100% 完整展示人像，无左右黑边) ── */}
        <div className="relative h-36 w-28 shrink-0 overflow-hidden rounded-xl border border-black/10 bg-[var(--bg-secondary)] shadow-inner sm:h-40 sm:w-32 dark:border-white/10">
          {avatarUrl ? (
            <img
              src={avatarUrl}
              alt={person.name}
              loading="lazy"
              decoding="async"
              className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
            />
          ) : (
            <div
              className={`flex h-full w-full flex-col items-center justify-center bg-gradient-to-br ${palette.bg} p-2 text-center`}
            >
              <div
                className={`flex h-12 w-12 items-center justify-center rounded-xl border ${palette.ring} bg-white/40 shadow-inner backdrop-blur-sm dark:bg-black/20`}
              >
                <span className={`text-xl font-extrabold tracking-tight ${palette.text}`}>
                  {initials}
                </span>
              </div>
              <span className="mt-1.5 flex items-center gap-1 text-[10px] font-medium text-[var(--text-muted)]">
                <User className="h-3 w-3 opacity-60" />
                <span>{t('card.noPhoto')}</span>
              </span>
            </div>
          )}

          {/* 若是主头像，左下角展示精致小徽标 */}
          {hasPhoto && (
            <div className="absolute bottom-1.5 left-1.5 z-10">
              <span className="inline-flex items-center gap-0.5 rounded-md border border-emerald-500/30 bg-black/65 px-1.5 py-0.5 text-[9px] font-semibold text-emerald-400 shadow-xs backdrop-blur-md">
                <Sparkles className="h-2.5 w-2.5 text-emerald-400" />
                <span>{t('card.primary', { defaultValue: '主' })}</span>
              </span>
            </div>
          )}
        </div>

        {/* ── 2. 右侧：紧凑充实的信息区 ── */}
        <div className="flex min-w-0 flex-1 flex-col justify-between self-stretch">
          <div>
            {/* 顶排：选择框 + 5 点刻度 + 更多菜单 */}
            <div className="flex items-center justify-between gap-1.5">
              <div className="flex items-center gap-1.5">
                {onToggleSelect && (
                  <div
                    className="flex items-center"
                    onClick={(e) => {
                      e.stopPropagation()
                      onToggleSelect(person.subjectId)
                    }}
                  >
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => {}}
                      aria-label={t('card.select', { defaultValue: '选择人员' })}
                      className={`h-4 w-4 cursor-pointer rounded-md border-[var(--border-strong)] text-[var(--accent)] accent-[var(--accent)] transition-all focus:ring-2 focus:ring-[var(--ring)] ${
                        isSelected ? 'opacity-100' : 'opacity-60 group-hover:opacity-100'
                      }`}
                    />
                  </div>
                )}

                {/* 5 段式微型样本健康度 */}
                <div
                  className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-medium transition-colors ${
                    isSaturated
                      ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                      : 'border-amber-500/25 bg-amber-500/10 text-amber-600 dark:text-amber-400'
                  }`}
                >
                  <div className="flex items-center gap-0.5">
                    {[1, 2, 3, 4, 5].map((slot) => {
                      let dotColor = 'bg-black/15 dark:bg-white/15'
                      if (slot <= person.faceCount) {
                        dotColor = isSaturated
                          ? 'bg-emerald-500 shadow-[0_0_3px_rgba(16,185,129,0.6)]'
                          : 'bg-amber-500'
                      }
                      return (
                        <span
                          key={slot}
                          className={`h-1.5 w-1.5 rounded-xs transition-colors ${dotColor}`}
                        />
                      )
                    })}
                  </div>
                  <span className="font-data font-semibold tabular-nums">{person.faceCount}/5</span>
                </div>
              </div>

              {/* 更多菜单按钮 */}
              <div className="relative" ref={menuRef} onClick={(e) => e.stopPropagation()}>
                <button
                  type="button"
                  aria-label={t('actions.more', { defaultValue: '更多操作' })}
                  aria-expanded={menuOpen}
                  aria-haspopup="menu"
                  onClick={() => setMenuOpen((prev) => !prev)}
                  className="flex h-6 w-6 items-center justify-center rounded-md text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                >
                  <MoreHorizontal className="h-3.5 w-3.5" />
                </button>

                {menuOpen && (
                  <div
                    role="menu"
                    className="lens-glass absolute top-full right-0 z-50 mt-1.5 w-max min-w-[160px] rounded-xl border border-[var(--border-strong)] bg-[var(--bg-surface-solid)]/98 p-1.5 whitespace-nowrap shadow-2xl backdrop-blur-xl dark:bg-[var(--bg-secondary)]/98"
                  >
                    <button
                      type="button"
                      role="menuitem"
                      onClick={(e) => {
                        e.stopPropagation()
                        setMenuOpen(false)
                        onView(person)
                      }}
                      className="flex min-h-7 w-full items-center gap-2 rounded-lg px-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                    >
                      <Eye className="h-3.5 w-3.5 opacity-70" />
                      <span>{t('actions.viewDetails', { defaultValue: '查看档案' })}</span>
                    </button>

                    <button
                      type="button"
                      role="menuitem"
                      onClick={(e) => {
                        e.stopPropagation()
                        setMenuOpen(false)
                        onEdit(person)
                      }}
                      className="flex min-h-7 w-full items-center gap-2 rounded-lg px-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                    >
                      <Pencil className="h-3.5 w-3.5 opacity-70" />
                      <span>{t('actions.editInfo', { defaultValue: '编辑资料' })}</span>
                    </button>

                    {!isSaturated && onAddFace && (
                      <button
                        type="button"
                        role="menuitem"
                        onClick={(e) => {
                          e.stopPropagation()
                          setMenuOpen(false)
                          onAddFace(person)
                        }}
                        className="flex min-h-7 w-full items-center gap-2 rounded-lg px-2 text-xs font-medium text-emerald-600 transition-colors hover:bg-emerald-500/10 dark:text-emerald-400"
                      >
                        <ImagePlus className="h-3.5 w-3.5 opacity-80" />
                        <span>{t('actions.addFaces', { defaultValue: '追加人脸' })}</span>
                      </button>
                    )}

                    <button
                      type="button"
                      role="menuitem"
                      onClick={handleCopyId}
                      className="flex min-h-7 w-full items-center gap-2 rounded-lg px-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                    >
                      {copied ? (
                        <Check className="h-3.5 w-3.5 text-emerald-500" />
                      ) : (
                        <Copy className="h-3.5 w-3.5 opacity-70" />
                      )}
                      <span>
                        {copied
                          ? t('actions.copied', { defaultValue: '已复制' })
                          : t('common:copy', { defaultValue: '复制工号' })}
                      </span>
                    </button>

                    <div className="my-1 border-t border-[var(--border)]" />

                    <button
                      type="button"
                      role="menuitem"
                      onClick={(e) => {
                        e.stopPropagation()
                        setMenuOpen(false)
                        onDelete(person)
                      }}
                      className="flex min-h-7 w-full items-center gap-2 rounded-lg px-2 text-xs font-medium text-rose-500 transition-colors hover:bg-rose-500/10"
                    >
                      <Trash2 className="h-3.5 w-3.5 opacity-80" />
                      <span>{t('actions.delete', { defaultValue: '删除人员' })}</span>
                    </button>
                  </div>
                )}
              </div>
            </div>

            {/* 姓名与工号（绝对不换行） */}
            <div className="mt-2 space-y-1">
              <h3 className="truncate text-base font-bold tracking-tight text-[var(--text-primary)] transition-colors group-hover:text-[var(--accent)]">
                {person.name}
              </h3>

              {/* 工号胶囊：加 whitespace-nowrap 与 shrink-0，绝不拆行折断 */}
              <div
                className="font-data inline-flex max-w-full items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg-secondary)]/90 px-1.5 py-0.5 text-[11px] font-semibold whitespace-nowrap text-[var(--accent)] shadow-2xs select-text"
                onClick={(e) => e.stopPropagation()}
              >
                <span className="truncate">{`#${person.subjectId}`}</span>
                <button
                  type="button"
                  aria-label={t('common:copy')}
                  onClick={handleCopyId}
                  className="shrink-0 rounded p-0.5 text-[var(--text-muted)] opacity-60 transition-opacity hover:text-[var(--text-primary)] hover:opacity-100"
                  title={t('common:copy')}
                >
                  {copied ? (
                    <Check className="h-3 w-3 text-emerald-500" />
                  ) : (
                    <Copy className="h-3 w-3" />
                  )}
                </button>
              </div>
            </div>

            {/* 部门 / 备注 */}
            <div className="mt-1.5">
              <p className="truncate text-xs text-[var(--text-secondary)]">
                {person.remark || (
                  <span className="font-normal text-[var(--text-muted)] italic">
                    {t('card.noRemark', { defaultValue: '未填写所属部门' })}
                  </span>
                )}
              </p>
            </div>

            {/* 证件号 */}
            <div className="mt-1 flex min-h-[18px] items-center gap-1 text-xs text-[var(--text-secondary)]">
              <CreditCard className="h-3 w-3 shrink-0 text-[var(--text-muted)] opacity-70" />
              {person.idCard ? (
                <span className="font-data truncate text-[11px] whitespace-nowrap text-[var(--text-secondary)] select-text">
                  {person.idCard}
                </span>
              ) : (
                <span className="text-[11px] font-normal text-[var(--text-muted)]">
                  {t('card.noIdCard', { defaultValue: '未登记证件号' })}
                </span>
              )}
            </div>
          </div>

          {/* 录入时间与查看档案操作 */}
          <div className="mt-2 flex items-center justify-between border-t border-[var(--border)]/60 pt-2 text-[11px] text-[var(--text-muted)]">
            <span className="font-data whitespace-nowrap tabular-nums">
              {formatDate(person.createdAt)}
            </span>

            <span className="flex items-center gap-0.5 font-medium text-[var(--accent)] opacity-85 transition-opacity group-hover:opacity-100">
              <span className="text-[11px]">{t('card.viewShort', { defaultValue: '档案' })}</span>
              <ArrowRight className="h-3 w-3 transition-transform duration-200 group-hover:translate-x-0.5" />
            </span>
          </div>
        </div>
      </div>
    </article>
  )
}

/**
 * 卡片骨架屏：与紧凑工牌卡片 100% 同构
 */
export function PersonnelCardSkeleton(): React.ReactElement {
  return (
    <div
      aria-hidden="true"
      className="frosted-glass flex min-w-0 animate-pulse flex-col justify-between overflow-hidden rounded-2xl border border-[var(--border)] p-3.5"
    >
      <div className="flex min-w-0 items-start gap-3.5">
        {/* 左侧肖像骨架 */}
        <div className="h-36 w-28 shrink-0 rounded-xl bg-[var(--bg-secondary)] sm:h-40 sm:w-32" />

        {/* 右侧信息骨架 */}
        <div className="flex min-w-0 flex-1 flex-col justify-between self-stretch">
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <div className="h-4 w-14 rounded-full bg-[var(--bg-secondary)]" />
              <div className="h-5 w-5 rounded bg-[var(--bg-secondary)]" />
            </div>
            <div className="h-5 w-24 rounded bg-[var(--bg-secondary)]" />
            <div className="h-4 w-20 rounded bg-[var(--bg-secondary)]" />
            <div className="h-3.5 w-28 rounded bg-[var(--bg-secondary)]" />
          </div>

          <div className="mt-2 flex items-center justify-between border-t border-[var(--border)]/60 pt-2">
            <div className="h-3.5 w-16 rounded bg-[var(--bg-secondary)]" />
            <div className="h-3.5 w-10 rounded bg-[var(--bg-secondary)]" />
          </div>
        </div>
      </div>
    </div>
  )
}
