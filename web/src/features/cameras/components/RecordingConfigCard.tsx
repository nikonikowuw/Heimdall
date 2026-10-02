import { Clapperboard, Loader2 } from 'lucide-react'
import React, { useCallback, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { NumericField } from '@/components/ui/NumericField'
import { cameraApi } from '@/lib/api'
import type { Camera, CameraRecordingConfig } from '@/types'
import {
  DEFAULT_RECORDING_CONFIG,
  RECORDING_LIMITS,
  clampRecordingConfig,
  isRecordingConfigDirty,
} from '../recordingConfig'

export interface RecordingConfigCardProps {
  camera: Camera
  /** 保存成功后的完整摄像头对象，由父层同步列表与详情快照 */
  onSaved: (camera: Camera) => void
}

interface NumberFieldCellProps {
  label: string
  unit: string
  children: React.ReactNode
}

/** 卡片内的数值字段单元格：只负责标签/单位排版，输入行为由 NumericField 承担 */
function NumberFieldCell({ label, unit, children }: NumberFieldCellProps): React.ReactElement {
  return (
    <label className="flex flex-col gap-1 rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/30 p-2.5">
      <span className="text-[11px] text-[var(--text-muted)]">{label}</span>
      <span className="flex items-baseline gap-1">
        {children}
        <span className="text-[11px] whitespace-nowrap text-[var(--text-muted)]">{unit}</span>
      </span>
    </label>
  )
}

/**
 * 通道级事件录像配置卡片。
 *
 * 开关即时保存（用户对 toggle 的预期是立即生效，失败回滚）；秒数等参数走草稿，
 * 由显式保存按钮提交，避免每次键入都产生请求。
 */
export function RecordingConfigCard({
  camera,
  onSaved,
}: RecordingConfigCardProps): React.ReactElement {
  const { t } = useTranslation('recording')
  const serverConfig = camera.recordingConfig

  const [draft, setDraft] = useState<CameraRecordingConfig>(
    serverConfig ?? DEFAULT_RECORDING_CONFIG,
  )
  const [saving, setSaving] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)

  // 仅在服务端值真正变化（保存成功或切换通道）时重置草稿；
  // 列表轮询换新对象引用不会清掉用户正在编辑的内容。
  const serverKey = serverConfig ? JSON.stringify(serverConfig) : ''
  const [synced, setSynced] = useState({ cameraId: camera.cameraId, key: serverKey })
  if (synced.cameraId !== camera.cameraId || synced.key !== serverKey) {
    setSynced({ cameraId: camera.cameraId, key: serverKey })
    setDraft(serverConfig ?? DEFAULT_RECORDING_CONFIG)
    setFailure(null)
  }

  const persist = useCallback(
    (next: CameraRecordingConfig, rollback?: CameraRecordingConfig) => {
      setSaving(true)
      setFailure(null)
      cameraApi
        .update(camera.cameraId, { recordingConfig: next })
        .then((updated) => {
          onSaved(updated)
          setSaving(false)
        })
        .catch((error: unknown) => {
          setFailure(error instanceof Error ? error.message : String(error))
          if (rollback) setDraft(rollback)
          setSaving(false)
        })
    },
    [camera.cameraId, onSaved],
  )

  const handleToggle = () => {
    if (saving) return
    const previous = draft
    const next = { ...draft, enabled: !draft.enabled }
    setDraft(next)
    persist(next, previous)
  }

  const dirty = isRecordingConfigDirty(draft, serverConfig)
  const unitSecond = t('config.unitSecond', { defaultValue: '秒' })
  const unitDay = t('config.unitDay', { defaultValue: '天' })

  return (
    <div className="overflow-hidden rounded-2xl border border-[var(--border)] bg-white p-4.5 dark:bg-[var(--bg-surface-solid)]">
      <div className="flex items-center justify-between gap-2 border-b border-[var(--border)]/60 pb-3">
        <div className="flex items-center gap-2">
          <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
            <Clapperboard className="h-4 w-4" />
          </div>
          <div>
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {t('config.title', { defaultValue: '事件录像' })}
            </h3>
            <p className="text-[11px] text-[var(--text-muted)]">
              {t('config.subtitle', { defaultValue: '告警或识别触发时保存事件前后片段' })}
            </p>
          </div>
        </div>

        <button
          type="button"
          role="switch"
          aria-checked={draft.enabled}
          aria-label={t('config.enable', { defaultValue: '启用事件录像' })}
          disabled={saving}
          onClick={handleToggle}
          className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors disabled:opacity-60 ${
            draft.enabled
              ? 'border-[var(--accent)] bg-[var(--accent)]'
              : 'border-[var(--border)] bg-[var(--bg-secondary)]'
          }`}
        >
          <span
            className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow-xs transition-transform ${
              draft.enabled ? 'translate-x-4' : 'translate-x-1'
            }`}
          />
        </button>
      </div>

      {draft.enabled ? (
        <div className="mt-3.5 space-y-3">
          <div className="grid grid-cols-2 gap-2.5">
            <NumberFieldCell
              label={t('config.preCapture', { defaultValue: '事件前录制' })}
              unit={unitSecond}
            >
              <NumericField
                label={t('config.preCapture', { defaultValue: '事件前录制' })}
                type="integer"
                min={RECORDING_LIMITS.preCaptureSeconds.min}
                max={RECORDING_LIMITS.preCaptureSeconds.max}
                value={draft.preCaptureSeconds}
                disabled={saving}
                onChange={(value) => setDraft({ ...draft, preCaptureSeconds: value })}
                className="font-data w-full bg-transparent text-sm font-semibold text-[var(--text-primary)] tabular-nums outline-none disabled:opacity-60"
              />
            </NumberFieldCell>
            <NumberFieldCell
              label={t('config.postCapture', { defaultValue: '事件后录制' })}
              unit={unitSecond}
            >
              <NumericField
                label={t('config.postCapture', { defaultValue: '事件后录制' })}
                type="integer"
                min={RECORDING_LIMITS.postCaptureSeconds.min}
                max={RECORDING_LIMITS.postCaptureSeconds.max}
                value={draft.postCaptureSeconds}
                disabled={saving}
                onChange={(value) => setDraft({ ...draft, postCaptureSeconds: value })}
                className="font-data w-full bg-transparent text-sm font-semibold text-[var(--text-primary)] tabular-nums outline-none disabled:opacity-60"
              />
            </NumberFieldCell>
            <NumberFieldCell
              label={t('config.maxFile', { defaultValue: '单文件上限' })}
              unit={unitSecond}
            >
              <NumericField
                label={t('config.maxFile', { defaultValue: '单文件上限' })}
                type="integer"
                min={RECORDING_LIMITS.maxFileSeconds.min}
                max={RECORDING_LIMITS.maxFileSeconds.max}
                value={draft.maxFileSeconds}
                disabled={saving}
                onChange={(value) => setDraft({ ...draft, maxFileSeconds: value })}
                className="font-data w-full bg-transparent text-sm font-semibold text-[var(--text-primary)] tabular-nums outline-none disabled:opacity-60"
              />
            </NumberFieldCell>
            <NumberFieldCell
              label={t('config.retention', { defaultValue: '保留天数' })}
              unit={unitDay}
            >
              <NumericField
                label={t('config.retention', { defaultValue: '保留天数' })}
                type="integer"
                min={RECORDING_LIMITS.retentionDays.min}
                max={RECORDING_LIMITS.retentionDays.max}
                value={draft.retentionDays}
                disabled={saving}
                onChange={(value) => setDraft({ ...draft, retentionDays: value })}
                className="font-data w-full bg-transparent text-sm font-semibold text-[var(--text-primary)] tabular-nums outline-none disabled:opacity-60"
              />
            </NumberFieldCell>
          </div>

          <div className="flex items-center justify-between gap-2">
            {failure ? (
              <p className="text-status-danger truncate text-[11px]" title={failure} role="alert">
                {t('config.saveFailed', { defaultValue: '保存失败，请重试' })}
              </p>
            ) : (
              <span className="text-[11px] text-[var(--text-muted)]">
                {t('config.dirtyHint', { defaultValue: '修改参数后需保存生效' })}
              </span>
            )}

            <button
              type="button"
              disabled={saving || !dirty}
              onClick={() => {
                if (saving || !dirty) return
                persist(clampRecordingConfig(draft))
              }}
              className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--accent)]/30 bg-[var(--accent)] px-3 py-1.5 text-xs font-semibold text-white transition-colors hover:bg-[var(--accent-strong)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              {saving && <Loader2 className="h-3 w-3 animate-spin" />}
              <span>{t('config.save', { defaultValue: '保存参数' })}</span>
            </button>
          </div>
        </div>
      ) : (
        <p className="mt-3 text-xs leading-relaxed text-[var(--text-muted)]">
          {t('config.disabledHint', {
            defaultValue: '开启后，告警与识别事件将自动保存事件前后片段，可在告警详情中回放。',
          })}
        </p>
      )}
    </div>
  )
}
