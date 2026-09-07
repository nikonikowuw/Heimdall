import { useState, useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { AlertTriangle, CheckCircle2, RotateCcw, ShieldAlert } from 'lucide-react'
import type { NetworkChangeOperation } from '../../../types/system'

interface NetworkTrialBannerProps {
  operation: NetworkChangeOperation
  onConfirm: (id: string) => Promise<void>
  onCancel: (id: string) => Promise<void>
}

function getRemainingSec(deadlineMs: number): number {
  return Math.max(0, Math.floor((deadlineMs - Date.now()) / 1000))
}

export function NetworkTrialBanner({
  operation,
  onConfirm,
  onCancel,
}: NetworkTrialBannerProps): React.ReactElement {
  const { t } = useTranslation('system')
  const [remainingSec, setRemainingSec] = useState<number>(() =>
    getRemainingSec(operation.confirmDeadlineMs),
  )
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    setRemainingSec(getRemainingSec(operation.confirmDeadlineMs))
    const timer = setInterval(() => {
      const diff = getRemainingSec(operation.confirmDeadlineMs)
      setRemainingSec(diff)
      if (diff <= 0) {
        clearInterval(timer)
      }
    }, 1000)
    return () => clearInterval(timer)
  }, [operation.confirmDeadlineMs])

  const handleConfirm = async () => {
    try {
      setBusy(true)
      await onConfirm(operation.id)
    } finally {
      setBusy(false)
    }
  }

  const handleCancel = async () => {
    try {
      setBusy(true)
      await onCancel(operation.id)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="relative overflow-hidden rounded-xl border border-[var(--accent-amber)]/40 bg-[var(--accent-amber)]/10 p-4.5 shadow-md">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-amber)]/20 text-[var(--accent-amber)]">
            <ShieldAlert className="h-5 w-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-[14px] font-semibold text-[var(--text-primary)]">
                {t('network.trialActiveTitle', { defaultValue: '网络变更试运行保护中' })}
              </h3>
              <span className="flex items-center gap-1 rounded-full bg-[var(--accent-amber)]/20 px-2 py-0.5 font-mono text-[12px] font-bold text-[var(--accent-amber)]">
                <AlertTriangle className="h-3 w-3" />
                {remainingSec}s {t('network.autoRollback', { defaultValue: '后自动回滚' })}
              </span>
            </div>
            <p className="mt-1 text-[12px] text-[var(--text-secondary)]">
              {t('network.trialDesc', {
                defaultValue:
                  '网卡 {{name}} 正在进行新配置试运行。请确认新 IP 在局域网内可达。若未在倒计时结束前确认，系统独立看门狗将无条件自动回滚至原配置。',
                name: operation.interfaceName,
              })}
            </p>
            {operation.newConfig.address && (
              <p className="mt-0.5 font-mono text-[12px] text-[var(--text-muted)]">
                {t('network.newIp', { defaultValue: '目标 IP' })}: {operation.newConfig.address}
                {operation.newAccessUrl && (
                  <span className="ml-2">
                    ({t('network.accessUrl', { defaultValue: '访问地址' })}:{' '}
                    <a
                      href={operation.newAccessUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="text-[var(--accent)] underline hover:opacity-80"
                    >
                      {operation.newAccessUrl}
                    </a>
                    )
                  </span>
                )}
              </p>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 self-end sm:self-center">
          <button
            onClick={handleCancel}
            disabled={busy || remainingSec <= 0}
            className="flex items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-1.5 text-[12px] font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--bg-secondary)] active:scale-[0.97] disabled:opacity-50"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            {t('network.cancelRollback', { defaultValue: '立即放弃回滚' })}
          </button>
          <button
            onClick={handleConfirm}
            disabled={busy || remainingSec <= 0}
            className="flex items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3.5 py-1.5 text-[12px] font-medium text-white shadow-sm transition-all hover:bg-[var(--accent)]/90 active:scale-[0.97] disabled:opacity-50"
          >
            <CheckCircle2 className="h-3.5 w-3.5" />
            {t('network.confirmPermanent', { defaultValue: '确认永久生效' })}
          </button>
        </div>
      </div>
    </div>
  )
}
