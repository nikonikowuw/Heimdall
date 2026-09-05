import React, { useCallback, useEffect, useMemo, useState } from 'react'
import {
  AlertCircle,
  Camera as CameraIcon,
  Check,
  CheckCircle2,
  Clock,
  ExternalLink,
  Filter,
  LayoutGrid,
  List,
  RefreshCw,
  ShieldAlert,
  UserCheck,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { alarmApi, cameraApi, evidenceApi } from '../../lib/api'
import type { AlarmRecord, Camera, CaptureRecord, RecognitionRecord } from '../../types'

type EvidenceTab = 'alarms' | 'captures' | 'recognition'
type ViewMode = 'cards' | 'table'

function formatTimestamp(val?: number | string | null): string {
  if (!val) return '-'
  if (typeof val === 'number') {
    return new Date(val).toLocaleString()
  }
  const parsed = Date.parse(val)
  if (!Number.isNaN(parsed)) {
    return new Date(parsed).toLocaleString()
  }
  return String(val)
}

function getRuleTypeLabel(ruleType: string | undefined, t: (key: string) => string): string {
  return ruleType === 'line' ? t('types.lineCrossing') : t('types.regionIntrusion')
}

interface AlarmStatusButtonProps {
  isProcessed: boolean
  onClick: (e: React.MouseEvent) => void
  t: (key: string) => string
  className?: string
}

function AlarmStatusButton({
  isProcessed,
  onClick,
  t,
  className = '',
}: AlarmStatusButtonProps): React.ReactElement {
  if (isProcessed) {
    return (
      <button
        onClick={onClick}
        className={`flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-3 py-1 text-xs font-semibold text-emerald-500 transition-all ${className}`}
      >
        <CheckCircle2 className="h-3.5 w-3.5" />
        <span>{t('card.processed')}</span>
      </button>
    )
  }

  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/15 px-3 py-1 text-xs font-semibold text-rose-500 transition-all hover:bg-rose-500 hover:text-white ${className}`}
    >
      <Check className="h-3.5 w-3.5" />
      <span>{t('card.markProcessed')}</span>
    </button>
  )
}

interface AlarmCardItemProps {
  alarm: AlarmRecord
  cameraName?: string
  onSelect: () => void
  onToggleStatus: () => void
  t: (key: string) => string
}

function AlarmCardItem({
  alarm,
  cameraName,
  onSelect,
  onToggleStatus,
  t,
}: AlarmCardItemProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'

  return (
    <div
      onClick={onSelect}
      className={`group flex cursor-pointer flex-col overflow-hidden rounded-2xl border bg-[var(--bg-surface)] shadow-xs transition-all duration-200 hover:shadow-md ${
        isProcessed
          ? 'border-[var(--border)] opacity-75'
          : 'border-rose-500/30 hover:border-rose-500/70'
      }`}
    >
      <div className="relative aspect-video w-full overflow-hidden bg-black/90">
        {alarm.imageRelPath ? (
          <img
            src={evidenceApi.getImageUrl(alarm.imageRelPath)}
            alt={alarm.eventId}
            className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center font-mono text-xs text-slate-500">
            {t('card.noImage')}
          </div>
        )}

        {alarm.cropImageRelPath && (
          <div className="absolute right-2 bottom-2 h-14 w-14 overflow-hidden rounded-lg border border-white/40 bg-black/80 p-0.5 shadow-md backdrop-blur-xs">
            <img
              src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
              alt="Crop"
              className="h-full w-full rounded object-cover"
            />
          </div>
        )}

        <div className="absolute top-2 left-2 flex items-center gap-1.5">
          <span
            className={`rounded-md px-2 py-0.5 text-[10px] font-bold tracking-wider uppercase shadow-xs backdrop-blur-md ${
              alarm.severity === 'critical'
                ? 'bg-rose-500/80 text-white'
                : 'bg-amber-500/80 text-white'
            }`}
          >
            {alarm.severity || 'WARNING'}
          </span>
          <span className="rounded-md bg-black/60 px-1.5 py-0.5 font-mono text-[10px] text-white backdrop-blur-xs">
            {getRuleTypeLabel(alarm.ruleType, t)}
          </span>
        </div>
      </div>

      <div className="space-y-2 p-3.5 text-xs">
        <div className="flex items-center justify-between">
          <span className="font-semibold text-[var(--text-primary)]">
            {t('card.target')}: {alarm.targetLabel}
          </span>
          <span className="font-mono text-[11px] font-semibold text-[var(--accent)]">
            {((alarm.confidence ?? 0) * 100).toFixed(0)}% {t('card.confidence')}
          </span>
        </div>

        <div className="flex items-center justify-between font-mono text-[11px] text-[var(--text-muted)]">
          <span className="font-sans font-medium text-[var(--text-secondary)]">
            {cameraName || alarm.cameraId}
          </span>
          <span className="flex items-center gap-1">
            <Clock className="h-3 w-3" />
            {formatTimestamp(alarm.occurredAt)}
          </span>
        </div>

        <div className="flex items-center justify-between border-t border-[var(--border)] pt-2">
          <AlarmStatusButton
            isProcessed={isProcessed}
            onClick={(e) => {
              e.stopPropagation()
              onToggleStatus()
            }}
            t={t}
          />

          <button
            onClick={(e) => {
              e.stopPropagation()
              onSelect()
            }}
            className="flex items-center gap-1 text-[11px] text-[var(--text-muted)] transition-all hover:text-[var(--text-primary)]"
          >
            <span>{t('card.viewHd')}</span>
            <ExternalLink className="h-3 w-3" />
          </button>
        </div>
      </div>
    </div>
  )
}

interface AlarmTableRowProps {
  alarm: AlarmRecord
  cameraName?: string
  onSelect: () => void
  onToggleStatus: () => void
  t: (key: string) => string
}

function AlarmTableRow({
  alarm,
  cameraName,
  onSelect,
  onToggleStatus,
  t,
}: AlarmTableRowProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'

  return (
    <tr
      onClick={onSelect}
      className="cursor-pointer transition-colors hover:bg-[var(--accent-soft)]/20"
    >
      <td className="px-3 py-2">
        <div className="h-10 w-16 shrink-0 overflow-hidden rounded border border-[var(--border)] bg-black/80">
          {alarm.cropImageRelPath || alarm.imageRelPath ? (
            <img
              src={evidenceApi.getImageUrl(alarm.cropImageRelPath || alarm.imageRelPath)}
              alt="Thumb"
              className="h-full w-full object-cover"
            />
          ) : (
            <div className="flex h-full items-center justify-center font-mono text-[9px] text-slate-500">
              N/A
            </div>
          )}
        </div>
      </td>
      <td className="px-3 py-2 font-mono text-[11px] text-[var(--text-primary)]">
        {alarm.eventId.slice(0, 12)}...
      </td>
      <td className="px-3 py-2 font-medium text-[var(--text-primary)]">
        {cameraName || alarm.cameraId}
      </td>
      <td className="px-3 py-2 font-semibold text-[var(--text-primary)]">{alarm.targetLabel}</td>
      <td className="px-3 py-2 font-mono text-[11px]">{getRuleTypeLabel(alarm.ruleType, t)}</td>
      <td className="px-3 py-2">
        <span
          className={`rounded px-1.5 py-0.5 text-[10px] font-bold uppercase ${
            alarm.severity === 'critical'
              ? 'bg-rose-500/20 text-rose-500'
              : 'bg-amber-500/20 text-amber-500'
          }`}
        >
          {alarm.severity || 'WARNING'}
        </span>
      </td>
      <td className="px-3 py-2 font-mono text-[11px]">
        {((alarm.confidence ?? 0) * 100).toFixed(0)}%
      </td>
      <td className="px-3 py-2">
        <span
          className={`rounded px-2 py-0.5 text-[10px] font-semibold ${
            isProcessed
              ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500'
              : 'border border-rose-500/30 bg-rose-500/15 text-rose-500'
          }`}
        >
          {isProcessed ? t('card.processed') : t('statusFilter.unprocessed')}
        </span>
      </td>
      <td className="px-3 py-2 font-mono text-[11px] text-[var(--text-muted)]">
        {formatTimestamp(alarm.occurredAt)}
      </td>
      <td className="px-3 py-2 text-right">
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleStatus()
          }}
          className={`rounded px-2.5 py-1 text-[11px] font-semibold transition-all ${
            isProcessed
              ? 'bg-emerald-500/15 text-emerald-500'
              : 'bg-rose-500 text-white hover:opacity-90'
          }`}
        >
          {isProcessed ? t('card.processed') : t('card.markProcessed')}
        </button>
      </td>
    </tr>
  )
}

interface AlarmsContentProps {
  alarms: AlarmRecord[]
  viewMode: ViewMode
  cameraNameMap?: Record<string, string>
  onSelect: (alarm: AlarmRecord) => void
  onToggleStatus: (alarm: AlarmRecord) => void
  t: (key: string) => string
}

function AlarmsContent({
  alarms,
  viewMode,
  cameraNameMap,
  onSelect,
  onToggleStatus,
  t,
}: AlarmsContentProps): React.ReactElement {
  if (alarms.length === 0) {
    return (
      <div className="py-24 text-center text-[var(--text-muted)]">
        <AlertCircle className="mx-auto mb-2 h-8 w-8 opacity-40" />
        <p className="font-medium text-[var(--text-secondary)]">{t('empty.alarms')}</p>
        <p className="text-xs opacity-75">{t('empty.alarmsDesc')}</p>
      </div>
    )
  }

  if (viewMode === 'cards') {
    return (
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
        {alarms.map((alarm) => (
          <AlarmCardItem
            key={alarm.id}
            alarm={alarm}
            cameraName={cameraNameMap?.[alarm.cameraId]}
            onSelect={() => onSelect(alarm)}
            onToggleStatus={() => onToggleStatus(alarm)}
            t={t}
          />
        ))}
      </div>
    )
  }

  return (
    <div className="overflow-x-auto rounded-xl border border-[var(--border)]">
      <table className="w-full text-left text-xs text-[var(--text-secondary)]">
        <thead className="border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[11px] font-semibold text-[var(--text-muted)] uppercase">
          <tr>
            <th className="px-3 py-2.5">{t('columns.thumbnail')}</th>
            <th className="px-3 py-2.5">{t('columns.eventId')}</th>
            <th className="px-3 py-2.5">{t('columns.camera')}</th>
            <th className="px-3 py-2.5">{t('columns.targetLabel')}</th>
            <th className="px-3 py-2.5">{t('columns.ruleType')}</th>
            <th className="px-3 py-2.5">{t('columns.severity')}</th>
            <th className="px-3 py-2.5">{t('columns.confidence')}</th>
            <th className="px-3 py-2.5">{t('columns.status')}</th>
            <th className="px-3 py-2.5">{t('columns.occurredAt')}</th>
            <th className="px-3 py-2.5 text-right">{t('columns.actions')}</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-[var(--border)]">
          {alarms.map((alarm) => (
            <AlarmTableRow
              key={alarm.id}
              alarm={alarm}
              cameraName={cameraNameMap?.[alarm.cameraId]}
              onSelect={() => onSelect(alarm)}
              onToggleStatus={() => onToggleStatus(alarm)}
              t={t}
            />
          ))}
        </tbody>
      </table>
    </div>
  )
}

interface CaptureCardItemProps {
  capture: CaptureRecord
  onSelect: () => void
  t: (key: string) => string
}

function CaptureCardItem({ capture, onSelect, t }: CaptureCardItemProps): React.ReactElement {
  return (
    <div
      onClick={onSelect}
      className="group relative flex cursor-pointer flex-col overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] transition-all duration-200 hover:border-cyan-500/50 hover:shadow-md"
    >
      <div className="relative aspect-square w-full overflow-hidden bg-black/90">
        {capture.cropImageRelPath || capture.imageRelPath ? (
          <img
            src={evidenceApi.getImageUrl(capture.cropImageRelPath || capture.imageRelPath)}
            alt={capture.captureId}
            className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center font-mono text-xs text-slate-500">
            {t('card.noImage')}
          </div>
        )}
        <span className="absolute top-1.5 left-1.5 rounded-md bg-black/70 px-1.5 py-0.5 font-mono text-[9px] text-cyan-400 backdrop-blur-xs">
          Track #{capture.trackId}
        </span>
        <span className="absolute right-1.5 bottom-1.5 rounded-md bg-emerald-600/90 px-1.5 py-0.5 font-mono text-[9px] font-bold text-white shadow-xs">
          {(capture.qualityScore ?? 0).toFixed(0)}
        </span>
      </div>
      <div className="space-y-1 p-2 text-[10px]">
        <div className="flex justify-between font-medium text-[var(--text-primary)]">
          <span>{capture.targetLabel}</span>
          <span className="font-mono text-[var(--text-muted)]">{capture.cameraId}</span>
        </div>
        <div className="truncate font-mono text-[9px] text-[var(--text-muted)]">
          {formatTimestamp(capture.capturedAt)}
        </div>
      </div>
    </div>
  )
}

interface CapturesContentProps {
  captures: CaptureRecord[]
  onSelect: (capture: CaptureRecord) => void
  t: (key: string) => string
}

function CapturesContent({ captures, onSelect, t }: CapturesContentProps): React.ReactElement {
  if (captures.length === 0) {
    return (
      <div className="py-24 text-center text-[var(--text-muted)]">
        <CameraIcon className="mx-auto mb-2 h-8 w-8 opacity-40" />
        <p className="font-medium text-[var(--text-secondary)]">{t('empty.captures')}</p>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-2 gap-3.5 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
      {captures.map((cap) => (
        <CaptureCardItem key={cap.id} capture={cap} onSelect={() => onSelect(cap)} t={t} />
      ))}
    </div>
  )
}

interface RecognitionCardItemProps {
  recognition: RecognitionRecord
  t: (key: string) => string
}

function RecognitionCardItem({ recognition, t }: RecognitionCardItemProps): React.ReactElement {
  return (
    <div className="flex flex-col space-y-3.5 rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 shadow-sm transition-all hover:border-emerald-500/40 hover:shadow-md">
      <div className="flex items-center justify-between gap-3">
        {/* 现场抓拍特写 */}
        <div className="flex flex-1 flex-col items-center gap-1.5">
          <div className="aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black/90 shadow-xs">
            {recognition.fieldCropPath ? (
              <img
                src={evidenceApi.getImageUrl(recognition.fieldCropPath)}
                alt="Site Crop"
                className="h-full w-full object-cover"
              />
            ) : (
              <div className="flex h-full items-center justify-center text-xs text-slate-500">
                {t('card.siteCrop')}
              </div>
            )}
          </div>
          <span className="text-[10px] font-medium text-[var(--text-secondary)]">
            {t('card.siteCrop')}
          </span>
        </div>

        {/* 相似度分值徽标 */}
        <div className="flex flex-col items-center gap-1 px-1">
          <span className="font-mono text-[9px] font-bold text-emerald-500 uppercase">
            {t('card.match')}
          </span>
          <div className="flex h-11 w-11 items-center justify-center rounded-full border border-emerald-500/40 bg-emerald-500/15 font-mono text-xs font-bold text-emerald-500 shadow-xs">
            {((recognition.similarity ?? 0) * 100).toFixed(0)}%
          </div>
          <span className="text-[9px] text-[var(--text-muted)]">{t('card.similarity')}</span>
        </div>

        {/* 底库登记照 */}
        <div className="flex flex-1 flex-col items-center gap-1.5">
          <div className="aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black/90 shadow-xs">
            {recognition.registeredPhotoPath ? (
              <img
                src={evidenceApi.getImageUrl(recognition.registeredPhotoPath)}
                alt="Registered"
                className="h-full w-full object-cover"
              />
            ) : (
              <div className="flex h-full items-center justify-center text-xs text-slate-500">
                {t('card.registeredPhoto')}
              </div>
            )}
          </div>
          <span className="text-[10px] font-medium text-[var(--text-secondary)]">
            {t('card.registeredPhoto')}
          </span>
        </div>
      </div>

      <div className="flex items-center justify-between border-t border-[var(--border)] pt-2 text-xs">
        <div>
          <span className="font-semibold text-[var(--text-primary)]">
            {recognition.subjectName}
          </span>
          <span className="ml-1.5 font-mono text-[10px] text-[var(--text-muted)]">
            ID: {recognition.subjectId}
          </span>
        </div>
        <span className="font-mono text-[10px] text-[var(--text-muted)]">
          {recognition.cameraId} · {formatTimestamp(recognition.recognizedAt)}
        </span>
      </div>
    </div>
  )
}

interface RecognitionContentProps {
  recognitions: RecognitionRecord[]
  t: (key: string) => string
}

function RecognitionContent({ recognitions, t }: RecognitionContentProps): React.ReactElement {
  if (recognitions.length === 0) {
    return (
      <div className="py-24 text-center text-[var(--text-muted)]">
        <UserCheck className="mx-auto mb-2 h-8 w-8 opacity-40" />
        <p className="font-medium text-[var(--text-secondary)]">{t('empty.recognitions')}</p>
        <p className="text-xs opacity-75">{t('empty.recognitionsDesc')}</p>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
      {recognitions.map((rec) => (
        <RecognitionCardItem key={rec.id} recognition={rec} t={t} />
      ))}
    </div>
  )
}

interface AlarmLightboxModalProps {
  alarm: AlarmRecord
  onClose: () => void
  onToggleStatus: () => void
  t: (key: string) => string
}

function AlarmLightboxModal({
  alarm,
  onClose,
  onToggleStatus,
  t,
}: AlarmLightboxModalProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[92vh] w-full max-w-5xl flex-col overflow-hidden rounded-3xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-[var(--border)] px-6 py-4">
          <div className="flex items-center gap-2.5">
            <ShieldAlert className="h-5 w-5 text-rose-500" />
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {alarm.targetLabel} · {getRuleTypeLabel(alarm.ruleType, t)}
            </h3>
            <span className="font-mono text-xs text-[var(--text-muted)]">{alarm.eventId}</span>
          </div>
          <button
            onClick={onClose}
            className="rounded-xl p-1.5 text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="flex-1 space-y-4 overflow-auto p-6">
          <div className="relative aspect-video w-full overflow-hidden rounded-2xl border border-[var(--border)] bg-black shadow-md">
            {alarm.imageRelPath ? (
              <img
                src={evidenceApi.getImageUrl(alarm.imageRelPath)}
                alt="Full Frame"
                className="h-full w-full object-contain"
              />
            ) : (
              <div className="flex h-full w-full items-center justify-center text-slate-500">
                {t('modal.noImage')}
              </div>
            )}
          </div>

          <div className="flex flex-wrap items-center justify-between gap-4 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4">
            <div className="flex items-center gap-4">
              {alarm.cropImageRelPath && (
                <div className="h-16 w-16 overflow-hidden rounded-xl border border-white/30 bg-black/80 shadow-xs">
                  <img
                    src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
                    alt="Crop"
                    className="h-full w-full object-cover"
                  />
                </div>
              )}
              <div className="space-y-1 text-xs">
                <div className="font-semibold text-[var(--text-primary)]">
                  {t('modal.cropImage')}
                </div>
                <div className="font-mono text-[11px] text-[var(--text-muted)]">
                  {t('modal.channel')}: {alarm.cameraId} · {t('modal.trackId')}: #{alarm.trackId}
                </div>
                <div className="font-mono text-[11px] text-[var(--text-muted)]">
                  {t('modal.time')}: {formatTimestamp(alarm.occurredAt)}
                </div>
              </div>
            </div>

            <div className="flex items-center gap-3">
              <button
                onClick={onToggleStatus}
                className={`flex items-center gap-1.5 rounded-xl px-4 py-2 text-xs font-semibold transition-all ${
                  isProcessed
                    ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500'
                    : 'bg-rose-500 text-white shadow-xs hover:opacity-90'
                }`}
              >
                {isProcessed ? (
                  <>
                    <CheckCircle2 className="h-4 w-4" />
                    <span>{t('card.processed')}</span>
                  </>
                ) : (
                  <>
                    <Check className="h-4 w-4" />
                    <span>{t('card.markProcessed')}</span>
                  </>
                )}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

interface CaptureLightboxModalProps {
  capture: CaptureRecord
  onClose: () => void
  t: (key: string) => string
}

function CaptureLightboxModal({
  capture,
  onClose,
  t,
}: CaptureLightboxModalProps): React.ReactElement {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[92vh] w-full max-w-5xl flex-col overflow-hidden rounded-3xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-[var(--border)] px-6 py-4">
          <div className="flex items-center gap-2">
            <CameraIcon className="h-5 w-5 text-cyan-400" />
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {capture.targetLabel} (Track #{capture.trackId})
            </h3>
          </div>
          <button
            onClick={onClose}
            className="rounded-xl p-1.5 text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>
        <div className="flex-1 space-y-4 overflow-auto p-6">
          <div className="relative aspect-video w-full overflow-hidden rounded-2xl border border-[var(--border)] bg-black shadow-md">
            {capture.imageRelPath ? (
              <img
                src={evidenceApi.getImageUrl(capture.imageRelPath)}
                alt="Capture"
                className="h-full w-full object-contain"
              />
            ) : (
              <div className="flex h-full w-full items-center justify-center text-slate-500">
                {t('modal.noImage')}
              </div>
            )}
          </div>
          <div className="flex items-center justify-between rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4 font-mono text-xs">
            <span>{capture.cameraId}</span>
            <span>
              {t('modal.qualityScore')}: {(capture.qualityScore ?? 0).toFixed(0)}
            </span>
            <span>{formatTimestamp(capture.capturedAt)}</span>
          </div>
        </div>
      </div>
    </div>
  )
}

export function AlarmsPage(): React.ReactElement {
  const { t } = useTranslation('alarm')
  const [activeTab, setActiveTab] = useState<EvidenceTab>('alarms')
  const [viewMode, setViewMode] = useState<ViewMode>('cards')
  const [cameras, setCameras] = useState<Camera[]>([])
  const [selectedCameraId, setSelectedCameraId] = useState<string>('')
  const [selectedTargetLabel, setSelectedTargetLabel] = useState<string>('')
  const [selectedStatus, setSelectedStatus] = useState<string>('all')
  const [startTime, setStartTime] = useState<string>('')
  const [endTime, setEndTime] = useState<string>('')
  const [page, setPage] = useState<number>(1)
  const pageSize = 20

  const [isLoading, setIsLoading] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  const [alarms, setAlarms] = useState<AlarmRecord[]>([])
  const [captures, setCaptures] = useState<CaptureRecord[]>([])
  const [recognitions, setRecognitions] = useState<RecognitionRecord[]>([])

  const [lightboxAlarm, setLightboxAlarm] = useState<AlarmRecord | null>(null)
  const [lightboxCapture, setLightboxCapture] = useState<CaptureRecord | null>(null)

  useEffect(() => {
    let isMounted = true
    cameraApi
      .list()
      .then((list) => {
        if (isMounted) setCameras(list)
      })
      .catch(() => {})
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

  const loadData = useCallback(async () => {
    setIsLoading(true)
    setErrorMessage(null)
    try {
      const camId = selectedCameraId || undefined
      const targetLbl = selectedTargetLabel || undefined
      const startMs = startTime ? new Date(startTime).getTime() : undefined
      const endMs = endTime ? new Date(endTime).getTime() : undefined
      const statusParam = selectedStatus === 'all' ? undefined : selectedStatus
      const offset = (page - 1) * pageSize

      if (activeTab === 'alarms') {
        const list = await alarmApi.list({
          cameraId: camId,
          status: statusParam,
          startTime: startMs,
          endTime: endMs,
          limit: pageSize,
          offset,
        })
        const filtered = targetLbl
          ? list.filter((a) => a.targetLabel?.toLowerCase() === targetLbl.toLowerCase())
          : list
        setAlarms(filtered)
      } else if (activeTab === 'captures') {
        const list = await evidenceApi.listCaptures({
          cameraId: camId,
          targetLabel: targetLbl,
          startTime: startMs,
          endTime: endMs,
          limit: pageSize,
          offset,
        })
        setCaptures(list)
      } else if (activeTab === 'recognition') {
        const list = await evidenceApi.listRecognitions({
          cameraId: camId,
          limit: pageSize,
          offset,
        })
        setRecognitions(list)
      }
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [activeTab, selectedCameraId, selectedTargetLabel, selectedStatus, startTime, endTime, page])

  useEffect(() => {
    loadData()
  }, [loadData])

  const handleFilterChange = (setter: (v: string) => void, val: string): void => {
    setter(val)
    setPage(1)
  }

  const handleToggleAlarmStatus = async (alarm: AlarmRecord): Promise<void> => {
    const nextStatus = alarm.status === 'processed' ? 'unprocessed' : 'processed'
    try {
      const updated = await alarmApi.updateStatus(alarm.id, nextStatus)
      setAlarms((prev) => prev.map((a) => (a.id === alarm.id ? updated : a)))
      if (lightboxAlarm && lightboxAlarm.id === alarm.id) {
        setLightboxAlarm(updated)
      }
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err))
    }
  }

  let currentItemCount = alarms.length
  if (activeTab === 'captures') {
    currentItemCount = captures.length
  } else if (activeTab === 'recognition') {
    currentItemCount = recognitions.length
  }

  return (
    <div className="flex h-full flex-col gap-4 bg-[var(--bg-primary)] p-4 text-[var(--text-primary)] select-none">
      {/* 顶部控制栏与三重视图切换 */}
      <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl p-3 shadow-xs">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-rose-500/10 text-rose-500">
            <ShieldAlert className="h-5 w-5" />
          </div>
          <div>
            <h2 className="text-sm font-semibold text-[var(--text-primary)]">{t('title')}</h2>
            <p className="text-xs text-[var(--text-muted)]">
              {t(`tabs.${activeTab}`)} · 1080P/4K 全链路原子闭环
            </p>
          </div>
        </div>

        {/* 证据分类 Tab 切换器 */}
        <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-xs">
          <button
            onClick={() => {
              setActiveTab('alarms')
              setPage(1)
            }}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all ${
              activeTab === 'alarms'
                ? 'border border-rose-500/30 bg-rose-500/15 text-rose-500 shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
          >
            <AlertCircle className="h-3.5 w-3.5 text-rose-500" />
            <span>{t('tabs.alarms')}</span>
            {alarms.length > 0 && (
              <span className="py-0.2 ml-1 rounded-full bg-rose-500/10 px-1.5 font-mono text-[10px] font-bold text-rose-500">
                {alarms.length}
              </span>
            )}
          </button>

          <button
            onClick={() => {
              setActiveTab('captures')
              setPage(1)
            }}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all ${
              activeTab === 'captures'
                ? 'border border-cyan-500/30 bg-cyan-500/15 text-cyan-500 shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
          >
            <CameraIcon className="h-3.5 w-3.5 text-cyan-500" />
            <span>{t('tabs.captures')}</span>
            {captures.length > 0 && (
              <span className="py-0.2 ml-1 rounded-full bg-cyan-500/10 px-1.5 font-mono text-[10px] font-bold text-cyan-500">
                {captures.length}
              </span>
            )}
          </button>

          <button
            onClick={() => {
              setActiveTab('recognition')
              setPage(1)
            }}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all ${
              activeTab === 'recognition'
                ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500 shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
          >
            <UserCheck className="h-3.5 w-3.5 text-emerald-500" />
            <span>{t('tabs.recognition')}</span>
            {recognitions.length > 0 && (
              <span className="py-0.2 ml-1 rounded-full bg-emerald-500/10 px-1.5 font-mono text-[10px] font-bold text-emerald-500">
                {recognitions.length}
              </span>
            )}
          </button>
        </div>
      </div>

      {/* 筛选工具条与视图切换 */}
      <div className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl p-3 shadow-xs">
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
                {c.name} ({c.cameraId})
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

          {/* 时间范围筛选 */}
          <div className="flex items-center gap-1.5">
            <input
              type="datetime-local"
              value={startTime}
              onChange={(e) => handleFilterChange(setStartTime, e.target.value)}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 text-[11px] text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
              title={t('timeFilter.start')}
            />
            <span className="text-[var(--text-muted)]">-</span>
            <input
              type="datetime-local"
              value={endTime}
              onChange={(e) => handleFilterChange(setEndTime, e.target.value)}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1 text-[11px] text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
              title={t('timeFilter.end')}
            />
            {(startTime || endTime) && (
              <button
                onClick={() => {
                  setStartTime('')
                  setEndTime('')
                  setPage(1)
                }}
                className="rounded-lg p-1 text-[var(--text-muted)] transition-all hover:bg-rose-500/10 hover:text-rose-400"
                title={t('timeFilter.clear')}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* 卡片与表格视图切换 */}
          {activeTab === 'alarms' && (
            <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-0.5 text-xs">
              <button
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
            viewMode={viewMode}
            cameraNameMap={cameraNameMap}
            onSelect={setLightboxAlarm}
            onToggleStatus={handleToggleAlarmStatus}
            t={t}
          />
        )}

        {activeTab === 'captures' && (
          <CapturesContent captures={captures} onSelect={setLightboxCapture} t={t} />
        )}

        {activeTab === 'recognition' && <RecognitionContent recognitions={recognitions} t={t} />}
      </div>

      {/* 分页控制栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-2xl px-4 py-2.5 text-xs text-[var(--text-secondary)] shadow-xs">
        <span>{t('pagination.page', { current: page })}</span>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setPage((p) => Math.max(1, p - 1))}
            disabled={page === 1 || isLoading}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-40"
          >
            {t('pagination.prev')}
          </button>
          <span className="px-1 font-mono font-semibold text-[var(--text-primary)]">{page}</span>
          <button
            onClick={() => setPage((p) => p + 1)}
            disabled={currentItemCount < pageSize || isLoading}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-40"
          >
            {t('pagination.next')}
          </button>
        </div>
      </div>

      {/* 告警大图灯箱 Modal */}
      {lightboxAlarm && (
        <AlarmLightboxModal
          alarm={lightboxAlarm}
          onClose={() => setLightboxAlarm(null)}
          onToggleStatus={() => handleToggleAlarmStatus(lightboxAlarm)}
          t={t}
        />
      )}

      {/* 抓拍大图灯箱 Modal */}
      {lightboxCapture && (
        <CaptureLightboxModal
          capture={lightboxCapture}
          onClose={() => setLightboxCapture(null)}
          t={t}
        />
      )}
    </div>
  )
}
