import { useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronDown, ChevronRight, Trash2, Check } from 'lucide-react'
import { systemApi } from '../../lib/system-api'
import { RefreshButton } from '../../components/RefreshButton'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import { ConfirmDialog } from './components/ConfirmDialog'
import type { StorageConfig, StorageStatus } from '../../types/system'

export function StorageSettings(): React.ReactElement {
  const { t } = useTranslation('system')
  const [status, setStatus] = useState<StorageStatus | null>(null)
  const [config, setConfig] = useState<StorageConfig | null>(null)
  const [draft, setDraft] = useState<StorageConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [saveSuccess, setSaveSuccess] = useState(false)
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [showCleanupConfirm, setShowCleanupConfirm] = useState(false)
  const [cleaning, setCleaning] = useState(false)
  const [cleanupError, setCleanupError] = useState<string | null>(null)

  const loadData = useCallback(
    async (signal?: AbortSignal) => {
      try {
        setLoading(true)
        setError(null)
        const [s, c] = await Promise.all([
          systemApi.getStorageStatus(signal),
          systemApi.getStorageConfig(signal),
        ])
        if (!signal?.aborted) {
          setStatus(s)
          setConfig(c)
          setDraft(c)
        }
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return
        if (!signal?.aborted)
          setError(
            err instanceof Error
              ? err.message
              : t('loadFailed', { defaultValue: 'Failed to load' }),
          )
      } finally {
        if (!signal?.aborted) setLoading(false)
      }
    },
    [t],
  )

  useEffect(() => {
    const controller = new AbortController()
    loadData(controller.signal)
    return () => controller.abort()
  }, [loadData])

  const handleSave = async () => {
    if (!draft || !config) return
    try {
      setSaving(true)
      setSaveError(null)
      setSaveSuccess(false)
      const updated = await systemApi.updateStorageConfig(draft)
      setConfig(updated)
      setDraft(updated)
      setSaveSuccess(true)
      setTimeout(() => setSaveSuccess(false), 2000)
    } catch (err) {
      setSaveError(
        err instanceof Error ? err.message : t('saveFailed', { defaultValue: 'Failed to save' }),
      )
    } finally {
      setSaving(false)
    }
  }

  const handleCleanup = async () => {
    try {
      setCleaning(true)
      setCleanupError(null)
      setShowCleanupConfirm(false)
      await systemApi.triggerCleanup()
      await loadData()
    } catch (err) {
      setCleanupError(
        err instanceof Error
          ? err.message
          : t('storage.cleanupFailed', { defaultValue: 'Cleanup failed' }),
      )
    } finally {
      setCleaning(false)
    }
  }

  const isDirty = draft && config && JSON.stringify(draft) !== JSON.stringify(config)

  const healthConfig: Record<string, { label: string; color: string; bg: string }> = {
    normal: {
      label: t('storage.healthLevel.normal', { defaultValue: '正常' }),
      color: 'var(--accent-green)',
      bg: 'var(--accent-green)',
    },
    evicting: {
      label: t('storage.healthLevel.evicting', { defaultValue: '淘汰中' }),
      color: 'var(--accent-amber)',
      bg: 'var(--accent-amber)',
    },
    emergency: {
      label: t('storage.healthLevel.emergency', { defaultValue: '紧急' }),
      color: 'var(--destructive)',
      bg: 'var(--destructive)',
    },
    critical: {
      label: t('storage.healthLevel.critical', { defaultValue: '熔断' }),
      color: 'var(--destructive)',
      bg: 'var(--destructive)',
    },
  }

  return (
    <div className="space-y-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-[var(--text-primary)]">
            {t('storage.title', { defaultValue: '存储与保留策略' })}
          </h2>
          <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">
            {t('storage.subtitle', { defaultValue: '管理磁盘配额、保留策略与自动清理' })}
          </p>
        </div>
        <RefreshButton onClick={() => loadData()} loading={loading} />
      </div>

      {error && <ErrorBanner message={error} onRetry={() => loadData()} />}
      {cleanupError && <ErrorBanner message={cleanupError} onRetry={() => handleCleanup()} />}

      {/* Disk Status */}
      <SettingsSection title={t('storage.diskStatus', { defaultValue: '磁盘状态' })}>
        {loading && !status ? (
          <LoadingSkeleton rows={4} />
        ) : status ? (
          <div className="space-y-4">
            {/* Progress bar */}
            <div>
              <div className="flex items-end justify-between">
                <span className="text-sm font-medium text-[var(--text-primary)]">
                  {t('storage.diskUsage', { defaultValue: '磁盘使用率' })}
                </span>
                <span
                  className="text-sm font-bold tabular-nums"
                  style={{
                    color:
                      status.usagePercent >= 95
                        ? 'var(--destructive)'
                        : status.usagePercent >= 80
                          ? 'var(--accent-amber)'
                          : 'var(--accent-green)',
                  }}
                >
                  {status.usagePercent.toFixed(1)}%
                </span>
              </div>
              <div className="mt-2 h-3 overflow-hidden rounded-full bg-[var(--bg-secondary)]">
                <div
                  className="h-full rounded-full transition-all duration-500 ease-out"
                  style={{
                    width: `${Math.min(status.usagePercent, 100)}%`,
                    backgroundColor:
                      status.usagePercent >= 95
                        ? 'var(--destructive)'
                        : status.usagePercent >= 80
                          ? 'var(--accent-amber)'
                          : 'var(--accent-green)',
                  }}
                />
              </div>
            </div>

            {/* Stats grid */}
            <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
              <StatTile
                label={t('storage.total', { defaultValue: '总容量' })}
                value={`${status.totalGb.toFixed(1)} GB`}
              />
              <StatTile
                label={t('storage.used', { defaultValue: '已用' })}
                value={`${status.usedGb.toFixed(1)} GB`}
              />
              <StatTile
                label={t('storage.available', { defaultValue: '可用' })}
                value={`${status.availableGb.toFixed(1)} GB`}
              />
              <HealthTile
                label={t('storage.health', { defaultValue: '健康等级' })}
                config={healthConfig[status.healthLevel] || healthConfig.normal}
              />
            </div>

            {/* Evidence breakdown */}
            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 p-4">
              <p className="mb-3 text-[12px] font-medium tracking-wider text-[var(--text-muted)] uppercase">
                {t('storage.evidenceBreakdown', { defaultValue: '证据存储分布' })}
              </p>
              <div className="grid grid-cols-3 gap-3">
                <EvidenceStat
                  label={t('storage.alarms', { defaultValue: '告警图' })}
                  count={status.alarmCount}
                  size={status.alarmSizeMb}
                  color="var(--accent-amber)"
                />
                <EvidenceStat
                  label={t('storage.recognitions', { defaultValue: '识别图' })}
                  count={status.recognitionCount}
                  size={status.recognitionSizeMb}
                  color="var(--accent)"
                />
                <EvidenceStat
                  label={t('storage.captures', { defaultValue: '抓拍图' })}
                  count={status.captureCount}
                  size={status.captureSizeMb}
                  color="var(--accent-green)"
                />
              </div>
            </div>
          </div>
        ) : null}
      </SettingsSection>

      {/* Retention Policy */}
      <SettingsSection
        title={t('storage.retention', { defaultValue: '保留策略' })}
        action={
          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowCleanupConfirm(true)}
              disabled={cleaning}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-[13px] font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] active:scale-[0.97] disabled:opacity-50"
            >
              <Trash2 className="h-3.5 w-3.5" />
              {t('storage.cleanup', { defaultValue: '手动清理' })}
            </button>
            <button
              onClick={handleSave}
              disabled={saving || !isDirty}
              className={`flex items-center gap-1.5 rounded-xl px-4 py-2 text-[13px] font-medium text-white transition-all active:scale-[0.97] disabled:opacity-50 ${
                saveSuccess
                  ? 'bg-[var(--accent-green)] shadow-[var(--accent-green)]/20 shadow-lg'
                  : 'bg-[var(--accent)] shadow-[var(--accent)]/20 shadow-lg hover:bg-[var(--accent)]/90'
              }`}
            >
              {saving ? (
                <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-white border-t-transparent" />
              ) : saveSuccess ? (
                <Check className="h-3.5 w-3.5" />
              ) : null}
              {saveSuccess
                ? t('saved', { defaultValue: '已保存' })
                : t('save', { defaultValue: '保存' })}
            </button>
          </div>
        }
      >
        {loading && !draft ? (
          <LoadingSkeleton rows={5} />
        ) : draft ? (
          <div className="space-y-5">
            {/* Retention table */}
            <div className="overflow-hidden rounded-xl border border-[var(--border)]">
              <table className="w-full text-[13px]">
                <thead>
                  <tr className="border-b border-[var(--border)] bg-[var(--bg-secondary)]/60">
                    <th className="px-4 py-2.5 text-left font-medium text-[var(--text-muted)]">
                      {t('storage.type', { defaultValue: '类型' })}
                    </th>
                    <th className="w-24 px-3 py-2.5 text-center font-medium text-[var(--text-muted)]">
                      {t('storage.retentionDays', { defaultValue: '保留天数' })}
                    </th>
                    <th className="w-28 px-3 py-2.5 text-center font-medium text-[var(--text-muted)]">
                      {t('storage.quotaMb', { defaultValue: '配额(MB)' })}
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--border)]">
                  <RetentionRow
                    label={t('storage.alarmImages', { defaultValue: '告警图' })}
                    days={draft.alarmRetentionDays}
                    quota={draft.alarmQuotaMb}
                    onChange={(days, quota) =>
                      setDraft({ ...draft, alarmRetentionDays: days, alarmQuotaMb: quota })
                    }
                  />
                  <RetentionRow
                    label={t('storage.recognitionImages', { defaultValue: '识别图' })}
                    days={draft.recognitionRetentionDays}
                    quota={draft.recognitionQuotaMb}
                    onChange={(days, quota) =>
                      setDraft({
                        ...draft,
                        recognitionRetentionDays: days,
                        recognitionQuotaMb: quota,
                      })
                    }
                  />
                  <RetentionRow
                    label={t('storage.captureImages', { defaultValue: '抓拍图' })}
                    days={draft.captureRetentionDays}
                    quota={draft.captureQuotaMb}
                    onChange={(days, quota) =>
                      setDraft({ ...draft, captureRetentionDays: days, captureQuotaMb: quota })
                    }
                  />
                </tbody>
              </table>
            </div>

            {/* Overwrite mode */}
            <div>
              <label className="mb-2 block text-[13px] font-medium text-[var(--text-primary)]">
                {t('storage.overwriteMode', { defaultValue: '覆盖策略' })}
              </label>
              <div className="flex gap-3">
                <RadioOption
                  name="overwriteMode"
                  checked={draft.overwriteMode === 'overwrite'}
                  onChange={() => setDraft({ ...draft, overwriteMode: 'overwrite' })}
                  label={t('storage.overwrite', { defaultValue: '循环覆盖' })}
                  description={t('storage.overwriteDesc', {
                    defaultValue: '磁盘写满时自动淘汰旧数据',
                  })}
                />
                <RadioOption
                  name="overwriteMode"
                  checked={draft.overwriteMode === 'stop'}
                  onChange={() => setDraft({ ...draft, overwriteMode: 'stop' })}
                  label={t('storage.stopOnFull', { defaultValue: '写满停止' })}
                  description={t('storage.stopOnFullDesc', { defaultValue: '磁盘写满后停止写入' })}
                />
              </div>
            </div>

            {/* Auto cleanup toggle */}
            <div className="flex items-center justify-between rounded-xl bg-[var(--bg-secondary)]/60 px-4 py-3.5">
              <div>
                <p className="text-[13px] font-medium text-[var(--text-primary)]">
                  {t('storage.autoCleanup', { defaultValue: '自动清理' })}
                </p>
                <p className="mt-0.5 text-[12px] text-[var(--text-muted)]">
                  {t('storage.autoCleanupDesc', { defaultValue: '按水位自动触发清理任务' })}
                </p>
              </div>
              <button
                onClick={() =>
                  setDraft({ ...draft, autoCleanupEnabled: !draft.autoCleanupEnabled })
                }
                className={`switch-track ${draft.autoCleanupEnabled ? 'on' : ''}`}
                role="switch"
                aria-checked={draft.autoCleanupEnabled}
              >
                <div className="switch-thumb" />
              </button>
            </div>

            {/* Advanced settings */}
            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/30">
              <button
                onClick={() => setShowAdvanced(!showAdvanced)}
                className="flex w-full items-center justify-between px-4 py-3 text-[13px] font-medium text-[var(--text-secondary)] transition-colors hover:text-[var(--text-primary)]"
              >
                <span>{t('storage.advanced', { defaultValue: '高级设置' })}</span>
                {showAdvanced ? (
                  <ChevronDown className="h-4 w-4" />
                ) : (
                  <ChevronRight className="h-4 w-4" />
                )}
              </button>

              {showAdvanced && (
                <div className="space-y-3 border-t border-[var(--border)] px-4 pt-3 pb-4">
                  <div className="grid grid-cols-2 gap-3">
                    <NumberInput
                      label={t('storage.minFree', { defaultValue: '触发清理水位(%)' })}
                      value={Math.round(draft.minFreeRatio * 100)}
                      onChange={(v) => setDraft({ ...draft, minFreeRatio: v / 100 })}
                    />
                    <NumberInput
                      label={t('storage.targetFree', { defaultValue: '目标水位(%)' })}
                      value={Math.round(draft.targetFreeRatio * 100)}
                      onChange={(v) => setDraft({ ...draft, targetFreeRatio: v / 100 })}
                    />
                    <NumberInput
                      label={t('storage.emergencyFree', { defaultValue: '紧急水位(%)' })}
                      value={Math.round(draft.emergencyFreeRatio * 100)}
                      onChange={(v) => setDraft({ ...draft, emergencyFreeRatio: v / 100 })}
                    />
                    <NumberInput
                      label={t('storage.criticalFree', { defaultValue: '熔断水位(%)' })}
                      value={Math.round(draft.criticalFreeRatio * 100)}
                      onChange={(v) => setDraft({ ...draft, criticalFreeRatio: v / 100 })}
                    />
                    <NumberInput
                      label={t('storage.batchSize', { defaultValue: '单批删除数' })}
                      value={draft.batchDeleteSize}
                      onChange={(v) => setDraft({ ...draft, batchDeleteSize: v })}
                    />
                  </div>
                </div>
              )}
            </div>

            {saveError && (
              <div className="rounded-lg border border-[var(--destructive)]/20 bg-[var(--destructive)]/5 px-3 py-2 text-[13px] text-[var(--destructive)]">
                {saveError}
              </div>
            )}
          </div>
        ) : null}
      </SettingsSection>

      <ConfirmDialog
        open={showCleanupConfirm}
        title={t('storage.cleanupConfirmTitle', { defaultValue: '手动清理' })}
        message={t('storage.cleanupConfirmMessage', {
          defaultValue: '将按当前保留策略立即执行一次清理，删除过期的证据图片。此操作不可撤销。',
        })}
        confirmLabel={t('storage.cleanupNow', { defaultValue: '立即清理' })}
        variant="warning"
        onConfirm={handleCleanup}
        onCancel={() => setShowCleanupConfirm(false)}
      />
    </div>
  )
}

