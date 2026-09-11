import { useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { Wifi, WifiOff, Shield, Pencil, Check, X, Cable, Network, Globe, Radio } from 'lucide-react'
import { systemApi } from '../../lib/system-api'
import { RefreshButton } from '../../components/RefreshButton'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import { ConfirmDialog } from './components/ConfirmDialog'
import { NetworkTrialBanner } from './components/NetworkTrialBanner'
import { NetworkConflictModal } from './components/NetworkConflictModal'
import { useNetworkTrial } from './hooks/use-network-trial'
import type { NetworkInterface, IpConfig } from '../../types/system'

const DEFAULT_IPV4_DRAFT: IpConfig = {
  method: 'dhcp',
  address: null,
  prefix: null,
  gateway: null,
  dns: [],
  metric: null,
}

function isVirtualInterface(iface: NetworkInterface): boolean {
  if (iface.type === 'virtual' || iface.type === 'loopback') {
    return true
  }
  const lower = iface.name.toLowerCase()
  return (
    lower === 'lo' ||
    lower === 'lo0' ||
    lower.startsWith('docker') ||
    lower.startsWith('br-') ||
    lower.startsWith('veth') ||
    lower.startsWith('virbr') ||
    lower.startsWith('vmnet') ||
    lower.startsWith('tun') ||
    lower.startsWith('tap') ||
    lower.startsWith('wg') ||
    lower.startsWith('tailscale') ||
    lower.startsWith('zt') ||
    lower.startsWith('dummy') ||
    lower.startsWith('p2p-dev-')
  )
}

function isCurrentAccessIface(iface: NetworkInterface): boolean {
  if (typeof window === 'undefined') return false
  const host = window.location.hostname
  if (iface.ipv4?.address && iface.ipv4.address === host) {
    return true
  }
  if (
    (host === 'localhost' || host === '127.0.0.1' || host === '::1') &&
    iface.capabilities.isManagementInterface
  ) {
    return true
  }
  return false
}

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

  // 冲突告警弹窗状态
  const [conflictModalOpen, setConflictModalOpen] = useState(false)
  const [conflictIp, setConflictIp] = useState('')
  const [conflictMac, setConflictMac] = useState<string | null>(null)

  const { pendingOp, syncPendingOp, trialError, confirmTrial, cancelTrial } = useNetworkTrial({
    onReload: () => loadData(),
  })

  const loadData = useCallback(async () => {
    try {
      setLoading(true)
      setError(null)
      const result = await systemApi.getNetworkInterfaces()
      const physicalOnly = result.interfaces.filter((i) => !isVirtualInterface(i))
      setInterfaces(physicalOnly)
      syncPendingOp(result.pendingOperation)
    } catch (err) {
      setError(
        err instanceof Error ? err.message : t('loadFailed', { defaultValue: 'Failed to load' }),
      )
    } finally {
      setLoading(false)
    }
  }, [syncPendingOp, t])

  useEffect(() => {
    loadData()
  }, [loadData])

  const startEditing = (iface: NetworkInterface) => {
    setEditingIface(iface.name)
    setDraft(iface.ipv4 || DEFAULT_IPV4_DRAFT)
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
      const res = await systemApi.updateNetworkInterface(name, {
        method: config.method,
        address: config.address || undefined,
        prefix: config.prefix || undefined,
        gateway: config.gateway || undefined,
        dns: config.dns.length > 0 ? config.dns : undefined,
        metric: config.metric ?? undefined,
      })
      cancelEditing()
      if (res.operation) {
        syncPendingOp(res.operation)
      }
      await loadData()
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : t('saveFailed', { defaultValue: 'Failed to save' })
      if (msg.includes('占用') || msg.includes('冲突') || msg.includes('51011')) {
        const macMatch = msg.match(/\[?([0-9A-Fa-f]{2}(?:[:-][0-9A-Fa-f]{2}){5})\]?/)
        setConflictIp(config.address || '')
        setConflictMac(macMatch ? macMatch[1] : null)
        setConflictModalOpen(true)
      }
      setSaveError(msg)
    } finally {
      setSaving(false)
    }
  }

  const handleConfirmManagement = () => {
    setShowConfirm(false)
    if (pendingIface && draft) executeSave(pendingIface, draft)
  }

  const currentAccessIface =
    interfaces.find(isCurrentAccessIface) ||
    interfaces.find((i) => i.capabilities.isManagementInterface) ||
    interfaces[0]

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-[var(--text-primary)]">
            {t('network.title', { defaultValue: '网络/服务' })}
          </h2>
          <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">
            {t('network.subtitle', { defaultValue: '管理工业边缘网络接口与 IP 配置' })}
          </p>
        </div>
        <RefreshButton onClick={() => loadData()} loading={loading} />
      </div>

      {/* 试运行防失联全屏横幅 */}
      {pendingOp && pendingOp.status === 'pending_confirm' && (
        <NetworkTrialBanner operation={pendingOp} onConfirm={confirmTrial} onCancel={cancelTrial} />
      )}

      {(error || trialError) && (
        <ErrorBanner message={error || trialError!} onRetry={() => loadData()} />
      )}

      {/* 当前连接与主网卡全局概览 */}
      {currentAccessIface && (
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-4 shadow-sm">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex items-center gap-3.5">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-green)]/10 text-[var(--accent-green)]">
                <Radio className="h-5 w-5 animate-pulse" />
              </div>
              <div>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs font-medium text-[var(--text-muted)]">
                    {t('network.currentActiveCard', { defaultValue: '当前活动网卡' })}
                  </span>
                  <span className="font-mono text-[14px] font-semibold text-[var(--text-primary)]">
                    {currentAccessIface.name}
                  </span>
                  <span className="inline-flex items-center gap-1 rounded-full bg-[var(--accent-green)]/15 px-2 py-0.5 text-[11px] font-medium text-[var(--accent-green)]">
                    <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent-green)]" />
                    {t('network.online', { defaultValue: '在线通信中' })}
                  </span>
                </div>
                <div className="mt-1 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-[var(--text-muted)]">
                  <span>
                    IP:{' '}
                    <strong className="font-mono font-medium text-[var(--text-primary)]">
                      {currentAccessIface.ipv4?.address ||
                        t('network.notAssigned', { defaultValue: '未分配' })}
                    </strong>
                    {currentAccessIface.ipv4?.prefix ? `/${currentAccessIface.ipv4.prefix}` : ''}
                  </span>
                  {currentAccessIface.ipv4?.gateway && (
                    <span>
                      {t('network.gateway', { defaultValue: '网关' })}:{' '}
                      <span className="font-mono text-[var(--text-secondary)]">
                        {currentAccessIface.ipv4.gateway}
                      </span>
                    </span>
                  )}
                  {currentAccessIface.ipv4?.dns && currentAccessIface.ipv4.dns.length > 0 && (
                    <span>
                      DNS:{' '}
                      <span className="font-mono text-[var(--text-secondary)]">
                        {currentAccessIface.ipv4.dns[0]}
                      </span>
                    </span>
                  )}
                  <span>
                    MAC:{' '}
                    <span className="font-mono text-[var(--text-secondary)]">
                      {currentAccessIface.mac}
                    </span>
                  </span>
                </div>
              </div>
            </div>

            <div className="flex items-center gap-2 rounded-lg bg-[var(--bg-secondary)] px-3 py-1.5 text-xs text-[var(--text-muted)]">
              <Globe className="h-4 w-4 text-[var(--accent)]" />
              <span>
                {t('network.currentAccess', { defaultValue: '当前访问口' })}:{' '}
                <span className="font-mono font-medium text-[var(--text-primary)]">
                  {typeof window !== 'undefined'
                    ? `${window.location.protocol}//${window.location.host}`
                    : ''}
                </span>
              </span>
            </div>
          </div>
        </div>
      )}

      <SettingsSection title={t('network.interfaces', { defaultValue: '网卡列表' })}>
        {loading && interfaces.length === 0 ? (
          <LoadingSkeleton rows={3} />
        ) : interfaces.length === 0 ? (
          <div className="rounded-xl border border-dashed border-[var(--border)] py-8 text-center text-sm text-[var(--text-muted)]">
            {t('network.noInterfaces', { defaultValue: '未检测到物理以太网卡' })}
          </div>
        ) : (
          <div className="space-y-3">
            {interfaces.map((iface) => (
              <NetworkCard
                key={iface.name}
                iface={iface}
                isCurrentAccess={isCurrentAccessIface(iface)}
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
        title={t('network.confirmTitle', { defaultValue: '修改管理网卡 IP（开启试运行安全保护）' })}
        message={t('network.confirmMessage', {
          defaultValue:
            '您正在修改承载 Web 控制台的管理网卡。系统将开启 60 秒试运行模式并启动看门狗保护。若修改后无法连接新 IP，系统将在倒计时结束后无条件自动回滚至原配置，杜绝设备失联。',
        })}
        confirmLabel={t('network.confirmApply', { defaultValue: '开启试运行并应用' })}
        variant="warning"
        onConfirm={handleConfirmManagement}
        onCancel={() => {
          setShowConfirm(false)
          setPendingIface(null)
        }}
      />

      <NetworkConflictModal
        open={conflictModalOpen}
        onClose={() => setConflictModalOpen(false)}
        conflictIp={conflictIp}
        conflictMac={conflictMac}
      />
    </div>
  )
}

interface NetworkCardProps {
  iface: NetworkInterface
  isCurrentAccess?: boolean
  isEditing: boolean
  draft: IpConfig | null
  saving: boolean
  saveError: string | null
  onStartEdit: () => void
  onCancelEdit: () => void
  onSave: () => void
  onDraftChange: (config: IpConfig) => void
}

function NetworkCard({
  iface,
  isCurrentAccess = false,
  isEditing,
  draft,
  saving,
  saveError,
  onStartEdit,
  onCancelEdit,
  onSave,
  onDraftChange,
}: NetworkCardProps): React.ReactElement {
  const { t } = useTranslation('system')
  const isUp = iface.state === 'up'
  const isMgmt = iface.capabilities.isManagementInterface

  return (
    <div
      className={`rounded-xl border transition-all ${
        isEditing
          ? 'border-[var(--accent)]/30 bg-[var(--accent)]/5'
          : isCurrentAccess
            ? 'border-[var(--accent-green)]/40 bg-[var(--bg-surface)] hover:border-[var(--accent-green)]/60'
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
            {iface.type === 'wifi' ? (
              isUp ? (
                <Wifi className="h-5 w-5 text-[var(--accent-green)]" />
              ) : (
                <WifiOff className="h-5 w-5 text-[var(--text-muted)]" />
              )
            ) : isUp ? (
              <Network className="h-5 w-5 text-[var(--accent-green)]" />
            ) : (
              <Network className="h-5 w-5 text-[var(--text-muted)] opacity-50" />
            )}
          </div>
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono text-[15px] font-semibold text-[var(--text-primary)]">
                {iface.name}
              </span>
              <span className="rounded-md bg-[var(--bg-secondary)] px-2 py-0.5 text-[11px] font-medium text-[var(--text-muted)]">
                {iface.type}
              </span>
              {isCurrentAccess && (
                <span className="flex items-center gap-1 rounded-md bg-[var(--accent-green)]/15 px-2 py-0.5 text-[11px] font-medium text-[var(--accent-green)]">
                  <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent-green)]" />
                  {t('network.currentAccess', { defaultValue: '当前访问口' })}
                </span>
              )}
              {isMgmt && (
                <span className="flex items-center gap-1 rounded-md bg-[var(--accent-amber)]/10 px-2 py-0.5 text-[11px] font-medium text-[var(--accent-amber)]">
                  <Shield className="h-3 w-3" />
                  {t('network.management', { defaultValue: '管理网卡' })}
                </span>
              )}
              {/* 物理载波状态 */}
              {iface.carrier !== undefined && iface.carrier !== null && (
                <span
                  className={`flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium ${
                    iface.carrier
                      ? 'bg-[var(--accent-green)]/10 text-[var(--accent-green)]'
                      : 'bg-[var(--text-muted)]/10 text-[var(--text-muted)]'
                  }`}
                >
                  {iface.type === 'wifi' ? (
                    <Wifi className="h-3 w-3" />
                  ) : (
                    <Cable className="h-3 w-3" />
                  )}
                  {iface.type === 'wifi'
                    ? iface.carrier
                      ? t('network.wifiConnected', { defaultValue: '无线已连' })
                      : t('network.wifiDisconnected', { defaultValue: '无线未连' })
                    : iface.carrier
                      ? t('network.carrierUp', { defaultValue: '网线已插' })
                      : t('network.carrierDown', { defaultValue: '网线未插' })}
                </span>
              )}
              {/* 协商速率 */}
              {iface.speed && (
                <span className="rounded-md bg-[var(--bg-secondary)] px-2 py-0.5 font-mono text-[11px] text-[var(--text-muted)]">
                  {iface.speed} Mbps {iface.duplex ? `(${iface.duplex})` : ''}
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
            {iface.ipv4.metric !== undefined && iface.ipv4.metric !== null && (
              <div className="flex items-center gap-1.5">
                <span className="text-[var(--text-muted)]">Metric</span>
                <span className="font-mono text-[var(--text-primary)]">{iface.ipv4.metric}</span>
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

      {/* Unconfigured / Offline interface hint */}
      {!iface.ipv4 && !isEditing && (
        <div className="border-t border-[var(--border)]/50 px-5 py-3 text-[13px] text-[var(--text-muted)]">
          {t('network.unconfigured', {
            defaultValue: '未配置 IP（可点击右上角“编辑”预设静态 IP）',
          })}
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
              <div className="grid grid-cols-[1fr_100px] gap-3">
                <FieldInput
                  label={t('network.gateway', { defaultValue: '网关 (从网卡可留空)' })}
                  value={draft.gateway || ''}
                  onChange={(v) => onDraftChange({ ...draft, gateway: v })}
                  placeholder="192.168.1.1"
                  mono
                />
                <FieldInput
                  label="Metric"
                  type="number"
                  value={draft.metric ?? ''}
                  onChange={(v) =>
                    onDraftChange({
                      ...draft,
                      metric: v === '' ? null : Number(v),
                    })
                  }
                  placeholder={isMgmt ? '100' : '500'}
                  mono
                />
              </div>
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
              {t('save', { defaultValue: '保存并应用' })}
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
