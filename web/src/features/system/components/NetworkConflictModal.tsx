import { useTranslation } from 'react-i18next'
import { AlertOctagon, Network, ShieldAlert, X } from 'lucide-react'

interface NetworkConflictModalProps {
  open: boolean
  onClose: () => void
  conflictIp: string
  conflictMac?: string | null
}

export function NetworkConflictModal({
  open,
  onClose,
  conflictIp,
  conflictMac,
}: NetworkConflictModalProps): React.ReactElement | null {
  const { t } = useTranslation('system')

  if (!open) return null

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      {/* 遮罩 */}
      <div
        className="fixed inset-0 bg-black/60 backdrop-blur-sm transition-opacity"
        onClick={onClose}
      />

      {/* 弹窗实体 */}
      <div className="relative w-full max-w-md overflow-hidden rounded-2xl border border-[var(--accent-red)]/30 bg-[var(--bg-surface)] p-6 shadow-2xl transition-all">
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--accent-red)]/15 text-[var(--accent-red)]">
              <AlertOctagon className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-[16px] font-semibold text-[var(--text-primary)]">
                {t('network.conflictTitle', { defaultValue: 'RFC 5227 静态 IP 冲突拦截' })}
              </h3>
              <p className="text-[12px] text-[var(--text-muted)]">
                {t('network.conflictSubtitle', { defaultValue: '局域网二层地址冲突防护已生效' })}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1 text-[var(--text-muted)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="mt-4 space-y-3">
          <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3.5">
            <div className="flex items-center justify-between text-[13px]">
              <span className="text-[var(--text-secondary)]">
                {t('network.targetIp', { defaultValue: '拟绑定 IP' })}
              </span>
              <span className="font-mono font-bold text-[var(--accent-red)]">{conflictIp}</span>
            </div>
            {conflictMac && (
              <div className="mt-2 flex items-center justify-between text-[13px]">
                <span className="text-[var(--text-secondary)]">
                  {t('network.occupyingMac', { defaultValue: '冲突主机 MAC' })}
                </span>
                <span className="font-mono font-semibold text-[var(--text-primary)]">
                  {conflictMac}
                </span>
              </div>
            )}
          </div>

          <div className="space-y-2 rounded-xl bg-[var(--accent-red)]/5 p-3.5 text-[12px] text-[var(--text-secondary)]">
            <div className="flex items-center gap-1.5 font-medium text-[var(--accent-red)]">
              <ShieldAlert className="h-4 w-4" />
              <span>{t('network.conflictWarning', { defaultValue: '工业安全拦截说明' })}</span>
            </div>
            <p>
              {t('network.conflictDesc', {
                defaultValue:
                  '系统在配置下发前通过 ARP Probe 探测到局域网中已有活跃主机占用该 IP。为防范广播风暴及设备通信瘫痪，系统已自动阻断应用。',
              })}
            </p>
            <div className="mt-2 space-y-1 text-[11px] text-[var(--text-muted)]">
              <div className="flex items-center gap-1.5">
                <Network className="h-3.5 w-3.5 shrink-0" />
                <span>
                  {t('network.conflictSolution1', {
                    defaultValue: '排查局域网冲突主机，或选择其他未被分配的静态 IP 地址',
                  })}
                </span>
              </div>
              <div className="flex items-center gap-1.5">
                <Network className="h-3.5 w-3.5 shrink-0" />
                <span>
                  {t('network.conflictSolution2', {
                    defaultValue: '或者将网卡切换为 DHCP 自动获取模式，由路由器分配可用 IP',
                  })}
                </span>
              </div>
            </div>
          </div>
        </div>

        <div className="mt-6 flex justify-end">
          <button
            onClick={onClose}
            className="rounded-lg bg-[var(--accent)] px-4 py-2 text-[13px] font-medium text-white shadow-sm transition-all hover:bg-[var(--accent)]/90 active:scale-[0.97]"
          >
            {t('network.known', { defaultValue: '我知道了' })}
          </button>
        </div>
      </div>
    </div>
  )
}