function RetentionRow({
  label,
  days,
  quota,
  onChange,
}: {
  label: string
  days: number
  quota: number
  onChange: (days: number, quota: number) => void
}): React.ReactElement {
  return (
    <tr className="bg-[var(--bg-surface)] transition-colors hover:bg-[var(--bg-secondary)]/50">
      <td className="px-4 py-3">
        <span className="font-medium text-[var(--text-primary)]">{label}</span>
      </td>
      <td className="px-3 py-3">
        <input
          type="number"
          min={1}
          max={365}
          value={days}
          onChange={(e) => onChange(Number(e.target.value) || 1, quota)}
          className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-center font-mono text-[13px] text-[var(--text-primary)] transition-colors focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none"
        />
      </td>
      <td className="px-3 py-3">
        <input
          type="number"
          min={0}
          value={quota}
          onChange={(e) => onChange(days, Number(e.target.value) || 0)}
          placeholder="0=不限"
          className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-center font-mono text-[13px] text-[var(--text-primary)] transition-colors placeholder:text-[var(--text-muted)] focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none"
        />
      </td>
    </tr>
  )
}

function StatTile({ label, value }: { label: string; value: string }): React.ReactElement {
  return (
    <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 text-center">
      <p className="text-[11px] text-[var(--text-muted)]">{label}</p>
      <p className="mt-1 font-mono text-[15px] font-bold text-[var(--text-primary)]">{value}</p>
    </div>
  )
}

