import { useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { KeyRound, User, Calendar, Clock } from 'lucide-react'
import { authApi } from '../../lib/api'
import { formatTimestampShort } from '../../lib/time'
import { RefreshButton } from '../../components/RefreshButton'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import type { AdminUserDto } from '../../types'

interface AccountSecurityProps {
  onOpenPasswordModal?: () => void
}

export function AccountSecurity({ onOpenPasswordModal }: AccountSecurityProps) {
  const { t, i18n } = useTranslation('system')
  const [user, setUser] = useState<AdminUserDto | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const loadUser = useCallback(
    async (signal?: AbortSignal) => {
      try {
        setLoading(true)
        setError(null)
        const me = await authApi.getMe(signal)
        if (!signal?.aborted) setUser(me)
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
    loadUser(controller.signal)
    return () => controller.abort()
  }, [loadUser])

  return (
    <div className="space-y-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-[var(--text-primary)]">
            {t('account.title', { defaultValue: '账号与安全' })}
          </h2>
          <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">
            {t('account.subtitle', { defaultValue: '管理当前管理员账号' })}
          </p>
        </div>
        <RefreshButton onClick={() => loadUser()} loading={loading} />
      </div>

      {error && <ErrorBanner message={error} onRetry={() => loadUser()} />}

      <SettingsSection
        title={t('account.currentAccount', { defaultValue: '当前账号' })}
        action={
          <button
            onClick={onOpenPasswordModal}
            className="flex items-center gap-2 rounded-xl bg-[var(--accent)] px-4 py-2 text-[13px] font-medium text-white shadow-[var(--accent)]/20 shadow-lg transition-all hover:bg-[var(--accent)]/90 active:scale-[0.97]"
          >
            <KeyRound className="h-3.5 w-3.5" />
            {t('account.changePassword', { defaultValue: '修改密码' })}
          </button>
        }
      >
        {loading && !user ? (
          <LoadingSkeleton rows={3} />
        ) : user ? (
          <div className="space-y-3">
            <div className="flex items-center gap-4 rounded-xl bg-[var(--bg-secondary)]/60 p-4">
              <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-[var(--accent)]/10">
                <User className="h-6 w-6 text-[var(--accent)]" />
              </div>
              <div>
                <p className="text-[11px] tracking-wider text-[var(--text-muted)] uppercase">
                  {t('account.username', { defaultValue: '用户名' })}
                </p>
                <p className="mt-0.5 font-mono text-lg font-semibold text-[var(--text-primary)]">
                  {user.username}
                </p>
              </div>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div className="flex items-center gap-3 rounded-xl bg-[var(--bg-secondary)]/60 p-3.5">
                <Calendar className="h-4 w-4 text-[var(--text-muted)]" />
                <div>
                  <p className="text-[11px] text-[var(--text-muted)]">
                    {t('account.createdAt', { defaultValue: '创建时间' })}
                  </p>
                  <p className="mt-0.5 font-mono text-[13px] text-[var(--text-primary)]">
                    {formatTimestampShort(user.createdAt, i18n.language)}
                  </p>
                </div>
              </div>
              <div className="flex items-center gap-3 rounded-xl bg-[var(--bg-secondary)]/60 p-3.5">
                <Clock className="h-4 w-4 text-[var(--text-muted)]" />
                <div>
                  <p className="text-[11px] text-[var(--text-muted)]">
                    {t('account.updatedAt', { defaultValue: '更新时间' })}
                  </p>
                  <p className="mt-0.5 font-mono text-[13px] text-[var(--text-primary)]">
                    {formatTimestampShort(user.updatedAt, i18n.language)}
                  </p>
                </div>
              </div>
            </div>
          </div>
        ) : null}
      </SettingsSection>
    </div>
  )
}
