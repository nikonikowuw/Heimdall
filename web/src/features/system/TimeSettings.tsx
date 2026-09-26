import { useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import type { LucideIcon } from 'lucide-react'
import { RefreshCw, Clock, Globe, AlertTriangle, Check } from 'lucide-react'
import { systemApi } from '@/lib/system-api'
import { formatTimestamp } from '@/lib/time'
import { RefreshButton } from '@/components/RefreshButton'
import { SystemSettingsHeader } from './components/SystemSettingsHeader'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import type { TimeStatus, TimeConfig } from '@/types/system'

const TIMEZONES = [
  'Asia/Shanghai',
  'Asia/Tokyo',
  'Asia/Seoul',
  'Asia/Singapore',
  'Asia/Kolkata',
  'Europe/London',
  'Europe/Paris',
  'Europe/Berlin',
  'America/New_York',
  'America/Chicago',
  'America/Los_Angeles',
  'UTC',
]

export function TimeSettings(): React.ReactElement {
  const { t, i18n } = useTranslation('system')
  const [status, setStatus] = useState<TimeStatus | null>(null)
  const [config, setConfig] = useState<TimeConfig | null>(null)
  const [draft, setDraft] = useState<TimeConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [saveSuccess, setSaveSuccess] = useState(false)
  const [manualDate, setManualDate] = useState('')
  const [manualTime, setManualTime] = useState('')
  const [showTimeConfirm, setShowTimeConfirm] = useState(false)
  const [syncing, setSyncing] = useState(false)

  // 取数用 `.then/.catch/.finally` 链：async + try/finally 中的 setState 会被
  // set-state-in-effect 保守判为可能同步执行。loading 置位由调用侧事件处理器承担。
  const loadData = useCallback(
    (signal?: AbortSignal): Promise<void> => {
      return Promise.all([systemApi.getTimeStatus(signal), systemApi.getTimeConfig(signal)])
        .then(([s, c]) => {
          if (signal?.aborted) return
          setStatus(s)
          setConfig(c)
          setDraft(c)
          setError(null)

          const now = new Date()
          setManualDate(now.toISOString().split('T')[0])
          setManualTime(now.toTimeString().slice(0, 8))
        })
        .catch((err: unknown) => {
          if (err instanceof DOMException && err.name === 'AbortError') return
          if (!signal?.aborted) {
            setError(
              err instanceof Error
                ? err.message
                : t('loadFailed', { defaultValue: 'Failed to load' }),
            )
          }
        })
        .finally(() => {
          if (!signal?.aborted) setLoading(false)
        })
    },
    [t],
  )

  useEffect(() => {
    const controller = new AbortController()
    void loadData(controller.signal)
    return () => controller.abort()
  }, [loadData])

  const handleSave = async () => {
    if (!draft || !config) return
    try {
      setSaving(true)
      setSaveError(null)
      setSaveSuccess(false)
      const updated = await systemApi.updateTimeConfig(draft)
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

  const handleForceSync = async () => {
    try {
      setSyncing(true)
      setSaveError(null)
      await systemApi.forceTimeSync()
      await loadData()
    } catch (err) {
      setSaveError(
        err instanceof Error
          ? err.message
          : t('time.syncFailed', { defaultValue: 'NTP time sync failed' }),
      )
    } finally {
      setSyncing(false)
    }
  }

  const handleManualTime = async () => {
    setShowTimeConfirm(false)
    try {
      setSaving(true)
      await systemApi.setSystemTime(`${manualDate} ${manualTime}`)
      await loadData()
    } catch (err) {
      setSaveError(
        err instanceof Error ? err.message : t('setFailed', { defaultValue: 'Failed to set' }),
      )
    } finally {
      setSaving(false)
    }
  }

  const isDirty = draft && config && JSON.stringify(draft) !== JSON.stringify(config)

  const ntpServiceLabels: Record<string, string> = {
    chrony: 'chrony',
    timesyncd: 'systemd-timesyncd',
    ntpd: 'ntpd',
    none: t('time.noService', { defaultValue: '无' }),
  }

  return (
    <div className="space-y-5">
      <SystemSettingsHeader
        title={t('time.title', { defaultValue: '对时服务' })}
        description={t('time.subtitle', { defaultValue: '管理系统时钟与 NTP 同步配置' })}
        actions={
          <RefreshButton
            onClick={() => {
              setLoading(true)
              void loadData()
            }}
            loading={loading}
          />
        }
      />

      {error && (
        <ErrorBanner
          message={error}
          onRetry={() => {
            setLoading(true)
            void loadData()
          }}
        />
      )}

      {/* System Time Status */}
      <SettingsSection title={t('time.systemTime', { defaultValue: '系统时间' })}>
        {loading && !status ? (
          <LoadingSkeleton rows={4} />
        ) : status ? (
          <div className="space-y-3">
            {/* Current time hero */}
            <div className="flex items-center gap-4 rounded-xl bg-[var(--bg-secondary)]/60 p-4">
              <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-[var(--accent)]/10">
                <Clock className="h-7 w-7 text-[var(--accent)]" />
              </div>
              <div>
                <p className="text-[12px] text-[var(--text-muted)]">
                  {t('time.currentTime', { defaultValue: '当前时间' })}
                </p>
                <p className="font-mono text-xl font-bold text-[var(--text-primary)]">
                  {formatTimestamp(status.systemTime, i18n.language)}
                </p>
              </div>
            </div>

            {/* Info rows */}
            <div className="grid grid-cols-2 gap-3">
              <InfoCard
                icon={Globe}
                label={t('time.timezone', { defaultValue: '时区' })}
                value={`${status.timezone} (UTC${status.timezoneOffset >= 0 ? '+' : ''}${(status.timezoneOffset / 3600).toFixed(0)})`}
              />
              <InfoCard
                icon={RefreshCw}
                label={t('time.ntpService', { defaultValue: 'NTP 服务' })}
                value={ntpServiceLabels[status.ntpService] || status.ntpService}
              />
              <div className="flex items-center gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5">
                <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--bg-secondary)]">
                  <RefreshCw className="h-5 w-5 text-[var(--text-muted)]" />
                </div>
                <div>
                  <p className="text-[11px] text-[var(--text-muted)]">
                    {t('time.syncStatus', { defaultValue: '同步状态' })}
                  </p>
                  <div className="mt-0.5 flex items-center gap-1.5">
                    <div
                      className={`h-2 w-2 rounded-full ${status.ntpSynced ? 'bg-[var(--status-success)]' : 'bg-[var(--status-warning)]'}`}
                    />
                    <span
                      className={`text-[13px] font-medium ${status.ntpSynced ? 'text-[var(--status-success)]' : 'text-[var(--status-warning)]'}`}
                    >
                      {status.ntpSynced
                        ? t('time.synced', { defaultValue: '已同步' })
                        : t('time.notSynced', { defaultValue: '未同步' })}
                    </span>
                  </div>
                </div>
              </div>
              {status.ntpServer && (
                <InfoCard
                  icon={Globe}
                  label={t('time.syncSource', { defaultValue: '同步源' })}
                  value={status.ntpServer}
                />
              )}
            </div>

            {status.offsetMs !== null && (
              <div className="info-row px-4">
                <span className="text-[13px] text-[var(--text-secondary)]">
                  {t('time.offset', { defaultValue: '时间偏移' })}
                </span>
                <span className="font-mono text-[13px] font-medium text-[var(--text-primary)]">
                  {status.offsetMs > 0 ? '+' : ''}
                  {(status.offsetMs / 1000).toFixed(3)}s
                </span>
              </div>
            )}
          </div>
        ) : null}
      </SettingsSection>

      {/* Time Configuration */}
      <SettingsSection
        title={t('time.timeConfig', { defaultValue: '时间配置' })}
        action={
          <button
            onClick={handleSave}
            disabled={saving || !isDirty}
            className={`flex items-center gap-1.5 rounded-xl px-4 py-2 text-[13px] font-medium text-white transition-all active:scale-[0.97] disabled:opacity-50 ${
              saveSuccess
                ? 'bg-[var(--status-success)] shadow-[var(--status-success)]/20 shadow-lg'
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
        }
      >
        {loading && !draft ? (
          <LoadingSkeleton rows={3} />
        ) : draft ? (
          <div className="space-y-4">
            {/* Timezone selector */}
            <div>
              <label
                htmlFor="time-timezone"
                className="mb-1.5 block text-[13px] font-medium text-[var(--text-primary)]"
              >
                {t('time.timezoneLabel', { defaultValue: '时区' })}
              </label>
              <select
                id="time-timezone"
                value={draft.timezone}
                onChange={(e) => setDraft({ ...draft, timezone: e.target.value })}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 text-[13px] text-[var(--text-primary)] transition-colors focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none"
              >
                {TIMEZONES.map((tz) => (
                  <option key={tz} value={tz}>
                    {tz}
                  </option>
                ))}
              </select>
            </div>

            {/* NTP toggle */}
            <div className="flex items-center justify-between rounded-xl bg-[var(--bg-secondary)]/60 px-4 py-3.5">
              <div>
                <p className="text-[13px] font-medium text-[var(--text-primary)]">
                  {t('time.ntpEnabled', { defaultValue: 'NTP 自动同步' })}
                </p>
                {!draft.ntpEnabled && (
                  <p className="mt-0.5 flex items-center gap-1 text-[12px] text-[var(--status-warning)]">
                    <AlertTriangle className="h-3 w-3" />
                    {t('time.ntpDisabledWarning', { defaultValue: '禁用后系统时间可能漂移' })}
                  </p>
                )}
              </div>
              <button
                onClick={() => setDraft({ ...draft, ntpEnabled: !draft.ntpEnabled })}
                className={`switch-track ${draft.ntpEnabled ? 'on' : ''}`}
                role="switch"
                aria-checked={draft.ntpEnabled}
              >
                <div className="switch-thumb" />
              </button>
            </div>

            {/* NTP server + sync button */}
            <div>
              <label className="mb-1.5 block text-[13px] font-medium text-[var(--text-primary)]">
                {t('time.ntpServer', { defaultValue: 'NTP 服务器' })}
              </label>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={draft.ntpServer}
                  onChange={(e) => setDraft({ ...draft, ntpServer: e.target.value })}
                  placeholder="pool.ntp.org"
                  className="flex-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 font-mono text-[13px] text-[var(--text-primary)] transition-colors placeholder:text-[var(--text-muted)] focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none"
                />
                <button
                  onClick={handleForceSync}
                  disabled={syncing || !draft.ntpEnabled}
                  className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2.5 text-[13px] font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] active:scale-[0.97] disabled:opacity-50"
                >
                  <RefreshCw className={`h-3.5 w-3.5 ${syncing ? 'animate-spin' : ''}`} />
                  {t('time.forceSync', { defaultValue: '立即同步' })}
                </button>
              </div>
            </div>

            {saveError && (
              <div className="rounded-lg border border-[var(--status-danger)]/20 bg-[var(--status-danger)]/5 px-3 py-2 text-[13px] text-[var(--status-danger)]">
                {saveError}
              </div>
            )}
          </div>
        ) : null}
      </SettingsSection>

      {/* Manual Time Set */}
      <SettingsSection
        title={t('time.manualSet', { defaultValue: '手动设置时间' })}
        description={t('time.manualWarning', {
          defaultValue:
            '手动设置系统时间可能导致视频时间戳跳变，影响录像回放连续性。建议优先使用 NTP 自动同步。',
        })}
      >
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1.5 block text-[12px] font-medium text-[var(--text-muted)]">
                {t('time.date', { defaultValue: '日期' })}
              </label>
              <input
                type="date"
                value={manualDate}
                onChange={(e) => setManualDate(e.target.value)}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 text-[13px] text-[var(--text-primary)] transition-colors focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none"
              />
            </div>
            <div>
              <label className="mb-1.5 block text-[12px] font-medium text-[var(--text-muted)]">
                {t('time.time', { defaultValue: '时间' })}
              </label>
              <input
                type="time"
                value={manualTime}
                onChange={(e) => setManualTime(e.target.value)}
                step={1}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 text-[13px] text-[var(--text-primary)] transition-colors focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none"
              />
            </div>
          </div>
          <button
            onClick={() => setShowTimeConfirm(true)}
            disabled={saving || !manualDate || !manualTime}
            className="flex items-center gap-2 rounded-xl border border-[var(--status-warning)]/30 bg-[var(--status-warning)]/10 px-4 py-2.5 text-[13px] font-medium text-[var(--status-warning)] transition-all hover:bg-[var(--status-warning)]/20 active:scale-[0.97] disabled:opacity-50"
          >
            <Clock className="h-3.5 w-3.5" />
            {t('time.setTime', { defaultValue: '立即设置' })}
          </button>
        </div>
      </SettingsSection>

      <ConfirmDialog
        isOpen={showTimeConfirm}
        title={t('time.confirmTitle', { defaultValue: '手动设置系统时间' })}
        description={t('time.confirmMessage', {
          defaultValue:
            '此操作将修改系统时钟，可能导致视频时间戳跳变，影响录像回放连续性和检测日志时间连续性。建议优先使用 NTP 自动同步。',
        })}
        confirmLabel={t('time.confirmSet', { defaultValue: '确认设置' })}
        cancelLabel={t('common:cancel')}
        variant="warning"
        onConfirm={handleManualTime}
        onClose={() => setShowTimeConfirm(false)}
      />
    </div>
  )
}

function InfoCard({
  icon: Icon,
  label,
  value,
}: {
  icon: LucideIcon
  label: string
  value: string
}): React.ReactElement {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5">
      <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--bg-secondary)]">
        <Icon className="h-5 w-5 text-[var(--text-muted)]" />
      </div>
      <div>
        <p className="text-[11px] text-[var(--text-muted)]">{label}</p>
        <p className="mt-0.5 font-mono text-[13px] font-medium text-[var(--text-primary)]">
          {value}
        </p>
      </div>
    </div>
  )
}
