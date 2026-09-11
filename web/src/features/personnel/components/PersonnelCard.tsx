import React from 'react'
import { Eye, Pencil, Trash2, User, Image as ImageIcon, ImagePlus } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { evidenceApi } from '../../../lib/api'
import type { PersonnelItem } from '../../../types'
import { formatTimestamp } from '../../../lib/time'

interface PersonnelCardProps {
  person: PersonnelItem
  onView: (person: PersonnelItem) => void
  onEdit: (person: PersonnelItem) => void
  onDelete: (person: PersonnelItem) => void
  onAddFace?: (person: PersonnelItem) => void
}

export const PersonnelCard: React.FC<PersonnelCardProps> = ({
  person,
  onView,
  onEdit,
  onDelete,
  onAddFace,
}) => {
  const { t } = useTranslation(['personnel', 'common'])
  const avatarUrl = person.primaryPhotoPath ? evidenceApi.getImageUrl(person.primaryPhotoPath) : ''

  const handleAddFaceClick = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (onAddFace) {
      onAddFace(person)
    } else {
      onView(person)
    }
  }

  return (
    <div
      onClick={() => onView(person)}
      className="group relative flex cursor-pointer flex-col justify-between overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4 shadow-sm transition-all duration-300 hover:border-emerald-500/50 hover:shadow-md"
    >
      {/* 顶部主头像与基础资料 */}
      <div className="flex items-start gap-3.5">
        <div className="relative h-16 w-16 shrink-0 overflow-hidden rounded-xl border border-[var(--border)] bg-black/40">
          {avatarUrl ? (
            <img
              src={avatarUrl}
              alt={person.name}
              className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center text-[var(--text-muted)]">
              <User className="h-8 w-8 opacity-40" />
            </div>
          )}
          <div className="absolute inset-x-0 bottom-0 bg-black/60 py-0.5 text-center text-[9px] font-medium text-emerald-400 backdrop-blur-xs">
            {t('card.primary')}
          </div>
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-1">
            <h3 className="truncate text-base font-semibold text-[var(--text-primary)]">
              {person.name}
            </h3>
            <button
              type="button"
              onClick={handleAddFaceClick}
              className="inline-flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-400 transition-colors hover:bg-emerald-500/20"
              title={person.faceCount < 5 ? t('actions.addFaces') : t('actions.viewDetails')}
            >
              <ImageIcon className="h-2.5 w-2.5" />
              <span>{person.faceCount}/5</span>
              {person.faceCount < 5 && <span className="text-[10px] font-bold">+</span>}
            </button>
          </div>

          <p className="mt-1 font-mono text-xs text-[var(--text-secondary)]">
            ID: {person.subjectId}
          </p>

          {person.idCard && (
            <p className="mt-0.5 truncate text-xs text-[var(--text-muted)]">
              {t('card.idCard')}: {person.idCard}
            </p>
          )}

          {person.remark && (
            <p className="mt-0.5 truncate text-xs text-[var(--text-muted)]">{person.remark}</p>
          )}
        </div>
      </div>

      {/* 底部元数据与操作按钮 */}
      <div className="mt-4 flex items-center justify-between border-t border-[var(--border)] pt-3 text-[11px] text-[var(--text-muted)]">
        <span>{formatTimestamp(person.createdAt)}</span>

        <div className="flex items-center gap-1">
          {person.faceCount < 5 && (
            <button
              type="button"
              onClick={handleAddFaceClick}
              className="flex h-7 items-center gap-1 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-2 text-[11px] font-medium text-emerald-400 transition-colors hover:bg-emerald-500/20"
              title={t('actions.addFaces')}
            >
              <ImagePlus className="h-3.5 w-3.5" />
              <span>{t('actions.addFacesShort')}</span>
            </button>
          )}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onView(person)
            }}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-tertiary)] text-[var(--text-secondary)] transition-colors hover:border-emerald-500/40 hover:text-emerald-400"
            title={t('actions.viewDetails')}
          >
            <Eye className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onEdit(person)
            }}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-tertiary)] text-[var(--text-secondary)] transition-colors hover:border-amber-500/40 hover:text-amber-400"
            title={t('actions.editInfo')}
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onDelete(person)
            }}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-tertiary)] text-[var(--text-secondary)] transition-colors hover:border-red-500/40 hover:text-red-400"
            title={t('actions.delete')}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </div>
  )
}
