import { useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { Wifi, WifiOff, Shield, Pencil, Check, X } from 'lucide-react'
import { systemApi } from '../../lib/system-api'
import { RefreshButton } from '../../components/RefreshButton'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import { ConfirmDialog } from './components/ConfirmDialog'
import type { NetworkInterface, IpConfig } from '../../types/system'

export function NetworkSettings(): React.ReactElement {
  const { t } = useTranslation('system')
  const [interfaces, setInterfaces] = useState<NetworkInterface[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [editingIface, setEditingIface] = useState<string | null>(null)
  const [draft, setDraft] = useState<IpConfig | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [showConfirm, setShowConfirm] = useState(false)
  const [pendingIface, setPendingIface] = useState<string | null>(null)

  const loadData = useCallback(async () => {
    try {
      setLoading(true)
      setError(null)
      const result = await systemApi.getNetworkInterfaces()
      setInterfaces(result.interfaces)
    } catch (err) {
      setError(
        err instanceof Error ? err.message : t('loadFailed', { defaultValue: 'Failed to load' }),
      )
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    loadData()
  }, [loadData])

  const startEditing = (iface: NetworkInterface) => {
    setEditingIface(iface.name)
    setDraft(iface.ipv4 || { method: 'dhcp', address: null, prefix: null, gateway: null, dns: [] })
    setSaveError(null)
  }

  const cancelEditing = () => {
    setEditingIface(null)
    setDraft(null)
    setSaveError(null)
  }

  const handleSave = async (iface: NetworkInterface) => {
    if (!draft) return
    if (iface.capabilities.isManagementInterface) {
      setPendingIface(iface.name)
      setShowConfirm(true)
      return
    }
    await executeSave(iface.name, draft)
  }

  const executeSave = async (name: string, config: IpConfig) => {
    try {
      setSaving(true)
      setSaveError(null)
      await systemApi.updateNetworkInterface(name, {
        method: config.method,
        address: config.address || undefined,
        prefix: config.prefix || undefined,
        gateway: config.gateway || undefined,
        dns: config.dns.length > 0 ? config.dns : undefined,
      })
      cancelEditing()
      await loadData()
    } catch (err) {
      setSaveError(
        err instanceof Error ? err.message : t('saveFailed', { defaultValue: 'Failed to save' }),
      )
    } finally {
      setSaving(false)
    }
  }

  const handleConfirmManagement = () => {
    setShowConfirm(false)
    if (pendingIface && draft) executeSave(pendingIface, draft)
  }

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-[var(--text-primary)]">
            {t('network.title', { defaultValue: '网络/服务' })}
          </h2>
          <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">
            {t('network.subtitle', { defaultValue: '管理设备网络接口与 IP 配置' })}
          </p>
        </div>
        <RefreshButton onClick={() => loadData()} loading={loading} />
      </div>

      {error && <ErrorBanner message={error} onRetry={() => loadData()} />}

      <SettingsSection title={t('network.interfaces', { defaultValue: '网卡列表' })}>
        {loading && interfaces.length === 0 ? (
          <LoadingSkeleton rows={3} />
        ) : (
          <div className="space-y-3">
            {interfaces.map((iface) => (
              <NetworkCard
                key={iface.name}
                iface={iface}
                isEditing={editingIface === iface.name}
                draft={editingIface === iface.name ? draft : null}
                saving={saving}
                saveError={editingIface === iface.name ? saveError : null}
                onStartEdit={() => startEditing(iface)}
                onCancelEdit={cancelEditing}
                onSave={() => handleSave(iface)}
                onDraftChange={setDraft}
              />
            ))}
          </div>
        )}
      </SettingsSection>

      <ConfirmDialog
        open={showConfirm}
        title={t('network.confirmTitle', { defaultValue: '修改管理网卡 IP' })}
        message={t('network.confirmMessage', {
          defaultValue:
            '修改管理网卡 IP 将导致当前连接立即中断。请确认新 IP 在本地子网中可达，修改生效后请使用新 IP 重新登录 Web 控制台。',
        })}
        confirmLabel={t('network.confirmApply', { defaultValue: '确认并应用' })}
        variant="warning"
        onConfirm={handleConfirmManagement}
        onCancel={() => {
          setShowConfirm(false)
          setPendingIface(null)
        }}
      />
    </div>
  )
}

function NetworkCard({
  iface,
  isEditing,
  draft,
  saving,
  saveError,
  onStartEdit,
  onCancelEdit,
  onSave,
  onDraftChange,
}: {
  iface: NetworkInterface
  isEditing: boolean
  draft: IpConfig | null
  saving: boolean
  saveError: string | null
  onStartEdit: () => void
  onCancelEdit: () => void
  onSave: () => void
  onDraftChange: (config: IpConfig) => void
}): React.ReactElement {
  const { t } = useTranslation('system')
  const isUp = iface.state === 'up'
  const isMgmt = iface.capabilities.isManagementInterface

  return (
    <div
      className={`rounded-xl border transition-all ${
        isEditing
          ? 'border-[var(--accent)]/30 bg-[var(--accent)]/5'
          : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
      }`}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-5 py-4">
        <div className="flex items-center gap-3.5">
          <div
            className={`flex h-10 w-10 items-center justify-center rounded-xl ${
              isUp ? 'bg-[var(--accent-green)]/10' : 'bg-[var(--bg-secondary)]'
            }`}
          >
            {isUp ? (
              <Wifi className="h-5 w-5 text-[var(--accent-green)]" />
            ) : (
              <WifiOff className="h-5 w-5 text-[var(--text-muted)]" />
            )}
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="font-mono text-[15px] font-semibold text-[var(--text-primary)]">
                {iface.name}
              </span>
              <span className="rounded-md bg-[var(--bg-secondary)] px-2 py-0.5 text-[11px] font-medium text-[var(--text-muted)]">
                {iface.type}
              </span>
              {isMgmt && (
                <span className="flex items-center gap-1 rounded-md bg-[var(--accent-amber)]/10 px-2 py-0.5 text-[11px] font-medium text-[var(--accent-amber)]">
                  <Shield className="h-3 w-3" />
                  {t('network.management', { defaultValue: '管理网卡' })}
                </span>
              )}
            </div>
            <p className="mt-0.5 font-mono text-[12px] text-[var(--text-muted)]">{iface.mac}</p>
          </div>
        </div>

        {!isEditing && iface.capabilities.canModifyIp && (
          <button
            onClick={onStartEdit}
            className="flex items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-[13px] font-medium text-[var(--accent)] transition-all hover:bg-[var(--accent-soft)] active:scale-[0.97]"
          >
            <Pencil className="h-3.5 w-3.5" />
            {t('edit', { defaultValue: '编辑' })}
          </button>
        )}
      </div>

      {/* Read-only display */}
      {iface.ipv4 && !isEditing && (
        <div className="border-t border-[var(--border)]/50 px-5 py-3.5">
          <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-[13px]">
            <div className="flex items-center gap-1.5">
              <span className="text-[var(--text-muted)]">
                {t('network.method', { defaultValue: '方式' })}
              </span>
              <span
                className={`rounded px-1.5 py-0.5 text-[12px] font-medium ${
                  iface.ipv4.method === 'dhcp'
                    ? 'bg-[var(--accent-green)]/10 text-[var(--accent-green)]'
                    : 'bg-[var(--accent)]/10 text-[var(--accent)]'
                }`}
              >
                {iface.ipv4.method.toUpperCase()}
              </span>
            </div>
            {iface.ipv4.address && (
              <div className="flex items-center gap-1.5">
                <span className="text-[var(--text-muted)]">IP</span>
                <span className="font-mono font-medium text-[var(--text-primary)]">
                  {iface.ipv4.address}/{iface.ipv4.prefix}
                </span>
              </div>
            )}
            {iface.ipv4.gateway && (
              <div className="flex items-center gap-1.5">
                <span className="text-[var(--text-muted)]">
                  {t('network.gateway', { defaultValue: '网关' })}
                </span>
                <span className="font-mono text-[var(--text-primary)]">{iface.ipv4.gateway}</span>
              </div>
            )}
            {iface.ipv4.dns.length > 0 && (
              <div className="flex items-center gap-1.5">
                <span className="text-[var(--text-muted)]">DNS</span>
                <span className="font-mono text-[var(--text-primary)]">
                  {iface.ipv4.dns.join(', ')}
                </span>
              </div>
            )}
          </div>
        </div>
      )}

      {!iface.capabilities.canModifyIp && !isEditing && iface.capabilities.reason && (
        <div className="border-t border-[var(--border)]/50 bg-[var(--bg-secondary)]/30 px-5 py-3 text-[13px] text-[var(--text-muted)]">
          {iface.capabilities.reason}
        </div>
      )}

      {/* Edit form */}
      {isEditing && draft && (
        <div className="space-y-4 border-t border-[var(--accent)]/20 px-5 py-4">
          {/* Method toggle */}
          <div className="flex gap-1 rounded-xl bg-[var(--bg-secondary)] p-1">
            <button
              onClick={() => onDraftChange({ ...draft, method: 'dhcp' })}
              className={`flex-1 rounded-lg px-4 py-2 text-[13px] font-medium transition-all ${
                draft.method === 'dhcp'
                  ? 'bg-[var(--accent)] text-white shadow-md'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              DHCP
            </button>
            <button
              onClick={() => onDraftChange({ ...draft, method: 'static' })}
              className={`flex-1 rounded-lg px-4 py-2 text-[13px] font-medium transition-all ${
                draft.method === 'static'
                  ? 'bg-[var(--accent)] text-white shadow-md'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              {t('network.static', { defaultValue: '静态' })}
            </button>
          </div>

          {draft.method === 'static' && (
            <div className="space-y-3">
              <div className="grid grid-cols-[1fr_72px] gap-3">
                <FieldInput
                  label={t('network.ipAddress', { defaultValue: 'IP 地址' })}
                  value={draft.address || ''}
                  onChange={(v) => onDraftChange({ ...draft, address: v })}
                  placeholder="192.168.1.100"
                  mono
                />
                <FieldInput
                  label={t('network.prefix', { defaultValue: '前缀' })}
                  type="number"
                  min={1}
                  max={32}
                  value={draft.prefix || ''}
                  onChange={(v) => onDraftChange({ ...draft, prefix: Number(v) || null })}
                  placeholder="24"
                  mono
                />
              </div>
              <FieldInput
                label={t('network.gateway', { defaultValue: '网关' })}
                value={draft.gateway || ''}
                onChange={(v) => onDraftChange({ ...draft, gateway: v })}
                placeholder="192.168.1.1"
                mono
              />
              <FieldInput
                label="DNS"
                value={draft.dns.join(', ')}
                onChange={(v) =>
                  onDraftChange({
                    ...draft,
                    dns: v
                      .split(',')
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
                placeholder="8.8.8.8, 8.8.4.4"
                mono
              />
            </div>
          )}

          {saveError && (
            <div className="rounded-lg border border-[var(--destructive)]/20 bg-[var(--destructive)]/5 px-3 py-2 text-[13px] text-[var(--destructive)]">
              {saveError}
            </div>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <button
              onClick={onCancelEdit}
              disabled={saving}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2 text-[13px] font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--bg-secondary)] active:scale-[0.97]"
            >
              <X className="h-3.5 w-3.5" />
              {t('cancel', { defaultValue: '取消' })}
            </button>
            <button
              onClick={onSave}
              disabled={saving}
              className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-[13px] font-medium text-white shadow-[var(--accent)]/20 shadow-lg transition-all hover:bg-[var(--accent)]/90 active:scale-[0.97] disabled:opacity-50"
            >
              {saving ? (
                <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-white border-t-transparent" />
              ) : (
                <Check className="h-3.5 w-3.5" />
              )}
              {t('save', { defaultValue: '保存' })}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

function FieldInput({
  label,
  value,
  onChange,
  placeholder,
  type = 'text',
  min,
  max,
  mono = false,
}: {
  label: string
  value: string | number
  onChange: (v: string) => void
  placeholder?: string
  type?: string
  min?: number
  max?: number
  mono?: boolean
}): React.ReactElement {
  return (
    <div>
      <label className="mb-1.5 block text-[12px] font-medium text-[var(--text-muted)]">
        {label}
      </label>
      <input
        type={type}
        min={min}
        max={max}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className={`w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-[13px] text-[var(--text-primary)] transition-colors placeholder:text-[var(--text-muted)] focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/20 focus:outline-none ${mono ? 'font-mono' : ''}`}
      />
    </div>
  )
}