function HealthTile({
  label,
  config,
}: {
  label: string
  config: { label: string; color: string; bg: string }
}): React.ReactElement {
  return (
    <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 text-center">
      <p className="text-[11px] text-[var(--text-muted)]">{label}</p>
      <div className="mt-1.5 flex items-center justify-center gap-1.5">
        <div className="h-2 w-2 rounded-full" style={{ backgroundColor: config.color }} />
        <span className="text-[15px] font-bold" style={{ color: config.color }}>
          {config.label}
        </span>
      </div>
    </div>
  )
}

function EvidenceStat({
  label,
  count,
  size,
  color,
}: {
  label: string
  count: number
  size: number
  color: string
}): React.ReactElement {
  return (
    <div className="text-center">
      <div className="mb-2 flex items-center justify-center gap-1.5">
        <div className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: color }} />
        <span className="text-[12px] font-medium text-[var(--text-secondary)]">{label}</span>
      </div>
      <p className="text-lg font-bold text-[var(--text-primary)] tabular-nums">{count}</p>
      <p className="text-[11px] text-[var(--text-muted)]">{size.toFixed(1)} MB</p>
    </div>
  )
}

function RadioOption({
  name,
  checked,
  onChange,
  label,
  description,
}: {
  name: string
  checked: boolean
  onChange: () => void
  label: string
  description: string
}): React.ReactElement {
  return (
    <label
      className={`flex flex-1 cursor-pointer items-start gap-3 rounded-xl border p-4 transition-all ${
        checked
          ? 'border-[var(--accent)]/30 bg-[var(--accent)]/5 ring-2 ring-[var(--accent)]/20'
          : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
      }`}
    >
      <input
        type="radio"
        name={name}
        checked={checked}
        onChange={onChange}
        className="mt-0.5 accent-[var(--accent)]"
      />
      <div>
        <p className="text-[13px] font-medium text-[var(--text-primary)]">{label}</p>
        <p className="mt-0.5 text-[12px] text-[var(--text-muted)]">{description}</p>
      </div>
    </label>
  )
}

function NumberInput({
  label,
  value,
  onChange,
}: {
  label: string
  value: number
  onChange: (v: number) => void
}): React.ReactElement {
  return (
    <div>
      <label className="mb-1.5 block text-[12px] font-medium text-[var(--text-muted)]">
        {label}
      </label>
      <input
        type="number"
        min={0}
        max={100}
        value={value}
        onChange={(e) => onChange(Number(e.target.value) || 0)}
        className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 font-mono text-[13px] text-[var(--text-primary)] transition-colors focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none"
      />
    </div>
  )
}
