import React, { useId } from 'react'
import { AlertOctagon, Network, ShieldAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ModalOverlay } from '@/components/ui/ModalOverlay'
import { CloseIconButton } from '@/components/ui/CloseIconButton'

export interface NetworkConflictModalProps {
  open: boolean
  onClose: () => void
  conflictIp: string
  conflictMac?: string | null
}

/**
 * RFC 5227 静态 IP 冲突拦截提示。
 *
 * 外壳、层级与焦点约束由共享 ModalOverlay 承担（原实现声明 `aria-modal` 但无焦点陷阱）。
 */
export function NetworkConflictModal({
  open,
  onClose,
  conflictIp,
  conflictMac,
}: NetworkConflictModalProps): React.ReactElement {
  const { t } = useTranslation('system')
  const titleId = useId()

  return (
    <ModalOverlay
      isOpen={open}
      onClose={onClose}
      ariaLabel={t('network.conflictTitle', { defaultValue: 'RFC 5227 静态 IP 冲突拦截' })}
      ariaLabelledBy={titleId}
      panelClassName="modal-surface--compact"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-3">
          <div className="text-status-danger flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--status-danger)]/15">
            <AlertOctagon className="h-5 w-5" aria-hidden="true" />
          </div>
          <div className="min-w-0">
            <h2 id={titleId} className="text-[16px] font-semibold text-[var(--text-primary)]">
              {t('network.conflictTitle', { defaultValue: 'RFC 5227 静态 IP 冲突拦截' })}
            </h2>
            <p className="text-[12px] text-[var(--text-muted)]">
              {t('network.conflictSubtitle', { defaultValue: '局域网二层地址冲突防护已生效' })}
            </p>
          </div>
        </div>
        <CloseIconButton onClick={onClose} label={t('common:close')} />
      </div>

      <div className="mt-4 space-y-3">
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3.5">
          <div className="flex items-center justify-between text-[13px]">
            <span className="text-[var(--text-secondary)]">
              {t('network.targetIp', { defaultValue: '拟绑定 IP' })}
            </span>
            <span className="text-status-danger font-mono font-bold">{conflictIp}</span>
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

        <div className="space-y-2 rounded-xl bg-[var(--status-danger)]/5 p-3.5 text-[12px] text-[var(--text-secondary)]">
          <div className="text-status-danger flex items-center gap-1.5 font-medium">
            <ShieldAlert className="h-4 w-4" aria-hidden="true" />
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
              <Network className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
              <span>
                {t('network.conflictSolution1', {
                  defaultValue: '排查局域网冲突主机，或选择其他未被分配的静态 IP 地址',
                })}
              </span>
            </div>
            <div className="flex items-center gap-1.5">
              <Network className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
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
          type="button"
          onClick={onClose}
          className="rounded-xl bg-[var(--accent)] px-4 py-2 text-[13px] font-medium text-white shadow-sm transition-all hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-[0.97]"
        >
          {t('network.known', { defaultValue: '我知道了' })}
        </button>
      </div>
    </ModalOverlay>
  )
}
